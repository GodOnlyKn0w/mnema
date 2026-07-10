use crate::event::Event;
use crate::graph;
/// Query-command family: cmd_list, cmd_show, cmd_search, cmd_timeline,
/// cmd_orient, cmd_tree (+ print_tree_text helper).
/// Moved from main.rs (Layer 4b refactor); function bodies are byte-identical to
/// the originals (only cross-module path qualification added where required).
///
/// Dependency direction: query -> journal, projection, render, tree, output, event (via crate::*)
/// query <- main.rs (mod commands; pub(crate) use commands::query::*)
use crate::journal::*;
use crate::markers::leading_marker;
use crate::output;
use crate::projection::{self, TimelineEventKind};
use crate::render::*;
use crate::tree;
use crate::util::{
    display_ts, humanize_duration, parse_duration, read_stdin_content, read_stdin_if_piped,
    shorten, truncate,
};
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;

fn corrupted_lines_error(skipped: usize) -> String {
    format!(
        "corrupt: [mnema] WARNING: {} corrupted lines skipped",
        skipped
    )
}

pub(crate) struct ListRequest<'a> {
    pub(crate) include_hidden: bool,
    pub(crate) links: Option<&'a str>,
    pub(crate) backlinks: Option<&'a str>,
    pub(crate) state: Option<&'a str>,
    pub(crate) list_type: Option<&'a str>,
    pub(crate) stale: Option<&'a str>,
    pub(crate) stale_offset: Option<usize>,
    pub(crate) since_offset: Option<usize>,
    /// Raw `--under` root (prefix ok); None = JournalScope.
    pub(crate) under: Option<&'a str>,
    pub(crate) allow_selection: bool,
}

/// Resolve optional `--under X` / `orient --id X` into a collection Scope.
/// Uses the same strand shorthand rules as other read commands.
pub(crate) fn scope_from_under(
    under: Option<&str>,
    strands: &[projection::ProjectedStrand],
    allow_selection: bool,
    current_max_offset: usize,
) -> Result<crate::scope::Scope, String> {
    match under {
        None => Ok(crate::scope::Scope::journal()),
        Some(root) => {
            let full = crate::reference::resolve_strand_with_selection(
                strands,
                root,
                allow_selection,
                current_max_offset,
            )?;
            Ok(crate::scope::Scope::subtree(full))
        }
    }
}

pub(crate) fn list_strands(
    events: &[(usize, Event)],
    req: &ListRequest<'_>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<projection::ProjectedStrand>, String> {
    let canonical_strands = projection::project_strands(events, true);
    let current_max_offset = events.last().map(|(offset, _)| *offset).unwrap_or(0);
    let links_id = req
        .links
        .map(|src| {
            crate::reference::resolve_strand_with_selection(
                &canonical_strands,
                src,
                req.allow_selection,
                current_max_offset,
            )
        })
        .transpose()?;
    let backlinks_id = req
        .backlinks
        .map(|tgt| {
            crate::reference::resolve_strand_with_selection(
                &canonical_strands,
                tgt,
                req.allow_selection,
                current_max_offset,
            )
        })
        .transpose()?;
    let scope = scope_from_under(
        req.under,
        &canonical_strands,
        req.allow_selection,
        current_max_offset,
    )?;
    let mut strands = projection::project_strands(events, req.include_hidden);
    // Scope changes only the candidate set; all other filters keep the same
    // field semantics (CORPUS §7.1).
    scope.retain_strands(&mut strands, &canonical_strands)?;
    strands.sort_by(|a, b| b.last_ts().cmp(a.last_ts()));

    if let Some(type_filter) = req.list_type {
        strands.retain(|n| n.strand_type.as_deref() == Some(type_filter));
    }
    if let Some(src) = links_id {
        let source_edges: Vec<String> = canonical_strands
            .iter()
            .filter(|n| n.id == src)
            .flat_map(|n| n.edges.iter().cloned())
            .collect();
        strands.retain(|n| source_edges.iter().any(|e| n.id == *e));
    }
    if let Some(tgt) = backlinks_id {
        strands.retain(|n| n.edges.iter().any(|e| e == &tgt));
    }
    if let Some(state_filter) = req.state {
        strands.retain(|n| match state_filter {
            "open" => n.state() == "registered",
            "closed" => n.state().starts_with("closed:"),
            s if projection::CLOSE_DISPOSITIONS.contains(&s) => {
                n.state() == format!("closed:{}", s)
            }
            _ => n.state() == state_filter,
        });
    }
    if let Some(dur_str) = req.stale {
        // Stale = handoff candidates: active (registered) and silent past threshold.
        // Closed lines are noise here (last event is often the close itself).
        let secs = parse_duration(dur_str)?;
        let cutoff = now - chrono::Duration::seconds(secs as i64);
        let cutoff_str = cutoff.to_rfc3339();
        strands.retain(|n| {
            n.state() == "registered" && {
                let last_ts = n.last_ts();
                !last_ts.is_empty() && last_ts < &cutoff_str
            }
        });
    }
    if let Some(offset) = req.stale_offset {
        // Same handoff intent as --stale: only registered lines.
        strands.retain(|n| n.state() == "registered" && n.last_offset() <= offset);
    }
    if let Some(offset) = req.since_offset {
        strands.retain(|n| n.last_offset() > offset);
    }
    Ok(strands)
}

pub(crate) fn cmd_list(
    include_hidden: bool,
    links: Option<&str>,
    backlinks: Option<&str>,
    state: Option<&str>,
    list_type: Option<&str>,
    stale: Option<&str>,
    stale_offset: Option<usize>,
    since_offset: Option<usize>,
    under: Option<&str>,
    format_json: bool,
) -> Result<(), String> {
    let started = Instant::now();
    let path = ensure_journal()?;
    let (events, skipped) = read_events_lossy(&path);
    let request = ListRequest {
        include_hidden,
        links,
        backlinks,
        state,
        list_type,
        stale,
        stale_offset,
        since_offset,
        under,
        allow_selection: !format_json,
    };
    let strands = list_strands(&events, &request, chrono::Utc::now())?;

    if format_json {
        let output = output::StrandListOutput {
            strands: strands
                .iter()
                .filter(|s| !s.hidden || include_hidden)
                .map(output::StrandListItem::from)
                .collect(),
        };
        println!("{}", serde_json::to_string(&output).expect("serialize"));
        if skipped > 0 {
            return Err(corrupted_lines_error(skipped));
        }
        eprintln!("[mnema] list: {:.0?}", started.elapsed());
        return Ok(());
    }

    for strand in &strands {
        if strand.hidden && !include_hidden {
            continue;
        }
        let type_str = strand.strand_type.as_deref().unwrap_or("");
        let type_info = if type_str.is_empty() {
            String::new()
        } else {
            format!(" [{}]", type_str)
        };
        println!(
            "{}  {}  \"{}\"  →  \"{}\"{}",
            shorten(&strand.id),
            strand.log_count(),
            truncate(strand.first_summary(), 40),
            truncate(strand.last_summary(), 40),
            type_info,
        );
    }
    if strands.is_empty() {
        println!("(no strands)");
    }
    let max_offset = events.last().map(|(offset, _)| *offset).unwrap_or(0);
    if let Err(e) = crate::reference::remember_list(&strands, max_offset) {
        eprintln!("[mnema] warning: {}", e);
    }
    if skipped > 0 {
        return Err(corrupted_lines_error(skipped));
    }
    eprintln!("[mnema] list: {:.0?}", started.elapsed());
    Ok(())
}

pub(crate) struct SearchRequest<'a> {
    pub(crate) query: &'a str,
    pub(crate) include_hidden: bool,
    /// Bare marker name without brackets (e.g. "friction"); None = unrestricted.
    pub(crate) marker: Option<&'a str>,
    /// Raw `--under` root (prefix ok); None = JournalScope.
    pub(crate) under: Option<&'a str>,
    pub(crate) allow_selection: bool,
    pub(crate) current_max_offset: usize,
}

pub(crate) struct SearchResult {
    pub(crate) output: output::SearchOutput,
    /// text rows: (entry_prefix, marker_display, content)
    pub(crate) text_rows: Vec<(String, String, String)>,
}

/// Normalize a `--marker` argument to the bare leading-marker form.
/// Accepts `friction` or `[friction]`; empty → error.
pub(crate) fn normalize_marker_filter(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("--marker requires a non-empty name (e.g. friction, decision, metric)".into());
    }
    let bare = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed)
        .trim();
    if bare.is_empty() {
        return Err("--marker requires a non-empty name (e.g. friction, decision, metric)".into());
    }
    Ok(bare.to_ascii_lowercase())
}

pub(crate) fn search_events(
    events: &[(usize, Event)],
    req: &SearchRequest<'_>,
) -> Result<SearchResult, String> {
    let q = req.query.to_lowercase();
    let marker_filter = req.marker.map(|m| m.to_ascii_lowercase());
    // Project with include_hidden so entry_id is always folded; filter per-strand below.
    let all_strands = projection::project_strands(events, true);
    let scope = scope_from_under(
        req.under,
        &all_strands,
        req.allow_selection,
        req.current_max_offset,
    )?;
    let scope_ids = scope.resolve_ids(&all_strands)?;

    let mut matches = Vec::new();
    let mut text_rows = Vec::new();
    for strand in &all_strands {
        if !scope_ids.contains(&strand.id) {
            continue;
        }
        if strand.hidden && !req.include_hidden {
            continue;
        }
        for entry in &strand.log {
            if !q.is_empty() && !entry.content.to_lowercase().contains(&q) {
                continue;
            }
            let marker_name = leading_marker(&entry.content)
                .unwrap_or("")
                .to_ascii_lowercase();
            if let Some(ref want) = marker_filter {
                if marker_name != *want {
                    continue;
                }
            }
            let content = truncate(&entry.content, 70);
            let entry_id = entry.entry_id.clone();
            let entry_prefix = entry_id
                .as_deref()
                .map(shorten)
                .unwrap_or_else(|| format!("offset{}", entry.offset));
            let marker_display = if marker_name.is_empty() {
                String::new()
            } else {
                format!("[{}]", marker_name)
            };
            text_rows.push((entry_prefix, marker_display, content.clone()));
            matches.push(output::SearchMatch {
                strand_id: strand.id.clone(),
                content,
                strand_type: strand.strand_type.clone(),
                hidden: strand.hidden,
                entry_id,
                marker: marker_name,
            });
        }
    }
    let count = matches.len();
    Ok(SearchResult {
        output: output::SearchOutput {
            matches,
            count,
            query: req.query.to_string(),
            marker: marker_filter,
        },
        text_rows,
    })
}

pub(crate) fn cmd_search(
    query: &str,
    format_json: bool,
    include_hidden: bool,
    marker: Option<&str>,
    under: Option<&str>,
) -> Result<(), String> {
    let started = Instant::now();
    let marker_norm = match marker {
        Some(raw) => Some(normalize_marker_filter(raw)?),
        None => None,
    };
    if query.is_empty() && marker_norm.is_none() {
        return Err("search requires a query and/or --marker".into());
    }
    let path = ensure_journal()?;
    let (events, skipped) = read_events_lossy(&path);
    let request = SearchRequest {
        query,
        include_hidden,
        marker: marker_norm.as_deref(),
        under,
        allow_selection: !format_json,
        current_max_offset: events.last().map(|(offset, _)| *offset).unwrap_or(0),
    };
    let result = search_events(&events, &request)?;

    if format_json {
        println!(
            "{}",
            serde_json::to_string(&result.output).expect("serialize")
        );
    } else if result.output.count == 0 {
        match (query.is_empty(), marker_norm.as_deref()) {
            (false, Some(m)) => println!("(no matches for: {} --marker {})", query, m),
            (false, None) => println!("(no matches for: {})", query),
            (true, Some(m)) => println!("(no matches for --marker {})", m),
            (true, None) => println!("(no matches)"),
        }
    } else {
        for (entry_prefix, marker_disp, content) in &result.text_rows {
            if marker_disp.is_empty() {
                println!("{}  {}", entry_prefix, content);
            } else {
                println!("{}  {}  {}", entry_prefix, marker_disp, content);
            }
        }
    }

    if skipped > 0 {
        return Err(corrupted_lines_error(skipped));
    }
    eprintln!(
        "[mnema] search: {:.0?}  ({} matches)",
        started.elapsed(),
        result.output.count
    );
    Ok(())
}
pub(crate) fn apply_timeline_window_limit(
    entries: &mut Vec<projection::TimelineEntry>,
    limit: Option<usize>,
    tail: Option<usize>,
) -> bool {
    // Capture pre-truncation length so `truncated` tells the truth (F1): a
    // hardcoded `false` here silently dropped events on the jq consumption path
    // (--limit + pagination loops keying on `truncated` would miss the tail).
    // --tail N keeps the last N (recent); --limit N keeps the first N (head).
    let pre_truncate_len = entries.len();
    if let Some(n) = tail {
        let skip = entries.len().saturating_sub(n);
        if skip > 0 {
            entries.drain(0..skip);
        }
    } else if let Some(lim) = limit {
        entries.truncate(lim);
    }
    entries.len() < pre_truncate_len
}

/// Apply `--since-ts` / `--until-ts` filters on a timeline projection.
///
/// Timestamps are parsed as RFC3339. Invalid caller input is rejected rather
/// than silently string-compared. A future `--since-ts` correctly produces an
/// empty window.
pub(crate) fn filter_timeline_by_ts(
    entries: &mut Vec<projection::TimelineEntry>,
    since_ts: Option<&str>,
    until_ts: Option<&str>,
) -> Result<(), String> {
    if let Some(st) = since_ts {
        let threshold = crate::util::parse_event_ts(st)
            .ok_or_else(|| format!("invalid --since-ts '{st}': expected RFC3339 timestamp"))?;
        entries.retain(|e| crate::util::parse_event_ts(&e.ts).is_some_and(|ts| ts >= threshold));
    }
    if let Some(ut) = until_ts {
        let threshold = crate::util::parse_event_ts(ut)
            .ok_or_else(|| format!("invalid --until-ts '{ut}': expected RFC3339 timestamp"))?;
        entries.retain(|e| crate::util::parse_event_ts(&e.ts).is_some_and(|ts| ts <= threshold));
    }
    Ok(())
}

pub(crate) fn cmd_timeline(
    since_offset: Option<usize>,
    since_ts: Option<&str>,
    until_offset: Option<usize>,
    until_ts: Option<&str>,
    strand: Option<&str>,
    links: Option<&str>,
    format_json: Option<&str>,
    limit: Option<usize>,
    tail: Option<usize>,
    under: Option<&str>,
) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _skipped) = read_events_lossy(&path);
    let mut entries = projection::project_timeline(&events);

    // Filter by offset range
    if let Some(so) = since_offset {
        entries.retain(|e| e.journal_offset > so);
    }
    if let Some(uo) = until_offset {
        entries.retain(|e| e.journal_offset <= uo);
    }
    filter_timeline_by_ts(&mut entries, since_ts, until_ts)?;

    // Filter by strand or links
    let canonical_strands = projection::project_strands(&events, true);
    let allow_selection = format_json != Some("json");
    let current_max_offset = events.last().map(|(offset, _)| *offset).unwrap_or(0);
    if let Some(sid) = strand {
        let full_id = crate::reference::resolve_strand_with_selection(
            &canonical_strands,
            sid,
            allow_selection,
            current_max_offset,
        )?;
        entries.retain(|e| e.strand_id == full_id);
    }
    if let Some(lid) = links {
        let full_id = crate::reference::resolve_strand_with_selection(
            &canonical_strands,
            lid,
            allow_selection,
            current_max_offset,
        )?;
        // Collect currently linked strand IDs from the projection so v2 effect
        // entries and unlink folds use the same semantics as list/tree/orient.
        let mut linked_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        linked_ids.insert(full_id.clone());
        if let Some(source) = canonical_strands.iter().find(|s| s.id == full_id) {
            linked_ids.extend(source.edges.iter().cloned());
        }
        entries.retain(|e| linked_ids.contains(&e.strand_id));
    }

    let scope = scope_from_under(
        under,
        &canonical_strands,
        allow_selection,
        current_max_offset,
    )?;
    let scope_ids = scope.resolve_ids(&canonical_strands)?;
    entries.retain(|entry| scope_ids.contains(&entry.strand_id));

    let truncated = apply_timeline_window_limit(&mut entries, limit, tail);

    let count = entries.len();
    let max_offset = entries.last().map(|e| e.journal_offset).unwrap_or(0);
    let is_json = format_json == Some("json");

    if is_json {
        let output = output::TimelineOutput {
            timeline: entries
                .iter()
                .map(output::TimelineEntryOutput::from)
                .collect(),
            truncated,
            count,
            max_offset,
        };
        println!("{}", serde_json::to_string(&output).expect("serialize"));
    } else if entries.is_empty() {
        // No dead ends (design principle): empty result must say something.
        let mut parts: Vec<String> = Vec::new();
        if let Some(so) = since_offset {
            parts.push(format!("since-offset {}", so));
        }
        if let Some(st) = since_ts {
            parts.push(format!("since-ts {}", st));
        }
        if let Some(uo) = until_offset {
            parts.push(format!("until-offset {}", uo));
        }
        if let Some(ut) = until_ts {
            parts.push(format!("until-ts {}", ut));
        }
        if let Some(sid) = strand {
            parts.push(format!("strand {}", sid));
        }
        if let Some(lid) = links {
            parts.push(format!("links {}", lid));
        }
        if let Some(root) = under {
            parts.push(format!("under {}", root));
        }
        if parts.is_empty() {
            println!("(journal is empty)");
        } else {
            println!("(no events match: {})", parts.join(", "));
        }
    } else {
        for e in &entries {
            let ts_short = &e.ts[11..19]; // HH:MM:SS
            let id_short = shorten(&e.strand_id);
            let kind_desc = match &e.kind {
                TimelineEventKind::StrandCreated { .. } => "created".to_string(),
                TimelineEventKind::LogAppended { content, .. } => {
                    content.chars().take(60).collect()
                }
                TimelineEventKind::EdgeLinked { target_id, .. } => {
                    format!("link -> {}", shorten(target_id))
                }
                TimelineEventKind::EdgeUnlinked { target_id } => {
                    format!("unlink -> {}", shorten(target_id))
                }
                TimelineEventKind::StrandHidden { .. } => "hidden".to_string(),
                TimelineEventKind::StrandUnhidden { .. } => "unhidden".to_string(),
                TimelineEventKind::CheckpointCreated { action, .. } => {
                    format!("checkpoint: {}", action)
                }
                TimelineEventKind::SubjectBound {
                    subject_type,
                    subject_id,
                    strand_id,
                } => {
                    format!(
                        "bind: {}:{} -> {}",
                        subject_type,
                        subject_id,
                        shorten(strand_id)
                    )
                }
                TimelineEventKind::StrandClosed { disposition } => {
                    format!("closed:{}", disposition)
                }
                TimelineEventKind::StrandReopened => "reopened".to_string(),
            };
            let skew = if e.ts_skew { " ⚠" } else { "" };
            println!("{}  {}  {}{}", ts_short, id_short, kind_desc, skew);
        }
    }
    Ok(())
}

pub(crate) struct OrientRequest {
    pub(crate) include_hidden: bool,
    pub(crate) limit: Option<usize>,
    /// Raw `orient --id X` root (prefix ok); None = JournalScope.
    /// Same candidate set as collection `--under X` (CORPUS §7.1–7.2).
    pub(crate) under: Option<String>,
    pub(crate) allow_selection: bool,
}

pub(crate) struct OrientPlan {
    pub(crate) strands: Vec<projection::ProjectedStrand>,
    pub(crate) view: projection::OrientView,
    pub(crate) output: output::OrientOutput,
    /// Resolved full root id when this plan used SubtreeScope; None = JournalScope.
    pub(crate) scope_root: Option<String>,
}

/// Default stale threshold surfaced by orient (matches `list --stale` help example).
pub(crate) const ORIENT_STALE_DURATION: &str = "2h";
pub(crate) const ORIENT_STALE_SECS: i64 = 2 * 3600;

/// Count active, non-hidden strands whose last entry is older than `cutoff`.
pub(crate) fn count_stale_active(
    strands: &[projection::ProjectedStrand],
    include_hidden: bool,
    now: chrono::DateTime<chrono::Utc>,
    stale_secs: i64,
) -> usize {
    let cutoff = now - chrono::Duration::seconds(stale_secs);
    let cutoff_str = cutoff.to_rfc3339();
    strands
        .iter()
        .filter(|s| s.state() == "registered")
        .filter(|s| !s.hidden || include_hidden)
        .filter(|s| {
            let last_ts = s.last_ts();
            !last_ts.is_empty() && last_ts < cutoff_str.as_str()
        })
        .count()
}

pub(crate) fn orient_plan(
    events: &[(usize, Event)],
    req: &OrientRequest,
) -> Result<OrientPlan, String> {
    orient_plan_at(events, req, chrono::Utc::now())
}

pub(crate) fn orient_plan_at(
    events: &[(usize, Event)],
    req: &OrientRequest,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<OrientPlan, String> {
    let max_offset = events.last().map(|(o, _)| *o).unwrap_or(0);
    let mut strands = projection::project_strands(events, true);
    // orient --id X = SubtreeScope(X): menu/candidate set is the root plus
    // belongs-to descendants. Integrity remains journal-level (container health).
    let scope = scope_from_under(
        req.under.as_deref(),
        &strands,
        req.allow_selection,
        max_offset,
    )?;
    // Collect scope ids first so retain can borrow strands mutably.
    if !scope.is_journal() {
        let ids = scope.resolve_ids(&strands)?;
        strands.retain(|s| ids.contains(&s.id));
    }
    let (entry_count, strand_count) = orient_maturity_counts(&strands, req.include_hidden);
    let score = orient_maturity_score(entry_count, strand_count);
    let limit = req.limit.unwrap_or_else(|| adaptive_orient_limit(score));
    let view = projection::build_orient_view(&strands, req.include_hidden, limit, max_offset);
    let mut output = output::OrientOutput::from((&view, strands.as_slice()));
    if let Some(root_id) = scope.root_id() {
        output.since_command = format!(
            "mnema timeline --since-offset {} --under {}",
            view.max_offset, root_id
        );
    }
    // Questions ① and ③ (CORPUS §8): the integrity glance needs the raw event
    // stream; needs-judgment notices use the scoped strand set.
    output.integrity = integrity_glance(events);
    output.notices = projection::orient_notices(&strands);
    output.remind = adaptive_orient_remind(&strands, &view, req.include_hidden, score);
    output.stale_count = count_stale_active(&strands, req.include_hidden, now, ORIENT_STALE_SECS);
    Ok(OrientPlan {
        strands,
        view,
        output,
        scope_root: scope.root_id().map(|s| s.to_string()),
    })
}

fn print_orient_stale(stale_count: usize) {
    // Always print so the discovery surface exists even when the count is zero.
    println!(
        "stale: {} active silent ≥{} → mnema list --stale {}",
        stale_count, ORIENT_STALE_DURATION, ORIENT_STALE_DURATION
    );
}

fn orient_maturity_counts(
    strands: &[projection::ProjectedStrand],
    include_hidden: bool,
) -> (usize, usize) {
    let visible = strands.iter().filter(|s| !s.hidden || include_hidden);
    let mut entry_count = 0usize;
    let mut strand_count = 0usize;
    for strand in visible {
        entry_count += strand.log_count();
        strand_count += 1;
    }
    (entry_count, strand_count)
}

fn orient_maturity_score(entry_count: usize, strand_count: usize) -> u8 {
    if entry_count == 0 || strand_count == 0 {
        return 0;
    }
    let entry_score = (entry_count.saturating_mul(100) / 40).min(100);
    let strand_score = (strand_count.saturating_mul(100) / 8).min(100);
    entry_score.min(strand_score) as u8
}

fn adaptive_orient_limit(score: u8) -> usize {
    4 + ((score as usize) * 12 / 100)
}

struct LatestOrientEntry<'a> {
    strand_id: &'a str,
    entry: &'a projection::LogEntry,
}

fn latest_visible_entry<'a>(
    strands: &'a [projection::ProjectedStrand],
    include_hidden: bool,
) -> Option<(&'a projection::ProjectedStrand, &'a projection::LogEntry)> {
    strands
        .iter()
        .filter(|s| !s.hidden || include_hidden)
        .flat_map(|strand| strand.log.iter().map(move |entry| (strand, entry)))
        .max_by_key(|(_, entry)| entry.offset)
}

fn latest_active_visible_entry<'a>(
    strands: &'a [projection::ProjectedStrand],
    include_hidden: bool,
) -> Option<LatestOrientEntry<'a>> {
    strands
        .iter()
        .filter(|s| s.state() == "registered")
        .filter(|s| !s.hidden || include_hidden)
        .flat_map(|strand| {
            strand.log.iter().map(move |entry| LatestOrientEntry {
                strand_id: strand.id.as_str(),
                entry,
            })
        })
        .max_by_key(|candidate| candidate.entry.offset)
}

fn active_pair_hint(view: &projection::OrientView) -> Option<String> {
    if view.active_ids.len() < 2 {
        return None;
    }
    let source = shorten(&view.active_ids[0]);
    let target = shorten(&view.active_ids[1]);
    Some(format!(
        "link candidate → mnema link {} {} --edge-type depends-on",
        source, target
    ))
}

fn adaptive_orient_remind(
    strands: &[projection::ProjectedStrand],
    view: &projection::OrientView,
    include_hidden: bool,
    score: u8,
) -> String {
    let link_hint = active_pair_hint(view);
    let Some((latest_strand, latest_entry)) = latest_visible_entry(strands, include_hidden) else {
        return r#"loop: 做一步·看现实变·再想 | write moments: 方案成形 / 判断被现实改变 / 收口或不可逆前 | start → echo "<summary>" | mnema add | writing drill → mnema explain writing | more → mnema --help"#.to_string();
    };

    if latest_strand.state() != "registered" {
        let strand = shorten(&latest_strand.id);
        let from = latest_entry
            .entry_id
            .as_deref()
            .map(shorten)
            .unwrap_or_else(|| strand.clone());
        let next = format!(
            r#"next: latest entry on closed line {}; continue as successor → echo "<summary>" | mnema add --from {}"#,
            strand, from
        );
        let teaching = if score < 30 {
            " | write moments: 方案成形 / 判断被现实改变 / 收口或不可逆前 | template: mnema explain writing"
        } else if score < 75 {
            " | writing template: mnema explain writing"
        } else {
            " | more: mnema --help"
        };
        return match link_hint {
            Some(hint) => format!(
                "loop: 做一步·看现实变·再想 | {} | {}{}",
                next, hint, teaching
            ),
            None => format!("loop: 做一步·看现实变·再想 | {}{}", next, teaching),
        };
    }

    let latest = latest_active_visible_entry(strands, include_hidden)
        .expect("latest visible registered strand should have a latest active entry");
    let strand = shorten(latest.strand_id);
    let entry_prefix = latest
        .entry
        .entry_id
        .as_deref()
        .map(shorten)
        .unwrap_or_else(|| format!("offset{}", latest.entry.offset));
    let marker = leading_marker(&latest.entry.content).unwrap_or("");
    let next = match marker {
        "friction" => format!(
            "next: latest [friction] {} on {}; after fixing → echo \"[fixed] fixes={} <what changed>; verified=<command>\" | mnema append --id {}",
            entry_prefix, strand, entry_prefix, strand
        ),
        "checkpoint" => format!(
            "next: checkpoint {} was written; after action/verification → echo \"[observed] <result>; source=<command>\" | mnema append --id {}",
            entry_prefix, strand
        ),
        "decision" => format!(
            "next: test decision {} against reality → echo \"[observed] <fact>; source=<command>\" | mnema append --id {}",
            entry_prefix, strand
        ),
        "deliverable" | "done" | "verified" | "failed" | "cancelled" | "merged" => {
            format!(
                "next: if concluded, close the line → mnema close --id {} [--as done|failed|cancelled|merged|verified]",
                strand
            )
        }
        _ => format!(
            "next: continue from latest entry {} on {} → echo \"[progress] <what changed>; verify=<command>\" | mnema append --id {}",
            entry_prefix, strand, strand
        ),
    };

    let teaching = if score < 30 {
        " | write moments: 方案成形 / 判断被现实改变 / 收口或不可逆前 | template: mnema explain writing"
    } else if score < 75 {
        " | writing template: mnema explain writing"
    } else {
        " | more: mnema --help"
    };

    match link_hint {
        Some(hint) => format!(
            "loop: 做一步·看现实变·再想 | {} | {}{}",
            next, hint, teaching
        ),
        None => format!("loop: 做一步·看现实变·再想 | {}{}", next, teaching),
    }
}

/// Print the needs-judgment block (CORPUS §8, question ③) — nothing when clear.

fn print_collaboration_pull(strands: &[projection::ProjectedStrand]) {
    if let Some(forest) = projection::find_recent_collaboration_forest(strands) {
        println!(
            "collaboration: mnema explain collaboration | mnema tree --id {}",
            shorten(&forest.root_id)
        );
    }
}

fn print_orient_notices(notices: &[String]) {
    if notices.is_empty() {
        return;
    }
    println!("needs judgment:");
    for n in notices {
        println!("  {}", n);
    }
}

/// One-line integrity summary for orient (CORPUS §8, question ①). Full chain +
/// anchor verification — O(events); fine at current scale, and can drop to a
/// latest-anchor-only check if orient ever gets slow on huge journals.
fn integrity_glance(events: &[(usize, Event)]) -> String {
    let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
    let report = crate::diagnostics::verify_journal_integrity(&raw);
    if report.has_errors() {
        let first = report
            .chain_errors
            .first()
            .or_else(|| report.anchor_errors.first())
            .map(|s| s.as_str())
            .unwrap_or("integrity error");
        format!("FAIL — {}", first)
    } else {
        format!(
            "ok ({} anchors, {} unanchored tail)",
            report.anchor_count, report.unanchored_event_count
        )
    }
}
pub(crate) fn cmd_orient(
    format: Option<&str>,
    include_hidden: bool,
    limit: Option<usize>,
    show_tree: bool,
    under: Option<&str>,
) -> Result<(), String> {
    let started = Instant::now();
    let path = ensure_journal()?;
    let (events, skipped) = read_events_lossy(&path);
    let request = OrientRequest {
        include_hidden,
        limit,
        under: under.map(|s| s.to_string()),
        allow_selection: format != Some("json"),
    };
    let plan = orient_plan(&events, &request)?;
    let strands = &plan.strands;
    let view = &plan.view;
    let out = &plan.output;
    let scope_label = plan.scope_root.as_deref().map(shorten);

    if show_tree {
        // Build the belongs-to forest from the active strand set.
        // The tree module returns projection nodes; Contract Surface maps them
        // to the public orient-tree DTO below.
        let active_strands: Vec<&projection::ProjectedStrand> = view
            .active_ids
            .iter()
            .filter_map(|id| strands.iter().find(|s| &s.id == id))
            .collect();
        let forest = tree::build_orient_forest(&active_strands);
        let roots: Vec<output::OrientForestNode> =
            forest.iter().map(output::OrientForestNode::from).collect();
        let tree_out = output::OrientTreeOutput {
            max_offset: out.max_offset,
            roots,
            closed_count: out.closed_count,
            hidden_count: out.hidden_count,
            integrity: out.integrity.clone(),
            notices: out.notices.clone(),
            since_command: out.since_command.clone(),
            delegation_command: out.delegation_command.clone(),
            remind: out.remind.clone(),
            pause: out.pause.clone(),
            stale_count: out.stale_count,
        };

        if format == Some("json") {
            println!("{}", serde_json::to_string(&tree_out).expect("serialize"));
        } else {
            println!(
                "journal: max_offset {} | {} active | {} closed | {} hidden (mnema list)",
                out.max_offset,
                out.active.len(),
                out.closed_count,
                out.hidden_count
            );
            if let Some(ref root) = scope_label {
                println!("scope: under {} (SubtreeScope)", root);
            }
            println!("integrity: {}", out.integrity);
            println!("since: {}", out.since_command);
            println!("delegation: {}", out.delegation_command);
            print_orient_stale(out.stale_count);
            print_orient_forest(&tree_out.roots, 0);
            if out.active.is_empty() {
                println!("(no active strands) — start one: echo \"<summary>\" | mnema add");
            }
            print_orient_notices(&out.notices);
            println!("remind: {}", out.remind);
            println!("{}", out.pause);
            print_collaboration_pull(strands);
        }
    } else if format == Some("json") {
        println!("{}", serde_json::to_string(&out).expect("serialize"));
    } else {
        println!(
            "journal: max_offset {} | {} active | {} closed | {} hidden (mnema list)",
            out.max_offset,
            out.active.len(),
            out.closed_count,
            out.hidden_count
        );
        if let Some(ref root) = scope_label {
            println!("scope: under {} (SubtreeScope)", root);
        }
        println!("integrity: {}", out.integrity);
        println!("since: {}", out.since_command);
        println!("delegation: {}", out.delegation_command);
        print_orient_stale(out.stale_count);
        for s in &out.active {
            let type_info = s
                .strand_type
                .as_deref()
                .map(|t| format!(" [{}]", t))
                .unwrap_or_default();
            let slug_info = s
                .slug
                .as_deref()
                .map(|slug| format!(" ({})", slug))
                .unwrap_or_default();
            println!(
                "  {}{}{}  {} entries | last_offset {}",
                shorten(&s.id),
                slug_info,
                type_info,
                s.entry_count,
                s.last_offset
            );
            println!("    {}", s.summary);
            if s.entry_count > 1 {
                println!("    last: {}", s.last_entry);
            }
            println!("    catch-up: {}", s.catch_up);
        }
        if out.active.is_empty() {
            println!("(no active strands) — start one: echo \"<summary>\" | mnema add");
        }
        print_orient_notices(&out.notices);
        println!("remind: {}", out.remind);
        println!("{}", out.pause);
        print_collaboration_pull(strands);
    }

    if skipped > 0 {
        return Err(corrupted_lines_error(skipped));
    }
    eprintln!("[mnema] orient: {:.0?}", started.elapsed());
    Ok(())
}

/// Render an entry's body for reading views. An entry carrying an effect shows
/// the canonical marker derived from that effect (CORPUS §5: `[closed(done)]`
/// etc.) rather than the raw machine-mirror content; the author's reason
/// (close/reopen) rides along as a trailing note.
fn entry_display_body(entry: &projection::LogEntry) -> String {
    use crate::event::EntryEffect;
    let effect = match &entry.effect {
        None => return entry.content.clone(),
        Some(e) => e,
    };
    // close/reopen keep the author reason after the machine-mirror prefix.
    let note = entry
        .content
        .split_once(": ")
        .map(|(_, n)| n.trim())
        .filter(|n| !n.is_empty());
    let with_note = |marker: String| match note {
        Some(n) => format!("{} {}", marker, n),
        None => marker,
    };
    match effect {
        EntryEffect::Close { disposition } => with_note(format!("[closed({})]", disposition)),
        EntryEffect::Reopen => with_note("[reopened]".to_string()),
        EntryEffect::Link { target, edge_type } => {
            format!("[linked({})] -> {}", edge_type, shorten(target))
        }
        EntryEffect::Unlink {
            target, edge_type, ..
        } => {
            format!("[unlinked({})] -> {}", edge_type, shorten(target))
        }
        EntryEffect::Hide => entry
            .content
            .strip_prefix("[hidden] ")
            .map(|r| format!("[hidden] {}", r))
            .unwrap_or_else(|| "[hidden]".to_string()),
        EntryEffect::Unhide => "[unhidden]".to_string(),
    }
}

pub(crate) fn cmd_show(
    id: Option<&str>,
    last: bool,
    tail: Option<usize>,
    format_json: bool,
    locked: bool,
    digest: bool,
    producer: Option<&str>,
) -> Result<(), String> {
    let started = Instant::now();
    let path = ensure_journal()?;
    let read = if locked {
        read_journal_lossy_locked()
    } else {
        read_journal_lossy(&path)
    };
    if let Some(error) = &read.read_error {
        return Err(error.clone());
    }
    let skipped = read.skipped();
    let events = read.events;
    let strands = projection::project_strands(&events, true);

    // Unified target convention: an explicit id wins; otherwise (--last, or no
    // target at all) default to the most recently active strand.
    let strand = match id {
        Some(id_str) => {
            let current_max_offset = events.last().map(|(offset, _)| *offset).unwrap_or(0);
            let full = crate::reference::resolve_strand_with_selection(
                &strands,
                id_str,
                !format_json,
                current_max_offset,
            )?;
            strands.iter().find(|s| s.id == full).unwrap()
        }
        None => {
            let _ = last; // --last is the explicit spelling of this default
            crate::commands::write::resolve_most_recent_strand(&strands)
                .ok_or("no active strand to show — pass <ID> or --id")?
        }
    };

    // --producer: narrow the view to one writer's entries — the
    // highest-frequency narrowing dimension in multi-writer journals.
    let producer_filtered;
    let strand = if let Some(name) = producer {
        producer_filtered = strand.with_producer_filter(name);
        &producer_filtered
    } else {
        strand
    };

    // Summary
    let entry_count = strand.log_count();
    let last_summary = strand.last_summary();
    let canonical_state = strand.state();

    if format_json {
        let output = output::StrandDetailOutput::from(strand);
        println!("{}", serde_json::to_string(&output).expect("serialize"));
        if skipped > 0 {
            return Err(corrupted_lines_error(skipped));
        }
        return Ok(());
    }

    let slug_info = strand
        .slug
        .as_deref()
        .map(|slug| format!(" | slug: {}", slug))
        .unwrap_or_default();
    println!(
        "strand: {}{} | {} entries | state: {} | last_entry_offset: {}",
        shorten(&strand.id),
        slug_info,
        entry_count,
        canonical_state,
        strand.last_offset()
    );
    println!("summary: {}", truncate(strand.first_summary(), 60));
    println!("next: {}", truncate(last_summary, 100));
    if let Some(name) = producer {
        println!("producer filter: {} ({} entries match)", name, entry_count);
    }
    if strand.hidden {
        println!("status: hidden");
    }
    if !strand.edges.is_empty() {
        println!("edges: {}", strand.edges.join(", "));
    }

    if digest {
        // One-glance digest: typed marker census, no full log dump.
        let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        let mut unmarked = 0usize;
        for entry in &strand.log {
            match leading_marker(&entry.content) {
                Some(m) => *counts.entry(m).or_insert(0) += 1,
                None => unmarked += 1,
            }
        }
        let mut pairs: Vec<(&str, usize)> = counts.into_iter().collect();
        pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let mut parts: Vec<String> = pairs.iter().map(|(m, c)| format!("{} {}", c, m)).collect();
        if unmarked > 0 {
            parts.push(format!("{} unmarked", unmarked));
        }
        let census = if parts.is_empty() {
            "—".to_string()
        } else {
            parts.join(", ")
        };
        println!("markers: {}", census);
        eprintln!(
            "[mnema] show:   {:.0?}  (digest, {} entries)",
            started.elapsed(),
            entry_count
        );
        if let Err(e) = crate::reference::remember_last_touched_current(&strand.id) {
            eprintln!("[mnema] warning: {}", e);
        }
        if skipped > 0 {
            return Err(corrupted_lines_error(skipped));
        }
        return Ok(());
    }

    // Determine which entries to show
    let entries: Vec<_> = strand.log.iter().collect();
    let slice = if let Some(n) = tail {
        let skip = entries.len().saturating_sub(n);
        &entries[skip..]
    } else {
        &entries[..]
    };
    let shown = slice.len();

    println!("log:");
    let entry_index = projection::EntryIndex::build(&strands);
    let now = chrono::Utc::now();
    let mut prev_ts: Option<&str> = None;
    for entry in slice {
        // Long in-line gaps are the machine's to point out, not the reader's
        // to compute (CORPUS §8) — annotate when consecutive entries are
        // two or more days apart.
        if let Some(prev) = prev_ts {
            if let Some(gap) = crate::util::ts_gap_seconds(prev, &entry.ts) {
                if gap >= 2 * 86_400 {
                    println!("  (gap: {} since previous entry)", humanize_duration(gap));
                }
            }
        }
        prev_ts = Some(&entry.ts);
        // v2 refs render as short handles. A cited entry whose line gained
        // later entries is annotated in place (ref-target-advanced is a
        // position fact; whether it overturns anything is the reader's call —
        // run the re-look command on the cited entry).
        let ref_str = if !entry.refs.is_empty() {
            let handles: Vec<String> = entry
                .refs
                .iter()
                .map(|h| {
                    let mut handle = shorten(h);
                    if entry_index.advanced_past(h, entry.offset) == Some(true) {
                        handle.push_str(" (advanced)");
                    }
                    handle
                })
                .collect();
            format!(" [refs: {}]", handles.join(", "))
        } else {
            String::new()
        };
        let id_str = entry
            .entry_id
            .as_deref()
            .map(shorten)
            .map(|h| format!(" [{}]", h))
            .unwrap_or_default();
        println!(
            "  [{}]{} {}{}",
            display_ts(&entry.ts, now),
            id_str,
            entry_display_body(entry),
            ref_str
        );
    }
    eprintln!(
        "[mnema] show:   {:.0?}  ({} entries, {} shown)",
        started.elapsed(),
        entry_count,
        shown
    );
    if let Err(e) = crate::reference::remember_last_touched_current(&strand.id) {
        eprintln!("[mnema] warning: {}", e);
    }
    if skipped > 0 {
        return Err(corrupted_lines_error(skipped));
    }
    Ok(())
}

/// Show one entry by hash prefix and expand its rationale refs `deref` hops.
/// Every pulled entry travels with mechanical coordinates (home line, position,
/// later-entry count) because the unit of self-containment is the line, not
/// the entry. Truncation at the depth boundary is honest: the frontier is
/// listed with retrieval commands and the size of what expanding would cost.
pub(crate) fn cmd_show_entry(
    prefix: &str,
    deref: usize,
    before: usize,
    after: usize,
    format_json: bool,
) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, skipped) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let view = projection::build_entry_view(&strands, prefix, deref)?;

    if format_json {
        let output = output::ShowEntryOutput {
            status: "ok",
            entry_id: view.nodes[0].entry.entry_id.clone().unwrap_or_default(),
            deref,
            nodes: view
                .nodes
                .iter()
                .map(|n| {
                    let neighbour = |e: &projection::LogEntry| output::EntryNeighbourOutput {
                        entry_id: e.entry_id.clone(),
                        ts: e.ts.clone(),
                        content: e.content.clone(),
                    };
                    let before_slice = if before > 0 {
                        let start = n.entry_index.saturating_sub(before);
                        n.strand.log[start..n.entry_index]
                            .iter()
                            .map(neighbour)
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let after_slice = if after > 0 {
                        let end = (n.entry_index + 1 + after).min(n.strand.log.len());
                        n.strand.log[n.entry_index + 1..end]
                            .iter()
                            .map(neighbour)
                            .collect()
                    } else {
                        Vec::new()
                    };
                    output::EntryDerefNodeOutput {
                        hop: n.hop,
                        cited_by: n.cited_by.clone(),
                        entry_id: n.entry.entry_id.clone().unwrap_or_default(),
                        strand_id: n.strand.id.clone(),
                        strand_summary: truncate(n.strand.first_summary(), 70),
                        entry_index: n.entry_index,
                        strand_entry_count: n.strand.log.len(),
                        later_entries: n.later_entries,
                        ts: n.entry.ts.clone(),
                        content: n.entry.content.clone(),
                        effect: n.entry.effect.as_ref().map(output::EntryEffectOutput::from),
                        refs: n.entry.refs.clone(),
                        before: before_slice,
                        after: after_slice,
                    }
                })
                .collect(),
            unresolved: view
                .stubs
                .iter()
                .map(|s| output::EntryDerefStubOutput {
                    hop: s.hop,
                    cited_by: s.cited_by.clone(),
                    entry_id: s.hash.clone(),
                    resolved: false,
                })
                .collect(),
            frontier: view
                .frontier
                .iter()
                .map(|f| output::EntryFrontierOutput {
                    entry_id: f.hash.clone(),
                    content_len: f.content_len,
                })
                .collect(),
        };
        println!("{}", serde_json::to_string(&output).unwrap());
    } else {
        let now = chrono::Utc::now();
        let total_chars: usize = view.nodes.iter().map(|n| n.entry.content.len()).sum();
        for node in &view.nodes {
            let handle = node
                .entry
                .entry_id
                .as_deref()
                .map(shorten)
                .unwrap_or_default();
            if node.hop == 0 {
                println!("entry: [{}]", handle);
            } else {
                println!(
                    "── hop {} · cited by {} ──",
                    node.hop,
                    shorten(node.cited_by.as_deref().unwrap_or("?"))
                );
                println!("[{}]", handle);
            }
            println!("  {}", entry_display_body(node.entry));
            let advanced = if node.later_entries > 0 {
                format!(" · {} later entries (advanced)", node.later_entries)
            } else {
                String::new()
            };
            println!(
                "  line: {} \"{}\" · entry {}/{}{}",
                shorten(&node.strand.id),
                truncate(node.strand.first_summary(), 60),
                node.entry_index + 1,
                node.strand.log.len(),
                advanced
            );
            // For advanced entries the retrieval command doubles as the
            // re-look: it prices the follow-up (--after <exact count>) and
            // the reader may lower it.
            let re_look = if node.later_entries > 0 {
                format!(" --after {}", node.later_entries)
            } else {
                String::new()
            };
            println!(
                "  at: {} · mnema show --entry {}{}",
                display_ts(&node.entry.ts, now),
                handle,
                re_look
            );
            // --before K: preceding entries from this entry's own line, one
            // line each — the local deliberation an entry may lean on.
            if before > 0 && node.entry_index > 0 {
                let start = node.entry_index.saturating_sub(before);
                for prev in &node.strand.log[start..node.entry_index] {
                    println!(
                        "    ↑ [{}] {}",
                        prev.entry_id.as_deref().map(shorten).unwrap_or_default(),
                        truncate(&prev.content, 120)
                    );
                }
            }
            // --after K: following entries from this entry's own line — what
            // the line did after this point (the substance behind (advanced)).
            if after > 0 && node.entry_index + 1 < node.strand.log.len() {
                let end = (node.entry_index + 1 + after).min(node.strand.log.len());
                for next in &node.strand.log[node.entry_index + 1..end] {
                    println!(
                        "    ↓ [{}] {}",
                        next.entry_id.as_deref().map(shorten).unwrap_or_default(),
                        truncate(&next.content, 120)
                    );
                }
            }
        }
        for stub in &view.stubs {
            println!(
                "── hop {} · cited by {} ──",
                stub.hop,
                shorten(&stub.cited_by)
            );
            println!(
                "[{}] points elsewhere — not verifiable locally (cross-journal or missing)",
                shorten(&stub.hash)
            );
        }
        if !view.frontier.is_empty() {
            let frontier_chars: usize = view.frontier.iter().filter_map(|f| f.content_len).sum();
            println!(
                "frontier (beyond depth {}): {} refs unexpanded, ~{} chars",
                deref,
                view.frontier.len(),
                frontier_chars
            );
            for f in &view.frontier {
                println!("  mnema show --entry {}", shorten(&f.hash));
            }
        }
        eprintln!(
            "[mnema] show --entry: {} entries pulled, ~{} chars, {} unresolved",
            view.nodes.len(),
            total_chars,
            view.stubs.len()
        );
    }
    if skipped > 0 {
        return Err(corrupted_lines_error(skipped));
    }
    Ok(())
}

// ── Human picker ─────────────────────────────────────────

pub(crate) fn cmd_pick(
    command: &str,
    print_id: bool,
    all: bool,
    under: Option<&str>,
) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _skipped) = read_events_lossy(&path);
    let universe = projection::project_strands(&events, true);
    let max_offset = events.last().map(|(o, _)| *o).unwrap_or(0);
    let scope = scope_from_under(under, &universe, true, max_offset)?;
    let mut strands = projection::project_strands(&events, all);
    scope.retain_strands(&mut strands, &universe)?;
    if !all {
        // Default picker view: active work only. Closed lines fold out (like
        // orient); `--all` brings closed and hidden back.
        strands.retain(|s| s.state() == "registered");
    }
    if strands.is_empty() {
        return Err(if under.is_some() {
            if all {
                "no strands to pick under scope".to_string()
            } else {
                "no active strands to pick under scope — use --all for closed/hidden".to_string()
            }
        } else if all {
            "no strands to pick".to_string()
        } else {
            "no active strands to pick — use --all for closed/hidden".to_string()
        });
    }
    let rows = pick_forest_rows(&strands);

    let append_body = if !print_id && command == "append" {
        if atty::is(atty::Stream::Stdin) {
            return Err("append body must be piped: echo ... | mnema pick append".to_string());
        }
        Some(read_stdin_content()?)
    } else {
        None
    };
    let lifecycle_reason = if !print_id && (command == "close" || command == "reopen") {
        read_stdin_if_piped()
    } else {
        None
    };

    let selected = pick_strand_id(&strands, &rows)?;
    if print_id {
        println!("{}", selected);
        return Ok(());
    }
    match command {
        "show" => cmd_show(Some(&selected), false, Some(8), false, false, false, None),
        "tree" => cmd_tree(&selected, None),
        "depends" => cmd_depends(&selected, None),
        "append" => crate::commands::write::cmd_append_with_seen_offset(
            append_body.as_deref(),
            None,
            false,
            false,
            None,
            Some(&selected),
            None,
            None,
            None,
            None,
        ),
        "close" => {
            crate::commands::write::cmd_close(&selected, None, lifecycle_reason.as_deref(), false)
        }
        "reopen" => {
            crate::commands::write::cmd_reopen(&selected, lifecycle_reason.as_deref(), false)
        }
        "hide" => crate::commands::manage::cmd_hide(&selected, None, false, None),
        "unhide" => crate::commands::manage::cmd_unhide(&selected, false),
        other => Err(format!(
            "pick command '{}' is unsupported; valid commands: show, tree, depends, append, close, reopen, hide, unhide, --print-id",
            other
        )),
    }
}

/// Build one picker row for a human. `seq` (1-based) replaces the 64-hex id
/// (the id travels as fzf's hidden first column / `PickItem.id`). `depth` indents
/// belongs-to children under their parent; `extra_parents` flags a strand with
/// more than one in-set parent (the single-parent basis is an anomaly, so it is
/// marked, not duplicated). The first summary takes the remaining width
/// (fzf/terminal clips it); the tail lives in the preview pane.
pub(crate) fn pick_label(
    seq: usize,
    depth: usize,
    extra_parents: usize,
    state_width: usize,
    strand: &projection::ProjectedStrand,
) -> String {
    pick_row(seq, depth, extra_parents, state_width, strand, false)
}

fn pick_label_ansi(
    seq: usize,
    depth: usize,
    extra_parents: usize,
    state_width: usize,
    strand: &projection::ProjectedStrand,
) -> String {
    pick_row(seq, depth, extra_parents, state_width, strand, true)
}

/// Widest state label among the rows, so the state column pads only as much as
/// the current view needs — an all-`open` default view stays tight, a mixed
/// `--all` view still aligns.
fn pick_state_width(
    strands: &[projection::ProjectedStrand],
    rows: &[(usize, usize, usize)],
) -> usize {
    rows.iter()
        .map(|&(_, idx, _)| pick_state_word(strands[idx].state()).chars().count())
        .max()
        .unwrap_or(0)
}

fn pick_row(
    seq: usize,
    depth: usize,
    extra_parents: usize,
    state_width: usize,
    strand: &projection::ProjectedStrand,
    ansi: bool,
) -> String {
    let state = strand.state();
    let padded = format!("{:<width$}", pick_state_word(state), width = state_width);
    let state_disp = match (ansi, pick_state_color(state)) {
        (true, Some(color)) => format!("{}{}\u{1b}[0m", color, padded),
        _ => padded,
    };
    let indent = if depth == 0 {
        String::new()
    } else {
        format!("{}└ ", "  ".repeat(depth - 1))
    };
    let slug = strand
        .slug
        .as_deref()
        .map(|s| format!("({}) ", s))
        .unwrap_or_default();
    let mark = if extra_parents > 0 {
        format!("  [+{}父]", extra_parents)
    } else {
        String::new()
    };
    let summary = strand.first_summary().replace(['\t', '\n', '\r'], " ");
    format!(
        "{:>3}. {} {}{}{}{}",
        seq, state_disp, indent, slug, summary, mark
    )
}

fn pick_state_word(state: &str) -> String {
    match state {
        "registered" => "○ open".to_string(),
        "closed:done" => "● done".to_string(),
        "closed:failed" => "● failed".to_string(),
        other if other.starts_with("closed:") => format!("● {}", &other["closed:".len()..]),
        other => other.to_string(),
    }
}

fn pick_state_color(state: &str) -> Option<&'static str> {
    match state {
        "registered" => Some("\u{1b}[32m"),
        "closed:done" => Some("\u{1b}[34m"),
        "closed:failed" => Some("\u{1b}[31m"),
        _ => None,
    }
}

/// Flatten strands into a belongs-to forest in DFS pre-order, so a parent and
/// its whole subtree stay contiguous (a flat last_ts sort would wedge unrelated
/// lines between them). Families are ranked by their most-recent member; within
/// a family, children are ranked the same way. Returns (depth, strand index,
/// extra_parent_count). First in-set parent wins for nesting; a strand with more
/// than one in-set parent is flagged via extra_parent_count, never duplicated.
fn pick_forest_rows(strands: &[projection::ProjectedStrand]) -> Vec<(usize, usize, usize)> {
    use std::collections::{HashMap, HashSet};
    let idx_of: HashMap<&str, usize> = strands
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();

    let mut primary_parent: HashMap<usize, usize> = HashMap::new();
    let mut extra_parents: HashMap<usize, usize> = HashMap::new();
    for (i, s) in strands.iter().enumerate() {
        let parents: Vec<usize> = s
            .belongs_to_edges
            .iter()
            .filter_map(|p| idx_of.get(p.as_str()).copied())
            .collect();
        if let Some(&first) = parents.first() {
            primary_parent.insert(i, first);
            extra_parents.insert(i, parents.len() - 1);
        }
    }

    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    for (&child, &parent) in &primary_parent {
        children.entry(parent).or_default().push(child);
    }

    // Subtree max-recency (last_ts), memoized; cycle-guarded defensively.
    fn subtree_rec(
        i: usize,
        children: &HashMap<usize, Vec<usize>>,
        strands: &[projection::ProjectedStrand],
        memo: &mut HashMap<usize, String>,
        visiting: &mut HashSet<usize>,
    ) -> String {
        if let Some(v) = memo.get(&i) {
            return v.clone();
        }
        if !visiting.insert(i) {
            return strands[i].last_ts().to_string();
        }
        let mut best = strands[i].last_ts().to_string();
        if let Some(kids) = children.get(&i) {
            for &k in kids {
                let r = subtree_rec(k, children, strands, memo, visiting);
                if r > best {
                    best = r;
                }
            }
        }
        visiting.remove(&i);
        memo.insert(i, best.clone());
        best
    }
    let mut memo: HashMap<usize, String> = HashMap::new();
    let mut visiting: HashSet<usize> = HashSet::new();
    for i in 0..strands.len() {
        subtree_rec(i, &children, strands, &mut memo, &mut visiting);
    }
    for i in 0..strands.len() {
        memo.entry(i)
            .or_insert_with(|| strands[i].last_ts().to_string());
    }
    let rank = |a: usize, b: usize| {
        memo[&b]
            .cmp(&memo[&a])
            .then_with(|| strands[a].id.cmp(&strands[b].id))
    };

    let mut roots: Vec<usize> = (0..strands.len())
        .filter(|i| !primary_parent.contains_key(i))
        .collect();
    roots.sort_by(|&a, &b| rank(a, b));

    // Iterative DFS pre-order: push reversed so children pop in ranked order.
    let mut rows: Vec<(usize, usize, usize)> = Vec::new();
    let mut visited: HashSet<usize> = HashSet::new();
    let mut stack: Vec<(usize, usize)> = roots.into_iter().rev().map(|r| (r, 0usize)).collect();
    while let Some((i, depth)) = stack.pop() {
        if !visited.insert(i) {
            continue;
        }
        rows.push((depth, i, extra_parents.get(&i).copied().unwrap_or(0)));
        if let Some(kids) = children.get(&i) {
            let mut kids = kids.clone();
            kids.sort_by(|&a, &b| rank(a, b));
            for k in kids.into_iter().rev() {
                stack.push((k, depth + 1));
            }
        }
    }
    rows
}

/// One selectable row. `Display` renders the label so inquire shows it;
/// selecting returns the whole item, so the canonical full id is recovered
/// without parsing it back out of the label.
struct PickItem {
    id: String,
    label: String,
}

impl std::fmt::Display for PickItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// Interactive strand chooser: fzf when available (with a preview pane),
/// otherwise an arrow-key inquire menu. stdout stays clean for --print-id. In a
/// non-TTY context it fails closed rather than blocking on input.
fn pick_strand_id(
    strands: &[projection::ProjectedStrand],
    rows: &[(usize, usize, usize)],
) -> Result<String, String> {
    let interactive = atty::is(atty::Stream::Stdout) || atty::is(atty::Stream::Stderr);
    if !interactive {
        return Err(
            "pick requires an interactive TTY; use --id/--print-id from an interactive shell"
                .to_string(),
        );
    }
    if let Some(selected) = pick_with_fzf(strands, rows)? {
        return Ok(selected);
    }
    let state_w = pick_state_width(strands, rows);
    let items: Vec<PickItem> = rows
        .iter()
        .enumerate()
        .map(|(seq0, &(depth, idx, extra))| PickItem {
            id: strands[idx].id.clone(),
            label: pick_label(seq0 + 1, depth, extra, state_w, &strands[idx]),
        })
        .collect();
    match inquire::Select::new("pick a strand", items)
        .with_page_size(15)
        .prompt()
    {
        Ok(item) => Ok(item.id),
        Err(inquire::InquireError::OperationCanceled)
        | Err(inquire::InquireError::OperationInterrupted) => Err("pick cancelled".to_string()),
        Err(e) => Err(format!("pick failed: {}", e)),
    }
}

fn pick_with_fzf(
    strands: &[projection::ProjectedStrand],
    rows: &[(usize, usize, usize)],
) -> Result<Option<String>, String> {
    let exe = std::env::current_exe().map_err(|e| format!("pick failed: {}", e))?;
    let preview = format!("{} show {{1}} --tail 8", quote_preview_exe(&exe));
    let mut child = match Command::new("fzf")
        .arg("--ansi")
        .arg("--with-nth=2..")
        .arg("--delimiter=\\t")
        .arg("--preview")
        .arg(preview)
        .arg("--preview-window=right:50%:wrap")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("pick failed: {}", e)),
    };

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "pick failed: fzf stdin unavailable".to_string())?;
        let state_w = pick_state_width(strands, rows);
        for (seq0, &(depth, idx, extra)) in rows.iter().enumerate() {
            writeln!(
                stdin,
                "{}\t{}",
                strands[idx].id,
                pick_label_ansi(seq0 + 1, depth, extra, state_w, &strands[idx])
            )
            .map_err(|e| format!("pick failed: {}", e))?;
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("pick failed: {}", e))?;
    if !output.status.success() {
        return Err("pick cancelled".to_string());
    }
    let selected = String::from_utf8_lossy(&output.stdout);
    let selected_id = selected
        .lines()
        .next()
        .unwrap_or("")
        .trim_end_matches('\r')
        .split('\t')
        .next()
        .unwrap_or("")
        .trim();
    if selected_id.is_empty() {
        Err("pick cancelled".to_string())
    } else {
        Ok(Some(selected_id.to_string()))
    }
}

fn quote_preview_exe(path: &std::path::Path) -> String {
    format!("\"{}\"", path.display().to_string().replace('"', "\\\""))
}

// ── Tree projection ─────────────────────────────────────

pub(crate) fn cmd_tree(root_id: &str, format_json: Option<&str>) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _skipped) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);

    let current_max_offset = events.last().map(|(offset, _)| *offset).unwrap_or(0);
    let full_root = crate::reference::resolve_strand_with_selection(
        &strands,
        root_id,
        format_json != Some("json"),
        current_max_offset,
    )?;
    match tree::project_tree(&full_root, &strands) {
        Some(root) => {
            if format_json == Some("json") {
                let output = output::TreeOutput::from(&root);
                println!("{}", serde_json::to_string_pretty(&output).unwrap());
            } else {
                print_tree_text(&root, 0);
            }
        }
        None => {
            return Err(format!("strand not found: {}", root_id));
        }
    }
    Ok(())
}

fn print_tree_text(node: &tree::TreeNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let marker = if node.children.is_empty() {
        "  "
    } else {
        "└─"
    };
    println!(
        "{}{} {} [{}] {}",
        indent,
        marker,
        &node.id[..12.min(node.id.len())],
        node.status,
        node.summary.chars().take(60).collect::<String>()
    );
    for child in &node.children {
        print_tree_text(child, depth + 1);
    }
}

/// depends-on review for one strand: upstream lifecycle facts and trace handles.
/// Built on the F3 typed projection; lifecycle is evidence, not a gate verdict.
/// Does not compute ready / blocker / critical-path.
pub(crate) fn cmd_depends(id: &str, format_json: Option<&str>) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _skipped) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let current_max_offset = events.last().map(|(offset, _)| *offset).unwrap_or(0);
    let full_id = crate::reference::resolve_strand_with_selection(
        &strands,
        id,
        format_json != Some("json"),
        current_max_offset,
    )?;
    let graph = graph::StrandGraph::from_strands(&strands);
    let review = graph
        .depends_review(&full_id)
        .ok_or_else(|| format!("strand {} not found", id))?;

    if format_json == Some("json") {
        let output = output::DependsOutput::from(&review);
        println!("{}", serde_json::to_string(&output).expect("serialize"));
    } else {
        print_depends_review_text(&review);
    }
    Ok(())
}

/// `depends --under X`: same per-strand review facts for every strand in
/// SubtreeScope(X). Schema per strand matches single-strand depends; no
/// ready/blocker/critical-path aggregation.
pub(crate) fn cmd_depends_under(under: &str, format_json: Option<&str>) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _skipped) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let current_max_offset = events.last().map(|(offset, _)| *offset).unwrap_or(0);
    let allow_selection = format_json != Some("json");
    let scope = scope_from_under(Some(under), &strands, allow_selection, current_max_offset)?;
    let root_id = scope
        .root_id()
        .ok_or_else(|| "depends --under requires a subtree root".to_string())?
        .to_string();
    let ids = scope.resolve_ids(&strands)?;
    let graph = graph::StrandGraph::from_strands(&strands);

    // Stable order: reverse-chrono by last activity among in-scope strands,
    // matching collection-query presentation.
    let mut ordered: Vec<&projection::ProjectedStrand> =
        strands.iter().filter(|s| ids.contains(&s.id)).collect();
    ordered.sort_by(|a, b| b.last_ts().cmp(a.last_ts()));

    let mut reviews = Vec::with_capacity(ordered.len());
    for strand in &ordered {
        let review = graph
            .depends_review(&strand.id)
            .ok_or_else(|| format!("strand {} not found", strand.id))?;
        reviews.push(review);
    }

    if format_json == Some("json") {
        let out = output::DependsScopeOutput {
            root_id: root_id.clone(),
            count: reviews.len(),
            strands: reviews.iter().map(output::DependsOutput::from).collect(),
        };
        println!("{}", serde_json::to_string(&out).expect("serialize"));
    } else {
        println!(
            "depends-on review under {} (SubtreeScope, {} strand{})",
            shorten(&root_id),
            reviews.len(),
            if reviews.len() == 1 { "" } else { "s" }
        );
        if reviews.is_empty() {
            println!("  (none)");
        } else {
            for review in &reviews {
                print_depends_review_text(review);
            }
        }
    }
    Ok(())
}

fn print_depends_review_text(review: &graph::DependsReview) {
    println!(
        "depends-on review: {}  {}",
        shorten(&review.id),
        review.summary.chars().take(50).collect::<String>()
    );
    println!(
        "  upstreams: {} ({} registered)",
        review.upstream_count, review.registered_upstream_count
    );
    if review.upstreams.is_empty() {
        println!("  (none)");
    } else {
        for up in &review.upstreams {
            println!(
                "    [{}] {}  {}",
                up.lifecycle,
                shorten(&up.id),
                up.summary.chars().take(45).collect::<String>()
            );
            println!(
                "      last: {}",
                up.last_entry.chars().take(60).collect::<String>()
            );
            println!("      show: {}", up.show_command);
        }
    }
}

/// Query-command family: cmd_list, cmd_show, cmd_search, cmd_timeline,
/// cmd_orient, cmd_agent_context, cmd_tree (+ print_tree_text helper).
/// Moved from main.rs (Layer 4b refactor); function bodies are byte-identical to
/// the originals (only cross-module path qualification added where required).
///
/// Dependency direction: query -> journal, projection, render, tree, output, event (via crate::*)
/// query <- main.rs (mod commands; pub(crate) use commands::query::*)
use crate::journal::*;
use crate::render::*;
use crate::projection;
use crate::tree;
use crate::output;
use crate::event::{Event, TimelineEventKind, find_strand};
use crate::util::{truncate, shorten, parse_duration};
use serde_json::json;
use std::time::Instant;

pub(crate) fn cmd_list(include_hidden: bool, links: Option<&str>, backlinks: Option<&str>, state: Option<&str>, list_type: Option<&str>, stale: Option<&str>, stale_offset: Option<usize>, since_offset: Option<usize>, format_json: bool) -> Result<(), String> {
    let started = Instant::now();
    let path = ensure_journal()?;
    let (events, skipped) = read_events_lossy(&path);
    let mut strands = projection::project_strands(&events, include_hidden);
    // Most recent last-append first
    strands.sort_by(|a, b| b.last_ts().cmp(&a.last_ts()));

    // --type: filter by strand_type (from StrandCreated event)
    if let Some(ref type_filter) = list_type {
        strands.retain(|n| n.strand_type.as_deref() == Some(type_filter));
    }

    // --links: filter strands that source links to
    if let Some(ref src) = links {
        let source_edges: Vec<String> = strands.iter()
            .filter(|n| n.id.starts_with(*src))
            .flat_map(|n| n.edges.iter().cloned())
            .collect();
        strands.retain(|n| source_edges.iter().any(|e| n.id.starts_with(e)));
    }
    // --backlinks: filter strands that link to target
    if let Some(ref tgt) = backlinks {
        strands.retain(|n| n.edges.iter().any(|e| e.starts_with(*tgt)));
    }
    // --state: filter by canonical state.
    // "open" matches registered; disposition names (done/failed/cancelled/merged/verified)
    // match closed:* strands; "closed" matches any closed strand.
    if let Some(ref state_filter) = state {
        strands.retain(|n| {
            match *state_filter {
                // "open" is not a canonical state; match default (registered)
                "open" => n.state() == "registered",
                // "closed" matches any closed strand regardless of disposition
                "closed" => n.state().starts_with("closed:"),
                // disposition shorthand: "done" matches "closed:done", etc.
                s if projection::CLOSE_DISPOSITIONS.contains(&s) => {
                    n.state() == format!("closed:{}", s)
                }
                _ => n.state() == *state_filter,
            }
        });
    }

    // --stale: filter by silence duration
    if let Some(dur_str) = stale {
        let secs = parse_duration(dur_str)?;
        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(secs as i64);
        let cutoff_str = cutoff.to_rfc3339();
        strands.retain(|n| {
            let last_ts = n.last_ts();
            if last_ts.is_empty() { return false; }
            last_ts < &cutoff_str
        });
    }

    // --stale-offset: filter by last entry offset (silent)
    if let Some(so) = stale_offset {
        strands.retain(|n| n.last_offset() <= so);
    }

    // --since-offset: filter by last entry offset (updated since)
    if let Some(so) = since_offset {
        strands.retain(|n| n.last_offset() > so);
    }

    if format_json {
        let output = output::StrandListOutput {
            strands: strands.iter()
                .filter(|s| !s.hidden || include_hidden)
                .map(output::StrandListItem::from)
                .collect(),
        };
        println!("{}", serde_json::to_string(&output).expect("serialize"));
        if skipped > 0 {
            eprintln!("[tasktree] WARNING: {} corrupted lines skipped", skipped);
            std::process::exit(2);
        }
        eprintln!("[tasktree] list: {:.0?}", started.elapsed());
        return Ok(());
    }

    for strand in &strands {
        if strand.hidden && !include_hidden {
            continue;
        }
        let type_str = strand.strand_type.as_deref().unwrap_or("");
        let type_info = if type_str.is_empty() { String::new() } else { format!(" [{}]", type_str) };
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
    if skipped > 0 {
        eprintln!("[tasktree] WARNING: {} corrupted lines skipped", skipped);
        std::process::exit(2);
    }
    eprintln!("[tasktree] list: {:.0?}", started.elapsed());
    Ok(())
}

pub(crate) fn cmd_search(query: &str, format_json: bool, include_hidden: bool) -> Result<(), String> {
    let started = Instant::now();
    let path = ensure_journal()?;
    let (events, skipped) = read_events_lossy(&path);
    let q = query.to_lowercase();
    // Honour the include_hidden flag: when false (default), the strand_map
    // is built from visible strands only, and the events loop below skips
    // events belonging to strands not in the map.
    let strands = projection::project_strands(&events, include_hidden);
    let strand_map: std::collections::HashMap<&str, &projection::ProjectedStrand> =
        strands.iter().map(|s| (s.id.as_str(), s)).collect();

    let mut found = 0;
    let mut matches: Vec<output::SearchMatch> = Vec::new();

    for (_, event) in &events {
        if let Event::LogAppended { content, .. } = event {
            if content.to_lowercase().contains(&q) {
                let strand_id = event.strand_id().to_string();
                // Skip matches inside strands the projection filtered out
                // (i.e. hidden strands when include_hidden is false).
                if !strand_map.contains_key(strand_id.as_str()) {
                    continue;
                }
                let projected = strand_map.get(strand_id.as_str());
                if format_json {
                    matches.push(output::SearchMatch {
                        strand_id,
                        content: truncate(content, 70),
                        strand_type: projected.and_then(|s| s.strand_type.clone()),
                        hidden: projected.map(|s| s.hidden).unwrap_or(false),
                    });
                } else {
                    println!(
                        "{}  {}",
                        shorten(&strand_id),
                        truncate(content, 70)
                    );
                }
                found += 1;
            }
        }
    }

    if format_json {
        let output = output::SearchOutput {
            matches,
            count: found,
            query: query.to_string(),
        };
        println!("{}", serde_json::to_string(&output).expect("serialize"));
    } else if found == 0 {
        println!("(no matches for: {})", query);
    }

    if skipped > 0 {
        eprintln!("[tasktree] WARNING: {} corrupted lines skipped", skipped);
        std::process::exit(2);
    }
    eprintln!(
        "[tasktree] search: {:.0?}  ({} matches)",
        started.elapsed(),
        found
    );
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
    tree_root: Option<&str>,
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
    // since_ts: convert to approximate offset
    if let Some(st) = since_ts {
        let first_idx = entries.iter().position(|e| e.ts.as_str() >= st);
        if let Some(idx) = first_idx {
            entries.drain(0..idx);
        }
    }
    if let Some(ut) = until_ts {
        entries.retain(|e| e.ts.as_str() <= ut);
    }

    // Filter by strand or links
    if let Some(sid) = strand {
        let full_id = find_strand(&events, sid).ok_or_else(|| format!("strand {} not found", sid))?;
        entries.retain(|e| e.strand_id == full_id);
    }
    if let Some(lid) = links {
        let full_id = find_strand(&events, lid).ok_or_else(|| format!("strand {} not found", lid))?;
        // Collect linked strand IDs
        let mut linked_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        linked_ids.insert(full_id.clone());
        for (_, event) in &events {
            if let Event::EdgeLinked { id, to, .. } = event {
                if *id == full_id {
                    linked_ids.insert(to.clone());
                }
            }
        }
        entries.retain(|e| linked_ids.contains(&e.strand_id));
    }

    // Filter by subtree — only events from strands reachable from root via edges
    if let Some(root_id) = tree_root {
        let strands = projection::project_strands(&events, true);
        if let Some(tree_ids) = tree::subtree_ids(root_id, &strands) {
            entries.retain(|e| tree_ids.contains(&e.strand_id));
        }
    }

    if let Some(lim) = limit {
        entries.truncate(lim);
    }

    let count = entries.len();
    let max_offset = entries.last().map(|e| e.journal_offset).unwrap_or(0);
    let is_json = format_json == Some("json");

    if is_json {
        println!("{}", json!({
            "timeline": entries,
            "truncated": false,
            "count": count,
            "max_offset": max_offset,
        }));
    } else if entries.is_empty() {
        // No dead ends (design principle): empty result must say something.
        let mut parts: Vec<String> = Vec::new();
        if let Some(so) = since_offset { parts.push(format!("since-offset {}", so)); }
        if let Some(st) = since_ts { parts.push(format!("since-ts {}", st)); }
        if let Some(uo) = until_offset { parts.push(format!("until-offset {}", uo)); }
        if let Some(ut) = until_ts { parts.push(format!("until-ts {}", ut)); }
        if let Some(sid) = strand { parts.push(format!("strand {}", sid)); }
        if let Some(lid) = links { parts.push(format!("links {}", lid)); }
        if let Some(root) = tree_root { parts.push(format!("tree {}", root)); }
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
                TimelineEventKind::SubjectBound { subject_type, subject_id, strand_id } => {
                    format!("bind: {}:{} -> {}", subject_type, subject_id, shorten(strand_id))
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


pub(crate) fn cmd_orient(format: Option<&str>, include_hidden: bool, limit: Option<usize>, show_tree: bool) -> Result<(), String> {
    let started = Instant::now();
    let path = ensure_journal()?;
    let (events, skipped) = read_events_lossy(&path);
    let max_offset = events.last().map(|(o, _)| *o).unwrap_or(0);
    // Always project with include_hidden=true so build_orient can count hidden
    // strands; the visible/hidden split is done inside build_orient.
    let strands = projection::project_strands(&events, true);
    let out = build_orient(&strands, include_hidden, limit.unwrap_or(10), max_offset);

    if show_tree {
        // Build the belongs-to forest from the active strand set
        let strand_cards: Vec<(&projection::ProjectedStrand, output::OrientStrand)> = out
            .active
            .iter()
            .filter_map(|card| {
                strands.iter().find(|s| s.id == card.id).map(|s| (s, card.clone()))
            })
            .collect();
        let roots = tree::build_orient_forest(&strand_cards);
        let tree_out = output::OrientTreeOutput {
            max_offset: out.max_offset,
            roots,
            closed_count: out.closed_count,
            hidden_count: out.hidden_count,
            remind: out.remind.clone(),
        };

        if format == Some("json") {
            println!("{}", serde_json::to_string(&tree_out).expect("serialize"));
        } else {
            println!(
                "journal: max_offset {} | {} active | {} closed | {} hidden (tasktree list)",
                out.max_offset,
                out.active.len(),
                out.closed_count,
                out.hidden_count
            );
            print_orient_forest(&tree_out.roots, 0);
            if out.active.is_empty() {
                println!("(no active strands) — start one: tasktree add \"<summary>\"");
            }
            println!("remind: {}", out.remind);
        }
    } else if format == Some("json") {
        println!("{}", serde_json::to_string(&out).expect("serialize"));
    } else {
        println!(
            "journal: max_offset {} | {} active | {} closed | {} hidden (tasktree list)",
            out.max_offset,
            out.active.len(),
            out.closed_count,
            out.hidden_count
        );
        for s in &out.active {
            let type_info = s
                .strand_type
                .as_deref()
                .map(|t| format!(" [{}]", t))
                .unwrap_or_default();
            println!("  {}{}  {} entries | last_offset {}", shorten(&s.id), type_info, s.entry_count, s.last_offset);
            println!("    {}", s.summary);
            if s.entry_count > 1 {
                println!("    last: {}", s.last_entry);
            }
            println!("    catch-up: {}", s.catch_up);
        }
        if out.active.is_empty() {
            println!("(no active strands) — start one: tasktree add \"<summary>\"");
        }
        println!("remind: {}", out.remind);
    }

    if skipped > 0 {
        eprintln!("[tasktree] WARNING: {} corrupted lines skipped", skipped);
        std::process::exit(2);
    }
    eprintln!("[tasktree] orient: {:.0?}", started.elapsed());
    Ok(())
}


pub(crate) fn cmd_agent_context(format_json: Option<&str>, include_hidden: bool) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _skipped) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, include_hidden);

    let mut prompt_strands: Vec<_> = strands
        .iter()
        .filter(|s| s.strand_type.as_deref() == Some("prompt-strand"))
        .collect();
    prompt_strands.sort_by(|a, b| b.last_offset().cmp(&a.last_offset()));

    let last_session_offset = strands
        .iter()
        .filter(|s| s.strand_type.as_deref() == Some("session"))
        .map(|s| s.last_offset())
        .max()
        .unwrap_or(0);

    let timeline_since_last_session: Vec<_> = projection::project_timeline(&events)
        .into_iter()
        .filter(|e| e.journal_offset > last_session_offset)
        .collect();

    let prompt_strand_json: Vec<_> = prompt_strands
        .iter()
        .map(|s| json!({
            "id": s.id,
            "entry_count": s.log_count(),
            "first_summary": s.first_summary(),
            "last_summary": s.last_summary(),
            "last_entry_offset": s.last_offset(),
            "last_entry_ts": s.last_ts(),
            "status": s.state(),
            "hidden": s.hidden,
        }))
        .collect();

    if format_json == Some("json") {
        println!("{}", json!({
            "prompt_strands": prompt_strand_json,
            "last_session_offset": last_session_offset,
            "timeline_since_last_session": timeline_since_last_session,
        }));
    } else {
        println!("prompt_strands: {}", prompt_strands.len());
        println!("last_session_offset: {}", last_session_offset);
        println!("timeline_since_last_session: {}", timeline_since_last_session.len());
        println!("\nUse JSON for machine startup context:\n  tasktree agent-context --format json");
    }
    Ok(())
}

pub(crate) fn cmd_show(id: Option<&str>, last: bool, tail: Option<usize>, format_json: bool, locked: bool, digest: bool) -> Result<(), String> {
    let started = Instant::now();
    let path = ensure_journal()?;
    let (events, skipped) = if locked {
        read_events_lossy_locked()
    } else {
        read_events_lossy(&path)
    };
    let strands = projection::project_strands(&events, true);

    let strand = if last {
        // Show most recently active strand
        if id.is_some() {
            return Err("choose one: positional id or --last, not both".to_string());
        }
        if strands.is_empty() {
            return Err("no strands found".to_string());
        }
        let mut sorted: Vec<_> = strands.iter().collect();
        sorted.sort_by(|a, b| b.last_ts().cmp(&a.last_ts()));
        sorted.into_iter().next().unwrap()
    } else {
        let id_str = id.ok_or("provide a strand id or use --last")?;
        let full = find_strand(&events, id_str)
            .ok_or_else(|| format!("strand {} not found", id_str))?;
        strands.iter().find(|s| s.id == full).unwrap()
    };

    // Summary
    let entry_count = strand.log_count();
    let last_summary = strand.last_summary();
    let canonical_state = strand.state();

    if format_json {
        let output = output::StrandDetailOutput::from(strand);
        println!("{}", serde_json::to_string(&output).expect("serialize"));
        if skipped > 0 {
            eprintln!("[tasktree] WARNING: {} corrupted lines skipped", skipped);
            std::process::exit(2);
        }
        return Ok(());
    }

    println!(
        "strand: {} | {} entries | state: {} | last_entry_offset: {}",
        shorten(&strand.id),
        entry_count,
        canonical_state,
        strand.last_offset()
    );
    println!("summary: {}", truncate(strand.first_summary(), 60));
    println!("next: {}", truncate(last_summary, 100));
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
        let census = if parts.is_empty() { "—".to_string() } else { parts.join(", ") };
        println!("markers: {}", census);
        eprintln!(
            "[tasktree] show:   {:.0?}  (digest, {} entries)",
            started.elapsed(),
            entry_count
        );
        if skipped > 0 {
            eprintln!("[tasktree] WARNING: {} corrupted lines skipped", skipped);
            std::process::exit(2);
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
    for entry in slice {
        let ref_str = entry
            .ref_
            .as_ref()
            .map(|r| format!(" [ref: {}]", r))
            .unwrap_or_default();
        let id_str = entry
            .append_id
            .as_ref()
            .map(|a| format!(" [{}]", &a[..12]))
            .unwrap_or_default();
        println!(
            "  [{}]{} {}{}",
            &entry.ts[..19],
            id_str,
            entry.content,
            ref_str
        );
    }
    eprintln!(
        "[tasktree] show:   {:.0?}  ({} entries, {} shown)",
        started.elapsed(),
        entry_count,
        shown
    );
    if skipped > 0 {
        eprintln!("[tasktree] WARNING: {} corrupted lines skipped", skipped);
        std::process::exit(2);
    }
    Ok(())
}

/// Extract the leading `[marker]` token from an entry's content, if present.
/// `"[decision] ..."` -> `Some("decision")`; unmarked content -> `None`.
pub(crate) fn leading_marker(content: &str) -> Option<&str> {
    let trimmed = content.trim_start();
    let rest = trimmed.strip_prefix('[')?;
    let end = rest.find(']')?;
    let token = &rest[..end];
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

// ── Tree projection ─────────────────────────────────────

pub(crate) fn cmd_tree(root_id: &str, format_json: Option<&str>) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _skipped) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);

    match tree::project_tree(root_id, &strands) {
        Some(root) => {
            if format_json == Some("json") {
                let output = tree::TreeOutput { root };
                println!("{}", serde_json::to_string_pretty(&output).unwrap());
            } else {
                print_tree_text(&root, 0);
            }
        }
        None => {
            return Err(format!("strand not found or ambiguous prefix: {}", root_id));
        }
    }
    Ok(())
}

fn print_tree_text(node: &tree::TreeNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let marker = if node.children.is_empty() { "  " } else { "└─" };
    println!("{}{} {} [{}] {}",
        indent, marker,
        &node.id[..12.min(node.id.len())],
        node.status,
        node.summary.chars().take(60).collect::<String>()
    );
    for child in &node.children {
        print_tree_text(child, depth + 1);
    }
}

use crate::event::{find_strand, Event, TimelineEventKind};
use crate::graph;
/// Query-command family: cmd_list, cmd_show, cmd_search, cmd_timeline,
/// cmd_orient, cmd_agent_context, cmd_tree (+ print_tree_text helper).
/// Moved from main.rs (Layer 4b refactor); function bodies are byte-identical to
/// the originals (only cross-module path qualification added where required).
///
/// Dependency direction: query -> journal, projection, render, tree, output, event (via crate::*)
/// query <- main.rs (mod commands; pub(crate) use commands::query::*)
use crate::journal::*;
use crate::markers::leading_marker;
use crate::output;
use crate::projection;
use crate::render::*;
use crate::tree;
use crate::util::{parse_duration, shorten, truncate};
use std::time::Instant;

pub(crate) struct ListRequest<'a> {
    pub(crate) include_hidden: bool,
    pub(crate) links: Option<&'a str>,
    pub(crate) backlinks: Option<&'a str>,
    pub(crate) state: Option<&'a str>,
    pub(crate) list_type: Option<&'a str>,
    pub(crate) stale: Option<&'a str>,
    pub(crate) stale_offset: Option<usize>,
    pub(crate) since_offset: Option<usize>,
}

pub(crate) fn list_strands(
    events: &[(usize, Event)],
    req: &ListRequest<'_>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<projection::ProjectedStrand>, String> {
    let mut strands = projection::project_strands(events, req.include_hidden);
    strands.sort_by(|a, b| b.last_ts().cmp(a.last_ts()));

    if let Some(type_filter) = req.list_type {
        strands.retain(|n| n.strand_type.as_deref() == Some(type_filter));
    }
    if let Some(src) = req.links {
        let source_edges: Vec<String> = strands
            .iter()
            .filter(|n| n.id.starts_with(src))
            .flat_map(|n| n.edges.iter().cloned())
            .collect();
        strands.retain(|n| source_edges.iter().any(|e| n.id.starts_with(e)));
    }
    if let Some(tgt) = req.backlinks {
        strands.retain(|n| n.edges.iter().any(|e| e.starts_with(tgt)));
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
        let secs = parse_duration(dur_str)?;
        let cutoff = now - chrono::Duration::seconds(secs as i64);
        let cutoff_str = cutoff.to_rfc3339();
        strands.retain(|n| {
            let last_ts = n.last_ts();
            !last_ts.is_empty() && last_ts < &cutoff_str
        });
    }
    if let Some(offset) = req.stale_offset {
        strands.retain(|n| n.last_offset() <= offset);
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
    if skipped > 0 {
        eprintln!("[tasktree] WARNING: {} corrupted lines skipped", skipped);
        std::process::exit(2);
    }
    eprintln!("[tasktree] list: {:.0?}", started.elapsed());
    Ok(())
}

pub(crate) struct SearchRequest<'a> {
    pub(crate) query: &'a str,
    pub(crate) include_hidden: bool,
}

pub(crate) struct SearchResult {
    pub(crate) output: output::SearchOutput,
    pub(crate) text_rows: Vec<(String, String)>,
}

pub(crate) fn search_events(events: &[(usize, Event)], req: &SearchRequest<'_>) -> SearchResult {
    let q = req.query.to_lowercase();
    let strands = projection::project_strands(events, req.include_hidden);
    let strand_map: std::collections::HashMap<&str, &projection::ProjectedStrand> =
        strands.iter().map(|s| (s.id.as_str(), s)).collect();

    let mut matches = Vec::new();
    let mut text_rows = Vec::new();
    for (_, event) in events {
        if let Event::LogAppended { content, .. } = event {
            if content.to_lowercase().contains(&q) {
                let strand_id = event.strand_id().to_string();
                if !strand_map.contains_key(strand_id.as_str()) {
                    continue;
                }
                let projected = strand_map.get(strand_id.as_str());
                let content = truncate(content, 70);
                text_rows.push((shorten(&strand_id), content.clone()));
                matches.push(output::SearchMatch {
                    strand_id,
                    content,
                    strand_type: projected.and_then(|s| s.strand_type.clone()),
                    hidden: projected.map(|s| s.hidden).unwrap_or(false),
                });
            }
        }
    }
    let count = matches.len();
    SearchResult {
        output: output::SearchOutput {
            matches,
            count,
            query: req.query.to_string(),
        },
        text_rows,
    }
}

pub(crate) fn cmd_search(
    query: &str,
    format_json: bool,
    include_hidden: bool,
) -> Result<(), String> {
    let started = Instant::now();
    let path = ensure_journal()?;
    let (events, skipped) = read_events_lossy(&path);
    let request = SearchRequest {
        query,
        include_hidden,
    };
    let result = search_events(&events, &request);

    if format_json {
        println!(
            "{}",
            serde_json::to_string(&result.output).expect("serialize")
        );
    } else if result.output.count == 0 {
        println!("(no matches for: {})", query);
    } else {
        for (id, content) in &result.text_rows {
            println!("{}  {}", id, content);
        }
    }

    if skipped > 0 {
        eprintln!("[tasktree] WARNING: {} corrupted lines skipped", skipped);
        std::process::exit(2);
    }
    eprintln!(
        "[tasktree] search: {:.0?}  ({} matches)",
        started.elapsed(),
        result.output.count
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
        let full_id =
            find_strand(&events, sid).ok_or_else(|| format!("strand {} not found", sid))?;
        entries.retain(|e| e.strand_id == full_id);
    }
    if let Some(lid) = links {
        let full_id =
            find_strand(&events, lid).ok_or_else(|| format!("strand {} not found", lid))?;
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

    // Capture pre-truncation length so `truncated` tells the truth (F1): a
    // hardcoded `false` here silently dropped events on the jq consumption path
    // (--limit + pagination loops keying on `truncated` would miss the tail).
    let pre_truncate_len = entries.len();
    if let Some(lim) = limit {
        entries.truncate(lim);
    }
    let truncated = entries.len() < pre_truncate_len;

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
        if let Some(root) = tree_root {
            parts.push(format!("tree {}", root));
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
    pub(crate) limit: usize,
}

pub(crate) struct OrientPlan {
    pub(crate) strands: Vec<projection::ProjectedStrand>,
    pub(crate) view: projection::OrientView,
    pub(crate) output: output::OrientOutput,
}

pub(crate) fn orient_plan(events: &[(usize, Event)], req: &OrientRequest) -> OrientPlan {
    let max_offset = events.last().map(|(o, _)| *o).unwrap_or(0);
    let strands = projection::project_strands(events, true);
    let view = projection::build_orient_view(&strands, req.include_hidden, req.limit, max_offset);
    let output = output::OrientOutput::from((&view, strands.as_slice()));
    OrientPlan {
        strands,
        view,
        output,
    }
}
pub(crate) fn cmd_orient(
    format: Option<&str>,
    include_hidden: bool,
    limit: Option<usize>,
    show_tree: bool,
) -> Result<(), String> {
    let started = Instant::now();
    let path = ensure_journal()?;
    let (events, skipped) = read_events_lossy(&path);
    let request = OrientRequest {
        include_hidden,
        limit: limit.unwrap_or(10),
    };
    let plan = orient_plan(&events, &request);
    let strands = &plan.strands;
    let view = &plan.view;
    let out = &plan.output;

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
            println!(
                "  {}{}  {} entries | last_offset {}",
                shorten(&s.id),
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

pub(crate) fn cmd_agent_context(
    format_json: Option<&str>,
    include_hidden: bool,
) -> Result<(), String> {
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

    if format_json == Some("json") {
        let output = output::AgentContextOutput {
            prompt_strands: prompt_strands
                .iter()
                .map(|s| output::AgentContextPromptStrandOutput::from(*s))
                .collect(),
            last_session_offset,
            timeline_since_last_session: timeline_since_last_session
                .iter()
                .map(output::TimelineEntryOutput::from)
                .collect(),
        };
        println!("{}", serde_json::to_string(&output).expect("serialize"));
    } else {
        println!("prompt_strands: {}", prompt_strands.len());
        println!("last_session_offset: {}", last_session_offset);
        println!(
            "timeline_since_last_session: {}",
            timeline_since_last_session.len()
        );
        println!("\nUse JSON for machine startup context:\n  tasktree agent-context --format json");
    }
    Ok(())
}

pub(crate) fn cmd_show(
    id: Option<&str>,
    last: bool,
    tail: Option<usize>,
    format_json: bool,
    locked: bool,
    digest: bool,
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
        let full =
            find_strand(&events, id_str).ok_or_else(|| format!("strand {} not found", id_str))?;
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
        let census = if parts.is_empty() {
            "—".to_string()
        } else {
            parts.join(", ")
        };
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

// ── Tree projection ─────────────────────────────────────

pub(crate) fn cmd_tree(root_id: &str, format_json: Option<&str>) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _skipped) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);

    match tree::project_tree(root_id, &strands) {
        Some(root) => {
            if format_json == Some("json") {
                let output = output::TreeOutput::from(&root);
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

/// depends-on DAG analysis for one strand (F6 / W2): direct blockers and their
/// state, readiness (all direct blockers closed), and the critical path — the
/// longest chain of still-open upstreams. Built on the F3 typed projection.
pub(crate) fn cmd_depends(id: &str, format_json: Option<&str>) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _skipped) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let graph = graph::StrandGraph::from_strands(&strands);
    let analysis = graph
        .depends_analysis(id)
        .ok_or_else(|| format!("strand {} not found", id))?;

    if format_json == Some("json") {
        let output = output::DependsOutput::from(&analysis);
        println!("{}", serde_json::to_string(&output).expect("serialize"));
    } else {
        println!(
            "depends-on analysis: {}  {}",
            shorten(&analysis.id),
            analysis.summary.chars().take(50).collect::<String>()
        );
        println!(
            "  ready: {}  ({} open blocker(s))",
            if analysis.ready { "yes" } else { "no" },
            analysis.open_blocker_count
        );
        if analysis.blockers.is_empty() {
            println!("  direct blockers: (none)");
        } else {
            println!("  direct blockers:");
            for b in &analysis.blockers {
                let mark = if b.closed { "closed" } else { "OPEN  " };
                println!(
                    "    [{}] {}  {}",
                    mark,
                    shorten(&b.id),
                    b.summary.chars().take(45).collect::<String>()
                );
            }
        }
        if analysis.critical_path.is_empty() {
            println!("  critical path: (none - no open upstreams)");
        } else {
            let chain: Vec<String> = analysis.critical_path.iter().map(|c| shorten(c)).collect();
            println!(
                "  critical path (longest open chain, len {}): {}",
                analysis.critical_path.len(),
                chain.join(" -> ")
            );
        }
    }
    Ok(())
}

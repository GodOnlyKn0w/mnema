/// Doctor command family: cmd_doctor_journal.
/// Moved from main.rs (Layer 5-shape refactor) — the last cmd_* handler that
/// still lived in main.rs; every other handler already lives under commands/.
/// Behaviour-preserving relocation: the inline lint passes and W-code call are
/// unchanged. Folding the lint passes into diagnostics.rs is deliberately left
/// as separate follow-up work.
use crate::event::Event;
use crate::journal::*;
use crate::{diagnostics, projection};
use std::time::Instant;

pub(crate) fn cmd_doctor_journal() -> Result<bool, String> {
    let path = resolve_journal_dir()?.join("journal.jsonl");

    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read journal: {}", e))?;

    let lines: Vec<&str> = raw.lines().collect();
    let total_lines = lines.len();
    let mut corrupted = 0usize;
    let mut events: Vec<Event> = Vec::new();
    for line in &lines {
        if line.trim().is_empty() { continue; }
        match serde_json::from_str::<Event>(line) {
            Ok(event) => events.push(event),
            Err(_) => corrupted += 1,
        }
    }

    use std::collections::{HashMap, HashSet};
    let mut created_ids: HashSet<String> = HashSet::new();
    let mut appended_ids: HashSet<String> = HashSet::new();
    let mut strand_event_counts: HashMap<String, usize> = HashMap::new();
    for event in &events {
        match event {
            Event::StrandCreated { id, .. } => { created_ids.insert(id.clone()); }
            Event::LogAppended { id, .. } => { appended_ids.insert(id.clone()); *strand_event_counts.entry(id.clone()).or_insert(0) += 1; }
            _ => {}
        }
    }
    let total_strands = created_ids.len();
    let with_events: Vec<_> = created_ids.iter().filter(|id| strand_event_counts.contains_key(*id)).collect();
    let noise_strands: Vec<_> = created_ids.iter().filter(|id| !strand_event_counts.contains_key(*id)).collect();
    let orphans: Vec<_> = appended_ids.iter().filter(|id| !created_ids.contains(*id)).collect();

    let mut git_head_count = 0usize;
    for line in &lines {
        if line.trim().is_empty() { continue; }
        if line.contains("git_head") || line.contains("git.head") { git_head_count += 1; }
    }

    let state_path = resolve_journal_dir()?.join("doctor-state.json");
    let timeline_status = if state_path.exists() {
        match std::fs::read_to_string(&state_path) {
            Ok(content) => {
                #[derive(serde::Deserialize)] struct DoctorState { line_count: usize }
                match serde_json::from_str::<DoctorState>(&content) {
                    Ok(state) => {
                        if total_lines < state.line_count {
                            format!("warning: {}->{} jump detected (lines decreased)", state.line_count, total_lines)
                        } else if total_lines > state.line_count {
                            format!("monotonic: yes ({}->{})", state.line_count, total_lines)
                        } else {
                            "monotonic: yes (unchanged)".to_string()
                        }
                    }
                    Err(_) => "monotonic: yes (no previous state)".to_string(),
                }
            }
            Err(_) => "monotonic: yes (cannot read previous state)".to_string(),
        }
    } else { "monotonic: yes (first run, no previous state)".to_string() };

    #[derive(serde::Serialize)] struct DoctorStateOut { line_count: usize, updated_at: String }
    let state = DoctorStateOut { line_count: total_lines, updated_at: chrono::Utc::now().to_rfc3339() };
    if let Ok(json) = serde_json::to_string_pretty(&state) { let _ = std::fs::write(&state_path, json); }

    println!("Doctor Journal Report");
    println!("  journal: {}", path.display());
    println!("  lines: {}, corrupted: {}, orphan events: {}", total_lines, corrupted, orphans.len());
    println!();
    println!("  strand coverage:");
    println!("    total strands: {}", total_strands);
    println!("    with events: {}", with_events.len());
    println!("    noise strands (no events): {}", noise_strands.len());
    println!();
    println!("  git context:");
    let pct = if total_lines > 0 { (git_head_count as f64 / total_lines as f64) * 100.0 } else { 0.0 };
    println!("    entries with git.head: {}/{} ({:.0}%)", git_head_count, total_lines, pct);
    println!();
    println!("  timeline:");
    println!("    {}", timeline_status);
    if !orphans.is_empty() {
        println!(); println!("  orphans:");
        for id in &orphans { println!("    {}  (log_appended, no strand_created)", id); }
    }

    // ── lint passes ─────────────────────────────────────
    let mut lint_count = 0usize;

    // Build strand summary map (first LogAppended content for each strand)
    let mut strand_summaries: HashMap<String, String> = HashMap::new();
    let mut strand_entries: HashMap<String, Vec<String>> = HashMap::new();
    for event in &events {
        if let Event::LogAppended { id, content, .. } = event {
            strand_summaries.entry(id.clone()).or_insert_with(|| content.clone());
            strand_entries.entry(id.clone()).or_default().push(content.clone());
        }
    }

    // Lint 1: DAG strands must not have [done] entries
    println!(); println!("  lint: dag-done:");
    for (id, summary) in &strand_summaries {
        if summary.starts_with("para group ") {
            if let Some(entries) = strand_entries.get(id) {
                let has_done = entries.iter().any(|e| e.contains("[done]"));
                if has_done {
                    eprintln!("[lint] DAG strand {} has [done] entry — DAG should only record layer events", id);
                    lint_count += 1;
                }
            }
        }
    }
    println!("    dag strands with [done]: {}", lint_count);

    // Lint 2: Task strands must not have task_created JSON events
    let mut task_lint_count = 0usize;
    println!("  lint: task-created:");
    for (id, summary) in &strand_summaries {
        if summary.starts_with('[') {
            if let Some(entries) = strand_entries.get(id) {
                let has_task_created = entries.iter().any(|e| e.contains("task_created"));
                if has_task_created {
                    eprintln!("[lint] Task strand {} has task_created JSON event — task strands should not have DAG events", id);
                    task_lint_count += 1;
                }
            }
        }
    }
    println!("    task strands with task_created: {}", task_lint_count);
    lint_count += task_lint_count;

    // Lint 3: Orphan links (EdgeLinked target doesn't exist)
    let mut orphan_link_count = 0usize;
    println!("  lint: orphan-links:");
    for event in &events {
        if let Event::EdgeLinked { to, .. } = event {
            if !created_ids.contains(to) {
                eprintln!("[lint] orphan link: target strand {} not found", to);
                orphan_link_count += 1;
            }
        }
    }
    println!("    orphan links: {}", orphan_link_count);
    lint_count += orphan_link_count;

    // Lint 4: [touches] format — known fields only
    let mut touches_format_count = 0usize;
    println!("  lint: touches-format:");
    for entries in strand_entries.values() {
        for entry in entries {
            if let Some(tail) = entry.strip_prefix("[touches] ") {
                for part in tail.split(' ') {
                    if part.is_empty() { continue; }
                    let field = part.split(':').next().unwrap_or("");
                    if field != "write" && field != "read" && field != "creates" && field != "readonly" {
                        eprintln!("[lint] touches format: unrecognized field '{}' in [touches] entry", field);
                        touches_format_count += 1;
                    }
                }
            }
        }
    }
    println!("    unrecognized touches fields: {}", touches_format_count);
    lint_count += touches_format_count;

    // Lint 5: link direction — source and target identity
    let mut link_direction_count = 0usize;
    println!("  lint: link-direction:");
    for event in &events {
        if let Event::EdgeLinked { id: source, to: target, .. } = event {
            let src_summary = strand_summaries.get(source).map(|s| s.as_str()).unwrap_or("");
            let tgt_summary = strand_summaries.get(target).map(|s| s.as_str()).unwrap_or("");
            let src_is_dag = src_summary.starts_with("para group ");
            let src_is_task = src_summary.starts_with('[') && src_summary[1..].chars().next().map_or(false, |c| c.is_ascii_digit());
            let tgt_is_dag = tgt_summary.starts_with("para group ");
            // task→DAG unusual (DAG should link to tasks, not vice versa)
            if src_is_task && tgt_is_dag {
                eprintln!("[lint] link direction: task {} links to DAG {} — unusual", source, target);
                link_direction_count += 1;
            }
            // session→DAG unusual
            if !src_is_dag && !src_is_task && tgt_is_dag {
                eprintln!("[lint] link direction: non-task {} links to DAG {} — unusual", source, target);
                link_direction_count += 1;
            }
        }
    }
    println!("    unusual link directions: {}", link_direction_count);
    lint_count += link_direction_count;

    // Lint 6: strand identity — first entry matches strand type
    let mut identity_count = 0usize;
    println!("  lint: strand-identity:");
    for (id, summary) in &strand_summaries {
        let is_dag = summary.starts_with("para group ");
        let is_task = summary.starts_with('[') && summary.chars().nth(1).map_or(false, |c| c.is_ascii_digit());
        if let Some(entries) = strand_entries.get(id) {
            // DAG strand: all entries should be para layer events (no [NN], no [done])
            if is_dag {
                let has_task_marker = entries.iter().any(|e| {
                    e.starts_with('[') && e.chars().nth(1).map_or(false, |c| c.is_ascii_digit())
                });
                if has_task_marker {
                    eprintln!("[lint] strand identity: DAG strand {} has task-like entries — identity mismatch", id);
                    identity_count += 1;
                }
            }
            // Task strand: all entries should be task layer events (no para group events)
            if is_task {
                // Only warn on para group prefix (task_created already covered by lint 2)
                let has_para_prefix = entries.iter().any(|e| e.starts_with("para group "));
                if has_para_prefix {
                    eprintln!("[lint] strand identity: task strand {} has DAG-like entries — identity mismatch", id);
                    identity_count += 1;
                }
            }
        }
    }
    println!("    identity mismatches: {}", identity_count);
    lint_count += identity_count;

    // Lint 7: edge-validity (F7) — semantic predicates over the causal graph.
    // Advisory only: read-time warnings, never persisted as scars. The system
    // flags suspicious edges; the cut/keep decision stays with the agent/llm.
    // Built from the typed projection so it follows EdgeUnlinked folds (F5).
    {
        let indexed: Vec<(usize, Event)> = events.iter().cloned().enumerate().collect();
        let strands = projection::project_strands(&indexed, true);
        let mut by_id: HashMap<String, &projection::ProjectedStrand> = HashMap::new();
        for s in &strands { by_id.insert(s.id.clone(), s); }

        let mut ev_count = 0usize;
        println!("  lint: edge-validity:");

        // 7a. belongs-to cardinality (D1: single-parent basis; >1 = warning, not error).
        for s in &strands {
            if s.belongs_to_edges.len() > 1 {
                eprintln!("[lint] edge-validity: strand {} has {} belongs-to parents — single-parent basis (D1) expects 1", s.id, s.belongs_to_edges.len());
                ev_count += 1;
            }
        }

        // 7b. dead parent / dead upstream: a belongs-to or depends-on target is closed.
        for s in &strands {
            for p in &s.belongs_to_edges {
                if by_id.get(p).map_or(false, |t| t.state().starts_with("closed")) {
                    eprintln!("[lint] edge-validity: strand {} belongs-to a closed parent {} — may warrant review", s.id, p);
                    ev_count += 1;
                }
            }
            for u in &s.depends_on_edges {
                if by_id.get(u).map_or(false, |t| t.state().starts_with("closed")) {
                    eprintln!("[lint] edge-validity: strand {} depends-on a closed upstream {} — may warrant review", s.id, u);
                    ev_count += 1;
                }
            }
        }

        // 7c. depends-on cycle (genuine deadlock). Iterative DFS with 3-color
        // marking: a back-edge to an in-stack (color 1) node closes a cycle.
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();
        for s in &strands { adj.insert(s.id.clone(), s.depends_on_edges.clone()); }
        let mut color: HashMap<String, u8> = HashMap::new();
        for start in adj.keys().cloned().collect::<Vec<_>>() {
            if color.get(&start).copied().unwrap_or(0) != 0 { continue; }
            let mut stack: Vec<(String, usize)> = vec![(start.clone(), 0)];
            color.insert(start.clone(), 1);
            while let Some((node, idx)) = stack.last().cloned() {
                let children = adj.get(&node).cloned().unwrap_or_default();
                if idx < children.len() {
                    stack.last_mut().unwrap().1 += 1;
                    let nx = children[idx].clone();
                    match color.get(&nx).copied().unwrap_or(0) {
                        1 => { eprintln!("[lint] edge-validity: depends-on cycle edge {} -> {} — deadlock", node, nx); ev_count += 1; }
                        0 => { color.insert(nx.clone(), 1); stack.push((nx, 0)); }
                        _ => {}
                    }
                } else {
                    color.insert(node.clone(), 2);
                    stack.pop();
                }
            }
        }

        println!("    edge-validity warnings: {}", ev_count);
        lint_count += ev_count;
    }

    // Lint 8: [metric] capturability (F8) — entries prefixed `[metric] ` should
    // carry a jq-capturable `name=value` (the explain jq idiom). Flag ones that
    // don't, so the miss is *told*, not silent. Convention discipline — NOT
    // regex-widening (the doctrine's minimal core stays minimal).
    let mut metric_count = 0usize;
    println!("  lint: metric-format:");
    for (id, entries) in &strand_entries {
        for e in entries {
            if let Some(rest) = e.strip_prefix("[metric] ") {
                let ok = rest.split_whitespace().any(|tok| {
                    if let Some((k, v)) = tok.split_once('=') {
                        !k.is_empty()
                            && k.chars().all(|c| c.is_alphanumeric() || c == '_')
                            && !v.is_empty()
                    } else {
                        false
                    }
                });
                if !ok {
                    let preview: String = rest.chars().take(40).collect();
                    eprintln!("[lint] metric-format: strand {} [metric] entry has no jq-capturable name=value: {:?}", id, preview);
                    metric_count += 1;
                }
            }
        }
    }
    println!("    uncapturable [metric] entries: {}", metric_count);
    lint_count += metric_count;

    // Lint 9: legacy `why` edges (F4 migration). D2 removed `why` from the edge
    // system (it is now an entry rationale). Any EdgeLinked with edge_type="why"
    // predates the decision — flag it so the reason can move into an entry.
    let mut why_edge_count = 0usize;
    println!("  lint: legacy-why-edges:");
    for event in &events {
        if let Event::EdgeLinked { id, to, edge_type, .. } = event {
            if edge_type.as_deref() == Some("why") {
                eprintln!("[lint] legacy why-edge {} -> {}: why is no longer a link (D2) — record the reason in an entry", id, to);
                why_edge_count += 1;
            }
        }
    }
    println!("    legacy why-edges: {}", why_edge_count);
    lint_count += why_edge_count;

    // Lint 10: why-staleness clerk (W1/F4-pin). Entries set by `append --why`
    // carry a pinned rationale ref `<id>@<offset>`. When the cited strand has
    // advanced past the pinned offset, the basis evolved — flag "may warrant
    // review". Clerk, not judge: it reports only that the clue moved; whether the
    // reason still holds is left to the llm/human (D2 / W076 lineage).
    {
        let indexed: Vec<(usize, Event)> = events.iter().cloned().enumerate().collect();
        let strands = projection::project_strands(&indexed, true);
        let mut cur_offset: HashMap<String, usize> = HashMap::new();
        for s in &strands { cur_offset.insert(s.id.clone(), s.last_offset()); }
        let mut stale_count = 0usize;
        println!("  lint: why-staleness:");
        for s in &strands {
            for entry in &s.log {
                if let Some(r) = &entry.ref_ {
                    if let Some((tgt, pin)) = r.rsplit_once('@') {
                        if let Ok(pin_off) = pin.parse::<usize>() {
                            if let Some(&cur) = cur_offset.get(tgt) {
                                if cur > pin_off {
                                    eprintln!("[lint] why-staleness: strand {} cites {} pinned@{} but it advanced to @{} — may warrant review", s.id, tgt, pin_off, cur);
                                    stale_count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        println!("    stale rationale refs: {}", stale_count);
        lint_count += stale_count;
    }

    if lint_count > 0 {
        println!();
        println!("  lint summary: {} issue(s) found (warnings only, not blocking)", lint_count);
    }

    // ── W-code diagnostics ──────────────────────────────
    let diags = diagnostics::run_journal_diagnostics(&events, chrono::Utc::now());
    println!();
    println!("  diagnostics:");
    if diags.is_empty() {
        println!("    (none)");
    } else {
        for (code, detail) in &diags {
            println!("    {} {}  (tasktree explain {})", code, detail, code);
        }
    }

    // Measure fresh projection timing
    let projection_start = Instant::now();
    let (journal_events, _) = read_events_lossy(&path);
    let _strands = projection::project_strands(&journal_events, true);
    let projection_ms = projection_start.elapsed().as_millis();
    println!();
    println!("  projection_ms: {}", projection_ms);
    println!("  total_lines: {}", total_lines);
    println!("  total_events: {}", journal_events.len());

    let has_issues = corrupted > 0 || !orphans.is_empty() || timeline_status.contains("warning") || lint_count > 0 || !diags.is_empty();
    Ok(has_issues)
}

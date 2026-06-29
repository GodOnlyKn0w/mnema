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

    // -- lint and W-code diagnostics -------------------------------------
    let audit = diagnostics::audit_journal(&events, chrono::Utc::now());
    for section in &audit.lint_sections {
        println!("  lint: {}:", section.name);
        for finding in &section.findings {
            eprintln!("[lint] {}", finding);
        }
        println!("    {}: {}", section.summary_label, section.count());
    }

    let lint_count = audit.lint_count();
    if lint_count > 0 {
        println!();
        println!("  lint summary: {} issue(s) found (warnings only, not blocking)", lint_count);
    }

    println!();
    println!("  diagnostics:");
    if audit.diagnostics.is_empty() {
        println!("    (none)");
    } else {
        for (code, detail) in &audit.diagnostics {
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

    let has_issues = corrupted > 0 || !orphans.is_empty() || timeline_status.contains("warning") || lint_count > 0 || !audit.diagnostics.is_empty();
    Ok(has_issues)
}



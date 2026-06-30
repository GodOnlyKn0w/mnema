/// Doctor command family: cmd_doctor_journal.
///
/// The command owns filesystem IO and text rendering. Journal health facts and
/// audit findings are built in diagnostics as read-side projections.
use crate::event::Event;
use crate::journal::*;
use crate::{diagnostics, projection};
use std::time::Instant;

pub(crate) fn cmd_doctor_journal(strict: bool) -> Result<bool, String> {
    let journal_dir = resolve_journal_dir()?;
    let path = journal_dir.join("journal.jsonl");

    let raw = std::fs::read_to_string(&path).map_err(|e| format!("cannot read journal: {}", e))?;

    let lines: Vec<&str> = raw.lines().collect();
    let total_lines = lines.len();
    let (events, corrupted) = parse_journal_lines(&lines);
    let (git_head_count, git_context_event_count) = count_git_context_events(&events);

    let state_path = journal_dir.join("doctor-state.json");
    let previous_state = read_previous_state(&state_path);
    write_current_state(&state_path, total_lines);

    let report = diagnostics::build_doctor_journal_report(
        &events,
        total_lines,
        corrupted,
        git_head_count,
        git_context_event_count,
        previous_state,
        chrono::Utc::now(),
    );

    render_doctor_report(&path, &report);

    // Measure fresh projection timing.
    let projection_start = Instant::now();
    let (journal_events, _) = read_events_lossy(&path);
    let _strands = projection::project_strands(&journal_events, true);
    let projection_ms = projection_start.elapsed().as_millis();
    println!();
    println!("  projection_ms: {}", projection_ms);
    println!("  total_lines: {}", total_lines);
    println!("  total_events: {}", journal_events.len());

    Ok(report.has_errors() || (strict && report.has_advisories()))
}

fn parse_journal_lines(lines: &[&str]) -> (Vec<Event>, usize) {
    let mut corrupted = 0usize;
    let mut events = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Event>(line) {
            Ok(event) => events.push(event),
            Err(_) => corrupted += 1,
        }
    }
    (events, corrupted)
}

fn count_git_context_events(events: &[Event]) -> (usize, usize) {
    let mut capable = 0usize;
    let mut with_head = 0usize;
    for event in events {
        if let Event::LogAppended { git, .. } = event {
            capable += 1;
            if git.as_ref().map_or(false, |g| !g.head.trim().is_empty()) {
                with_head += 1;
            }
        }
    }
    (with_head, capable)
}

fn read_previous_state(path: &std::path::Path) -> diagnostics::DoctorPreviousState {
    if !path.exists() {
        return diagnostics::DoctorPreviousState::FirstRun;
    }
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return diagnostics::DoctorPreviousState::Unreadable,
    };
    #[derive(serde::Deserialize)]
    struct DoctorState {
        line_count: usize,
    }
    match serde_json::from_str::<DoctorState>(&content) {
        Ok(state) => diagnostics::DoctorPreviousState::LineCount(state.line_count),
        Err(_) => diagnostics::DoctorPreviousState::Invalid,
    }
}

fn write_current_state(path: &std::path::Path, total_lines: usize) {
    #[derive(serde::Serialize)]
    struct DoctorStateOut {
        line_count: usize,
        updated_at: String,
    }
    let state = DoctorStateOut {
        line_count: total_lines,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&state) {
        let _ = std::fs::write(path, json);
    }
}

fn render_doctor_report(path: &std::path::Path, report: &diagnostics::DoctorJournalReport) {
    println!("Doctor Journal Report");
    println!("  journal: {}", path.display());
    println!(
        "  lines: {}, corrupted: {}, orphan events: {}",
        report.total_lines,
        report.corrupted,
        report.orphans.len()
    );
    println!();
    println!("  strand coverage:");
    println!("    total strands: {}", report.total_strands);
    println!("    with events: {}", report.strands_with_events_count);
    println!(
        "    noise strands (no events): {}",
        report.noise_strands_count
    );
    println!();
    println!("  git context:");
    let pct = if report.git_context_event_count > 0 {
        (report.git_head_count as f64 / report.git_context_event_count as f64) * 100.0
    } else {
        0.0
    };
    println!(
        "    entries with git.head: {}/{} ({:.0}%)",
        report.git_head_count, report.git_context_event_count, pct
    );
    println!();
    println!("  timeline:");
    println!("    {}", report.timeline_status);
    if !report.orphans.is_empty() {
        println!();
        println!("  orphans:");
        for id in &report.orphans {
            println!("    {}  (log_appended, no strand_created)", id);
        }
    }

    for section in &report.audit.lint_sections {
        println!("  lint: {}:", section.name);
        for finding in &section.findings {
            eprintln!("[lint] {}", finding);
        }
        println!("    {}: {}", section.summary_label, section.count());
    }

    let lint_count = report.audit.lint_count();
    if lint_count > 0 {
        println!();
        println!(
            "  lint summary: {} issue(s) found (warnings only, not blocking)",
            lint_count
        );
    }

    println!();
    println!("  diagnostics:");
    if report.audit.diagnostics.is_empty() {
        println!("    (none)");
    } else {
        for (code, detail) in &report.audit.diagnostics {
            println!("    {} {}  (tasktree explain {})", code, detail, code);
        }
    }
}

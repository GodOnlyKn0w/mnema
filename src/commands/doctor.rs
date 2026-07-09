/// Doctor command family: cmd_doctor_journal, cmd_doctor_edges.
///
/// The command owns filesystem IO and text rendering. Journal health facts and
/// audit findings are built in diagnostics as read-side projections.
use crate::event::Event;
use crate::journal::*;
use crate::output;
use crate::util::shorten;
use crate::{diagnostics, projection};
use std::time::Instant;

pub(crate) fn cmd_doctor_journal() -> Result<bool, String> {
    let journal_dir = resolve_journal_dir()?;
    let path = journal_dir.join("journal.jsonl");

    let raw = std::fs::read_to_string(&path).map_err(|e| format!("cannot read journal: {}", e))?;

    let lines: Vec<&str> = raw.lines().collect();
    let total_lines = lines.len();
    let (events, corrupted) = parse_journal_lines(&lines);
    let (git_head_count, git_context_event_count) = count_git_context_events(&events);

    // CORPUS §9: doctor keeps no cross-run state (no doctor-state.json).
    let mut report = diagnostics::build_doctor_journal_report(
        &events,
        total_lines,
        corrupted,
        git_head_count,
        git_context_event_count,
        chrono::Utc::now(),
    );
    report.cutover_certificate = check_cutover_certificate(&journal_dir, &path, &raw);

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

    // CORPUS §9: only integrity/parse failures make doctor fail. Advisories
    // are surfaced, never blocking — the reader decides.
    Ok(report.has_errors())
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

fn check_cutover_certificate(
    journal_dir: &std::path::Path,
    journal_path: &std::path::Path,
    journal_raw: &str,
) -> diagnostics::CutoverCertificateReport {
    let map_path = journal_dir.join("migration-v1-to-v2.json");
    let archive_path = journal_dir.join("journal.v1.jsonl");
    let certificate_path = cutover_certificate_path_for_map(&map_path);
    let mut report = diagnostics::CutoverCertificateReport {
        checked: false,
        path: Some(certificate_path.display().to_string()),
        errors: Vec::new(),
        warnings: Vec::new(),
    };

    if !certificate_path.exists() {
        if archive_path.exists() || map_path.exists() {
            report.warnings.push(format!(
                "cutover certificate missing: {}",
                certificate_path.display()
            ));
        }
        return report;
    }

    report.checked = true;
    let cert_bytes = match std::fs::read(&certificate_path) {
        Ok(bytes) => bytes,
        Err(e) => {
            report.errors.push(format!(
                "cannot read cutover certificate {}: {}",
                certificate_path.display(),
                e
            ));
            return report;
        }
    };
    let certificate: CutoverV2Certificate = match serde_json::from_slice(&cert_bytes) {
        Ok(certificate) => certificate,
        Err(e) => {
            report
                .errors
                .push(format!("cannot parse cutover certificate: {}", e));
            return report;
        }
    };

    if certificate.schema != "tasktree-v2-cutover-certificate-v1" {
        report.errors.push(format!(
            "cutover certificate schema {} is not supported",
            certificate.schema
        ));
    }

    match std::fs::read(&archive_path) {
        Ok(bytes) => {
            let actual = sha256_bytes(&bytes);
            if actual != certificate.source_journal_sha256 {
                report.errors.push(format!(
                    "source journal hash mismatch: expected {}, got {}",
                    certificate.source_journal_sha256, actual
                ));
            }
        }
        Err(e) => report.errors.push(format!(
            "cannot read archived v1 journal {}: {}",
            archive_path.display(),
            e
        )),
    }

    let map_bytes = match std::fs::read(&map_path) {
        Ok(bytes) => {
            let actual = sha256_bytes(&bytes);
            if actual != certificate.map_sha256 {
                report.errors.push(format!(
                    "migration map hash mismatch: expected {}, got {}",
                    certificate.map_sha256, actual
                ));
            }
            Some(bytes)
        }
        Err(e) => {
            report.errors.push(format!(
                "cannot read migration map {}: {}",
                map_path.display(),
                e
            ));
            None
        }
    };

    if let Some(prefix) =
        first_jsonl_lines_bytes(journal_raw.as_bytes(), certificate.imported_event_count)
    {
        let actual = sha256_bytes(&prefix);
        if actual != certificate.target_journal_initial_sha256 {
            report.errors.push(format!(
                "initial v2 journal prefix hash mismatch: expected {}, got {}",
                certificate.target_journal_initial_sha256, actual
            ));
        }
    } else {
        report.errors.push(format!(
            "current journal {} has fewer than {} lines recorded by cutover certificate",
            journal_path.display(),
            certificate.imported_event_count
        ));
    }

    if let Some(bytes) = map_bytes {
        match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(map) => {
                let source_count = map.get("source_event_count").and_then(|v| v.as_u64());
                if source_count != Some(certificate.source_event_count as u64) {
                    report.errors.push(format!(
                        "migration map source_event_count {:?} does not match certificate {}",
                        source_count, certificate.source_event_count
                    ));
                }
                let imported_count = map.get("imported_event_count").and_then(|v| v.as_u64());
                if imported_count != Some(certificate.imported_event_count as u64) {
                    report.errors.push(format!(
                        "migration map imported_event_count {:?} does not match certificate {}",
                        imported_count, certificate.imported_event_count
                    ));
                }
                let source_digest = map.get("source_digest").and_then(|v| v.as_str());
                if source_digest != Some(certificate.source_event_digest.as_str()) {
                    report.errors.push(format!(
                        "migration map source_digest {:?} does not match certificate {}",
                        source_digest, certificate.source_event_digest
                    ));
                }
            }
            Err(e) => report
                .errors
                .push(format!("cannot parse migration map JSON: {}", e)),
        }
    }

    report
}

/// Edge-discipline self-check: open unfixed frictions + decisions without --why.
/// Advisory only — never fails the process (CORPUS §9: only integrity/parse fails).
pub(crate) fn cmd_doctor_edges(format_json: bool) -> Result<bool, String> {
    let started = Instant::now();
    let path = ensure_journal()?;
    let (events, skipped) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let report = projection::edges_discipline_report(&strands);
    let out = output::EdgesOutput {
        open_friction_count: report.open_frictions.len(),
        decision_without_why_count: report.decisions_without_why.len(),
        open_frictions: report
            .open_frictions
            .into_iter()
            .map(|i| output::EdgesItem {
                entry_id: i.entry_id,
                strand_id: i.strand_id,
                marker: i.marker,
                content: i.content,
                offset: i.offset,
            })
            .collect(),
        decisions_without_why: report
            .decisions_without_why
            .into_iter()
            .map(|i| output::EdgesItem {
                entry_id: i.entry_id,
                strand_id: i.strand_id,
                marker: i.marker,
                content: i.content,
                offset: i.offset,
            })
            .collect(),
    };

    if format_json {
        println!("{}", serde_json::to_string(&out).expect("serialize"));
    } else {
        println!("Doctor Edges Report (edge-discipline self-check)");
        println!(
            "  open unfixed [friction]: {}",
            out.open_friction_count
        );
        for item in &out.open_frictions {
            println!(
                "    {}  strand {}  {}",
                shorten(&item.entry_id),
                shorten(&item.strand_id),
                item.content
            );
        }
        println!(
            "  [decision] without --why: {}",
            out.decision_without_why_count
        );
        for item in &out.decisions_without_why {
            println!(
                "    {}  strand {}  {}",
                shorten(&item.entry_id),
                shorten(&item.strand_id),
                item.content
            );
        }
        if out.open_friction_count == 0 && out.decision_without_why_count == 0 {
            println!("  (clean — no open frictions, no why-less decisions)");
        }
    }

    if skipped > 0 {
        return Err(format!(
            "corrupt: [mnema] WARNING: {} corrupted lines skipped",
            skipped
        ));
    }
    eprintln!("[mnema] doctor edges: {:.0?}", started.elapsed());
    // Advisory: never fail the process solely for open edges.
    Ok(false)
}

fn first_jsonl_lines_bytes(bytes: &[u8], line_count: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut seen = 0usize;
    for (idx, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            out.extend_from_slice(&bytes[start..=idx]);
            start = idx + 1;
            seen += 1;
            if seen == line_count {
                return Some(out);
            }
        }
    }
    if start < bytes.len() && seen + 1 == line_count {
        out.extend_from_slice(&bytes[start..]);
        return Some(out);
    }
    if line_count == 0 { Some(out) } else { None }
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
    println!("  integrity:");
    println!("    anchors: {}", report.integrity.anchor_count);
    println!(
        "    unanchored tail events: {}",
        report.integrity.unanchored_event_count
    );
    println!("    chain errors: {}", report.integrity.chain_errors.len());
    println!(
        "    anchor errors: {}",
        report.integrity.anchor_errors.len()
    );
    for finding in &report.integrity.chain_errors {
        eprintln!("[integrity] {}", finding);
    }
    for finding in &report.integrity.anchor_errors {
        eprintln!("[integrity] {}", finding);
    }
    let cutover_status = if report.cutover_certificate.checked {
        "checked"
    } else if !report.cutover_certificate.warnings.is_empty() {
        "missing"
    } else {
        "not present"
    };
    println!("    cutover certificate: {}", cutover_status);
    if let Some(path) = &report.cutover_certificate.path {
        if report.cutover_certificate.checked || !report.cutover_certificate.warnings.is_empty() {
            println!("      path: {}", path);
        }
    }
    for finding in &report.cutover_certificate.errors {
        eprintln!("[integrity] cutover-certificate: {}", finding);
    }
    for finding in &report.cutover_certificate.warnings {
        eprintln!("[integrity-warning] {}", finding);
    }
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
            println!("    {} {}  (mnema explain {})", code, detail, code);
        }
    }
}

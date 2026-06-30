//! Journal audit lint sections.

use super::run_journal_diagnostics;

#[derive(Debug, Clone)]
pub struct LintSection {
    pub name: &'static str,
    pub summary_label: &'static str,
    pub findings: Vec<String>,
}

impl LintSection {
    pub fn count(&self) -> usize {
        self.findings.len()
    }
}

#[derive(Debug, Clone)]
pub struct JournalAudit {
    pub lint_sections: Vec<LintSection>,
    pub diagnostics: Vec<(String, String)>,
}

impl JournalAudit {
    pub fn lint_count(&self) -> usize {
        self.lint_sections.iter().map(LintSection::count).sum()
    }
}

pub fn audit_journal(
    events: &[crate::event::Event],
    now: chrono::DateTime<chrono::Utc>,
) -> JournalAudit {
    use crate::event::Event;
    use std::collections::{HashMap, HashSet};

    let mut created_ids: HashSet<String> = HashSet::new();
    let mut strand_summaries: HashMap<String, String> = HashMap::new();
    let mut strand_entries: HashMap<String, Vec<String>> = HashMap::new();
    for event in events {
        match event {
            Event::StrandCreated { id, .. } => {
                created_ids.insert(id.clone());
            }
            Event::LogAppended { id, content, .. } => {
                strand_summaries
                    .entry(id.clone())
                    .or_insert_with(|| content.clone());
                strand_entries
                    .entry(id.clone())
                    .or_default()
                    .push(content.clone());
            }
            _ => {}
        }
    }

    let mut sections = Vec::new();

    let mut dag_done = Vec::new();
    for (id, summary) in &strand_summaries {
        if summary.starts_with("para group ") {
            if let Some(entries) = strand_entries.get(id) {
                if entries.iter().any(|e| e.contains("[done]")) {
                    dag_done.push(format!(
                        "DAG strand {} has [done] entry - DAG should only record layer events",
                        id
                    ));
                }
            }
        }
    }
    sections.push(LintSection {
        name: "dag-done",
        summary_label: "dag strands with [done]",
        findings: dag_done,
    });

    let mut task_created = Vec::new();
    for (id, summary) in &strand_summaries {
        if summary.starts_with('[') {
            if let Some(entries) = strand_entries.get(id) {
                if entries.iter().any(|e| e.contains("task_created")) {
                    task_created.push(format!("Task strand {} has task_created JSON event - task strands should not have DAG events", id));
                }
            }
        }
    }
    sections.push(LintSection {
        name: "task-created",
        summary_label: "task strands with task_created",
        findings: task_created,
    });

    let mut orphan_links = Vec::new();
    for event in events {
        if let Event::EdgeLinked { to, .. } = event {
            if !created_ids.contains(to) {
                orphan_links.push(format!("orphan link: target strand {} not found", to));
            }
        }
    }
    sections.push(LintSection {
        name: "orphan-links",
        summary_label: "orphan links",
        findings: orphan_links,
    });

    let mut touches_format = Vec::new();
    for entries in strand_entries.values() {
        for entry in entries {
            if let Some(tail) = entry.strip_prefix("[touches] ") {
                for part in tail.split(' ') {
                    if part.is_empty() {
                        continue;
                    }
                    let field = part.split(':').next().unwrap_or("");
                    if field != "write"
                        && field != "read"
                        && field != "creates"
                        && field != "readonly"
                    {
                        touches_format.push(format!(
                            "touches format: unrecognized field '{}' in [touches] entry",
                            field
                        ));
                    }
                }
            }
        }
    }
    sections.push(LintSection {
        name: "touches-format",
        summary_label: "unrecognized touches fields",
        findings: touches_format,
    });

    let mut link_direction = Vec::new();
    for event in events {
        if let Event::EdgeLinked {
            id: source,
            to: target,
            ..
        } = event
        {
            let src_summary = strand_summaries
                .get(source)
                .map(|s| s.as_str())
                .unwrap_or("");
            let tgt_summary = strand_summaries
                .get(target)
                .map(|s| s.as_str())
                .unwrap_or("");
            let src_is_dag = src_summary.starts_with("para group ");
            let src_is_task = src_summary.starts_with('[')
                && src_summary[1..]
                    .chars()
                    .next()
                    .map_or(false, |c| c.is_ascii_digit());
            let tgt_is_dag = tgt_summary.starts_with("para group ");
            if src_is_task && tgt_is_dag {
                link_direction.push(format!(
                    "link direction: task {} links to DAG {} - unusual",
                    source, target
                ));
            }
            if !src_is_dag && !src_is_task && tgt_is_dag {
                link_direction.push(format!(
                    "link direction: non-task {} links to DAG {} - unusual",
                    source, target
                ));
            }
        }
    }
    sections.push(LintSection {
        name: "link-direction",
        summary_label: "unusual link directions",
        findings: link_direction,
    });

    let mut strand_identity = Vec::new();
    for (id, summary) in &strand_summaries {
        let is_dag = summary.starts_with("para group ");
        let is_task = summary.starts_with('[')
            && summary.chars().nth(1).map_or(false, |c| c.is_ascii_digit());
        if let Some(entries) = strand_entries.get(id) {
            if is_dag {
                let has_task_marker = entries.iter().any(|e| {
                    e.starts_with('[') && e.chars().nth(1).map_or(false, |c| c.is_ascii_digit())
                });
                if has_task_marker {
                    strand_identity.push(format!(
                        "strand identity: DAG strand {} has task-like entries - identity mismatch",
                        id
                    ));
                }
            }
            if is_task {
                let has_para_prefix = entries.iter().any(|e| e.starts_with("para group "));
                if has_para_prefix {
                    strand_identity.push(format!(
                        "strand identity: task strand {} has DAG-like entries - identity mismatch",
                        id
                    ));
                }
            }
        }
    }
    sections.push(LintSection {
        name: "strand-identity",
        summary_label: "identity mismatches",
        findings: strand_identity,
    });

    let indexed: Vec<(usize, Event)> = events.iter().cloned().enumerate().collect();
    let strands = crate::projection::project_strands(&indexed, true);
    let graph = crate::graph::StrandGraph::from_strands(&strands);
    let edge_findings = graph
        .edge_findings()
        .into_iter()
        .map(|f| f.detail)
        .collect();
    sections.push(LintSection {
        name: "edge-validity",
        summary_label: "edge-validity warnings",
        findings: edge_findings,
    });

    let mut metric_format = Vec::new();
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
                    metric_format.push(format!("metric-format: strand {} [metric] entry has no jq-capturable name=value: {:?}", id, preview));
                }
            }
        }
    }
    sections.push(LintSection {
        name: "metric-format",
        summary_label: "uncapturable [metric] entries",
        findings: metric_format,
    });

    let mut legacy_why = Vec::new();
    for event in events {
        if let Event::EdgeLinked {
            id, to, edge_type, ..
        } = event
        {
            if edge_type.as_deref() == Some("why") {
                legacy_why.push(format!("legacy why-edge {} -> {}: why is no longer a link (D2) - record the reason in an entry", id, to));
            }
        }
    }
    sections.push(LintSection {
        name: "legacy-why-edges",
        summary_label: "legacy why-edges",
        findings: legacy_why,
    });

    let mut cur_offset: HashMap<String, usize> = HashMap::new();
    for s in &strands {
        cur_offset.insert(s.id.clone(), s.last_offset());
    }
    let mut stale_why = Vec::new();
    for s in &strands {
        for entry in &s.log {
            if let Some(r) = &entry.ref_ {
                if let Some((tgt, pin)) = r.rsplit_once('@') {
                    if let Ok(pin_off) = pin.parse::<usize>() {
                        if let Some(&cur) = cur_offset.get(tgt) {
                            if cur > pin_off {
                                stale_why.push(format!("why-staleness: strand {} cites {} pinned@{} but it advanced to @{} - may warrant review", s.id, tgt, pin_off, cur));
                            }
                        }
                    }
                }
            }
        }
    }
    sections.push(LintSection {
        name: "why-staleness",
        summary_label: "stale rationale refs",
        findings: stale_why,
    });

    JournalAudit {
        lint_sections: sections,
        diagnostics: run_journal_diagnostics(events, now)
            .into_iter()
            .map(|(code, detail)| (code.to_string(), detail))
            .collect(),
    }
}

#[derive(Debug, Clone)]
pub enum DoctorPreviousState {
    FirstRun,
    Unreadable,
    Invalid,
    LineCount(usize),
}

#[derive(Debug, Clone)]
pub struct DoctorJournalReport {
    pub total_lines: usize,
    pub corrupted: usize,
    pub orphans: Vec<String>,
    pub total_strands: usize,
    pub strands_with_events_count: usize,
    pub noise_strands_count: usize,
    pub git_head_count: usize,
    pub timeline_status: String,
    pub timeline_warning: bool,
    pub audit: JournalAudit,
}

impl DoctorJournalReport {
    pub fn has_issues(&self) -> bool {
        self.corrupted > 0
            || !self.orphans.is_empty()
            || self.timeline_warning
            || self.audit.lint_count() > 0
            || !self.audit.diagnostics.is_empty()
    }
}

pub fn build_doctor_journal_report(
    events: &[crate::event::Event],
    total_lines: usize,
    corrupted: usize,
    git_head_count: usize,
    previous_state: DoctorPreviousState,
    now: chrono::DateTime<chrono::Utc>,
) -> DoctorJournalReport {
    use crate::event::Event;
    use std::collections::{HashMap, HashSet};

    let mut created_ids: HashSet<String> = HashSet::new();
    let mut appended_ids: HashSet<String> = HashSet::new();
    let mut strand_event_counts: HashMap<String, usize> = HashMap::new();
    for event in events {
        match event {
            Event::StrandCreated { id, .. } => {
                created_ids.insert(id.clone());
            }
            Event::LogAppended { id, .. } => {
                appended_ids.insert(id.clone());
                *strand_event_counts.entry(id.clone()).or_insert(0) += 1;
            }
            _ => {}
        }
    }

    let mut orphans: Vec<String> = appended_ids
        .iter()
        .filter(|id| !created_ids.contains(*id))
        .cloned()
        .collect();
    orphans.sort();

    let timeline_status = match previous_state {
        DoctorPreviousState::FirstRun => "monotonic: yes (first run, no previous state)".to_string(),
        DoctorPreviousState::Unreadable => "monotonic: yes (cannot read previous state)".to_string(),
        DoctorPreviousState::Invalid => "monotonic: yes (no previous state)".to_string(),
        DoctorPreviousState::LineCount(previous) => {
            if total_lines < previous {
                format!("warning: {}->{} jump detected (lines decreased)", previous, total_lines)
            } else if total_lines > previous {
                format!("monotonic: yes ({}->{})", previous, total_lines)
            } else {
                "monotonic: yes (unchanged)".to_string()
            }
        }
    };
    let timeline_warning = timeline_status.contains("warning");

    DoctorJournalReport {
        total_lines,
        corrupted,
        orphans,
        total_strands: created_ids.len(),
        strands_with_events_count: created_ids
            .iter()
            .filter(|id| strand_event_counts.contains_key(*id))
            .count(),
        noise_strands_count: created_ids
            .iter()
            .filter(|id| !strand_event_counts.contains_key(*id))
            .count(),
        git_head_count,
        timeline_status,
        timeline_warning,
        audit: audit_journal(events, now),
    }
}
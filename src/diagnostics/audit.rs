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
#[derive(Debug, Clone, Default)]
pub struct IntegrityReport {
    pub anchor_count: usize,
    pub chain_errors: Vec<String>,
    pub anchor_errors: Vec<String>,
    pub unanchored_event_count: usize,
}

impl IntegrityReport {
    pub fn has_errors(&self) -> bool {
        !self.chain_errors.is_empty() || !self.anchor_errors.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct CutoverCertificateReport {
    pub checked: bool,
    pub path: Option<String>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl CutoverCertificateReport {
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

pub fn audit_journal(
    events: &[crate::event::Event],
    now: chrono::DateTime<chrono::Utc>,
) -> JournalAudit {
    use crate::event::Event;
    use std::collections::{HashMap, HashSet};

    let mut created_ids: HashSet<String> = HashSet::new();
    let mut strand_entries: HashMap<String, Vec<String>> = HashMap::new();
    for event in events {
        match event {
            Event::StrandCreated { id, .. } => {
                created_ids.insert(id.clone());
            }
            Event::LogAppended { id, content, .. } => {
                strand_entries
                    .entry(id.clone())
                    .or_default()
                    .push(content.clone());
            }
            _ => {}
        }
    }

    let mut sections = Vec::new();
    fn link_view(event: &Event) -> Option<(String, String, Option<String>)> {
        let delta = crate::projection::edge_delta(event)?;
        if !delta.linked {
            return None;
        }
        let source = event.strand_id()?.to_string();
        Some((source, delta.target, delta.edge_type))
    }

    let mut orphan_links = Vec::new();
    for event in events {
        if let Some((_, target, _)) = link_view(event) {
            if !created_ids.contains(&target) {
                orphan_links.push(format!("orphan link: target strand {} not found", target));
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
        if let Some((source, target, Some(edge_type))) = link_view(event) {
            if edge_type != "why" {
                continue;
            }
            legacy_why.push(format!("legacy why-edge {} -> {}: why is no longer a link (D2) - record the reason in an entry", source, target));
        }
    }
    sections.push(LintSection {
        name: "legacy-why-edges",
        summary_label: "legacy why-edges",
        findings: legacy_why,
    });

    // v2 hash refs: position fact only — the cited entry's line gained
    // entries after the citation. Whether that overturns the citing
    // conclusion is the reader's judgment, not ours.
    let entry_index = crate::projection::EntryIndex::build(&strands);
    let mut ref_advanced = Vec::new();
    for s in &strands {
        for entry in &s.log {
            for cited in &entry.refs {
                if entry_index.advanced_past(cited, entry.offset) == Some(true) {
                    ref_advanced.push(format!(
                        "ref-target-advanced: strand {} entry @{} cites {} whose line gained later entries - may warrant review",
                        crate::util::shorten(&s.id),
                        entry.offset,
                        crate::util::shorten(cited)
                    ));
                }
            }
        }
    }
    sections.push(LintSection {
        name: "ref-target-advanced",
        summary_label: "cited entries whose line advanced",
        findings: ref_advanced,
    });

    JournalAudit {
        lint_sections: sections,
        diagnostics: run_journal_diagnostics(events, now)
            .into_iter()
            .map(|(code, detail)| (code.to_string(), detail))
            .collect(),
    }
}

pub fn verify_journal_integrity(events: &[crate::event::Event]) -> IntegrityReport {
    use crate::event::{EntryChainFold, EntryChainMode, Event, compute_journal_anchor_digest};
    use std::collections::HashMap;

    let mut entry_chain = EntryChainFold::new(EntryChainMode::Integrity);
    let mut log_counts: HashMap<String, usize> = HashMap::new();
    let mut previous_anchor: Option<String> = None;
    let mut last_anchor_index: Option<usize> = None;
    let mut report = IntegrityReport::default();

    for (idx, event) in events.iter().enumerate() {
        match event {
            Event::LogAppended { .. } => {
                let chain_step = entry_chain.apply(event).expect("log event folds");
                if chain_step.stored_entry_id.is_some()
                    && chain_step.prev_entry_id.as_deref()
                        != chain_step.expected_prev_entry_id.as_deref()
                {
                    report.chain_errors.push(format!(
                        "hash-chain: event {} strand {} prev_entry_id {:?} expected {:?}",
                        idx,
                        chain_step.strand_id,
                        chain_step.prev_entry_id.as_deref(),
                        chain_step.expected_prev_entry_id.as_deref()
                    ));
                }
                if let Some(stored) = &chain_step.stored_entry_id {
                    if stored != &chain_step.computed_entry_id {
                        report.chain_errors.push(format!(
                            "hash-chain: event {} strand {} entry_id {} expected {}",
                            idx, chain_step.strand_id, stored, chain_step.computed_entry_id
                        ));
                    }
                    let count = log_counts.entry(chain_step.strand_id.clone()).or_insert(0);
                    if *count == 0 && stored != &chain_step.strand_id {
                        report.chain_errors.push(format!(
                            "hash-chain: strand {} first entry_id {} does not equal strand id",
                            chain_step.strand_id, stored
                        ));
                    }
                    *count += 1;
                } else {
                    *log_counts.entry(chain_step.strand_id).or_insert(0) += 1;
                }
            }
            Event::JournalAnchored {
                covered_event_count,
                heads: stored_heads,
                digest,
                previous_anchor: stored_previous,
                ..
            } => {
                report.anchor_count += 1;
                let expected_heads = entry_chain.anchor_heads();
                if *covered_event_count != idx {
                    report.anchor_errors.push(format!(
                        "anchor: event {} covers {} events, expected {}",
                        idx, covered_event_count, idx
                    ));
                }
                if stored_previous.as_deref() != previous_anchor.as_deref() {
                    report.anchor_errors.push(format!(
                        "anchor: event {} previous_anchor {:?} expected {:?}",
                        idx,
                        stored_previous.as_deref(),
                        previous_anchor.as_deref()
                    ));
                }
                if stored_heads != &expected_heads {
                    report.anchor_errors.push(format!(
                        "anchor: event {} head list mismatch (stored {}, expected {})",
                        idx,
                        stored_heads.len(),
                        expected_heads.len()
                    ));
                }
                let expected_digest =
                    compute_journal_anchor_digest(&expected_heads, previous_anchor.as_deref(), idx);
                if digest != &expected_digest {
                    report.anchor_errors.push(format!(
                        "anchor: event {} digest {} expected {}",
                        idx, digest, expected_digest
                    ));
                }
                previous_anchor = Some(digest.clone());
                last_anchor_index = Some(idx);
            }
            _ => {}
        }
    }

    report.unanchored_event_count = match last_anchor_index {
        Some(idx) => events.len().saturating_sub(idx + 1),
        None => events.len(),
    };
    report
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
    pub git_context_event_count: usize,
    pub integrity: IntegrityReport,
    pub cutover_certificate: CutoverCertificateReport,
    pub audit: JournalAudit,
}

impl DoctorJournalReport {
    /// Integrity/parse failures — the only class doctor is allowed to fail on
    /// (CORPUS §9). Advisories never block; doctor keeps no cross-run state.
    pub fn has_errors(&self) -> bool {
        self.corrupted > 0
            || !self.orphans.is_empty()
            || self.integrity.has_errors()
            || self.cutover_certificate.has_errors()
    }
}

pub fn build_doctor_journal_report(
    events: &[crate::event::Event],
    total_lines: usize,
    corrupted: usize,
    git_head_count: usize,
    git_context_event_count: usize,
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

    let integrity = verify_journal_integrity(events);

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
        git_context_event_count,
        integrity,
        cutover_certificate: CutoverCertificateReport::default(),
        audit: audit_journal(events, now),
    }
}

//! Runtime diagnostic emitters.

/// One emitted diagnostic: (code, one-line detail). The code resolves via
/// `mnema explain <code>`.
pub(crate) type EmittedDiag = (&'static str, String);

/// Run the W068 emitter over the journal events.
/// Pure: `now` is a parameter, nothing is written.
pub(crate) fn run_journal_diagnostics(
    events: &[crate::event::Event],
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<EmittedDiag> {
    use crate::event::{EntryEffect, Event};
    use std::collections::HashMap;
    let mut diags: Vec<EmittedDiag> = Vec::new();

    let mut per_strand: HashMap<&str, Vec<&str>> = HashMap::new();
    for event in events {
        if let Event::LogAppended { id, content, .. } = event {
            per_strand
                .entry(id.as_str())
                .or_default()
                .push(content.as_str());
        }
    }

    let mut closed_strands: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for event in events {
        let Some(id) = event.strand_id() else {
            continue;
        };
        match crate::projection::lifecycle_effect(event) {
            Some(EntryEffect::Close { .. }) => {
                closed_strands.insert(id);
            }
            Some(EntryEffect::Reopen) => {
                closed_strands.remove(id);
            }
            _ => {}
        }
    }

    for (id, entries) in &per_strand {
        if closed_strands.contains(id) {
            continue;
        }
        for content in entries {
            if !content.starts_with("[deadline]") {
                continue;
            }
            if let Some(by) = crate::util::parse_deadline_by(content) {
                if now > by {
                    diags.push((
                        "W068",
                        format!(
                            "strand {} deadline passed ({})",
                            crate::util::shorten(id),
                            by.to_rfc3339()
                        ),
                    ));
                }
            }
        }
    }

    diags
}

/// Check W071: checkpoint target strand state is not "registered" (already closed).
///
/// Returns `Some((code, detail))` when the check fires, `None` otherwise.
pub(crate) fn check_w071_closed_strand(
    strand: &crate::projection::ProjectedStrand,
) -> Option<EmittedDiag> {
    if strand.state() != "registered" {
        Some((
            "W071",
            format!("checkpoint on closed strand: state is {}", strand.state()),
        ))
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClosedTargetWarning {
    pub(crate) code: &'static str,
    pub(crate) detail: String,
    pub(crate) state: String,
    pub(crate) add_from: String,
    pub(crate) reopen: String,
}

/// Check W059: explicit append target strand is already closed.
pub(crate) fn check_w059_append_closed_strand(
    strand: &crate::projection::ProjectedStrand,
) -> Option<ClosedTargetWarning> {
    let state = strand.state();
    if state == "registered" {
        return None;
    }

    let id = crate::util::shorten(&strand.id);
    let add_from = format!("mnema add --from {}", id);
    let reopen = format!("mnema reopen --id {}", id);
    Some(ClosedTargetWarning {
        code: "W059",
        detail: format!(
            "append target {} is {}; new result: {}; wrong close: {}",
            id, state, add_from, reopen
        ),
        state: state.to_string(),
        add_from,
        reopen,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeenOffsetWarning {
    pub(crate) code: &'static str,
    pub(crate) detail: String,
    pub(crate) seen_offset: usize,
    pub(crate) strand_last_offset: usize,
    pub(crate) seen_gap: usize,
    pub(crate) catch_up: String,
}

/// Check W076: caller-declared --seen-offset is behind the target strand's
/// pre-write last_offset. Missing or future offsets are best-effort ignored.
pub(crate) fn check_w076_seen_offset(
    strand_id: &str,
    seen_offset: Option<usize>,
    strand_last_offset: usize,
) -> Option<SeenOffsetWarning> {
    let seen = seen_offset?;
    if seen >= strand_last_offset {
        return None;
    }
    let gap = strand_last_offset - seen;
    let catch_up = format!(
        "mnema timeline --since-offset {} --links {}",
        seen,
        crate::util::shorten(strand_id)
    );
    Some(SeenOffsetWarning {
        code: "W076",
        detail: format!(
            "seen offset {} is {} entries behind strand last offset {}; catch-up: {}",
            seen, gap, strand_last_offset, catch_up
        ),
        seen_offset: seen,
        strand_last_offset,
        seen_gap: gap,
        catch_up,
    })
}

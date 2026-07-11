//! CLI adapter helper for fresh post-write views.
//!
//! This module reads the journal after write/manage commands and maps the fresh
//! projection to contract values or human text helpers. It is intentionally an
//! outer adapter: durable facts stay in `journal`, derived meaning in
//! `projection`, and public DTO shape in `output`.

use crate::journal::{ensure_journal, read_events_lossy};
use crate::{output, projection};
pub(crate) fn strand_card_fresh(strand_id: &str) -> Option<output::OrientStrand> {
    let path = ensure_journal().ok()?;
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    strands
        .iter()
        .find(|s| s.id == strand_id)
        .map(output::OrientStrand::from)
}
pub(crate) fn strand_card_fresh_with_state(
    strand_id: &str,
) -> Option<(output::OrientStrand, String)> {
    let path = ensure_journal().ok()?;
    let (events, _) = read_events_lossy(&path);
    strand_card_with_state(&events, strand_id)
}

pub(crate) fn strand_card_with_state(
    events: &[(usize, crate::event::Event)],
    strand_id: &str,
) -> Option<(output::OrientStrand, String)> {
    let strands = projection::project_strands(events, true);
    strands
        .iter()
        .find(|s| s.id == strand_id)
        .map(|s| (output::OrientStrand::from(s), s.state().to_string()))
}

pub(crate) fn visibility_ledger() -> Option<projection::VisibilityLedger> {
    let path = ensure_journal().ok()?;
    let (events, _) = read_events_lossy(&path);
    let all = projection::project_strands(&events, true);
    Some(projection::project_visibility_ledger(&all))
}

pub(crate) fn print_visibility_ledger() {
    if let Some(ledger) = visibility_ledger() {
        eprintln!(
            "journal: {} active | {} closed | {} hidden",
            ledger.active_count, ledger.closed_count, ledger.hidden_count
        );
    }
}

pub(crate) fn visibility_ledger_json(strand_id: &str, noop: bool) -> serde_json::Value {
    let ledger = visibility_ledger().unwrap_or(projection::VisibilityLedger {
        active_count: 0,
        closed_count: 0,
        hidden_count: 0,
    });
    let output = output::VisibilityLedgerOutput {
        strand_id: strand_id.to_string(),
        status: "ok",
        noop,
        active_count: ledger.active_count,
        closed_count: ledger.closed_count,
        hidden_count: ledger.hidden_count,
        result: strand_card_fresh(strand_id),
    };
    serde_json::to_value(output).unwrap_or(serde_json::Value::Null)
}

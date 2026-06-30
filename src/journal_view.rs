//! Journal-backed view adapter.
//!
//! This module is the seam between append-only journal state and pure render
//! functions. `render` builds and prints presentation values; this module reads
//! the journal when callers need a fresh post-write view.

use crate::journal::{ensure_journal, read_events_lossy};
use crate::{output, projection, render};
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VisibilityLedger {
    pub(crate) active_count: usize,
    pub(crate) closed_count: usize,
    pub(crate) hidden_count: usize,
}

pub(crate) fn strand_card_fresh(strand_id: &str) -> Option<output::OrientStrand> {
    let path = ensure_journal().ok()?;
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    strands
        .iter()
        .find(|s| s.id == strand_id)
        .map(render::make_card)
}

pub(crate) fn strand_card_fresh_with_state(
    strand_id: &str,
) -> Option<(output::OrientStrand, String)> {
    let path = ensure_journal().ok()?;
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    strands
        .iter()
        .find(|s| s.id == strand_id)
        .map(|s| (render::make_card(s), s.state().to_string()))
}

pub(crate) fn visibility_ledger() -> Option<VisibilityLedger> {
    let path = ensure_journal().ok()?;
    let (events, _) = read_events_lossy(&path);
    let all = projection::project_strands(&events, true);
    let hidden_count = all.iter().filter(|s| s.hidden).count();
    let visible: Vec<_> = all.iter().filter(|s| !s.hidden).collect();
    let active_count = visible.iter().filter(|s| s.state() == "registered").count();
    Some(VisibilityLedger {
        active_count,
        closed_count: visible.len() - active_count,
        hidden_count,
    })
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
    let card_val = strand_card_fresh(strand_id)
        .as_ref()
        .and_then(|c| serde_json::to_value(c).ok());
    let ledger = visibility_ledger().unwrap_or(VisibilityLedger {
        active_count: 0,
        closed_count: 0,
        hidden_count: 0,
    });
    json!({
        "strand_id": strand_id,
        "status": "ok",
        "noop": noop,
        "active_count": ledger.active_count,
        "closed_count": ledger.closed_count,
        "hidden_count": ledger.hidden_count,
        "result": card_val,
    })
}

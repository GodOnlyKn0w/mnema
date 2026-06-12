/// Output rendering layer: card/orient projections, visibility ledger.
/// Pure presentation — no journal writes, no clap. Moved from main.rs (Layer 3
/// refactor); function bodies are byte-identical to the originals.
///
/// Dependency direction: render → output, projection, tree, journal (read-only)
/// render ← main.rs (mod render; pub(crate) use render::*)
use crate::journal::{ensure_journal, read_events_lossy};
use crate::{output, projection, tree};
use crate::{shorten, truncate};
use serde_json::json;

/// Orient remind line: the whole operating loop in one line (ADR-0001:
/// the rules travel with the orientation, the weave-in pointer stays thin).
pub(crate) const ORIENT_REMIND: &str = "loop: 做一步·看现实变·再想 | continue → append --id <ID> \"[decision] ...\" | new matter → add \"<summary>\" | matter concluded → close --id <ID> [--as done|failed|cancelled|merged|verified] | before irreversible → checkpoint --id <ID> --action \"<why>\" | more → tasktree --help";

/// Build an OrientStrand card from a projected strand. Identical to the
/// inline construction in build_orient; extracted so write commands can
/// call the same logic without duplicating the truncation/shorten rules.
// Contract: card id is the FULL 24-hex strand id — same width as show/list
// JSON, so consumers can join across outputs. Display sites shorten at
// print time; the prefix form stays a valid argument either way.
pub(crate) fn make_card(s: &projection::ProjectedStrand) -> output::OrientStrand {
    output::OrientStrand {
        id: s.id.clone(),
        strand_type: s.strand_type.clone(),
        entry_count: s.log_count(),
        summary: truncate(s.first_summary(), 70),
        last_entry: truncate(s.last_summary(), 70),
        last_offset: s.last_offset(),
        catch_up: format!("tasktree show --id {} --tail 8", s.id),
        lifecycle: s.state().to_string(),
    }
}

/// The card printer used by write commands. Callers supply the state
/// string directly so we avoid re-projecting a second time.
// Card echo goes to stderr: stdout is the value (capturable by
// `ID=$(tasktree add ...)`), stderr is the narration — same split as the
// perf footers. JSON mode is unaffected (result field on stdout).
pub(crate) fn print_card_with_state(card: &output::OrientStrand, state: &str) {
    print_handle_line(card, state);
    eprintln!("    {}", card.summary);
    if card.entry_count > 1 {
        eprintln!("    last: {}", card.last_entry);
    }
}

/// Re-project a single strand from a fresh journal read and build its card.
/// Uses include_hidden=true so hidden strands can still echo their own card.
pub(crate) fn strand_card_fresh(strand_id: &str) -> Option<output::OrientStrand> {
    let path = ensure_journal().ok()?;
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    strands.iter().find(|s| s.id == strand_id).map(make_card)
}

/// Like strand_card_fresh but also returns the state string (to avoid a
/// second projection scan when the caller needs both).
pub(crate) fn strand_card_fresh_with_state(strand_id: &str) -> Option<(output::OrientStrand, String)> {
    let path = ensure_journal().ok()?;
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    strands.iter().find(|s| s.id == strand_id).map(|s| {
        (make_card(s), s.state().to_string())
    })
}

/// Print the global visibility ledger line used by hide/unhide echo.
/// Reads the journal fresh. Counts: active = visible & state=="registered",
/// closed = visible - active, hidden = strands with hidden==true.
pub(crate) fn print_visibility_ledger() {
    if let Ok(path) = ensure_journal() {
        let (events, _) = read_events_lossy(&path);
        let all = projection::project_strands(&events, true);
        let hidden_n = all.iter().filter(|s| s.hidden).count();
        let visible: Vec<_> = all.iter().filter(|s| !s.hidden).collect();
        let active_n = visible.iter().filter(|s| s.state() == "registered").count();
        let closed_n = visible.len() - active_n;
        eprintln!("journal: {} active | {} closed | {} hidden", active_n, closed_n, hidden_n);
    }
}

/// Print the handle line only (id + type + entries + state). Used by
/// hide/unhide/link/bind where we show a reduced card.
pub(crate) fn print_handle_line(card: &output::OrientStrand, state: &str) {
    let type_info = card
        .strand_type
        .as_deref()
        .map(|t| format!(" [{}]", t))
        .unwrap_or_default();
    eprintln!(
        "  {}{} | {} entries | {}",
        shorten(&card.id), type_info, card.entry_count, state
    );
}

/// Pure projection for orient. Never touches the journal (ADR-0003: orient
/// stays pure-read; the catch-up cursor is each strand's own last_offset).
pub(crate) fn build_orient(
    strands: &[projection::ProjectedStrand],
    include_hidden: bool,
    limit: usize,
    max_offset: usize,
) -> output::OrientOutput {
    // strands contains ALL strands (hidden + visible); split here so that
    // hidden_count can be computed regardless of include_hidden.
    let hidden_count = if include_hidden {
        0
    } else {
        strands.iter().filter(|s| s.hidden).count()
    };
    let visible: Vec<&projection::ProjectedStrand> = strands
        .iter()
        .filter(|s| !s.hidden || include_hidden)
        .collect();
    let mut active: Vec<&projection::ProjectedStrand> = visible
        .iter()
        .copied()
        .filter(|s| s.state() == "registered")
        .collect();
    let closed_count = visible.len() - active.len();
    // Most recently touched first; the menu is an index, not a dump.
    active.sort_by(|a, b| b.last_offset().cmp(&a.last_offset()));
    active.truncate(limit);

    output::OrientOutput {
        max_offset,
        active: active.iter().map(|s| make_card(s)).collect(),
        closed_count,
        hidden_count,
        remind: ORIENT_REMIND.to_string(),
    }
}

/// Visibility ledger JSON shared by the hide/unhide twins. Extracted as a
/// function so the grammar naming CI can sample the shape — write-command
/// JSON built inline with json!() is invisible to projection-based sampling.
pub(crate) fn visibility_ledger_json(strand_id: &str, noop: bool) -> serde_json::Value {
    let card_val = strand_card_fresh(strand_id)
        .as_ref()
        .and_then(|c| serde_json::to_value(c).ok());
    let (active, closed, hidden) = match ensure_journal().ok() {
        Some(p) => {
            let (events, _) = read_events_lossy(&p);
            let all = projection::project_strands(&events, true);
            let hidden_n = all.iter().filter(|s| s.hidden).count();
            let visible: Vec<_> = all.iter().filter(|s| !s.hidden).collect();
            let active_n = visible.iter().filter(|s| s.state() == "registered").count();
            (active_n, visible.len() - active_n, hidden_n)
        }
        None => (0, 0, 0),
    };
    json!({
        "strand_id": strand_id,
        "status": "ok",
        "noop": noop,
        "active_count": active,
        "closed_count": closed,
        "hidden_count": hidden,
        "result": card_val,
    })
}

/// Recursively print an orient forest node with indentation.
/// Each node shows the same card fields as flat orient, prefixed with
/// an indentation level so parent-child nesting is visible.
pub(crate) fn print_orient_forest(nodes: &[tree::OrientForestNode], depth: usize) {
    let indent = "  ".repeat(depth);
    for node in nodes {
        let s = &node.card;
        let type_info = s
            .strand_type
            .as_deref()
            .map(|t| format!(" [{}]", t))
            .unwrap_or_default();
        println!("{}  {}{}  {} entries", indent, shorten(&s.id), type_info, s.entry_count);
        println!("{}    {}", indent, s.summary);
        if s.entry_count > 1 {
            println!("{}    last: {}", indent, s.last_entry);
        }
        println!("{}    catch-up: {}", indent, s.catch_up);
        if !node.children.is_empty() {
            print_orient_forest(&node.children, depth + 1);
        }
    }
}

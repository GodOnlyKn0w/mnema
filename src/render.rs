/// Output rendering layer: card/orient projections and text printing.
/// Pure presentation: no journal reads, no journal writes, no clap.
use crate::util::shorten;
use crate::{output, projection};

/// Orient remind line: the whole operating loop in one line (ADR-0001:
/// the rules travel with the orientation, the weave-in pointer stays thin).
pub(crate) const ORIENT_REMIND: &str = "loop: 做一步·看现实变·再想 | continue → append --id <ID> \"[decision] ...\" | new matter → add \"<summary>\" | matter concluded → close --id <ID> [--as done|failed|cancelled|merged|verified] | before irreversible → checkpoint --id <ID> --action \"<why>\" | read/extract → --format json | jq（id/offset/status，非文本切割）| more → tasktree --help";

/// Build an OrientStrand card from a projected strand. Identical to the
/// inline construction in build_orient; extracted so write commands can
/// call the same logic without duplicating the truncation/shorten rules.
// Contract: card id is the FULL 24-hex strand id — same width as show/list
// JSON, so consumers can join across outputs. Display sites shorten at
// print time; the prefix form stays a valid argument either way.
pub(crate) fn make_card(s: &projection::ProjectedStrand) -> output::OrientStrand {
    output::OrientStrand::from(s)
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
        shorten(&card.id),
        type_info,
        card.entry_count,
        state
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

/// Recursively print an orient forest node with indentation.
/// Each node shows the same card fields as flat orient, prefixed with
/// an indentation level so parent-child nesting is visible.
pub(crate) fn print_orient_forest(nodes: &[output::OrientForestNode], depth: usize) {
    let indent = "  ".repeat(depth);
    for node in nodes {
        let s = &node.card;
        let type_info = s
            .strand_type
            .as_deref()
            .map(|t| format!(" [{}]", t))
            .unwrap_or_default();
        println!(
            "{}  {}{}  {} entries | last_offset {}",
            indent,
            shorten(&s.id),
            type_info,
            s.entry_count,
            s.last_offset
        );
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



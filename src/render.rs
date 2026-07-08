use crate::output;
/// Output rendering layer: human-readable text printing.
/// Pure presentation: no journal reads, no journal writes, no clap.
use crate::util::shorten;

/// The card printer used by write commands. Callers supply the state
/// string directly so we avoid re-projecting a second time.
// Card echo goes to stderr: stdout is the value (capturable by
// `ID=$(mnema add ...)`), stderr is the narration — same split as the
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
    let slug_info = card
        .slug
        .as_deref()
        .map(|slug| format!(" ({})", slug))
        .unwrap_or_default();
    eprintln!(
        "  {}{}{} | {} entries | {}",
        shorten(&card.id),
        slug_info,
        type_info,
        card.entry_count,
        state
    );
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
        let slug_info = s
            .slug
            .as_deref()
            .map(|slug| format!(" ({})", slug))
            .unwrap_or_default();
        println!(
            "{}  {}{}{}  {} entries | last_offset {}",
            indent,
            shorten(&s.id),
            slug_info,
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

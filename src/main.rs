mod commands;
mod diagnostics;
mod event;
mod graph;
mod journal;
mod journal_view;
mod markers;
mod output;
mod projection;
mod reference;
mod render;
mod scope;
mod tree;
mod util;

pub(crate) use journal_view::*;
pub(crate) use render::*;

mod cli;
// Removed when journal resolution follows active-journal.json.
#[allow(dead_code)]
mod activation;
// Removed when the v3 runtime starts constructing canonical entries.
#[allow(dead_code)]
mod canonical;

fn main() {
    cli::main();
}

#[cfg(test)]
mod tests;

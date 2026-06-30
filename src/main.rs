mod commands;
mod diagnostics;
mod event;
mod graph;
mod journal;
mod journal_view;
mod markers;
mod output;
mod projection;
mod render;
mod tree;
mod util;

pub(crate) use journal_view::*;
pub(crate) use render::*;

mod cli;

fn main() {
    cli::main();
}

#[cfg(test)]
mod tests;

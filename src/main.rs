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
mod activation;
mod canonical;
mod cutover_v3;
mod journal_v3;
mod strict_json;

fn main() {
    cli::main();
}

#[cfg(test)]
mod tests;

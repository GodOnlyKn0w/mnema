mod commands;
mod diagnostics;
mod event;
mod graph;
mod journal;
mod projection;
mod output;
mod render;
mod tree;
mod util;

pub(crate) use render::*;

mod cli;

fn main() {
    cli::main();
}

#[cfg(test)]
mod tests;


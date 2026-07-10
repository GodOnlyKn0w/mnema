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

mod activation;
mod canonical;
mod cli;
mod cutover_v3;
mod journal_v3;
mod strict_json;

fn main() {
    // clap builds the complete command/help tree during parsing. The command
    // surface is intentionally broad and can exceed Windows' small default
    // main-thread stack, so run the CLI on an explicitly sized stack.
    let cli_thread = std::thread::Builder::new()
        .name("mnema-cli".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(cli::main)
        .expect("spawn mnema CLI thread");
    if let Err(panic) = cli_thread.join() {
        std::panic::resume_unwind(panic);
    }
}

#[cfg(test)]
mod tests;

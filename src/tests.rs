use crate::cli::{Cli, exit_code_for};
use crate::commands::manage::*;
use crate::commands::query::*;
use crate::commands::write::*;
use crate::diagnostics;
use crate::event::{self, Event, find_strand};
use crate::journal::*;
use crate::journal_view::*;
use crate::markers::{
    is_closing_annotation_marker, known_marker_spellings as known_markers, leading_marker,
    levenshtein, suggest_marker, validate_lifecycle_marker,
};
use crate::output;
use crate::output::ORIENT_REMIND;
use crate::projection;
use crate::tree;
use crate::util::*;
use std::fs;

fn orient_output(
    strands: &[projection::ProjectedStrand],
    include_hidden: bool,
    limit: usize,
    max_offset: usize,
) -> output::OrientOutput {
    let view = projection::build_orient_view(strands, include_hidden, limit, max_offset);
    output::OrientOutput::from((&view, strands))
}
mod support;
use support::*;

mod cli_tests;
mod diagnostics_tests;
mod journal_tests;
mod manage_tests;
mod output_tests;
mod query_tests;
mod util_tests;
mod write_tests;

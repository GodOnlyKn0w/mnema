//! Compatibility wrappers for strand tree projection.
//!
//! The graph rules live in `crate::graph`; this module keeps the historical
//! `tree::...` interface stable for callers and JSON DTO references.

pub use crate::graph::{OrientForestNode, TreeNode, TreeOutput};

use crate::graph;
use crate::output::OrientStrand;
use crate::projection::ProjectedStrand;
use std::collections::HashSet;

pub fn project_tree(root_id: &str, strands: &[ProjectedStrand]) -> Option<TreeNode> {
    graph::StrandGraph::from_strands(strands).project_tree(root_id)
}

pub fn subtree_ids(root_id: &str, strands: &[ProjectedStrand]) -> Option<HashSet<String>> {
    graph::StrandGraph::from_strands(strands).subtree_ids(root_id)
}

pub fn build_orient_forest(
    strand_cards: &[(&ProjectedStrand, OrientStrand)],
) -> Vec<OrientForestNode> {
    graph::build_orient_forest(strand_cards)
}

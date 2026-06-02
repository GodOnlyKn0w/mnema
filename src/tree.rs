//! tasktree tree projection layer.
//! Projects strand edges into nested tree structures.
//! First-order projection only — builds tree structure, never interprets meaning.

use crate::projection::ProjectedStrand;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

// ── Tree Node ───────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct TreeNode {
    pub id: String,
    pub summary: String,
    pub status: String,
    pub state_marker: Option<String>,
    pub state_offset: usize,
    pub strand_type: Option<String>,
    pub entries: usize,
    pub children: Vec<TreeNode>,
}

// ── Tree Output ─────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TreeOutput {
    pub root: TreeNode,
}

// ── Entry point ──────────────────────────────────────────────

/// Build a nested tree rooted at `root_id` from the given strand projections.
/// BFS traversal — avoids deep recursion stack. Tracks visited IDs to guard
/// against cycles. Returns a single TreeNode representing the root and all
/// reachable descendants via edges.
pub fn project_tree(root_id: &str, strands: &[ProjectedStrand]) -> Option<TreeNode> {
    // 1. Build ID→strand map + adjacency list (children: parent_id → child_ids)
    let mut strand_map: HashMap<String, &ProjectedStrand> = HashMap::new();
    // Edges on strand S mean "S links TO target" — so target is a child of S.
    // adjacency[parent_id] = vec of child strand IDs
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();

    for s in strands {
        strand_map.insert(s.id.clone(), s);
        for edge_target in &s.edges {
            adjacency
                .entry(s.id.clone())
                .or_default()
                .push(edge_target.clone());
        }
    }

    // 2. Resolve root_id via prefix match
    let resolved_root = resolve_id(root_id, &strand_map)?;

    // 3. BFS collect all reachable strand IDs
    let mut queue: Vec<String> = vec![resolved_root.clone()];
    let mut reachable: HashSet<String> = HashSet::new();
    reachable.insert(resolved_root.clone());

    while let Some(current) = queue.pop() {
        if let Some(children) = adjacency.get(&current) {
            for child_id in children {
                if !reachable.contains(child_id) && strand_map.contains_key(child_id) {
                    reachable.insert(child_id.clone());
                    queue.push(child_id.clone());
                }
            }
        }
    }

    // 4. Build tree from leaves up: start from nodes with no children in reachable set
    let mut node_map: HashMap<String, TreeNode> = HashMap::new();

    // Build all reachable nodes (bare, no children yet)
    for id in &reachable {
        if let Some(s) = strand_map.get(id) {
            let node = TreeNode {
                id: s.id.clone(),
                summary: s.first_summary().to_string(),
                status: s.state().to_string(),
                state_marker: s.state_marker.clone(),
                state_offset: s.state_offset,
                strand_type: s.strand_type.clone(),
                entries: s.log_count(),
                children: Vec::new(),
            };
            node_map.insert(s.id.clone(), node);
        }
    }

    // Attach children: for each parent in reachable, find its children that are also reachable.
    // Use .get() not .remove() — parent may also be a child of another node (nested trees).
    for parent_id in &reachable {
        if let Some(child_ids) = adjacency.get(parent_id) {
            let child_nodes: Vec<TreeNode> = child_ids
                .iter()
                .filter(|cid| reachable.contains(*cid))
                .filter_map(|cid| node_map.get(cid).cloned())
                .collect();
            if let Some(parent_node) = node_map.get_mut(parent_id) {
                parent_node.children = child_nodes;
            }
        }
    }

    // Root = resolved_root (always in node_map, may have children but no parent in reachable)
    node_map.remove(&resolved_root)
}

/// Collect all strand IDs reachable from root via edges (BFS).
/// Returns the set of IDs including root. Does NOT build full TreeNode structures.
/// Used by subtree timeline filtering — avoids building the full tree when only
/// ID membership is needed.
pub fn subtree_ids(root_id: &str, strands: &[ProjectedStrand]) -> Option<HashSet<String>> {
    let mut strand_map: HashMap<String, &ProjectedStrand> = HashMap::new();
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();

    for s in strands {
        strand_map.insert(s.id.clone(), s);
        for edge_target in &s.edges {
            adjacency
                .entry(s.id.clone())
                .or_default()
                .push(edge_target.clone());
        }
    }

    let resolved_root = resolve_id(root_id, &strand_map)?;

    let mut queue: Vec<String> = vec![resolved_root.clone()];
    let mut reachable: HashSet<String> = HashSet::new();
    reachable.insert(resolved_root.clone());

    while let Some(current) = queue.pop() {
        if let Some(children) = adjacency.get(&current) {
            for child_id in children {
                if !reachable.contains(child_id) && strand_map.contains_key(child_id) {
                    reachable.insert(child_id.clone());
                    queue.push(child_id.clone());
                }
            }
        }
    }

    Some(reachable)
}

// ── Helpers ──────────────────────────────────────────────────

fn resolve_id<'a>(
    prefix: &str,
    map: &HashMap<String, &'a ProjectedStrand>,
) -> Option<String> {
    if map.contains_key(prefix) {
        return Some(prefix.to_string());
    }
    let matches: Vec<&String> = map.keys().filter(|k| k.starts_with(prefix)).collect();
    if matches.len() == 1 {
        Some(matches[0].clone())
    } else {
        None
    }
}

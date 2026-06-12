//! tasktree tree projection layer.
//! Projects strand edges into nested tree structures.
//! First-order projection only — builds tree structure, never interprets meaning.

use crate::projection::ProjectedStrand;
use crate::output::OrientStrand;
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

// ── Orient forest (belongs-to projection) ────────────────────

/// A node in the orient --tree forest. Carries the full OrientStrand card
/// plus children that declared `belongs-to` this strand. Children are sorted
/// by last_offset descending (same ordering as flat orient).
#[derive(Debug, Serialize, Clone)]
pub struct OrientForestNode {
    #[serde(flatten)]
    pub card: OrientStrand,
    pub children: Vec<OrientForestNode>,
}

/// Build a belongs-to forest from a slice of active strands and their cards.
///
/// Algorithm:
///   1. For each strand S, collect parent_ids = S.belongs_to_edges ∩ strand_ids_in_set.
///      (A parent referenced but absent from the active set is treated as missing —
///       the child becomes a root, visible at top level, not lost.)
///   2. Strands with at least one known parent in the set get attached as children
///      of their first resolved parent (deterministic: first belongs-to edge wins).
///   3. Strands with no known parent in the set are roots.
///   4. Children within each parent are sorted by last_offset descending.
///   5. Roots are sorted by last_offset descending.
///
/// Contract: `strand_cards` are (ProjectedStrand, OrientStrand) pairs for the
/// same strand set (same order not required). The set is typically the active
/// (registered) strands from orient.
pub fn build_orient_forest(
    strand_cards: &[(&ProjectedStrand, OrientStrand)],
) -> Vec<OrientForestNode> {
    // 1. Index strand ids in the set for O(1) parent lookup
    let id_set: HashSet<&str> = strand_cards.iter().map(|(s, _)| s.id.as_str()).collect();

    // 2. Build adjacency: parent_id → vec of child strand_ids (in order encountered)
    //    A strand declares its parent via belongs_to_edges. We use the first resolved
    //    parent (first belongs_to edge whose target is in id_set).
    let mut parent_of: HashMap<String, String> = HashMap::new(); // child_id → parent_id
    for (s, _) in strand_cards {
        for target in &s.belongs_to_edges {
            if id_set.contains(target.as_str()) {
                // First valid parent wins
                parent_of.entry(s.id.clone()).or_insert_with(|| target.clone());
                break;
            }
        }
    }

    // 3. Build node map: id → OrientForestNode (children empty for now)
    let mut node_map: HashMap<String, OrientForestNode> = strand_cards
        .iter()
        .map(|(s, card)| {
            (
                s.id.clone(),
                OrientForestNode {
                    card: card.clone(),
                    children: Vec::new(),
                },
            )
        })
        .collect();

    // 4. Identify roots (strands with no parent in the set)
    let child_ids: HashSet<String> = parent_of.keys().cloned().collect();
    let mut root_ids: Vec<String> = strand_cards
        .iter()
        .filter(|(s, _)| !child_ids.contains(&s.id))
        .map(|(s, _)| s.id.clone())
        .collect();

    // 5. Attach children to parents.
    //    Collect (parent_id, child_id, child_last_offset) tuples for sorting.
    let mut children_of: HashMap<String, Vec<(String, usize)>> = HashMap::new();
    for (child_id, parent_id) in &parent_of {
        if let Some(node) = node_map.get(child_id) {
            let offset = node.card.last_offset;
            children_of
                .entry(parent_id.clone())
                .or_default()
                .push((child_id.clone(), offset));
        }
    }

    // Sort children by last_offset descending, then attach
    for (parent_id, mut kids) in children_of {
        kids.sort_by(|a, b| b.1.cmp(&a.1));
        let child_nodes: Vec<OrientForestNode> = kids
            .iter()
            .filter_map(|(cid, _)| node_map.remove(cid))
            .collect();
        if let Some(parent_node) = node_map.get_mut(&parent_id) {
            parent_node.children = child_nodes;
        }
    }

    // 6. Collect roots; sort by last_offset descending
    // Note: some child nodes were removed from node_map above; roots remain.
    root_ids.sort_by(|a, b| {
        let oa = node_map.get(a).map(|n| n.card.last_offset).unwrap_or(0);
        let ob = node_map.get(b).map(|n| n.card.last_offset).unwrap_or(0);
        ob.cmp(&oa)
    });

    root_ids
        .into_iter()
        .filter_map(|id| node_map.remove(&id))
        .collect()
}

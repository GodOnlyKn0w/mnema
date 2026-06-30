use crate::projection::ProjectedStrand;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
struct StrandInfo {
    id: String,
    summary: String,
    status: String,
    state_marker: Option<String>,
    state_offset: usize,
    strand_type: Option<String>,
    entries: usize,
    belongs_to_edges: Vec<String>,
    depends_on_edges: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct StrandGraph {
    nodes: HashMap<String, StrandInfo>,
    belongs_children: HashMap<String, Vec<String>>,
    depends: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct OrientForestNode {
    pub id: String,
    pub strand_type: Option<String>,
    pub entry_count: usize,
    pub summary: String,
    pub last_entry: String,
    pub last_offset: usize,
    pub lifecycle: String,
    pub children: Vec<OrientForestNode>,
}

#[derive(Debug, Clone)]
pub(crate) struct DependsBlocker {
    pub id: String,
    pub status: String,
    pub closed: bool,
    pub summary: String,
}

#[derive(Debug, Clone)]
pub(crate) struct DependsAnalysis {
    pub id: String,
    pub summary: String,
    pub ready: bool,
    pub open_blocker_count: usize,
    pub blockers: Vec<DependsBlocker>,
    pub critical_path: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EdgeFindingKind {
    MultipleBelongsToParents,
    ClosedBelongsToParent,
    DependsOnCycle,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct EdgeFinding {
    pub kind: EdgeFindingKind,
    pub source_id: String,
    pub target_id: Option<String>,
    pub detail: String,
}

impl StrandGraph {
    pub(crate) fn from_strands(strands: &[ProjectedStrand]) -> Self {
        let mut nodes = HashMap::new();
        let mut belongs_children: HashMap<String, Vec<String>> = HashMap::new();
        let mut seen_child: HashMap<String, HashSet<String>> = HashMap::new();
        let mut depends = HashMap::new();

        for s in strands {
            let info = StrandInfo {
                id: s.id.clone(),
                summary: s.first_summary().to_string(),
                status: s.state().to_string(),
                state_marker: s.state_marker.clone(),
                state_offset: s.state_offset,
                strand_type: s.strand_type.clone(),
                entries: s.log_count(),
                belongs_to_edges: s.belongs_to_edges.clone(),
                depends_on_edges: s.depends_on_edges.clone(),
            };
            for parent_id in &info.belongs_to_edges {
                let children = belongs_children.entry(parent_id.clone()).or_default();
                let seen = seen_child.entry(parent_id.clone()).or_default();
                if seen.insert(info.id.clone()) {
                    children.push(info.id.clone());
                }
            }
            depends.insert(info.id.clone(), info.depends_on_edges.clone());
            nodes.insert(info.id.clone(), info);
        }

        Self {
            nodes,
            belongs_children,
            depends,
        }
    }

    pub(crate) fn resolve_id(&self, prefix: &str) -> Option<String> {
        if self.nodes.contains_key(prefix) {
            return Some(prefix.to_string());
        }
        let matches: Vec<&String> = self
            .nodes
            .keys()
            .filter(|k| k.starts_with(prefix))
            .collect();
        if matches.len() == 1 {
            Some(matches[0].clone())
        } else {
            None
        }
    }

    pub(crate) fn subtree_ids(&self, root_id: &str) -> Option<HashSet<String>> {
        let resolved_root = self.resolve_id(root_id)?;
        let mut queue = vec![resolved_root.clone()];
        let mut reachable = HashSet::new();
        reachable.insert(resolved_root);

        while let Some(current) = queue.pop() {
            if let Some(children) = self.belongs_children.get(&current) {
                for child_id in children {
                    if !reachable.contains(child_id) && self.nodes.contains_key(child_id) {
                        reachable.insert(child_id.clone());
                        queue.push(child_id.clone());
                    }
                }
            }
        }
        Some(reachable)
    }

    pub(crate) fn project_tree(&self, root_id: &str) -> Option<TreeNode> {
        let resolved_root = self.resolve_id(root_id)?;
        let reachable = self.subtree_ids(&resolved_root)?;
        self.tree_node(&resolved_root, &reachable, &mut HashSet::new())
    }

    fn tree_node(
        &self,
        id: &str,
        reachable: &HashSet<String>,
        visiting: &mut HashSet<String>,
    ) -> Option<TreeNode> {
        if !visiting.insert(id.to_string()) {
            return None;
        }
        let info = self.nodes.get(id)?;
        let mut children = Vec::new();
        if let Some(child_ids) = self.belongs_children.get(id) {
            for child_id in child_ids {
                if reachable.contains(child_id) {
                    if let Some(child) = self.tree_node(child_id, reachable, visiting) {
                        children.push(child);
                    }
                }
            }
        }
        visiting.remove(id);
        Some(TreeNode {
            id: info.id.clone(),
            summary: info.summary.clone(),
            status: info.status.clone(),
            state_marker: info.state_marker.clone(),
            state_offset: info.state_offset,
            strand_type: info.strand_type.clone(),
            entries: info.entries,
            children,
        })
    }

    pub(crate) fn depends_analysis(&self, id: &str) -> Option<DependsAnalysis> {
        let full_id = self.resolve_id(id)?;
        let node = self.nodes.get(&full_id)?;
        let blockers: Vec<DependsBlocker> = self
            .depends
            .get(&full_id)
            .map(|v| {
                v.iter()
                    .filter_map(|b| self.nodes.get(b))
                    .map(|b| {
                        let closed = b.status.starts_with("closed");
                        DependsBlocker {
                            id: b.id.clone(),
                            status: b.status.clone(),
                            closed,
                            summary: b.summary.clone(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let open_blocker_count = blockers.iter().filter(|b| !b.closed).count();
        let mut seen = HashSet::new();
        seen.insert(full_id.clone());
        let critical_path = self.longest_open_chain(&full_id, &mut seen);
        Some(DependsAnalysis {
            id: full_id,
            summary: node.summary.clone(),
            ready: open_blocker_count == 0,
            open_blocker_count,
            blockers,
            critical_path,
        })
    }

    fn longest_open_chain(&self, node: &str, seen: &mut HashSet<String>) -> Vec<String> {
        let mut best = Vec::new();
        if let Some(ups) = self.depends.get(node) {
            for up in ups {
                let Some(info) = self.nodes.get(up) else {
                    continue;
                };
                if info.status.starts_with("closed") || seen.contains(up) {
                    continue;
                }
                seen.insert(up.clone());
                let mut chain = self.longest_open_chain(up, seen);
                seen.remove(up);
                chain.insert(0, up.clone());
                if chain.len() > best.len() {
                    best = chain;
                }
            }
        }
        best
    }

    pub(crate) fn edge_findings(&self) -> Vec<EdgeFinding> {
        let mut findings = Vec::new();
        for info in self.nodes.values() {
            if info.belongs_to_edges.len() > 1 {
                findings.push(EdgeFinding {
                    kind: EdgeFindingKind::MultipleBelongsToParents,
                    source_id: info.id.clone(),
                    target_id: None,
                    detail: format!(
                        "strand {} has {} belongs-to parents - single-parent basis (D1) expects 1",
                        info.id,
                        info.belongs_to_edges.len()
                    ),
                });
            }
            for parent in &info.belongs_to_edges {
                if self
                    .nodes
                    .get(parent)
                    .map_or(false, |p| p.status.starts_with("closed"))
                {
                    findings.push(EdgeFinding {
                        kind: EdgeFindingKind::ClosedBelongsToParent,
                        source_id: info.id.clone(),
                        target_id: Some(parent.clone()),
                        detail: format!(
                            "strand {} belongs-to a closed parent {} - may warrant review",
                            info.id, parent
                        ),
                    });
                }
            }
        }

        let mut color: HashMap<String, u8> = HashMap::new();
        for start in self.depends.keys().cloned().collect::<Vec<_>>() {
            if color.get(&start).copied().unwrap_or(0) != 0 {
                continue;
            }
            let mut stack: Vec<(String, usize)> = vec![(start.clone(), 0)];
            color.insert(start, 1);
            while let Some((node, idx)) = stack.last().cloned() {
                let children = self.depends.get(&node).cloned().unwrap_or_default();
                if idx < children.len() {
                    stack.last_mut().unwrap().1 += 1;
                    let nx = children[idx].clone();
                    match color.get(&nx).copied().unwrap_or(0) {
                        1 => findings.push(EdgeFinding {
                            kind: EdgeFindingKind::DependsOnCycle,
                            source_id: node.clone(),
                            target_id: Some(nx.clone()),
                            detail: format!("depends-on cycle edge {} -> {} - deadlock", node, nx),
                        }),
                        0 => {
                            color.insert(nx.clone(), 1);
                            stack.push((nx, 0));
                        }
                        _ => {}
                    }
                } else {
                    color.insert(node, 2);
                    stack.pop();
                }
            }
        }
        findings
    }
}

fn orient_forest_from_strands(strands: &[&ProjectedStrand]) -> Vec<OrientForestNode> {
    let id_set: HashSet<&str> = strands.iter().map(|s| s.id.as_str()).collect();
    let mut parent_of: HashMap<String, String> = HashMap::new();
    for s in strands {
        for target in &s.belongs_to_edges {
            if id_set.contains(target.as_str()) {
                parent_of
                    .entry(s.id.clone())
                    .or_insert_with(|| target.clone());
                break;
            }
        }
    }

    let mut node_map: HashMap<String, OrientForestNode> = strands
        .iter()
        .map(|s| {
            (
                s.id.clone(),
                OrientForestNode {
                    id: s.id.clone(),
                    strand_type: s.strand_type.clone(),
                    entry_count: s.log_count(),
                    summary: s.first_summary().to_string(),
                    last_entry: s.last_summary().to_string(),
                    last_offset: s.last_offset(),
                    lifecycle: s.state().to_string(),
                    children: Vec::new(),
                },
            )
        })
        .collect();
    let child_ids: HashSet<String> = parent_of.keys().cloned().collect();
    let mut root_ids: Vec<String> = strands
        .iter()
        .filter(|s| !child_ids.contains(&s.id))
        .map(|s| s.id.clone())
        .collect();

    let mut children_of: HashMap<String, Vec<(String, usize)>> = HashMap::new();
    for (child_id, parent_id) in &parent_of {
        if let Some(node) = node_map.get(child_id) {
            children_of
                .entry(parent_id.clone())
                .or_default()
                .push((child_id.clone(), node.last_offset));
        }
    }
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
    root_ids.sort_by(|a, b| {
        let oa = node_map.get(a).map(|n| n.last_offset).unwrap_or(0);
        let ob = node_map.get(b).map(|n| n.last_offset).unwrap_or(0);
        ob.cmp(&oa)
    });
    root_ids
        .into_iter()
        .filter_map(|id| node_map.remove(&id))
        .collect()
}

pub(crate) fn build_orient_forest(strands: &[&ProjectedStrand]) -> Vec<OrientForestNode> {
    orient_forest_from_strands(strands)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::{LogEntry, ProjectedStrand};

    fn strand(
        id: &str,
        belongs: Vec<&str>,
        depends: Vec<&str>,
        status: &str,
        offset: usize,
    ) -> ProjectedStrand {
        ProjectedStrand {
            id: id.to_string(),
            log: vec![LogEntry {
                offset,
                ts: format!("2026-01-01T00:00:{:02}Z", offset),
                content: format!("summary {id}"),
                ref_: None,
                append_id: None,
                provenance: None,
            }],
            edges: belongs
                .iter()
                .chain(depends.iter())
                .map(|s| s.to_string())
                .collect(),
            belongs_to_edges: belongs.into_iter().map(str::to_string).collect(),
            depends_on_edges: depends.into_iter().map(str::to_string).collect(),
            hidden: false,
            strand_type: None,
            cached_state: Some(status.to_string()),
            state_marker: None,
            state_offset: 0,
        }
    }

    #[test]
    fn belongs_to_subtree_descends_from_parent_to_child() {
        let strands = vec![
            strand("parent", vec![], vec![], "registered", 1),
            strand("child", vec!["parent"], vec![], "registered", 2),
        ];
        let graph = StrandGraph::from_strands(&strands);
        let ids = graph.subtree_ids("parent").unwrap();
        assert!(ids.contains("parent"));
        assert!(ids.contains("child"));
        assert!(!graph.subtree_ids("child").unwrap().contains("parent"));
    }

    #[test]
    fn depends_analysis_ignores_closed_upstream_in_critical_path() {
        let strands = vec![
            strand("task", vec![], vec!["open", "closed"], "registered", 1),
            strand("open", vec![], vec![], "registered", 2),
            strand("closed", vec![], vec![], "closed:done", 3),
        ];
        let graph = StrandGraph::from_strands(&strands);
        let analysis = graph.depends_analysis("task").unwrap();
        assert!(!analysis.ready);
        assert_eq!(analysis.open_blocker_count, 1);
        assert_eq!(analysis.critical_path, vec!["open".to_string()]);
    }

    #[test]
    fn edge_findings_do_not_warn_on_closed_depends_upstream() {
        let strands = vec![
            strand("task", vec![], vec!["closed"], "registered", 1),
            strand("closed", vec![], vec![], "closed:done", 2),
        ];
        let graph = StrandGraph::from_strands(&strands);
        let findings = graph.edge_findings();
        assert!(
            findings.is_empty(),
            "closed depends-on upstream is review context, not lint"
        );
    }
    #[test]
    fn edge_findings_reports_cycle_and_multi_parent() {
        let strands = vec![
            strand("a", vec!["p1", "p2"], vec!["b"], "registered", 1),
            strand("b", vec![], vec!["a"], "registered", 2),
            strand("p1", vec![], vec![], "registered", 3),
            strand("p2", vec![], vec![], "registered", 4),
        ];
        let graph = StrandGraph::from_strands(&strands);
        let findings = graph.edge_findings();
        assert!(
            findings
                .iter()
                .any(|f| f.kind == EdgeFindingKind::MultipleBelongsToParents)
        );
        assert!(
            findings
                .iter()
                .any(|f| f.kind == EdgeFindingKind::DependsOnCycle)
        );
    }
}

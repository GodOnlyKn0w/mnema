use crate::graph::StrandGraph;
use crate::projection::ProjectedStrand;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextPointer {
    pub(crate) kind: &'static str,
    pub(crate) id: String,
    pub(crate) command: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ScopeContext {
    pub(crate) root_id: String,
    pub(crate) pointers: Vec<ContextPointer>,
}

/// The candidate set used by a collection query.
///
/// Scope changes which strands participate; it never changes the query's
/// fields or interpretation. `Subtree` is rooted in the belongs-to forest and
/// includes the root itself plus all descendants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Scope {
    Journal,
    Subtree(String),
}

impl Scope {
    pub(crate) fn journal() -> Self {
        Self::Journal
    }

    pub(crate) fn subtree(root_id: impl Into<String>) -> Self {
        Self::Subtree(root_id.into())
    }

    /// True when this is a journal-wide candidate set (no subtree root).
    pub(crate) fn is_journal(&self) -> bool {
        matches!(self, Self::Journal)
    }

    /// Root id when this is a subtree scope.
    pub(crate) fn root_id(&self) -> Option<&str> {
        match self {
            Self::Journal => None,
            Self::Subtree(id) => Some(id.as_str()),
        }
    }

    pub(crate) fn resolve_ids(
        &self,
        strands: &[ProjectedStrand],
    ) -> Result<HashSet<String>, String> {
        match self {
            Self::Journal => Ok(strands.iter().map(|strand| strand.id.clone()).collect()),
            Self::Subtree(root_id) => StrandGraph::from_strands(strands)
                .subtree_ids(root_id)
                .ok_or_else(|| format!("scope root {} not found or ambiguous", root_id)),
        }
    }

    /// Retain only strands whose id is in this scope. Graph is built from
    /// `universe` (typically the full projection) so descendants resolve even
    /// when the working set is already partially filtered. For Journal scope
    /// this is a no-op.
    ///
    /// `universe` may alias the same storage as `strands` only when the set has
    /// not yet been narrowed (resolve collects ids first, then retain).
    pub(crate) fn retain_strands(
        &self,
        strands: &mut Vec<ProjectedStrand>,
        universe: &[ProjectedStrand],
    ) -> Result<(), String> {
        if self.is_journal() {
            return Ok(());
        }
        let ids = self.resolve_ids(universe)?;
        strands.retain(|strand| ids.contains(&strand.id));
        Ok(())
    }

    /// Direct, deliberately unexpanded context for a delegated entry point.
    /// Descendants belong to the scope itself; ancestors, attention edges and
    /// entry refs remain pointers so orient never manufactures a summary.
    pub(crate) fn context(
        &self,
        universe: &[ProjectedStrand],
    ) -> Result<Option<ScopeContext>, String> {
        let Some(root_id) = self.root_id() else {
            return Ok(None);
        };
        let root = universe
            .iter()
            .find(|strand| strand.id == root_id)
            .ok_or_else(|| format!("scope root {} not found or ambiguous", root_id))?;
        let mut pointers = Vec::new();
        let mut seen = HashSet::new();
        for id in &root.belongs_to_edges {
            if seen.insert(("parent", id.clone())) {
                pointers.push(ContextPointer {
                    kind: "parent",
                    id: id.clone(),
                    command: format!("mnema show --id {} --digest", id),
                });
            }
        }
        for id in &root.depends_on_edges {
            if seen.insert(("depends-on", id.clone())) {
                pointers.push(ContextPointer {
                    kind: "depends-on",
                    id: id.clone(),
                    command: format!("mnema show --id {} --digest", id),
                });
            }
        }
        for entry in &root.log {
            for id in &entry.refs {
                if seen.insert(("ref", id.clone())) {
                    pointers.push(ContextPointer {
                        kind: "ref",
                        id: id.clone(),
                        command: format!("mnema show --entry {} --deref 0", id),
                    });
                }
            }
        }
        Ok(Some(ScopeContext {
            root_id: root.id.clone(),
            pointers,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::LogEntry;

    fn strand(id: &str, parent: Option<&str>, depends_on: &[&str]) -> ProjectedStrand {
        ProjectedStrand {
            id: id.to_string(),
            slug: None,
            log: vec![LogEntry {
                offset: 1,
                ts: "2026-01-01T00:00:00Z".to_string(),
                content: format!("strand {id}"),
                effect: None,
                prev_entry_id: None,
                entry_id: None,
                refs: Vec::new(),
                ref_: None,
                append_id: None,
                provenance: None,
            }],
            edges: parent
                .into_iter()
                .chain(depends_on.iter().copied())
                .map(str::to_string)
                .collect(),
            belongs_to_edges: parent.into_iter().map(str::to_string).collect(),
            depends_on_edges: depends_on.iter().map(|id| (*id).to_string()).collect(),
            hidden: false,
            strand_type: None,
            cached_state: Some("registered".to_string()),
            state_marker: None,
            state_offset: 0,
        }
    }

    #[test]
    fn journal_scope_contains_every_strand() {
        let strands = vec![strand("root", None, &[]), strand("other", None, &[])];
        let ids = Scope::journal().resolve_ids(&strands).unwrap();
        assert_eq!(
            ids,
            HashSet::from(["root".to_string(), "other".to_string()])
        );
    }

    #[test]
    fn subtree_scope_contains_root_and_all_belongs_to_descendants() {
        let strands = vec![
            strand("root", None, &[]),
            strand("child", Some("root"), &[]),
            strand("grandchild", Some("child"), &[]),
            strand("review", None, &["root"]),
        ];
        let ids = Scope::subtree("root").resolve_ids(&strands).unwrap();
        assert_eq!(
            ids,
            HashSet::from([
                "root".to_string(),
                "child".to_string(),
                "grandchild".to_string(),
            ])
        );
        assert!(!ids.contains("review"), "depends-on does not define scope");
    }

    #[test]
    fn missing_subtree_root_is_an_error() {
        let strands = vec![strand("root", None, &[])];
        let error = Scope::subtree("missing").resolve_ids(&strands).unwrap_err();
        assert!(error.contains("scope root missing"));
    }

    #[test]
    fn subtree_context_exposes_pointers_without_expanding_other_strands() {
        let mut root = strand("root", Some("parent"), &["upstream"]);
        root.log[0].refs = vec!["entry-ref".to_string()];
        let strands = vec![
            root,
            strand("parent", None, &[]),
            strand("upstream", None, &[]),
        ];
        let context = Scope::subtree("root").context(&strands).unwrap().unwrap();
        assert_eq!(context.root_id, "root");
        assert_eq!(
            context.pointers.iter().map(|p| p.kind).collect::<Vec<_>>(),
            vec!["parent", "depends-on", "ref"]
        );
        assert!(context.pointers[2].command.ends_with("--deref 0"));
    }
}

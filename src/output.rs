//! JSON output DTOs.
//!
//! These are NOT the internal projection model — they are the external contract.
//! Every field name here is a compatibility commitment. Do not rename fields
//! without updating all consumers (Claude Code, shuttle gate, scripts).
//!
//! # Design rule
//!
//! DTO structs always serialise every field — even when `null` or empty — to
//! match the existing contract. Adding `#[serde(skip_serializing_if)]` would
//! change the output shape and break consumers that expect a field (even if its
//! value is `null`).

use serde::Serialize;

use crate::projection::{OrientView, ProjectedStrand};
use crate::util::truncate;

// ── explain --format json ─────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct ExplainTopicOutput {
    pub(crate) ok: bool,
    pub(crate) topic: String,
    pub(crate) title: String,
    pub(crate) body: String,
}

impl From<&crate::diagnostics::TopicInfo> for ExplainTopicOutput {
    fn from(topic: &crate::diagnostics::TopicInfo) -> Self {
        ExplainTopicOutput {
            ok: true,
            topic: topic.name.to_string(),
            title: topic.title.to_string(),
            body: topic.body.to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct RecoveryInfoOutput {
    pub(crate) kind: &'static str,
    pub(crate) command: String,
    pub(crate) executable: bool,
    pub(crate) requires_human: bool,
}

impl From<&crate::diagnostics::RecoveryInfo> for RecoveryInfoOutput {
    fn from(recovery: &crate::diagnostics::RecoveryInfo) -> Self {
        RecoveryInfoOutput {
            kind: recovery_kind_name(&recovery.kind),
            command: recovery.command_str.to_string(),
            executable: recovery.executable,
            requires_human: recovery.requires_human,
        }
    }
}

fn recovery_kind_name(kind: &crate::diagnostics::RecoveryKind) -> &'static str {
    match kind {
        crate::diagnostics::RecoveryKind::Verify => "verify",
        crate::diagnostics::RecoveryKind::Edit => "edit",
        crate::diagnostics::RecoveryKind::MoveOrRename => "move_or_rename",
        crate::diagnostics::RecoveryKind::CreateCoverStrand => "create_cover_strand",
        crate::diagnostics::RecoveryKind::AppendMarker => "append_marker",
        crate::diagnostics::RecoveryKind::Dispatch => "dispatch",
        crate::diagnostics::RecoveryKind::Cancel => "cancel",
        crate::diagnostics::RecoveryKind::Manual => "manual",
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ExplainSuccessOutput {
    pub(crate) ok: bool,
    pub(crate) code: String,
    pub(crate) severity: &'static str,
    pub(crate) category: String,
    pub(crate) title: String,
    pub(crate) finding: String,
    pub(crate) impact: String,
    pub(crate) recovery: RecoveryInfoOutput,
    pub(crate) producer: String,
}

impl From<&crate::diagnostics::DiagnosticInfo> for ExplainSuccessOutput {
    fn from(diagnostic: &crate::diagnostics::DiagnosticInfo) -> Self {
        ExplainSuccessOutput {
            ok: true,
            code: diagnostic.code.to_string(),
            severity: match diagnostic.severity {
                crate::diagnostics::Severity::Error => "error",
                crate::diagnostics::Severity::Warning => "warning",
            },
            category: diagnostic.category.to_string(),
            title: diagnostic.title.to_string(),
            finding: diagnostic.finding.to_string(),
            impact: diagnostic.impact.to_string(),
            recovery: RecoveryInfoOutput::from(&diagnostic.recovery),
            producer: diagnostic.producer.to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ExplainUnknownOutput<'a> {
    pub(crate) ok: bool,
    pub(crate) input: &'a str,
    pub(crate) error: String,
    pub(crate) available_topics: Vec<&'static str>,
    pub(crate) hint: &'static str,
}

impl<'a> ExplainUnknownOutput<'a> {
    pub(crate) fn new(input: &'a str, available_topics: Vec<&'static str>) -> Self {
        ExplainUnknownOutput {
            ok: false,
            input,
            error: format!("unknown code or topic: {}", input),
            available_topics,
            hint: "diagnostic codes: mnema explain W068 etc",
        }
    }
}
// ── orient --format json ───────────────────────────────────

/// Orient remind line: the operating loop surfaced by orient outputs.
pub(crate) const ORIENT_REMIND: &str = "loop: 做一步·看现实变·再想 | continue → echo \"[decision] ...\" | mnema append --id <ID> | new matter → echo \"<summary>\" | mnema add | matter concluded → mnema close --id <ID> [--as done|failed|cancelled|merged|verified] | before irreversible → mnema checkpoint --id <ID> --action \"<why>\" | writing example → mnema explain writing | read/extract → --format json | jq（id/offset/status，非文本切割）| more → mnema --help";

/// Pause guidance — the one place it can live (CORPUS §8): the tool can't stop
/// the irreversible moment and a cold-start LLM won't go looking for it, so its
/// full text rides on orient. Pause is discipline, not a gate.
pub(crate) const ORIENT_PAUSE: &str = "pause（动手前停一下，是纪律不是关卡）：不可逆或收口状态的动作前，先 mnema checkpoint --id <ID> --action \"<为什么>\" 留一条自省痕——工具拦不住动作本身，只让『停一下』留下痕迹。判据：这一步撤得回吗？撤不回，先 pause。";

/// One active strand in the orient menu.
#[derive(Debug, Serialize, Clone)]
pub struct OrientStrand {
    pub id: String,
    pub slug: Option<String>,
    pub strand_type: Option<String>,
    pub entry_count: usize,
    pub summary: String,
    pub last_entry: String,
    pub last_offset: usize,
    /// Ready-to-run catch-up command for this strand: a recent-content window
    /// (`mnema show --id <id> --tail 8`), not an observer-offset delta.
    pub catch_up: String,
    /// Lifecycle state: "registered" (open) or "closed:<disposition>"
    /// (e.g. "closed:done", "closed:failed"). Set by close/reopen commands,
    /// not by append markers. New field — additive to schema.
    pub lifecycle: String,
}
impl From<&ProjectedStrand> for OrientStrand {
    fn from(s: &ProjectedStrand) -> Self {
        OrientStrand {
            id: s.id.clone(),
            slug: s.slug.clone(),
            strand_type: s.strand_type.clone(),
            entry_count: s.log_count(),
            summary: truncate(s.first_summary(), 70),
            last_entry: truncate(s.last_summary(), 70),
            last_offset: s.last_offset(),
            catch_up: format!("mnema show --id {} --tail 8", crate::util::shorten(&s.id)),
            lifecycle: s.state().to_string(),
        }
    }
}

/// One node in the public `orient --tree --format json` forest.
#[derive(Debug, Serialize, Clone)]
pub struct OrientForestNode {
    #[serde(flatten)]
    pub card: OrientStrand,
    pub children: Vec<OrientForestNode>,
}

impl From<&crate::graph::OrientForestNode> for OrientForestNode {
    fn from(node: &crate::graph::OrientForestNode) -> Self {
        OrientForestNode {
            card: OrientStrand {
                id: node.id.clone(),
                slug: node.slug.clone(),
                strand_type: node.strand_type.clone(),
                entry_count: node.entry_count,
                summary: truncate(&node.summary, 70),
                last_entry: truncate(&node.last_entry, 70),
                last_offset: node.last_offset,
                catch_up: format!(
                    "mnema show --id {} --tail 8",
                    crate::util::shorten(&node.id)
                ),
                lifecycle: node.lifecycle.clone(),
            },
            children: node.children.iter().map(OrientForestNode::from).collect(),
        }
    }
}
/// External contract for `orient --format json`.
#[derive(Debug, Serialize)]
pub struct OrientOutput {
    pub max_offset: usize,
    pub active: Vec<OrientStrand>,
    /// Closed/hidden strands folded to a count (exposure axis: the dead
    /// folds into a scar, retrievable via `list`).
    pub closed_count: usize,
    /// Strands excluded solely because they are hidden (scar principle).
    /// Zero when include_hidden=true (they join the menu/closed pools instead).
    pub hidden_count: usize,
    /// Integrity glance (CORPUS §8, question ①): "ok (...)" or a failure with
    /// the first chain/anchor error. Empty when built without an event stream.
    pub integrity: String,
    /// Needs-judgment notices (question ③): active strands that look done
    /// (closing-annotation marker on the last entry) but aren't closed.
    pub notices: Vec<String>,
    /// Cursor command for reading entries appended after this orient snapshot.
    pub since_command: String,
    /// Stable discovery pointer for recursive asynchronous delegation semantics.
    pub delegation_command: String,
    pub remind: String,
    /// Pause full text (question ④) — see ORIENT_PAUSE.
    pub pause: String,
    /// Active, non-hidden strands whose last entry is older than the orient
    /// stale threshold (2h). Pure read projection; use `mnema list --stale 2h`.
    pub stale_count: usize,
    pub stale_command: String,
    /// Explicit scope metadata. Added without changing the existing orient
    /// fields so JournalScope and SubtreeScope share one public schema.
    pub scope: OrientScopeOutput,
}

#[derive(Debug, Serialize, Clone)]
pub struct OrientContextPointer {
    pub kind: String,
    pub id: String,
    pub command: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct OrientScopeOutput {
    pub kind: String,
    pub root: Option<OrientStrand>,
    pub context: Vec<OrientContextPointer>,
}

impl OrientScopeOutput {
    pub(crate) fn journal() -> Self {
        Self {
            kind: "journal".to_string(),
            root: None,
            context: Vec::new(),
        }
    }
}

impl From<(&OrientView, &[ProjectedStrand])> for OrientOutput {
    fn from((view, strands): (&OrientView, &[ProjectedStrand])) -> Self {
        OrientOutput {
            max_offset: view.max_offset,
            active: view
                .active_ids
                .iter()
                .filter_map(|id| strands.iter().find(|s| &s.id == id))
                .map(OrientStrand::from)
                .collect(),
            closed_count: view.closed_count,
            hidden_count: view.hidden_count,
            // Set by orient_plan, which has the event stream / full strand set.
            integrity: String::new(),
            notices: Vec::new(),
            since_command: format!("mnema timeline --since-offset {}", view.max_offset),
            delegation_command: "mnema explain delegation".to_string(),
            remind: ORIENT_REMIND.to_string(),
            pause: ORIENT_PAUSE.to_string(),
            stale_count: 0,
            stale_command: "mnema list --stale 2h".to_string(),
            scope: OrientScopeOutput::journal(),
        }
    }
}
/// External contract for `orient --tree --format json`.
/// Strands are arranged as a belongs-to forest: strands declaring
/// `belongs-to` edges to other active strands are nested under their parent.
/// Strands with no known active parent appear as roots.
#[derive(Debug, Serialize)]
pub struct OrientTreeOutput {
    pub max_offset: usize,
    /// Forest roots (strands with no belongs-to parent in the active set).
    /// Each root's `children` hold strands that declared `belongs-to` this root.
    pub roots: Vec<OrientForestNode>,
    pub closed_count: usize,
    pub hidden_count: usize,
    pub integrity: String,
    pub notices: Vec<String>,
    pub since_command: String,
    pub delegation_command: String,
    pub remind: String,
    pub pause: String,
    /// Same meaning as `OrientOutput.stale_count`.
    pub stale_count: usize,
    pub stale_command: String,
    pub scope: OrientScopeOutput,
}

// ── query JSON DTOs ────────────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct TreeOutput {
    pub(crate) root: TreeNodeOutput,
}

impl From<&crate::tree::TreeNode> for TreeOutput {
    fn from(root: &crate::tree::TreeNode) -> Self {
        TreeOutput {
            root: TreeNodeOutput::from(root),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct TreeNodeOutput {
    pub(crate) id: String,
    pub(crate) slug: Option<String>,
    pub(crate) summary: String,
    pub(crate) status: String,
    pub(crate) state_marker: Option<String>,
    pub(crate) state_offset: usize,
    pub(crate) strand_type: Option<String>,
    pub(crate) entries: usize,
    pub(crate) children: Vec<TreeNodeOutput>,
}

impl From<&crate::tree::TreeNode> for TreeNodeOutput {
    fn from(node: &crate::tree::TreeNode) -> Self {
        TreeNodeOutput {
            id: node.id.clone(),
            slug: node.slug.clone(),
            summary: node.summary.clone(),
            status: node.status.clone(),
            state_marker: node.state_marker.clone(),
            state_offset: node.state_offset,
            strand_type: node.strand_type.clone(),
            entries: node.entries,
            children: node.children.iter().map(TreeNodeOutput::from).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct DependsUpstreamOutput {
    pub(crate) id: String,
    pub(crate) lifecycle: String,
    pub(crate) summary: String,
    pub(crate) last_entry: String,
    pub(crate) show_command: String,
}

impl From<&crate::graph::DependsUpstream> for DependsUpstreamOutput {
    fn from(upstream: &crate::graph::DependsUpstream) -> Self {
        DependsUpstreamOutput {
            id: upstream.id.clone(),
            lifecycle: upstream.lifecycle.clone(),
            summary: upstream.summary.clone(),
            last_entry: upstream.last_entry.clone(),
            show_command: upstream.show_command.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct DependsOutput {
    pub(crate) id: String,
    pub(crate) summary: String,
    pub(crate) upstream_count: usize,
    pub(crate) registered_upstream_count: usize,
    pub(crate) upstreams: Vec<DependsUpstreamOutput>,
}

impl From<&crate::graph::DependsReview> for DependsOutput {
    fn from(review: &crate::graph::DependsReview) -> Self {
        DependsOutput {
            id: review.id.clone(),
            summary: review.summary.clone(),
            upstream_count: review.upstream_count,
            registered_upstream_count: review.registered_upstream_count,
            upstreams: review
                .upstreams
                .iter()
                .map(DependsUpstreamOutput::from)
                .collect(),
        }
    }
}

/// `depends --under X`: one DependsOutput per strand in SubtreeScope(X).
/// Single-strand `depends` keeps DependsOutput; this shape is under-only.
#[derive(Debug, Serialize)]
pub(crate) struct DependsScopeOutput {
    pub(crate) root_id: String,
    pub(crate) count: usize,
    pub(crate) strands: Vec<DependsOutput>,
}
// ── command result JSON DTOs ───────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct AddOutput<'a> {
    pub(crate) id: String,
    pub(crate) status: &'static str,
    pub(crate) provenance: Option<&'a serde_json::Value>,
    pub(crate) slug: Option<String>,
    pub(crate) parent_id: Option<String>,
    pub(crate) edge_type: Option<&'static str>,
    pub(crate) refs: Vec<String>,
    pub(crate) result: Option<OrientStrand>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SeenOffsetWarningOutput<'a> {
    pub(crate) code: &'a str,
    pub(crate) detail: &'a str,
    pub(crate) seen_offset: usize,
    pub(crate) strand_last_offset: usize,
    pub(crate) seen_gap: usize,
    pub(crate) catch_up: &'a str,
}

impl<'a> From<&'a crate::diagnostics::SeenOffsetWarning> for SeenOffsetWarningOutput<'a> {
    fn from(warning: &'a crate::diagnostics::SeenOffsetWarning) -> Self {
        SeenOffsetWarningOutput {
            code: warning.code,
            detail: &warning.detail,
            seen_offset: warning.seen_offset,
            strand_last_offset: warning.strand_last_offset,
            seen_gap: warning.seen_gap,
            catch_up: &warning.catch_up,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ClosedTargetOutput<'a> {
    pub(crate) code: &'a str,
    pub(crate) detail: &'a str,
    pub(crate) state: &'a str,
    pub(crate) add_from: &'a str,
    pub(crate) reopen: &'a str,
}

impl<'a> From<&'a crate::diagnostics::ClosedTargetWarning> for ClosedTargetOutput<'a> {
    fn from(warning: &'a crate::diagnostics::ClosedTargetWarning) -> Self {
        ClosedTargetOutput {
            code: warning.code,
            detail: &warning.detail,
            state: &warning.state,
            add_from: &warning.add_from,
            reopen: &warning.reopen,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct AppendOutput<'a> {
    pub(crate) strand_id: &'a str,
    pub(crate) entry_id: &'a Option<String>,
    pub(crate) entry_id_prefix: Option<String>,
    pub(crate) content_preview: String,
    pub(crate) refs: &'a [String],
    pub(crate) provenance: &'a Option<serde_json::Value>,
    pub(crate) seen_offset: Option<usize>,
    pub(crate) seen_gap: Option<usize>,
    pub(crate) warnings: Vec<SeenOffsetWarningOutput<'a>>,
    pub(crate) closed_target: Option<ClosedTargetOutput<'a>>,
    pub(crate) result: Option<OrientStrand>,
    /// How the target was chosen when no explicit --id (`most_recent_active_strand`).
    /// Null for explicit --id / --new. Additive field (JSON contract: fields only grow).
    pub(crate) resolved_by: Option<&'a str>,
    /// Active (non-closed) strand count at default-resolve time; null when not default-resolved.
    pub(crate) active_count: Option<usize>,
}

#[derive(Debug, Serialize)]
pub(crate) struct LifecycleOutput {
    pub(crate) strand_id: String,
    pub(crate) disposition: Option<String>,
    pub(crate) lifecycle: String,
    pub(crate) status: &'static str,
    pub(crate) result: Option<OrientStrand>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CheckpointErrorOutput<'a> {
    pub(crate) ok: bool,
    pub(crate) error: &'a str,
    pub(crate) requested_strand: &'a Option<String>,
    pub(crate) resolved_strand: &'a Option<String>,
    pub(crate) journal_appended: bool,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct CheckpointWarningOutput {
    pub(crate) code: String,
    pub(crate) detail: String,
    pub(crate) seen_offset: Option<usize>,
    pub(crate) strand_last_offset: Option<usize>,
    pub(crate) seen_gap: Option<usize>,
    pub(crate) catch_up: Option<String>,
}
#[derive(Debug, Serialize)]
pub(crate) struct CheckpointOutput<'a> {
    pub(crate) ok: bool,
    pub(crate) strand: String,
    pub(crate) resolved_strand: &'a str,
    pub(crate) resolved_by: &'a str,
    pub(crate) observed_entries_before_append: usize,
    pub(crate) shown_entries: usize,
    pub(crate) action: &'a str,
    pub(crate) entry_id: &'a Option<String>,
    pub(crate) entry_id_prefix: Option<String>,
    pub(crate) journal_appended: bool,
    pub(crate) diagnostics_count: usize,
    pub(crate) result: Option<OrientStrand>,
    pub(crate) staleness_seconds: Option<i64>,
    pub(crate) journal_delta: usize,
    pub(crate) seen_offset: Option<usize>,
    pub(crate) seen_gap: Option<usize>,
    pub(crate) catch_up: Option<&'a str>,
    pub(crate) warnings: &'a [CheckpointWarningOutput],
}

#[derive(Debug, Serialize)]
pub(crate) struct FindOutput {
    pub(crate) id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct LinkOutput {
    pub(crate) source_id: String,
    pub(crate) target_id: String,
    pub(crate) edge_type: String,
    pub(crate) status: &'static str,
    pub(crate) result: LinkResultOutput,
}

#[derive(Debug, Serialize)]
pub(crate) struct LinkResultOutput {
    pub(crate) source: Option<OrientStrand>,
    pub(crate) target: Option<OrientStrand>,
}

#[derive(Debug, Serialize)]
pub(crate) struct UnlinkOutput {
    pub(crate) source_id: String,
    pub(crate) target_id: String,
    pub(crate) edge_type: String,
    pub(crate) status: &'static str,
    pub(crate) unlinked: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct VisibilityLedgerOutput {
    pub(crate) strand_id: String,
    pub(crate) status: &'static str,
    pub(crate) noop: bool,
    pub(crate) active_count: usize,
    pub(crate) closed_count: usize,
    pub(crate) hidden_count: usize,
    pub(crate) result: Option<OrientStrand>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CutoverV2ReportOutput {
    pub(crate) applied: bool,
    pub(crate) source_journal: String,
    pub(crate) archive_journal: String,
    pub(crate) map_path: String,
    pub(crate) certificate_path: String,
    pub(crate) source_event_count: usize,
    pub(crate) imported_event_count: usize,
    pub(crate) strand_count: usize,
    pub(crate) entry_count: usize,
    pub(crate) anchor_count: usize,
    pub(crate) unresolved_ref_count: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct CutoverV3ReportOutput {
    pub(crate) applied: bool,
    pub(crate) outcome: String,
    pub(crate) source_journal: String,
    pub(crate) history_journal: String,
    pub(crate) target_journal: String,
    pub(crate) map_path: String,
    pub(crate) certificate_path: String,
    pub(crate) migration_id: String,
    pub(crate) source_event_count: usize,
    pub(crate) target_record_count: usize,
    pub(crate) strand_count: usize,
    pub(crate) entry_count: usize,
    pub(crate) unresolved_ref_count: usize,
    pub(crate) projection_ok: bool,
}

// ── list --format json ─────────────────────────────────────

/// External contract for `list --format json`. One element in the `strands` array.
#[derive(Debug, Serialize)]
pub struct StrandListItem {
    pub id: String,
    pub slug: Option<String>,
    pub entry_count: usize,
    pub first_summary: String,
    pub last_summary: String,
    pub hidden: bool,
    pub strand_type: Option<String>,
    pub edges: Vec<String>,
    /// Typed subsets of `edges` (additive; schema only grows). `belongs_to_edges`
    /// are this strand's parents; `depends_on_edges` are its review upstreams (F3).
    pub belongs_to_edges: Vec<String>,
    pub depends_on_edges: Vec<String>,
    pub status: String,
    pub state_marker: Option<String>,
    pub state_offset: usize,
    pub last_entry_ts: String,
    pub last_entry_offset: usize,
}

/// Top-level list output: `{"strands": [...]}`.
#[derive(Debug, Serialize)]
pub struct StrandListOutput {
    pub strands: Vec<StrandListItem>,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntryEffectOutput {
    Close {
        disposition: String,
    },
    Reopen,
    Link {
        target: String,
        edge_type: String,
    },
    Unlink {
        target: String,
        edge_type: String,
        /// entry_id of the Link entry this reverses (CORPUS §4); null for
        /// legacy key-tombstone unlinks.
        link_entry_id: Option<String>,
    },
    Hide,
    Unhide,
}

impl From<&crate::event::EntryEffect> for EntryEffectOutput {
    fn from(effect: &crate::event::EntryEffect) -> Self {
        match effect {
            crate::event::EntryEffect::Close { disposition } => EntryEffectOutput::Close {
                disposition: disposition.clone(),
            },
            crate::event::EntryEffect::Reopen => EntryEffectOutput::Reopen,
            crate::event::EntryEffect::Link { target, edge_type } => EntryEffectOutput::Link {
                target: target.clone(),
                edge_type: edge_type.clone(),
            },
            crate::event::EntryEffect::Unlink {
                target,
                edge_type,
                link_entry_id,
            } => EntryEffectOutput::Unlink {
                target: target.clone(),
                edge_type: edge_type.clone(),
                link_entry_id: link_entry_id.clone(),
            },
            crate::event::EntryEffect::Hide => EntryEffectOutput::Hide,
            crate::event::EntryEffect::Unhide => EntryEffectOutput::Unhide,
        }
    }
}
// ── show --entry JSON DTOs ──────────────────────────────────

/// One pulled entry in `show --entry --deref` JSON. Flat by design: hop and
/// cited_by are plain fields so jq can regroup by any dimension.
#[derive(Debug, Serialize)]
pub(crate) struct EntryDerefNodeOutput {
    pub(crate) hop: usize,
    pub(crate) cited_by: Option<String>,
    pub(crate) entry_id: String,
    pub(crate) strand_id: String,
    pub(crate) strand_summary: String,
    pub(crate) entry_index: usize,
    pub(crate) strand_entry_count: usize,
    pub(crate) later_entries: usize,
    pub(crate) ts: String,
    pub(crate) content: String,
    pub(crate) effect: Option<EntryEffectOutput>,
    pub(crate) refs: Vec<String>,
    /// Neighbourhood slices from this entry's own line (--before/--after K);
    /// empty arrays when not requested.
    pub(crate) before: Vec<EntryNeighbourOutput>,
    pub(crate) after: Vec<EntryNeighbourOutput>,
}

/// One neighbourhood entry (--before/--after) on a pulled entry's own line.
#[derive(Debug, Serialize)]
pub(crate) struct EntryNeighbourOutput {
    pub(crate) entry_id: Option<String>,
    pub(crate) ts: String,
    pub(crate) content: String,
}

/// A ref that does not resolve locally: pointer reported, target unasserted.
#[derive(Debug, Serialize)]
pub(crate) struct EntryDerefStubOutput {
    pub(crate) hop: usize,
    pub(crate) cited_by: String,
    pub(crate) entry_id: String,
    pub(crate) resolved: bool,
}

/// A ref beyond the requested depth, with the price of expanding it.
#[derive(Debug, Serialize)]
pub(crate) struct EntryFrontierOutput {
    pub(crate) entry_id: String,
    pub(crate) content_len: Option<usize>,
}

/// External contract for `show --entry --format json`.
#[derive(Debug, Serialize)]
pub(crate) struct ShowEntryOutput {
    pub(crate) status: &'static str,
    pub(crate) entry_id: String,
    pub(crate) deref: usize,
    pub(crate) nodes: Vec<EntryDerefNodeOutput>,
    pub(crate) unresolved: Vec<EntryDerefStubOutput>,
    pub(crate) frontier: Vec<EntryFrontierOutput>,
}

// ── show --format json ─────────────────────────────────────

/// One event entry in the `events` array (projection of LogEntry, not the raw struct).
/// The legacy `append_id`/`ref` outputs were retired 2026-07-04 with the
/// dual-track compatibility surface (docs/MIGRATION-v1-to-v2.md §7): entry
/// identity is `entry_id`, rationale pointers are `refs`.
#[derive(Debug, Serialize)]
pub struct EventOutput {
    pub ts: String,
    /// v2 machine effect carried by this entry; null for ordinary notes.
    pub effect: Option<EntryEffectOutput>,
    /// v2 content-addressed identity of this entry. For retained v1 rows this is
    /// the projection-computed effective id; new rows persist the same value.
    pub entry_id: Option<String>,
    /// v2 previous entry hash in this strand. Null on the first entry.
    pub prev_entry_id: Option<String>,
    /// v2 rationale references: entry hashes, not strand@offset pins.
    pub refs: Vec<String>,
    pub entry: String,
    /// Per-entry provenance (e.g. {"producer":"codex"}). Always serialised —
    /// `null` when absent — per the show JSON contract (see module header).
    pub provenance: Option<serde_json::Value>,
}

/// External contract for `show --format json`.
#[derive(Debug, Serialize)]
pub struct StrandDetailOutput {
    pub id: String,
    pub slug: Option<String>,
    pub hidden: bool,
    pub summary: String,
    pub entry_count: usize,
    pub status: String,
    pub state_marker: Option<String>,
    pub state_offset: usize,
    /// Journal offset of this strand's last log entry — the value to pass as
    /// `--seen-offset <N>` on the next write so W076 can detect drift. Mirrors
    /// the list contract's field of the same name. Additive (schema only grows).
    pub last_entry_offset: usize,
    pub edges: Vec<String>,
    /// Typed subsets of `edges` (additive). See StrandListItem for semantics (F3).
    pub belongs_to_edges: Vec<String>,
    pub depends_on_edges: Vec<String>,
    /// Deprecated field; always null; consumers must not rely on this value.
    pub strand_branch: Option<String>,
    pub events: Vec<EventOutput>,
}

// ── search --format json ───────────────────────────────────

/// One match entry in search results.
///
/// Entry-level identity: `entry_id` is the full content-addressed hash of the
/// matching log line (the value needed for `fixes=` / `--why`). `marker` is the
/// leading bracket token without brackets (e.g. `"friction"`), or `""` when the
/// line has no `[...]` prefix. Existing fields are preserved (schema only grows).
#[derive(Debug, Serialize)]
pub struct SearchMatch {
    pub strand_id: String,
    pub content: String,
    pub strand_type: Option<String>,
    pub hidden: bool,
    /// Full entry hash of the matching log line; null only for unprojectable rows.
    pub entry_id: Option<String>,
    /// Leading marker name without brackets; empty string when unmarked.
    pub marker: String,
}

/// Top-level search output.
#[derive(Debug, Serialize)]
pub struct SearchOutput {
    pub matches: Vec<SearchMatch>,
    pub count: usize,
    pub query: String,
    /// Marker filter applied (`--marker`), if any; null when unrestricted.
    pub marker: Option<String>,
}

// ── doctor edges --format json ─────────────────────────────

/// One entry in the edge-discipline dangling report.
#[derive(Debug, Serialize)]
pub struct EdgesItem {
    pub entry_id: String,
    pub strand_id: String,
    pub marker: String,
    pub content: String,
    pub offset: usize,
}

/// External contract for `doctor edges --format json`.
/// Lists open unfixed `[friction]` entries and `[decision]` entries lacking
/// a `--why` ref — the tool's self-check surface for its own edge discipline.
///
/// `open_friction_count` is the total unfixed list length (home strand open/
/// closed does not matter). `open_friction_active_count` is the dual count:
/// how many of those sit on a registered (active) strand.
#[derive(Debug, Serialize)]
pub struct EdgesOutput {
    pub open_frictions: Vec<EdgesItem>,
    pub decisions_without_why: Vec<EdgesItem>,
    pub open_friction_count: usize,
    /// Subset of `open_friction_count` whose home strand is registered/active.
    pub open_friction_active_count: usize,
    pub decision_without_why_count: usize,
}

// ── From impls: projection → DTO ───────────────────────────

impl From<&ProjectedStrand> for StrandListItem {
    fn from(s: &ProjectedStrand) -> Self {
        StrandListItem {
            id: s.id.clone(),
            slug: s.slug.clone(),
            entry_count: s.log_count(),
            first_summary: s.first_summary().to_string(),
            last_summary: s.last_summary().to_string(),
            hidden: s.hidden,
            strand_type: s.strand_type.clone(),
            edges: s.edges.clone(),
            belongs_to_edges: s.belongs_to_edges.clone(),
            depends_on_edges: s.depends_on_edges.clone(),
            status: s.state().to_string(),
            state_marker: s.state_marker.clone(),
            state_offset: s.state_offset,
            last_entry_ts: s.last_ts().to_string(),
            last_entry_offset: s.last_offset(),
        }
    }
}

impl From<&ProjectedStrand> for StrandDetailOutput {
    fn from(s: &ProjectedStrand) -> Self {
        StrandDetailOutput {
            id: s.id.clone(),
            slug: s.slug.clone(),
            hidden: s.hidden,
            summary: s.first_summary().to_string(),
            entry_count: s.log_count(),
            status: s.state().to_string(),
            state_marker: s.state_marker.clone(),
            state_offset: s.state_offset,
            last_entry_offset: s.last_offset(),
            edges: s.edges.clone(),
            belongs_to_edges: s.belongs_to_edges.clone(),
            depends_on_edges: s.depends_on_edges.clone(),
            strand_branch: None, // deprecated; always null
            events: s
                .log
                .iter()
                .map(|e| EventOutput {
                    ts: e.ts.clone(),
                    effect: e.effect.as_ref().map(EntryEffectOutput::from),
                    entry_id: e.entry_id.clone(),
                    prev_entry_id: e.prev_entry_id.clone(),
                    refs: e.refs.clone(),
                    entry: e.content.clone(),
                    provenance: e.provenance.clone(),
                })
                .collect(),
        }
    }
}

// ── timeline --format json ───────────────────────────────────

/// Event kind in timeline JSON output — matches old `#[serde(tag = "kind")]` shape.
#[derive(Debug, Serialize)]
#[serde(tag = "kind")]
pub enum TimelineEventKindOutput {
    #[serde(rename = "strand_created")]
    StrandCreated { summary: Option<String> },
    #[serde(rename = "log_appended")]
    LogAppended {
        content: String,
        effect: Option<EntryEffectOutput>,
    },
    #[serde(rename = "edge_linked")]
    EdgeLinked {
        target_id: String,
        edge_type: Option<String>,
    },
    #[serde(rename = "edge_unlinked")]
    EdgeUnlinked { target_id: String },
    #[serde(rename = "strand_hidden")]
    StrandHidden,
    #[serde(rename = "strand_unhidden")]
    StrandUnhidden,
    #[serde(rename = "checkpoint")]
    CheckpointCreated { observed: String, action: String },
    #[serde(rename = "subject_bound")]
    SubjectBound {
        subject_type: String,
        subject_id: String,
        strand_id: String,
    },
    #[serde(rename = "strand_closed")]
    StrandClosed { disposition: String },
    #[serde(rename = "strand_reopened")]
    StrandReopened,
}

/// One timeline entry in JSON output.
#[derive(Debug, Serialize)]
pub struct TimelineEntryOutput {
    pub journal_offset: usize,
    pub ts: String,
    pub strand_id: String,
    pub strand_type: Option<String>,
    pub kind: TimelineEventKindOutput,
    pub ts_skew: bool,
}

/// Top-level timeline output.
#[derive(Debug, Serialize)]
pub struct TimelineOutput {
    pub timeline: Vec<TimelineEntryOutput>,
    pub truncated: bool,
    pub count: usize,
    pub max_offset: usize,
}

impl From<&crate::projection::TimelineEntry> for TimelineEntryOutput {
    fn from(e: &crate::projection::TimelineEntry) -> Self {
        TimelineEntryOutput {
            journal_offset: e.journal_offset,
            ts: e.ts.clone(),
            strand_id: e.strand_id.clone(),
            strand_type: e.strand_type.clone(),
            kind: match &e.kind {
                crate::projection::TimelineEventKind::StrandCreated { summary } => {
                    TimelineEventKindOutput::StrandCreated {
                        summary: summary.clone(),
                    }
                }
                crate::projection::TimelineEventKind::LogAppended { content, effect } => {
                    TimelineEventKindOutput::LogAppended {
                        content: content.clone(),
                        effect: effect.as_ref().map(EntryEffectOutput::from),
                    }
                }
                crate::projection::TimelineEventKind::EdgeLinked {
                    target_id,
                    edge_type,
                } => TimelineEventKindOutput::EdgeLinked {
                    target_id: target_id.clone(),
                    edge_type: edge_type.clone(),
                },
                crate::projection::TimelineEventKind::EdgeUnlinked { target_id } => {
                    TimelineEventKindOutput::EdgeUnlinked {
                        target_id: target_id.clone(),
                    }
                }
                crate::projection::TimelineEventKind::StrandHidden => {
                    TimelineEventKindOutput::StrandHidden
                }
                crate::projection::TimelineEventKind::StrandUnhidden => {
                    TimelineEventKindOutput::StrandUnhidden
                }
                crate::projection::TimelineEventKind::CheckpointCreated { observed, action } => {
                    TimelineEventKindOutput::CheckpointCreated {
                        observed: observed.clone(),
                        action: action.clone(),
                    }
                }
                crate::projection::TimelineEventKind::SubjectBound {
                    subject_type,
                    subject_id,
                    strand_id,
                } => TimelineEventKindOutput::SubjectBound {
                    subject_type: subject_type.clone(),
                    subject_id: subject_id.clone(),
                    strand_id: strand_id.clone(),
                },
                crate::projection::TimelineEventKind::StrandClosed { disposition } => {
                    TimelineEventKindOutput::StrandClosed {
                        disposition: disposition.clone(),
                    }
                }
                crate::projection::TimelineEventKind::StrandReopened => {
                    TimelineEventKindOutput::StrandReopened
                }
            },
            ts_skew: e.ts_skew,
        }
    }
}

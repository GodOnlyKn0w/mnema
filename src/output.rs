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
            hint: "diagnostic codes: tasktree explain W062 etc",
        }
    }
}
// ── orient --format json ───────────────────────────────────

/// Orient remind line: the operating loop surfaced by orient outputs.
pub(crate) const ORIENT_REMIND: &str = "loop: 做一步·看现实变·再想 | continue → append --id <ID> \"[decision] ...\" | new matter → add \"<summary>\" | matter concluded → close --id <ID> [--as done|failed|cancelled|merged|verified] | read/extract → --format json | jq（id/offset/status，非文本切割）| more → tasktree --help\npause（行动前自省，然后自愿 append 你的推理）：\n  这一步会在世界上留下改不回的后果——你别的一切都还在 journal 里可复看可重判，这一个一旦发生就收不回，而发生前这一刻是唯一能介入的点。平实说出：它做下哪个改不回的改动？（改动本身，具体地，不是\"我打算做X\"）这正是你此刻想要的那个改动吗？若不完全是——停下重想，这是你唯一能改主意的点。写下来，再去做。";

/// One active strand in the orient menu.
#[derive(Debug, Serialize, Clone)]
pub struct OrientStrand {
    pub id: String,
    pub strand_type: Option<String>,
    pub entry_count: usize,
    pub summary: String,
    pub last_entry: String,
    pub last_offset: usize,
    /// Ready-to-run catch-up command for this strand (ADR-0003: the cursor
    /// lives on the strand's last_offset, not on an observer).
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
            strand_type: s.strand_type.clone(),
            entry_count: s.log_count(),
            summary: truncate(s.first_summary(), 70),
            last_entry: truncate(s.last_summary(), 70),
            last_offset: s.last_offset(),
            catch_up: format!("tasktree show --id {} --tail 8", s.id),
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
                strand_type: node.strand_type.clone(),
                entry_count: node.entry_count,
                summary: truncate(&node.summary, 70),
                last_entry: truncate(&node.last_entry, 70),
                last_offset: node.last_offset,
                catch_up: format!("tasktree show --id {} --tail 8", node.id),
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
    pub remind: String,
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
            remind: ORIENT_REMIND.to_string(),
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
    pub remind: String,
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
pub(crate) struct DependsBlockerOutput {
    pub(crate) id: String,
    pub(crate) status: String,
    pub(crate) closed: bool,
}

impl From<&crate::graph::DependsBlocker> for DependsBlockerOutput {
    fn from(blocker: &crate::graph::DependsBlocker) -> Self {
        DependsBlockerOutput {
            id: blocker.id.clone(),
            status: blocker.status.clone(),
            closed: blocker.closed,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct DependsOutput {
    pub(crate) id: String,
    pub(crate) summary: String,
    pub(crate) ready: bool,
    pub(crate) open_blocker_count: usize,
    pub(crate) blockers: Vec<DependsBlockerOutput>,
    pub(crate) critical_path: Vec<String>,
    pub(crate) critical_path_len: usize,
}

impl From<&crate::graph::DependsAnalysis> for DependsOutput {
    fn from(analysis: &crate::graph::DependsAnalysis) -> Self {
        DependsOutput {
            id: analysis.id.clone(),
            summary: analysis.summary.clone(),
            ready: analysis.ready,
            open_blocker_count: analysis.open_blocker_count,
            blockers: analysis
                .blockers
                .iter()
                .map(DependsBlockerOutput::from)
                .collect(),
            critical_path: analysis.critical_path.clone(),
            critical_path_len: analysis.critical_path.len(),
        }
    }
}
// ── command result JSON DTOs ───────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct AddOutput<'a> {
    pub(crate) id: String,
    pub(crate) status: &'static str,
    pub(crate) provenance: Option<&'a serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) edge_type: Option<&'static str>,
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
pub(crate) struct AppendOutput<'a> {
    pub(crate) strand_id: &'a str,
    pub(crate) append_id: &'a Option<String>,
    pub(crate) content_preview: String,
    pub(crate) provenance: &'a Option<serde_json::Value>,
    pub(crate) seen_offset: Option<usize>,
    pub(crate) seen_gap: Option<usize>,
    pub(crate) warnings: Vec<SeenOffsetWarningOutput<'a>>,
    pub(crate) result: Option<OrientStrand>,
}

#[derive(Debug, Serialize)]
pub(crate) struct LifecycleOutput {
    pub(crate) strand_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) disposition: Option<String>,
    pub(crate) lifecycle: String,
    pub(crate) status: &'static str,
    pub(crate) result: Option<OrientStrand>,
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

// ── list --format json ─────────────────────────────────────

/// External contract for `list --format json`. One element in the `strands` array.
#[derive(Debug, Serialize)]
pub struct StrandListItem {
    pub id: String,
    pub entry_count: usize,
    pub first_summary: String,
    pub last_summary: String,
    /// Structured marker prefixes of `first_summary`/`last_summary` (additive, W5).
    /// The summary fields still carry the full original line. Empty string when
    /// the summary has no marker.
    pub first_marker: String,
    pub last_marker: String,
    pub hidden: bool,
    pub strand_type: Option<String>,
    pub edges: Vec<String>,
    /// Typed subsets of `edges` (additive; schema only grows). `belongs_to_edges`
    /// are this strand's parents; `depends_on_edges` are its blockers (F3 — makes
    /// depends-on a queryable typed view instead of write-only).
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

// ── show --format json ─────────────────────────────────────

/// One event entry in the `events` array (projection of LogEntry, not the raw struct).
#[derive(Debug, Serialize)]
pub struct EventOutput {
    pub ts: String,
    pub append_id: Option<String>,
    pub entry: String,
    /// Structured marker prefix of `entry` (e.g. `"[decision]"`), split out so
    /// consumers can `select`/`group_by` on it without parsing the raw line.
    /// Additive (W5): `entry` still carries the full original line. Empty string
    /// (never null) when the entry has no marker; unknown/misspelled markers pass
    /// through verbatim (no vocabulary lookup).
    pub marker: String,
    /// Per-entry provenance (e.g. {"producer":"codex"}). Always serialised —
    /// `null` when absent — per the show JSON contract (see module header).
    pub provenance: Option<serde_json::Value>,
    /// Entry rationale pointer (D2/F4): the reserved `ref_` field surfaced so a
    /// recorded reason-reference is queryable. Always serialised; null when absent.
    #[serde(rename = "ref")]
    pub ref_field: Option<String>,
}

/// External contract for `show --format json`.
#[derive(Debug, Serialize)]
pub struct StrandDetailOutput {
    pub id: String,
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
#[derive(Debug, Serialize)]
pub struct SearchMatch {
    pub strand_id: String,
    pub content: String,
    pub strand_type: Option<String>,
    pub hidden: bool,
}

/// Top-level search output.
#[derive(Debug, Serialize)]
pub struct SearchOutput {
    pub matches: Vec<SearchMatch>,
    pub count: usize,
    pub query: String,
}

// ── doctor journal --format json ───────────────────────────

/// One lint section in the doctor JSON report.
#[derive(Debug, Serialize)]
pub struct DoctorLintSectionOutput {
    pub name: String,
    pub summary_label: String,
    pub count: usize,
    pub findings: Vec<String>,
}

/// One diagnostic in the doctor JSON report. Projects the internal
/// `(code, detail)` tuple into a jq-friendly `{code, detail}` object.
#[derive(Debug, Serialize)]
pub struct DoctorDiagnosticOutput {
    pub code: String,
    pub detail: String,
}

/// External contract for `doctor journal --format json` (W5, additive command
/// surface). A projection of the internal `DoctorJournalReport` — the tuple-typed
/// `diagnostics` are reshaped into `{code, detail}` objects so jq consumers see
/// structured fields rather than nested arrays.
#[derive(Debug, Serialize)]
pub struct DoctorReportOutput {
    pub journal: String,
    pub total_lines: usize,
    pub corrupted: usize,
    pub orphans: Vec<String>,
    pub total_strands: usize,
    pub strands_with_events: usize,
    pub noise_strands: usize,
    pub timeline_status: String,
    pub timeline_warning: bool,
    pub lint_sections: Vec<DoctorLintSectionOutput>,
    pub lint_count: usize,
    pub diagnostics: Vec<DoctorDiagnosticOutput>,
    pub has_errors: bool,
    pub has_advisories: bool,
}

impl DoctorReportOutput {
    pub fn from_report(
        journal: String,
        report: &crate::diagnostics::DoctorJournalReport,
    ) -> Self {
        DoctorReportOutput {
            journal,
            total_lines: report.total_lines,
            corrupted: report.corrupted,
            orphans: report.orphans.clone(),
            total_strands: report.total_strands,
            strands_with_events: report.strands_with_events_count,
            noise_strands: report.noise_strands_count,
            timeline_status: report.timeline_status.clone(),
            timeline_warning: report.timeline_warning,
            lint_sections: report
                .audit
                .lint_sections
                .iter()
                .map(|s| DoctorLintSectionOutput {
                    name: s.name.to_string(),
                    summary_label: s.summary_label.to_string(),
                    count: s.count(),
                    findings: s.findings.clone(),
                })
                .collect(),
            lint_count: report.audit.lint_count(),
            diagnostics: report
                .audit
                .diagnostics
                .iter()
                .map(|(code, detail)| DoctorDiagnosticOutput {
                    code: code.clone(),
                    detail: detail.clone(),
                })
                .collect(),
            has_errors: report.has_errors(),
            has_advisories: report.has_advisories(),
        }
    }
}

// ── From impls: projection → DTO ───────────────────────────

impl From<&ProjectedStrand> for StrandListItem {
    fn from(s: &ProjectedStrand) -> Self {
        StrandListItem {
            id: s.id.clone(),
            entry_count: s.log_count(),
            first_summary: s.first_summary().to_string(),
            last_summary: s.last_summary().to_string(),
            first_marker: crate::markers::split_marker(s.first_summary()).0.to_string(),
            last_marker: crate::markers::split_marker(s.last_summary()).0.to_string(),
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
                    append_id: e.append_id.clone(),
                    entry: e.content.clone(),
                    marker: crate::markers::split_marker(&e.content).0.to_string(),
                    provenance: e.provenance.clone(),
                    ref_field: e.ref_.clone(),
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
    StrandCreated {
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    #[serde(rename = "log_appended")]
    LogAppended {
        content: String,
        /// Structured marker prefix of `content` (additive, W5). `content` still
        /// carries the full original line. Empty string when no marker.
        marker: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        append_id: Option<String>,
    },
    #[serde(rename = "edge_linked")]
    EdgeLinked {
        target_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        edge_type: Option<String>,
    },
    #[serde(rename = "edge_unlinked")]
    EdgeUnlinked { target_id: String },
    #[serde(rename = "strand_hidden")]
    StrandHidden,
    #[serde(rename = "strand_unhidden")]
    StrandUnhidden,
    #[serde(rename = "checkpoint")]
    CheckpointCreated {
        observed: String,
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        append_id: Option<String>,
    },
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

fn is_false(b: &bool) -> bool {
    !b
}

/// One timeline entry in JSON output.
#[derive(Debug, Serialize)]
pub struct TimelineEntryOutput {
    pub journal_offset: usize,
    pub ts: String,
    pub strand_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strand_type: Option<String>,
    pub kind: TimelineEventKindOutput,
    #[serde(skip_serializing_if = "is_false")]
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
                crate::projection::TimelineEventKind::LogAppended { content, append_id } => {
                    TimelineEventKindOutput::LogAppended {
                        content: content.clone(),
                        marker: crate::markers::split_marker(content).0.to_string(),
                        append_id: append_id.clone(),
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
                crate::projection::TimelineEventKind::CheckpointCreated {
                    observed,
                    action,
                    append_id,
                } => TimelineEventKindOutput::CheckpointCreated {
                    observed: observed.clone(),
                    action: action.clone(),
                    append_id: append_id.clone(),
                },
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

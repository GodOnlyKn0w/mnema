//! JSON output DTOs.
//!
//! These are NOT the internal projection model — they are the external contract.
//! Every field name here is a compatibility commitment. Do not rename fields
//! without updating all consumers (Claude Code, shuttle gate, scripts).
//!
//! # Design rule
//!
//! Every field is always serialised — even when `null` or empty — to match the
//! existing contract. The current `json!({...})` code paths always emit every
//! field. Adding `#[serde(skip_serializing_if)]` would change the output shape
//! and break consumers that expect a field (even if its value is `null`).

use serde::Serialize;

use crate::projection::ProjectedStrand;

// ── orient --format json ───────────────────────────────────

/// One active strand in the orient menu.
#[derive(Debug, Serialize)]
pub struct OrientStrand {
    pub id: String,
    pub strand_type: Option<String>,
    pub entries: usize,
    pub summary: String,
    pub last_entry: String,
    pub last_offset: usize,
    /// Ready-to-run catch-up command for this strand (ADR-0003: the cursor
    /// lives on the strand's last_offset, not on an observer).
    pub catch_up: String,
}

/// External contract for `orient --format json`.
#[derive(Debug, Serialize)]
pub struct OrientOutput {
    pub max_offset: usize,
    pub active: Vec<OrientStrand>,
    /// Closed/hidden strands folded to a count (exposure axis: the dead
    /// folds into a scar, retrievable via `list`).
    pub closed_count: usize,
    pub remind: String,
}

// ── list --format json ─────────────────────────────────────

/// External contract for `list --format json`. One element in the `strands` array.
#[derive(Debug, Serialize)]
pub struct StrandListItem {
    pub id: String,
    pub entries: usize,
    pub first_summary: String,
    pub last_summary: String,
    pub hidden: bool,
    pub strand_type: Option<String>,
    pub edges: Vec<String>,
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
}

/// External contract for `show --format json`.
#[derive(Debug, Serialize)]
pub struct StrandDetailOutput {
    pub id: String,
    pub hidden: bool,
    pub summary: String,
    pub entries: usize,
    pub status: String,
    pub state_marker: Option<String>,
    pub state_offset: usize,
    pub edges: Vec<String>,
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

// ── From impls: projection → DTO ───────────────────────────

impl From<&ProjectedStrand> for StrandListItem {
    fn from(s: &ProjectedStrand) -> Self {
        StrandListItem {
            id: s.id.clone(),
            entries: s.log_count(),
            first_summary: s.first_summary().to_string(),
            last_summary: s.last_summary().to_string(),
            hidden: s.hidden,
            strand_type: s.strand_type.clone(),
            edges: s.edges.clone(),
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
            entries: s.log_count(),
            status: s.state().to_string(),
            state_marker: s.state_marker.clone(),
            state_offset: s.state_offset,
            edges: s.edges.clone(),
            strand_branch: None,   // deprecated; always null
            events: s.log.iter().map(|e| EventOutput {
                ts: e.ts.clone(),
                append_id: e.append_id.clone(),
                entry: e.content.clone(),
            }).collect(),
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
    EdgeUnlinked {
        target_id: String,
    },
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
}

fn is_false(b: &bool) -> bool { !b }

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

impl From<&crate::event::TimelineEntry> for TimelineEntryOutput {
    fn from(e: &crate::event::TimelineEntry) -> Self {
        TimelineEntryOutput {
            journal_offset: e.journal_offset,
            ts: e.ts.clone(),
            strand_id: e.strand_id.clone(),
            strand_type: e.strand_type.clone(),
            kind: match &e.kind {
                crate::event::TimelineEventKind::StrandCreated { summary } =>
                    TimelineEventKindOutput::StrandCreated { summary: summary.clone() },
                crate::event::TimelineEventKind::LogAppended { content, append_id } =>
                    TimelineEventKindOutput::LogAppended { content: content.clone(), append_id: append_id.clone() },
                crate::event::TimelineEventKind::EdgeLinked { target_id, edge_type } =>
                    TimelineEventKindOutput::EdgeLinked { target_id: target_id.clone(), edge_type: edge_type.clone() },
                crate::event::TimelineEventKind::EdgeUnlinked { target_id } =>
                    TimelineEventKindOutput::EdgeUnlinked { target_id: target_id.clone() },
                crate::event::TimelineEventKind::StrandHidden =>
                    TimelineEventKindOutput::StrandHidden,
                crate::event::TimelineEventKind::StrandUnhidden =>
                    TimelineEventKindOutput::StrandUnhidden,
                crate::event::TimelineEventKind::CheckpointCreated { observed, action, append_id } =>
                    TimelineEventKindOutput::CheckpointCreated {
                        observed: observed.clone(),
                        action: action.clone(),
                        append_id: append_id.clone(),
                    },
                crate::event::TimelineEventKind::SubjectBound { subject_type, subject_id, strand_id } =>
                    TimelineEventKindOutput::SubjectBound {
                        subject_type: subject_type.clone(),
                        subject_id: subject_id.clone(),
                        strand_id: strand_id.clone(),
                    },
            },
            ts_skew: e.ts_skew,
        }
    }
}

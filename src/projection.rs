//! Tasktree journal projection layer.
//! Projects raw event streams into structured strand and timeline views.

use crate::event::{Event, TimelineEntry, TimelineEventKind};
use std::collections::HashSet;

/// Collapse repeated values, keeping the first occurrence of each (order
/// preserved). Used to fold duplicate edge links at the read layer.
fn dedup_preserve_order<I: Iterator<Item = String>>(iter: I) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for v in iter {
        if seen.insert(v.clone()) {
            out.push(v);
        }
    }
    out
}

// ── Log Entry ──────────────────────────────────────────────

#[derive(Debug)]
pub struct LogEntry {
    pub offset: usize,
    pub ts: String,
    pub content: String,
    pub ref_: Option<String>,
    pub append_id: Option<String>,
    pub provenance: Option<serde_json::Value>,
}

// ── State Markers ──────────────────────────────────────────

/// Legacy content-based state markers — kept for display/annotation purposes
/// only. These no longer affect compute_state; lifecycle state is set
/// exclusively by StrandClosed / StrandReopened events.
pub const STATE_MARKERS: &[&str] = &[
    "[merged]",
    "[cancelled]",
    "[failed]",
    "[verified]",
    "[done]",
    "[dispatched]",
    "[registered]",
];

/// Valid close dispositions accepted by `tasktree close --as <DISPOSITION>`.
pub const CLOSE_DISPOSITIONS: &[&str] = &["done", "failed", "cancelled", "merged", "verified"];

/// Compute canonical lifecycle state from raw events (not log content).
/// Only StrandClosed and StrandReopened events affect state;
/// the last such event wins. No events → "registered" (open).
///
/// Returns (state_str, disposition_or_empty, deciding_offset).
/// state_str: "registered" (open) or "closed:<disposition>"
/// disposition_or_empty: the disposition string when closed, empty when open
/// deciding_offset: journal offset of the deciding event (0 when no event)
pub fn compute_state_from_events(
    raw_events: &[(usize, crate::event::Event)],
    strand_id: &str,
) -> (String, String, usize) {
    use crate::event::Event;
    let mut last: Option<(usize, &Event)> = None;
    for (offset, event) in raw_events {
        if event.strand_id() != strand_id {
            continue;
        }
        match event {
            Event::StrandClosed { .. } | Event::StrandReopened { .. } => {
                last = Some((*offset, event));
            }
            _ => {}
        }
    }
    match last {
        Some((offset, Event::StrandClosed { disposition, .. })) => (
            format!("closed:{}", disposition),
            disposition.clone(),
            offset,
        ),
        Some((_, Event::StrandReopened { .. })) => ("registered".to_string(), String::new(), 0),
        _ => ("registered".to_string(), String::new(), 0),
    }
}

/// Compute canonical state from log entries (legacy stub — used only for
/// test compatibility during the transition. Prefer compute_state_from_events
/// when the event stream is available).
/// Returns (state, marker_name, marker_offset).
pub fn compute_state(log: &[LogEntry]) -> (String, String, usize) {
    let _ = log; // legacy markers no longer drive state
    ("registered".to_string(), String::new(), 0)
}

// ── Projected Strand ───────────────────────────────────────

/// Internal projection model. Not serialised directly.
/// Consumers (text renderer, DTO layer) read via accessor methods.
#[derive(Debug)]
pub struct ProjectedStrand {
    pub id: String,
    pub log: Vec<LogEntry>,
    pub edges: Vec<String>,
    /// Target IDs of edges whose edge_type is "belongs-to". Subset of `edges`.
    /// Used by orient --tree to build the belongs-to forest.
    pub belongs_to_edges: Vec<String>,
    /// Target IDs of edges whose edge_type is "depends-on" (F3). Subset of
    /// `edges`. Makes depends-on a typed, queryable view instead of write-only:
    /// the targets are this strand's blockers (SOURCE depends-on TARGET).
    pub depends_on_edges: Vec<String>,
    pub hidden: bool,
    pub strand_type: Option<String>,
    pub cached_state: Option<String>,
    pub state_marker: Option<String>,
    pub state_offset: usize,
}

impl ProjectedStrand {
    pub fn first_summary(&self) -> &str {
        self.log
            .first()
            .map(|l| l.content.as_str())
            .unwrap_or("(empty)")
    }

    pub fn last_summary(&self) -> &str {
        self.log
            .last()
            .map(|l| l.content.as_str())
            .unwrap_or("(empty)")
    }

    pub fn last_ts(&self) -> &str {
        self.log.last().map(|l| l.ts.as_str()).unwrap_or("")
    }

    pub fn last_offset(&self) -> usize {
        self.log.last().map(|l| l.offset).unwrap_or(0)
    }

    pub fn log_count(&self) -> usize {
        self.log.len()
    }

    /// Lazy accessor for canonical state. Returns one of the 7 marker values
    /// or "registered" (default) if no state marker is found in the log.
    pub fn state(&self) -> &str {
        self.cached_state.as_deref().unwrap_or("registered")
    }
}

// ── Orient view ────────────────────────────────────────────

/// Internal derived view for `orient`.
///
/// This is projection state, not the public JSON contract. It keeps only the
/// selected active strand IDs plus fold counts; Contract Surface maps those IDs
/// back to DTO cards.
#[derive(Debug)]
pub struct OrientView {
    pub max_offset: usize,
    pub active_ids: Vec<String>,
    pub closed_count: usize,
    pub hidden_count: usize,
}

/// Build the orient menu view from a full strand projection.
///
/// `strands` must include hidden strands so the default view can exclude them
/// while still reporting `hidden_count`.
pub fn build_orient_view(
    strands: &[ProjectedStrand],
    include_hidden: bool,
    limit: usize,
    max_offset: usize,
) -> OrientView {
    let hidden_count = if include_hidden {
        0
    } else {
        strands.iter().filter(|s| s.hidden).count()
    };
    let visible: Vec<&ProjectedStrand> = strands
        .iter()
        .filter(|s| !s.hidden || include_hidden)
        .collect();
    let mut active: Vec<&ProjectedStrand> = visible
        .iter()
        .copied()
        .filter(|s| s.state() == "registered")
        .collect();
    let closed_count = visible.len() - active.len();

    active.sort_by(|a, b| b.last_offset().cmp(&a.last_offset()));
    active.truncate(limit);

    OrientView {
        max_offset,
        active_ids: active.iter().map(|s| s.id.clone()).collect(),
        closed_count,
        hidden_count,
    }
}
// ── Entry point: project_raw → structured ──────────────────

/// Project raw event stream into a Vec<ProjectedStrand>.
/// Each strand is aggregated from all its events (created, log entries, edges, hide toggles).
/// Hidden state is derived from StrandHidden/StrandUnhidden balance, not a stored flag.
///
/// When `include_hidden` is false, strands with `hidden == true` are filtered out of
/// the returned vector. Callers that need to inspect a known hidden strand explicitly
/// (e.g. `cmd_show <id>`) should call `project_strands(..., true)` and look up by id.
pub fn project_strands(events: &[(usize, Event)], include_hidden: bool) -> Vec<ProjectedStrand> {
    use std::collections::BTreeMap;
    let mut by_id: BTreeMap<String, Vec<(usize, &Event)>> = BTreeMap::new();
    for (offset, event) in events {
        by_id
            .entry(event.strand_id().to_string())
            .or_default()
            .push((*offset, event));
    }
    let mut nodes = Vec::new();
    for (_id, node_events) in by_id {
        let mut hide_count: i32 = 0;
        for (_offset, e) in node_events.iter() {
            match e {
                Event::StrandHidden { .. } => hide_count += 1,
                Event::StrandUnhidden { .. } => hide_count -= 1,
                _ => {}
            }
        }
        let hidden = hide_count > 0;
        let has_created = node_events
            .iter()
            .any(|(_, e)| matches!(e, Event::StrandCreated { .. }));
        if !has_created {
            continue;
        }
        // Collect log entries
        let logs: Vec<LogEntry> = node_events
            .iter()
            .filter_map(|(offset, e)| {
                if let Event::LogAppended {
                    ts,
                    content,
                    ref_,
                    append_id,
                    provenance,
                    ..
                } = e
                {
                    Some(LogEntry {
                        offset: *offset,
                        ts: ts.clone(),
                        content: content.clone(),
                        ref_: ref_.clone(),
                        append_id: append_id.clone(),
                        provenance: provenance.clone(),
                    })
                } else {
                    None
                }
            })
            .collect();
        // Fold link/unlink into the live edge set (F5). Key is (to, edge_type);
        // last write wins (EdgeLinked=true, EdgeUnlinked=false). First-occurrence
        // order is preserved, so for a journal with no unlinks this reduces to the
        // old first-wins dedup — belongs-to ordering (tree/orient) is unchanged.
        // The journal keeps every event (append-only); folding only shapes reads.
        let mut edge_live: std::collections::HashMap<(String, Option<String>), bool> =
            std::collections::HashMap::new();
        let mut edge_order: Vec<(String, Option<String>)> = Vec::new();
        for (_, e) in &node_events {
            let (to, etype, linked) = match e {
                Event::EdgeLinked { to, edge_type, .. } => (to, edge_type, true),
                Event::EdgeUnlinked { to, edge_type, .. } => (to, edge_type, false),
                _ => continue,
            };
            let key = (to.clone(), etype.clone());
            if !edge_live.contains_key(&key) {
                edge_order.push(key.clone());
            }
            edge_live.insert(key, linked);
        }
        let live: Vec<&(String, Option<String>)> =
            edge_order.iter().filter(|k| edge_live[*k]).collect();
        // edges = all live targets, deduped by target id (a target reachable via
        // two edge types lists once).
        let edges: Vec<String> = dedup_preserve_order(live.iter().map(|(to, _)| to.clone()));
        let belongs_to_edges: Vec<String> = live
            .iter()
            .filter(|(_, et)| et.as_deref() == Some("belongs-to"))
            .map(|(to, _)| to.clone())
            .collect();
        // depends-on subset (F3): typed view of blockers.
        let depends_on_edges: Vec<String> = live
            .iter()
            .filter(|(_, et)| et.as_deref() == Some("depends-on"))
            .map(|(to, _)| to.clone())
            .collect();
        // Extract strand_type from StrandCreated event
        let strand_type: Option<String> = node_events.iter().find_map(|(_, e)| {
            if let Event::StrandCreated { strand_type, .. } = e {
                strand_type.clone()
            } else {
                None
            }
        });
        let strand_id_str = node_events[0].1.strand_id().to_string();
        let (state, state_marker, state_offset) = compute_state_from_events(events, &strand_id_str);
        if !include_hidden && hidden {
            continue;
        }
        nodes.push(ProjectedStrand {
            id: strand_id_str,
            log: logs,
            edges,
            belongs_to_edges,
            depends_on_edges,
            hidden,
            strand_type,
            cached_state: Some(state),
            state_marker: Some(state_marker),
            state_offset,
        });
    }
    nodes
}

/// Project all events onto a timeline ordered by journal_offset.
pub fn project_timeline(events: &[(usize, Event)]) -> Vec<TimelineEntry> {
    let mut entries: Vec<TimelineEntry> = Vec::new();
    let mut prev_ts: Option<String> = None;
    // Collect strand_type from StrandCreated events
    let mut strand_types: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    for (_, event) in events {
        if let Event::StrandCreated {
            id, strand_type, ..
        } = event
        {
            strand_types.insert(id.clone(), strand_type.clone());
        }
    }
    for (offset, event) in events {
        let strand_id = event.strand_id().to_string();
        let strand_type = strand_types.get(&strand_id).cloned().flatten();
        let ts = match event {
            Event::StrandCreated { ts, .. } => ts,
            Event::LogAppended { ts, .. } => ts,
            Event::EdgeLinked { ts, .. } => ts,
            Event::EdgeUnlinked { ts, .. } => ts,
            Event::StrandHidden { ts, .. } => ts,
            Event::StrandUnhidden { ts, .. } => ts,
            Event::CheckpointCreated { ts, .. } => ts,
            Event::SubjectBound { ts, .. } => ts,
            Event::StrandClosed { ts, .. } => ts,
            Event::StrandReopened { ts, .. } => ts,
        };
        let ts_str = ts.clone();
        let ts_skew = match &prev_ts {
            Some(prev) if ts_str < *prev => true,
            _ => false,
        };
        prev_ts = Some(ts_str.clone());
        let kind = match event {
            Event::StrandCreated { .. } => TimelineEventKind::StrandCreated { summary: None },
            Event::LogAppended {
                content, append_id, ..
            } => TimelineEventKind::LogAppended {
                content: content.clone(),
                append_id: append_id.clone(),
            },
            Event::EdgeLinked { to, edge_type, .. } => TimelineEventKind::EdgeLinked {
                target_id: to.clone(),
                edge_type: edge_type.clone(),
            },
            Event::EdgeUnlinked { to, .. } => TimelineEventKind::EdgeUnlinked {
                target_id: to.clone(),
            },
            Event::StrandHidden { .. } => TimelineEventKind::StrandHidden,
            Event::StrandUnhidden { .. } => TimelineEventKind::StrandUnhidden,
            Event::CheckpointCreated {
                observed,
                action,
                append_id,
                ..
            } => TimelineEventKind::CheckpointCreated {
                observed: observed.clone(),
                action: action.clone(),
                append_id: append_id.clone(),
            },
            Event::SubjectBound {
                subject_type,
                subject_id,
                strand_id,
                ..
            } => TimelineEventKind::SubjectBound {
                subject_type: subject_type.clone(),
                subject_id: subject_id.clone(),
                strand_id: strand_id.clone(),
            },
            Event::StrandClosed { disposition, .. } => TimelineEventKind::StrandClosed {
                disposition: disposition.clone(),
            },
            Event::StrandReopened { .. } => TimelineEventKind::StrandReopened,
        };
        entries.push(TimelineEntry {
            journal_offset: *offset,
            ts: ts_str,
            strand_id,
            strand_type,
            kind,
            ts_skew,
        });
    }
    entries.sort_by_key(|e| e.journal_offset);
    entries
}

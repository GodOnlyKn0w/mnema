//! Tasktree journal projection layer.
//! Projects raw event streams into structured strand and timeline views.

use crate::event::{Event, TimelineEntry, TimelineEventKind};

// ── Log Entry ──────────────────────────────────────────────

#[derive(Debug)]
pub struct LogEntry {
    pub offset: usize,
    pub ts: String,
    pub content: String,
    pub ref_: Option<String>,
    pub append_id: Option<String>,
}

// ── State Markers ──────────────────────────────────────────

const STATE_MARKERS: &[&str] = &[
    "[merged]",
    "[cancelled]",
    "[failed]",
    "[verified]",
    "[done]",
    "[dispatched]",
    "[registered]",
];

/// Compute canonical state from log entries by priority scan.
/// Returns (state, marker_name, marker_offset).
/// state: one of merged/cancelled/failed/verified/done/dispatched/registered
/// marker_name: the bracket prefix that decided the state (e.g. "[verified]")
/// marker_offset: journal_offset of the deciding log entry (0 if no marker)
pub fn compute_state(log: &[LogEntry]) -> (String, String, usize) {
    for marker in STATE_MARKERS {
        for entry in log {
            if entry.content.starts_with(marker) {
                let state = marker
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_string();
                return (state, marker.to_string(), entry.offset);
            }
        }
    }
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
                    ..
                } = e
                {
                    Some(LogEntry {
                        offset: *offset,
                        ts: ts.clone(),
                        content: content.clone(),
                        ref_: ref_.clone(),
                        append_id: append_id.clone(),
                    })
                } else {
                    None
                }
            })
            .collect();
        // Collect edges
        let edges: Vec<String> = node_events
            .iter()
            .filter_map(|(_, e)| {
                if let Event::EdgeLinked { to, .. } = e {
                    Some(to.clone())
                } else {
                    None
                }
            })
            .collect();
        // Extract strand_type from StrandCreated event
        let strand_type: Option<String> = node_events.iter().find_map(|(_, e)| {
            if let Event::StrandCreated { strand_type, .. } = e {
                strand_type.clone()
            } else {
                None
            }
        });
        let (state, state_marker, state_offset) = compute_state(&logs);
        if !include_hidden && hidden {
            continue;
        }
        nodes.push(ProjectedStrand {
            id: node_events[0].1.strand_id().to_string(),
            log: logs,
            edges,
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
        if let Event::StrandCreated { id, strand_type, .. } = event {
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
        };
        let ts_str = ts.clone();
        let ts_skew = match &prev_ts {
            Some(prev) if ts_str < *prev => true,
            _ => false,
        };
        prev_ts = Some(ts_str.clone());
        let kind = match event {
            Event::StrandCreated { .. } => TimelineEventKind::StrandCreated { summary: None },
            Event::LogAppended { content, append_id, .. } => TimelineEventKind::LogAppended {
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
            Event::CheckpointCreated { observed, action, append_id, .. } => {
                TimelineEventKind::CheckpointCreated {
                    observed: observed.clone(),
                    action: action.clone(),
                    append_id: append_id.clone(),
                }
            }
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

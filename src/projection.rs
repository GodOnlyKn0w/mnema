//! Tasktree journal projection layer.
//! Projects raw event streams into structured strand and timeline views.

use crate::event::{EntryEffect, Event};
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
    pub effect: Option<EntryEffect>,
    pub prev_entry_id: Option<String>,
    pub entry_id: Option<String>,
    pub refs: Vec<String>,
    pub ref_: Option<String>,
    pub append_id: Option<String>,
    pub provenance: Option<serde_json::Value>,
}

// ── Timeline Projection types ────────────────────────────

/// A single event in timeline projection.
///
/// Data model only — serialization lives in `output.rs` DTOs.
#[derive(Debug, Clone)]
pub struct TimelineEntry {
    pub journal_offset: usize,
    pub ts: String,
    pub strand_id: String,
    pub strand_type: Option<String>,
    pub kind: TimelineEventKind,
    pub ts_skew: bool,
}

/// Event kind in timeline projection.
///
/// Data model only — serialization (tagged union) lives in `output.rs` DTOs.
/// Pattern matching on this enum is the intended consumer interface.
#[derive(Debug, Clone)]
pub enum TimelineEventKind {
    StrandCreated {
        summary: Option<String>,
    },
    LogAppended {
        content: String,
        append_id: Option<String>,
        effect: Option<EntryEffect>,
    },
    EdgeLinked {
        target_id: String,
        edge_type: Option<String>,
    },
    EdgeUnlinked {
        target_id: String,
    },
    StrandHidden,
    StrandUnhidden,
    CheckpointCreated {
        observed: String,
        action: String,
        append_id: Option<String>,
    },
    SubjectBound {
        subject_type: String,
        subject_id: String,
        strand_id: String,
    },
    StrandClosed {
        disposition: String,
    },
    StrandReopened,
}
// ── State Markers ──────────────────────────────────────────

/// Legacy content-based state markers — kept for display/annotation purposes
/// only. These no longer affect compute_state; lifecycle state is set
/// by legacy StrandClosed / StrandReopened events and v2 close/reopen effects.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EdgeDelta {
    pub(crate) target: String,
    pub(crate) edge_type: Option<String>,
    pub(crate) linked: bool,
}

pub(crate) fn lifecycle_effect(event: &Event) -> Option<EntryEffect> {
    match event {
        Event::StrandClosed { disposition, .. } => Some(EntryEffect::Close {
            disposition: disposition.clone(),
        }),
        Event::StrandReopened { .. } => Some(EntryEffect::Reopen),
        Event::LogAppended {
            effect: Some(effect @ (EntryEffect::Close { .. } | EntryEffect::Reopen)),
            ..
        } => Some(effect.clone()),
        _ => None,
    }
}

pub(crate) fn visibility_delta(event: &Event) -> Option<i32> {
    match event {
        Event::StrandHidden { .. }
        | Event::LogAppended {
            effect: Some(EntryEffect::Hide),
            ..
        } => Some(1),
        Event::StrandUnhidden { .. }
        | Event::LogAppended {
            effect: Some(EntryEffect::Unhide),
            ..
        } => Some(-1),
        _ => None,
    }
}

pub(crate) fn edge_delta(event: &Event) -> Option<EdgeDelta> {
    match event {
        Event::EdgeLinked { to, edge_type, .. } => Some(EdgeDelta {
            target: to.clone(),
            edge_type: edge_type.clone(),
            linked: true,
        }),
        Event::EdgeUnlinked { to, edge_type, .. } => Some(EdgeDelta {
            target: to.clone(),
            edge_type: edge_type.clone(),
            linked: false,
        }),
        Event::LogAppended {
            effect: Some(EntryEffect::Link { target, edge_type }),
            ..
        } => Some(EdgeDelta {
            target: target.clone(),
            edge_type: Some(edge_type.clone()),
            linked: true,
        }),
        Event::LogAppended {
            effect: Some(EntryEffect::Unlink { target, edge_type }),
            ..
        } => Some(EdgeDelta {
            target: target.clone(),
            edge_type: Some(edge_type.clone()),
            linked: false,
        }),
        _ => None,
    }
}
/// Compute canonical lifecycle state from raw events (not log content).
/// Only legacy StrandClosed/StrandReopened events and v2 close/reopen effects affect state;
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
    let mut last: Option<(usize, EntryEffect)> = None;
    for (offset, event) in raw_events {
        if event.strand_id() != Some(strand_id) {
            continue;
        }
        if let Some(effect) = lifecycle_effect(event) {
            last = Some((*offset, effect));
        }
    }
    match last {
        Some((offset, EntryEffect::Close { disposition })) => {
            (format!("closed:{}", disposition), disposition, offset)
        }
        Some((_, EntryEffect::Reopen)) => ("registered".to_string(), String::new(), 0),
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
// ── Context view ───────────────────────────────────────────

#[derive(Debug)]
pub(crate) struct FrictionPairing {
    pub(crate) paired_friction: std::collections::HashSet<usize>,
    pub(crate) paired_fixed: std::collections::HashSet<usize>,
    pub(crate) scar_content: std::collections::HashMap<usize, String>,
    pub(crate) dangling_fixes: Vec<(usize, String)>,
}

pub(crate) fn pair_frictions(log: &[LogEntry]) -> FrictionPairing {
    use std::collections::{HashMap, HashSet};

    let mut friction_by_append_id: Vec<(String, usize)> = Vec::new();

    let mut paired_friction: HashSet<usize> = HashSet::new();
    let mut paired_fixed: HashSet<usize> = HashSet::new();
    let mut scar_content: HashMap<usize, String> = HashMap::new();
    let mut dangling_fixes: Vec<(usize, String)> = Vec::new();

    for (idx, entry) in log.iter().enumerate() {
        if entry.content.starts_with("[friction]") {
            if let Some(ref aid) = entry.append_id {
                if !aid.is_empty() {
                    friction_by_append_id.push((aid.clone(), idx));
                }
            }
        }
    }

    for (idx, entry) in log.iter().enumerate() {
        if !entry.content.starts_with("[fixed]") {
            continue;
        }

        let fixes_prefix: Option<String> = {
            let body = entry.content.trim_start_matches("[fixed]").trim();
            let mut found = None;
            for token in body.split_whitespace() {
                if let Some(prefix) = token.strip_prefix("fixes=") {
                    if prefix.len() >= 8 {
                        found = Some(prefix.to_string());
                    }
                    break;
                }
            }
            found
        };

        let prefix = match fixes_prefix {
            None => continue,
            Some(p) => p,
        };

        let matched: Option<usize> = friction_by_append_id.iter().find_map(|(aid, fidx)| {
            if aid.starts_with(prefix.as_str()) && !paired_friction.contains(fidx) {
                Some(*fidx)
            } else {
                None
            }
        });

        match matched {
            Some(fidx) => {
                let friction_body = log[fidx].content.trim_start_matches("[friction]").trim();
                let truncated: String = friction_body.chars().take(50).collect();
                let scar = format!("{} → fixed", truncated);

                paired_friction.insert(fidx);
                paired_fixed.insert(idx);
                scar_content.insert(fidx, scar);
            }
            None => {
                dangling_fixes.push((idx, prefix));
            }
        }
    }

    FrictionPairing {
        paired_friction,
        paired_fixed,
        scar_content,
        dangling_fixes,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ContextView {
    pub(crate) strands: Vec<ContextStrand>,
    pub(crate) warnings: Vec<ContextWarning>,
}

#[derive(Debug, Clone)]
pub(crate) struct ContextWarning {
    pub(crate) code: &'static str,
    pub(crate) strand_id: String,
    pub(crate) fixes_prefix: String,
    pub(crate) entry_offset: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct FoldedCounts {
    pub(crate) progress: usize,
    pub(crate) observed: usize,
    pub(crate) check: usize,
}

impl FoldedCounts {
    pub(crate) fn zero() -> Self {
        FoldedCounts {
            progress: 0,
            observed: 0,
            check: 0,
        }
    }
    pub(crate) fn any_folded(&self) -> bool {
        self.progress > 0 || self.observed > 0 || self.check > 0
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ContextStrand {
    pub(crate) id: String,
    pub(crate) covers: Vec<String>,
    pub(crate) entries: Vec<ContextEntry>,
    pub(crate) friction_folded: usize,
    pub(crate) friction_paired: usize,
    pub(crate) folded_counts: FoldedCounts,
}

#[derive(Debug, Clone)]
pub(crate) struct ContextEntry {
    pub(crate) marker: String,
    pub(crate) content: String,
    pub(crate) offset: usize,
    pub(crate) ts: String,
}

pub(crate) fn build_context_view(
    strands: &[ProjectedStrand],
    target_type: &str,
    covers: &[String],
    since_offset: Option<usize>,
    exclude_friction: bool,
    include_observations: bool,
) -> ContextView {
    let mut matching: Vec<&ProjectedStrand> = strands
        .iter()
        .filter(|s| s.strand_type.as_deref() == Some(target_type))
        .collect();

    if let Some(so) = since_offset {
        matching.retain(|s| s.last_offset() > so);
    }

    let mut output_strands: Vec<ContextStrand> = Vec::new();
    let mut warnings: Vec<ContextWarning> = Vec::new();
    const OBS_MARKERS: [&str; 3] = ["[progress]", "[observed]", "[check]"];

    for strand in &matching {
        let covers_list: Vec<String> = strand
            .log
            .iter()
            .filter(|e| e.content.starts_with("[covers]"))
            .map(|e| e.content.trim_start_matches("[covers]").trim().to_string())
            .collect();

        if !covers.is_empty() {
            let has_match = covers_list
                .iter()
                .any(|c| covers.iter().any(|p| c.contains(p.as_str())));
            if !has_match {
                continue;
            }
        }

        let strand_is_live = strand.state() == "registered";
        let mut friction_folded = 0usize;

        let pairing = if strand_is_live && !exclude_friction {
            pair_frictions(&strand.log)
        } else {
            FrictionPairing {
                paired_friction: std::collections::HashSet::new(),
                paired_fixed: std::collections::HashSet::new(),
                scar_content: std::collections::HashMap::new(),
                dangling_fixes: Vec::new(),
            }
        };
        for (fix_idx, prefix) in &pairing.dangling_fixes {
            let fix_entry = &strand.log[*fix_idx];
            warnings.push(ContextWarning {
                code: "W075",
                strand_id: strand.id.clone(),
                fixes_prefix: prefix.clone(),
                entry_offset: fix_entry.offset,
            });
        }
        let friction_paired = pairing.paired_friction.len();

        let mut last_obs_idx: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        if !include_observations {
            for (idx, entry) in strand.log.iter().enumerate() {
                for &om in &OBS_MARKERS {
                    if entry.content.starts_with(om) {
                        last_obs_idx.insert(om, idx);
                    }
                }
            }
        }

        let mut folded_counts = FoldedCounts::zero();

        let entries: Vec<ContextEntry> = strand
            .log
            .iter()
            .enumerate()
            .filter_map(|(idx, e)| {
                if e.content.starts_with("[friction]") {
                    if exclude_friction {
                        return None;
                    }
                    if !strand_is_live {
                        friction_folded += 1;
                        return None;
                    }
                    if pairing.paired_friction.contains(&idx) {
                        let scar = pairing
                            .scar_content
                            .get(&idx)
                            .cloned()
                            .unwrap_or_else(|| "→ fixed".to_string());
                        return Some(ContextEntry {
                            marker: "[friction]".to_string(),
                            content: scar,
                            offset: e.offset,
                            ts: e.ts.clone(),
                        });
                    }
                    let (marker, content) = crate::markers::split_marker(&e.content);
                    return Some(ContextEntry {
                        marker: marker.to_string(),
                        content: content.to_string(),
                        offset: e.offset,
                        ts: e.ts.clone(),
                    });
                }

                if e.content.starts_with("[fixed]") {
                    if pairing.paired_fixed.contains(&idx) {
                        return None;
                    }
                    let (marker, content) = crate::markers::split_marker(&e.content);
                    return Some(ContextEntry {
                        marker: marker.to_string(),
                        content: content.to_string(),
                        offset: e.offset,
                        ts: e.ts.clone(),
                    });
                }

                if e.content.starts_with("[covers]") {
                    return None;
                }

                if !include_observations {
                    for &om in &OBS_MARKERS {
                        if e.content.starts_with(om) {
                            let tail_idx = last_obs_idx.get(om).copied().unwrap_or(idx);
                            if idx != tail_idx {
                                match om {
                                    "[progress]" => folded_counts.progress += 1,
                                    "[observed]" => folded_counts.observed += 1,
                                    "[check]" => folded_counts.check += 1,
                                    _ => {}
                                }
                                return None;
                            }
                            break;
                        }
                    }
                }

                let (marker, content) = crate::markers::split_marker(&e.content);
                Some(ContextEntry {
                    marker: marker.to_string(),
                    content: content.to_string(),
                    offset: e.offset,
                    ts: e.ts.clone(),
                })
            })
            .collect();

        if entries.is_empty() {
            continue;
        }

        let mut unique_covers: Vec<String> = Vec::new();
        for c in &covers_list {
            if !unique_covers.contains(c) {
                unique_covers.push(c.clone());
            }
        }

        output_strands.push(ContextStrand {
            id: strand.id.clone(),
            covers: unique_covers,
            entries,
            friction_folded,
            friction_paired,
            folded_counts,
        });
    }

    output_strands.sort_by(|a, b| {
        let ts_a = a.entries.last().map(|e| e.ts.as_str()).unwrap_or("");
        let ts_b = b.entries.last().map(|e| e.ts.as_str()).unwrap_or("");
        ts_b.cmp(ts_a)
    });

    ContextView {
        strands: output_strands,
        warnings,
    }
}

#[cfg(test)]
pub(crate) fn build_context_strands(
    strands: &[ProjectedStrand],
    target_type: &str,
    covers: &[String],
    since_offset: Option<usize>,
    exclude_friction: bool,
    include_observations: bool,
) -> Vec<ContextStrand> {
    build_context_view(
        strands,
        target_type,
        covers,
        since_offset,
        exclude_friction,
        include_observations,
    )
    .strands
}

// ── Small derived views ───────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VisibilityLedger {
    pub(crate) active_count: usize,
    pub(crate) closed_count: usize,
    pub(crate) hidden_count: usize,
}

pub(crate) fn project_visibility_ledger(strands: &[ProjectedStrand]) -> VisibilityLedger {
    let hidden_count = strands.iter().filter(|s| s.hidden).count();
    let visible_count = strands.len() - hidden_count;
    let active_count = strands
        .iter()
        .filter(|s| !s.hidden && s.state() == "registered")
        .count();
    VisibilityLedger {
        active_count,
        closed_count: visible_count - active_count,
        hidden_count,
    }
}

/// Balance of legacy visibility events and v2 hide/unhide effects for one strand.
pub(crate) fn hide_balance(events: &[(usize, Event)], strand_id: &str) -> i32 {
    let mut count: i32 = 0;
    for (_, event) in events {
        if event.strand_id() != Some(strand_id) {
            continue;
        }
        if let Some(delta) = visibility_delta(event) {
            count += delta;
        }
    }
    count
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CurrentBinding {
    pub(crate) binding_id: String,
    pub(crate) ts: String,
    pub(crate) strand_id: String,
}

pub(crate) fn current_binding(
    events: &[(usize, Event)],
    subject_type: &str,
    subject_id: &str,
) -> Option<CurrentBinding> {
    let mut latest: Option<CurrentBinding> = None;
    for (_offset, event) in events {
        if let Event::SubjectBound {
            id,
            ts,
            subject_type: event_subject_type,
            subject_id: event_subject_id,
            strand_id,
            ..
        } = event
        {
            if event_subject_type == subject_type && event_subject_id == subject_id {
                match &latest {
                    Some(prev) if ts.as_str() <= prev.ts.as_str() => {}
                    _ => {
                        latest = Some(CurrentBinding {
                            binding_id: id.clone(),
                            ts: ts.clone(),
                            strand_id: strand_id.clone(),
                        })
                    }
                }
            }
        }
    }
    latest
}
// ── Entry point: project_raw → structured ──────────────────

/// Project raw event stream into a Vec<ProjectedStrand>.
/// Each strand is aggregated from all its events (created, log entries, edges, visibility toggles).
/// Hidden state is derived from legacy StrandHidden/StrandUnhidden rows and v2 hide/unhide effects.
///
/// When `include_hidden` is false, strands with `hidden == true` are filtered out of
/// the returned vector. Callers that need to inspect a known hidden strand explicitly
/// (e.g. `cmd_show <id>`) should call `project_strands(..., true)` and look up by id.
pub fn project_strands(events: &[(usize, Event)], include_hidden: bool) -> Vec<ProjectedStrand> {
    use std::collections::BTreeMap;
    let mut by_id: BTreeMap<String, Vec<(usize, &Event)>> = BTreeMap::new();
    for (offset, event) in events {
        let Some(strand_id) = event.strand_id() else {
            continue;
        };
        by_id
            .entry(strand_id.to_string())
            .or_default()
            .push((*offset, event));
    }
    let mut nodes = Vec::new();
    for (_id, node_events) in by_id {
        let mut hide_count: i32 = 0;
        for (_offset, e) in node_events.iter() {
            if let Some(delta) = visibility_delta(e) {
                hide_count += delta;
            }
        }
        let hidden = hide_count > 0;
        let has_created = node_events
            .iter()
            .any(|(_, e)| matches!(e, Event::StrandCreated { .. }));
        if !has_created {
            continue;
        }
        // Collect log entries and compute an effective v2 hash chain. Retained
        // v1 rows have no stored entry_id, so the projection gives them a
        // deterministic virtual identity and lets new entries chain forward.
        let mut logs: Vec<LogEntry> = Vec::new();
        let mut previous_effective_entry_id: Option<String> = None;
        for (offset, e) in node_events.iter() {
            if let Event::LogAppended {
                ts,
                content,
                prev_entry_id,
                entry_id,
                refs,
                ref_,
                append_id,
                effect,
                git,
                provenance,
                ..
            } = e
            {
                let effective_prev = prev_entry_id
                    .clone()
                    .or_else(|| previous_effective_entry_id.clone());
                let effective_entry_id = crate::event::effective_entry_id(
                    entry_id.as_deref(),
                    effective_prev.as_deref(),
                    ts,
                    content,
                    refs,
                    effect.as_ref(),
                    provenance.as_ref(),
                    git.as_ref(),
                );
                logs.push(LogEntry {
                    offset: *offset,
                    ts: ts.clone(),
                    content: content.clone(),
                    effect: effect.clone(),
                    prev_entry_id: effective_prev,
                    entry_id: Some(effective_entry_id.clone()),
                    refs: refs.clone(),
                    ref_: ref_.clone(),
                    append_id: append_id.clone(),
                    provenance: provenance.clone(),
                });
                previous_effective_entry_id = Some(effective_entry_id);
            }
        }
        // Fold legacy edge events and v2 link/unlink effects into the live edge set (F5).
        // Key is (to, edge_type); last write wins. First-occurrence
        // order is preserved, so for a journal with no unlinks this reduces to the
        // old first-wins dedup — belongs-to ordering (tree/orient) is unchanged.
        // The journal keeps every event (append-only); folding only shapes reads.
        let mut edge_live: std::collections::HashMap<(String, Option<String>), bool> =
            std::collections::HashMap::new();
        let mut edge_order: Vec<(String, Option<String>)> = Vec::new();
        for (_, e) in &node_events {
            let Some(delta) = edge_delta(e) else {
                continue;
            };
            let key = (delta.target, delta.edge_type);
            if !edge_live.contains_key(&key) {
                edge_order.push(key.clone());
            }
            edge_live.insert(key, delta.linked);
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
        let strand_id_str = node_events[0]
            .1
            .strand_id()
            .expect("strand-scoped event")
            .to_string();
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
        if matches!(event, Event::JournalAnchored { .. }) {
            continue;
        }
        let strand_id = match event {
            Event::SubjectBound { strand_id, .. } => strand_id.clone(),
            Event::JournalAnchored { .. } => unreachable!("journal anchors are skipped above"),
            _ => event
                .strand_id()
                .expect("strand-scoped timeline event")
                .to_string(),
        };
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
            Event::JournalAnchored { .. } => unreachable!("journal anchors are skipped above"),
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
                content,
                append_id,
                effect,
                ..
            } => TimelineEventKind::LogAppended {
                content: content.clone(),
                append_id: append_id.clone(),
                effect: effect.clone(),
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
            Event::JournalAnchored { .. } => unreachable!("journal anchors are skipped above"),
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

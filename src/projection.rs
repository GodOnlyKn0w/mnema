//! Mnema journal projection layer.
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

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub offset: usize,
    pub ts: String,
    pub content: String,
    pub effect: Option<EntryEffect>,
    pub prev_entry_id: Option<String>,
    pub entry_id: Option<String>,
    pub refs: Vec<String>,
    /// Legacy v1 strand@offset pin — stored for replay fidelity, not read.
    #[allow(dead_code)]
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

/// Valid close dispositions — owned by the event factory alongside the
/// close effect constructor; re-exported here for read-side callers.
pub use crate::event::CLOSE_DISPOSITIONS;

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
            effect: Some(EntryEffect::Unlink {
                target, edge_type, ..
            }),
            ..
        } => Some(EdgeDelta {
            target: target.clone(),
            edge_type: Some(edge_type.clone()),
            linked: false,
        }),
        _ => None,
    }
}
/// Fold the last close/reopen lifecycle effect into canonical strand state.
///
/// Returns (state_str, disposition_or_empty, deciding_offset).
/// state_str: "registered" (open) or "closed:<disposition>"
/// disposition_or_empty: the disposition string when closed, empty when open
/// deciding_offset: journal offset of the deciding event (0 when open / no event)
fn fold_lifecycle_state(last: Option<(usize, EntryEffect)>) -> (String, String, usize) {
    match last {
        Some((offset, EntryEffect::Close { disposition })) => {
            (format!("closed:{}", disposition), disposition, offset)
        }
        Some((_, EntryEffect::Reopen)) => ("registered".to_string(), String::new(), 0),
        _ => ("registered".to_string(), String::new(), 0),
    }
}

/// Compute canonical lifecycle state from raw events (not log content).
/// Only legacy StrandClosed/StrandReopened events and v2 close/reopen effects affect state;
/// the last such event wins. No events → "registered" (open).
///
/// Scans the full stream filtering by `strand_id` — fine for single-strand lookups
/// (write gates). `project_strands` uses [`compute_state_from_strand_events`] on the
/// already-grouped per-strand slice so full projection stays O(E), not O(S×E).
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
    fold_lifecycle_state(last)
}

/// Same lifecycle fold as [`compute_state_from_events`], but over events already
/// scoped to one strand (e.g. `project_strands` grouping). Avoids re-scanning the
/// entire journal once per strand.
fn compute_state_from_strand_events(node_events: &[(usize, &Event)]) -> (String, String, usize) {
    let mut last: Option<(usize, EntryEffect)> = None;
    for (offset, event) in node_events {
        if let Some(effect) = lifecycle_effect(event) {
            last = Some((*offset, effect));
        }
    }
    fold_lifecycle_state(last)
}

// ── Projected Strand ───────────────────────────────────────

/// Internal projection model. Not serialised directly.
/// Consumers (text renderer, DTO layer) read via accessor methods.
#[derive(Debug)]
pub struct ProjectedStrand {
    pub id: String,
    pub slug: Option<String>,
    pub log: Vec<LogEntry>,
    pub edges: Vec<String>,
    /// Target IDs of edges whose edge_type is "belongs-to". Subset of `edges`.
    /// Used by orient --tree to build the belongs-to forest.
    pub belongs_to_edges: Vec<String>,
    /// Target IDs of edges whose edge_type is "depends-on" (F3). Subset of
    /// `edges`. Makes depends-on a typed, queryable view instead of write-only:
    /// the targets are this strand's review upstreams (SOURCE depends-on TARGET).
    pub depends_on_edges: Vec<String>,
    pub hidden: bool,
    pub strand_type: Option<String>,
    pub cached_state: Option<String>,
    pub state_marker: Option<String>,
    pub state_offset: usize,
}

impl ProjectedStrand {
    /// Request-scoped view with only one writer's entries (matched on
    /// provenance.producer). Display narrowing for multi-writer journals —
    /// the strand's durable state (lifecycle, edges) is untouched.
    pub(crate) fn with_producer_filter(&self, name: &str) -> ProjectedStrand {
        ProjectedStrand {
            id: self.id.clone(),
            slug: self.slug.clone(),
            log: self
                .log
                .iter()
                .filter(|e| {
                    e.provenance
                        .as_ref()
                        .and_then(|p| p.get("producer"))
                        .and_then(|v| v.as_str())
                        == Some(name)
                })
                .cloned()
                .collect(),
            edges: self.edges.clone(),
            belongs_to_edges: self.belongs_to_edges.clone(),
            depends_on_edges: self.depends_on_edges.clone(),
            hidden: self.hidden,
            strand_type: self.strand_type.clone(),
            cached_state: self.cached_state.clone(),
            state_marker: self.state_marker.clone(),
            state_offset: self.state_offset,
        }
    }

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

/// entry_ids of currently-live Link instances on `strand` to
/// (target, edge_type), in append order (CORPUS §4). Used by unlink to name a
/// specific live link to reverse; the most recent is the natural default.
pub(crate) fn live_link_entry_ids(
    strand: &ProjectedStrand,
    target: &str,
    edge_type: &str,
) -> Vec<String> {
    struct Inst {
        id: Option<String>,
        live: bool,
    }
    let mut insts: Vec<Inst> = Vec::new();
    for e in &strand.log {
        match &e.effect {
            Some(EntryEffect::Link {
                target: t,
                edge_type: et,
            }) if t == target && et == edge_type => insts.push(Inst {
                id: e.entry_id.clone(),
                live: true,
            }),
            Some(EntryEffect::Unlink {
                target: t,
                edge_type: et,
                link_entry_id,
            }) if t == target && et == edge_type => match link_entry_id {
                Some(cancel) => {
                    if let Some(i) = insts
                        .iter_mut()
                        .find(|i| i.live && i.id.as_deref() == Some(cancel.as_str()))
                    {
                        i.live = false;
                    }
                }
                None => insts
                    .iter_mut()
                    .filter(|i| i.live)
                    .for_each(|i| i.live = false),
            },
            _ => {}
        }
    }
    insts
        .iter()
        .filter(|i| i.live)
        .filter_map(|i| i.id.clone())
        .collect()
}

/// One dangling edge-discipline item (open unfixed friction, or decision without why).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgesDisciplineItem {
    pub entry_id: String,
    pub strand_id: String,
    pub marker: String,
    pub content: String,
    pub offset: usize,
}

/// Edge-discipline self-check: (a) open unfixed `[friction]`, (b) `[decision]` lacking refs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EdgesDisciplineReport {
    /// Every unfixed `[friction]` (home strand open/closed does not matter).
    pub open_frictions: Vec<EdgesDisciplineItem>,
    /// How many of [`Self::open_frictions`] live on a registered (active) strand.
    pub open_friction_active_count: usize,
    pub decisions_without_why: Vec<EdgesDisciplineItem>,
}

/// Extract `fixes=<hex prefix ≥8>` token from a `[fixed]` entry body, if present.
pub(crate) fn extract_fixes_prefix(content: &str) -> Option<&str> {
    content.split_whitespace().find_map(|tok| {
        let prefix = tok.strip_prefix("fixes=")?;
        if prefix.len() >= 8 && prefix.chars().all(|c| c.is_ascii_hexdigit()) {
            Some(prefix)
        } else {
            None
        }
    })
}

/// Build the dangling edge-discipline report from a full strand projection.
///
/// - **unfixed frictions**: any `[friction]` that no `[fixed] fixes=<prefix>`
///   points at (prefix match against entry_id or append_id). Home-strand
///   open/closed is **not** a filter — a closed pilot line can still carry an
///   unclosed design gap. Dual count: total list + how many sit on active
///   (registered) strands. Hidden strands are skipped.
/// - **decisions without why**: `[decision]` entries whose `refs` is empty
///   (no `--why` was recorded). Hidden strands are skipped.
/// - `since_offset`: when set, skip `[decision]` entries at or before that
///   journal offset (legacy pre-policy stock); frictions are never skipped.
#[cfg(test)]
pub fn edges_discipline_report(strands: &[ProjectedStrand]) -> EdgesDisciplineReport {
    edges_discipline_report_since(strands, None, None)
}

/// Same as [`edges_discipline_report`], with optional decision-offset floor
/// and optional candidate-set shrink.
///
/// `candidate_ids`: when `Some`, only those home strands emit findings
/// (JournalScope when `None`). Fix-prefix knowledge always scans the full
/// `strands` slice so a `[fixed]` outside the candidate set still closes a
/// friction inside it — scope only shrinks the candidate set, not semantics.
pub fn edges_discipline_report_since(
    strands: &[ProjectedStrand],
    since_offset: Option<usize>,
    candidate_ids: Option<&std::collections::HashSet<String>>,
) -> EdgesDisciplineReport {
    // Collect every fixes= target prefix across the whole journal.
    let mut fix_prefixes: Vec<&str> = Vec::new();
    for strand in strands {
        for entry in &strand.log {
            if crate::markers::leading_marker(&entry.content) == Some("fixed") {
                if let Some(p) = extract_fixes_prefix(&entry.content) {
                    fix_prefixes.push(p);
                }
            }
        }
    }

    let is_fixed = |entry: &LogEntry| -> bool {
        let candidates: Vec<&str> = entry
            .entry_id
            .as_deref()
            .into_iter()
            .chain(entry.append_id.as_deref())
            .collect();
        if candidates.is_empty() {
            return false;
        }
        fix_prefixes.iter().any(|prefix| {
            candidates
                .iter()
                .any(|id| id.starts_with(prefix) || prefix.starts_with(id))
        })
    };

    let mut open_frictions = Vec::new();
    let mut open_friction_active_count = 0usize;
    let mut decisions_without_why = Vec::new();

    for strand in strands {
        if strand.hidden {
            continue;
        }
        if let Some(ids) = candidate_ids {
            if !ids.contains(&strand.id) {
                continue;
            }
        }
        let strand_active = strand.state() == "registered";
        for entry in &strand.log {
            let marker = crate::markers::leading_marker(&entry.content).unwrap_or("");
            match marker {
                // Unfixed = no fixes= pointer; strand open/closed is irrelevant.
                "friction" if !is_fixed(entry) => {
                    if let Some(entry_id) = entry.entry_id.clone() {
                        if strand_active {
                            open_friction_active_count += 1;
                        }
                        open_frictions.push(EdgesDisciplineItem {
                            entry_id,
                            strand_id: strand.id.clone(),
                            marker: marker.to_string(),
                            content: crate::util::truncate(&entry.content, 70),
                            offset: entry.offset,
                        });
                    }
                }
                "decision" if entry.refs.is_empty() => {
                    if let Some(floor) = since_offset {
                        if entry.offset <= floor {
                            continue;
                        }
                    }
                    if let Some(entry_id) = entry.entry_id.clone() {
                        decisions_without_why.push(EdgesDisciplineItem {
                            entry_id,
                            strand_id: strand.id.clone(),
                            marker: marker.to_string(),
                            content: crate::util::truncate(&entry.content, 70),
                            offset: entry.offset,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    EdgesDisciplineReport {
        open_frictions,
        open_friction_active_count,
        decisions_without_why,
    }
}

/// Needs-judgment notices for orient (CORPUS §8, question ③): active,
/// non-hidden strands whose last entry carries a closing-annotation marker
/// (`[done]`/`[verified]`/…) yet the strand is still open. A fact for the
/// reader — close it or keep working — not a verdict.
pub fn orient_notices(strands: &[ProjectedStrand]) -> Vec<String> {
    strands
        .iter()
        .filter(|s| !s.hidden && s.state() == "registered")
        .filter_map(|s| {
            let last = s.log.last()?;
            if crate::markers::is_closing_annotation_marker(&last.content) {
                let marker = crate::markers::leading_marker(&last.content).unwrap_or("[done]");
                Some(format!(
                    "{} last entry is {} but the strand is still open — close it or keep working",
                    crate::util::shorten(&s.id),
                    marker
                ))
            } else {
                None
            }
        })
        .collect()
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

// ── Entry lookup by hash prefix ─────────────────────────────

/// Result of resolving an entry-hash prefix against projected strands.
/// Mirrors `find_strand`'s contract but keeps ambiguity explicit so the
/// caller can list candidates instead of guessing.
pub(crate) enum EntryLookup<'a> {
    None,
    One {
        strand: &'a ProjectedStrand,
        entry: &'a LogEntry,
    },
    Ambiguous(Vec<String>),
}

/// Resolve `prefix` against every effective entry id (stored v2 hashes and
/// the deterministic virtual ids projected for retained v1 rows).
pub(crate) fn find_entry<'a>(strands: &'a [ProjectedStrand], prefix: &str) -> EntryLookup<'a> {
    let mut hits: Vec<(&ProjectedStrand, &LogEntry)> = Vec::new();
    for strand in strands {
        for entry in &strand.log {
            if let Some(entry_id) = entry.entry_id.as_deref() {
                if entry_id.starts_with(prefix) {
                    hits.push((strand, entry));
                }
            }
        }
    }
    match hits.len() {
        0 => EntryLookup::None,
        1 => EntryLookup::One {
            strand: hits[0].0,
            entry: hits[0].1,
        },
        _ => EntryLookup::Ambiguous(
            hits.iter()
                .filter_map(|(_, e)| e.entry_id.clone())
                .collect(),
        ),
    }
}

/// Index from effective entry hash to its home strand and offset. Shared by
/// the read paths that resolve refs (show annotations, audit
/// ref-target-advanced) so the position logic has a single owner.
pub(crate) struct EntryIndex<'a> {
    by_hash: std::collections::HashMap<&'a str, (&'a ProjectedStrand, usize)>,
}

impl<'a> EntryIndex<'a> {
    pub(crate) fn build(strands: &'a [ProjectedStrand]) -> Self {
        let mut by_hash = std::collections::HashMap::new();
        for strand in strands {
            for entry in &strand.log {
                if let Some(hash) = entry.entry_id.as_deref() {
                    by_hash.insert(hash, (strand, entry.offset));
                }
            }
        }
        Self { by_hash }
    }

    /// The position fact behind ref-target-advanced: did the cited entry's
    /// line gain entries after the citation was written? Journal offsets are
    /// globally monotonic, so "the line moved past the citation point" is
    /// exactly `target.last_offset() > citing_offset` — no stored pin needed,
    /// and self-referencing entries resolve consistently. Returns `None`
    /// when the hash does not resolve locally (cross-journal or dangling):
    /// the machine asserts nothing it cannot verify.
    pub(crate) fn advanced_past(&self, cited_hash: &str, citing_offset: usize) -> Option<bool> {
        self.by_hash
            .get(cited_hash)
            .map(|(strand, _)| strand.last_offset() > citing_offset)
    }
}

// ── Entry deref view (show --entry [--deref N]) ─────────────

/// One pulled entry in a deref expansion, with the mechanical coordinates a
/// reader needs to interpret it outside its home line (CORPUS §2: the unit
/// of self-containment is the line, so a bare entry travels with its line's
/// identity and its position facts).
pub(crate) struct EntryViewNode<'a> {
    pub(crate) hop: usize,
    /// Hash of the entry whose ref pulled this node in (None for the root).
    pub(crate) cited_by: Option<String>,
    pub(crate) strand: &'a ProjectedStrand,
    pub(crate) entry: &'a LogEntry,
    /// 0-based position within the strand's log.
    pub(crate) entry_index: usize,
    /// Log entries after this one on its own line — the position fact behind
    /// the (advanced) annotation.
    pub(crate) later_entries: usize,
}

/// A ref that does not resolve locally (cross-journal or dangling). The
/// machine reports the pointer and asserts nothing about its target.
pub(crate) struct EntryViewStub {
    pub(crate) hop: usize,
    pub(crate) cited_by: String,
    pub(crate) hash: String,
}

/// A ref at the depth boundary, left unexpanded on purpose. `content_len`
/// prices the next hop; None when the target is unresolvable locally.
pub(crate) struct EntryViewFrontier {
    pub(crate) hash: String,
    pub(crate) content_len: Option<usize>,
}

pub(crate) struct EntryView<'a> {
    /// Root first, then hop-ascending (BFS order).
    pub(crate) nodes: Vec<EntryViewNode<'a>>,
    pub(crate) stubs: Vec<EntryViewStub>,
    pub(crate) frontier: Vec<EntryViewFrontier>,
}

/// Resolve `prefix` to an entry and expand its rationale refs `deref` hops.
/// Pure pointer following over the projected read model: refs participate in
/// their entry's own hash, so the ref graph is a DAG by construction and the
/// walk terminates without cycle checks. Deduplication is by hash — the same
/// entry cited via several paths is pulled once.
pub(crate) fn build_entry_view<'a>(
    strands: &'a [ProjectedStrand],
    prefix: &str,
    deref: usize,
) -> Result<EntryView<'a>, String> {
    use std::collections::{HashMap, HashSet, VecDeque};
    let mut by_hash: HashMap<&str, (&ProjectedStrand, usize)> = HashMap::new();
    for strand in strands {
        for (idx, entry) in strand.log.iter().enumerate() {
            if let Some(hash) = entry.entry_id.as_deref() {
                by_hash.insert(hash, (strand, idx));
            }
        }
    }

    let root_hash = match find_entry(strands, prefix) {
        EntryLookup::One { entry, .. } => entry
            .entry_id
            .clone()
            .expect("find_entry only matches entries with ids"),
        EntryLookup::None => {
            return Err(format!(
                "no entry matches {} (strand views resolve with 'mnema show <ID>')",
                prefix
            ));
        }
        EntryLookup::Ambiguous(candidates) => {
            let sample: Vec<String> = candidates
                .iter()
                .take(4)
                .map(|c| crate::util::shorten(c))
                .collect();
            return Err(format!(
                "entry prefix {} is ambiguous: {} entries match (e.g. {})",
                prefix,
                candidates.len(),
                sample.join(", ")
            ));
        }
    };

    let mut nodes = Vec::new();
    let mut stubs = Vec::new();
    let mut frontier = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut frontier_seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, usize, Option<String>)> = VecDeque::new();
    queue.push_back((root_hash, 0, None));

    while let Some((hash, hop, cited_by)) = queue.pop_front() {
        if !visited.insert(hash.clone()) {
            continue;
        }
        match by_hash.get(hash.as_str()) {
            Some(&(strand, idx)) => {
                let entry = &strand.log[idx];
                for cited in &entry.refs {
                    if hop < deref {
                        queue.push_back((cited.clone(), hop + 1, Some(hash.clone())));
                    } else if !visited.contains(cited) && frontier_seen.insert(cited.clone()) {
                        frontier.push(EntryViewFrontier {
                            hash: cited.clone(),
                            content_len: by_hash
                                .get(cited.as_str())
                                .map(|&(s, i)| s.log[i].content.len()),
                        });
                    }
                }
                nodes.push(EntryViewNode {
                    hop,
                    cited_by,
                    strand,
                    entry,
                    entry_index: idx,
                    later_entries: strand.log.len() - idx - 1,
                });
            }
            None => stubs.push(EntryViewStub {
                hop,
                cited_by: cited_by.unwrap_or_default(),
                hash,
            }),
        }
    }
    Ok(EntryView {
        nodes,
        stubs,
        frontier,
    })
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
        let mut entry_chain =
            crate::event::EntryChainFold::new(crate::event::EntryChainMode::Effective);
        for (offset, e) in node_events.iter() {
            if let Event::LogAppended {
                ts,
                content,
                refs,
                ref_,
                append_id,
                effect,
                provenance,
                ..
            } = e
            {
                let chain_step = entry_chain.apply(e).expect("log event folds");
                logs.push(LogEntry {
                    offset: *offset,
                    ts: ts.clone(),
                    content: content.clone(),
                    effect: effect.clone(),
                    prev_entry_id: chain_step.prev_entry_id,
                    entry_id: Some(chain_step.folded_entry_id),
                    refs: refs.clone(),
                    ref_: ref_.clone(),
                    append_id: append_id.clone(),
                    provenance: provenance.clone(),
                });
            }
        }
        // Instance-based edge fold (CORPUS §4). Each Link effect is a distinct
        // edge instance identified by its entry_id; an Unlink cancels the one
        // instance it names (link_entry_id), or — legacy, no id — tombstones the
        // whole (target, edge_type) key. Two links to one target survive
        // cancelling one of them. For a journal with at most one link per key
        // this reduces to the old fold, so tree/orient ordering is unchanged.
        // The journal keeps every event (append-only); folding only shapes reads.
        let offset_to_eid: std::collections::HashMap<usize, String> = logs
            .iter()
            .filter_map(|l| l.entry_id.clone().map(|id| (l.offset, id)))
            .collect();
        struct EdgeInstance {
            id: Option<String>,
            target: String,
            edge_type: Option<String>,
            live: bool,
        }
        let mut instances: Vec<EdgeInstance> = Vec::new();
        for entry in node_events.iter() {
            let offset = entry.0;
            let ev: &Event = entry.1;
            match ev {
                Event::LogAppended {
                    effect: Some(EntryEffect::Link { target, edge_type }),
                    ..
                } => instances.push(EdgeInstance {
                    id: offset_to_eid.get(&offset).cloned(),
                    target: target.clone(),
                    edge_type: Some(edge_type.clone()),
                    live: true,
                }),
                Event::EdgeLinked { to, edge_type, .. } => instances.push(EdgeInstance {
                    id: None,
                    target: to.clone(),
                    edge_type: edge_type.clone(),
                    live: true,
                }),
                Event::LogAppended {
                    effect:
                        Some(EntryEffect::Unlink {
                            target,
                            edge_type,
                            link_entry_id,
                        }),
                    ..
                } => match link_entry_id {
                    Some(cancel) => {
                        if let Some(inst) = instances
                            .iter_mut()
                            .find(|i| i.live && i.id.as_deref() == Some(cancel.as_str()))
                        {
                            inst.live = false;
                        }
                    }
                    None => {
                        for inst in instances.iter_mut().filter(|i| {
                            i.live
                                && i.target == *target
                                && i.edge_type.as_deref() == Some(edge_type.as_str())
                        }) {
                            inst.live = false;
                        }
                    }
                },
                Event::EdgeUnlinked { to, edge_type, .. } => {
                    for inst in instances
                        .iter_mut()
                        .filter(|i| i.live && i.target == *to && i.edge_type == *edge_type)
                    {
                        inst.live = false;
                    }
                }
                _ => {}
            }
        }
        let live: Vec<&EdgeInstance> = instances.iter().filter(|i| i.live).collect();
        // edges = all live targets, deduped by target id (a target reachable via
        // two edge types, or two live links, lists once).
        let edges: Vec<String> = dedup_preserve_order(live.iter().map(|i| i.target.clone()));
        let belongs_to_edges: Vec<String> = dedup_preserve_order(
            live.iter()
                .filter(|i| i.edge_type.as_deref() == Some("belongs-to"))
                .map(|i| i.target.clone()),
        );
        // depends-on subset (F3): typed view of review upstreams.
        let depends_on_edges: Vec<String> = dedup_preserve_order(
            live.iter()
                .filter(|i| i.edge_type.as_deref() == Some("depends-on"))
                .map(|i| i.target.clone()),
        );
        // Extract durable strand metadata from the creation event.
        let (strand_type, slug): (Option<String>, Option<String>) = node_events
            .iter()
            .find_map(|(_, e)| {
                if let Event::StrandCreated {
                    strand_type, slug, ..
                } = e
                {
                    Some((strand_type.clone(), slug.clone()))
                } else {
                    None
                }
            })
            .unwrap_or((None, None));
        let strand_id_str = node_events[0]
            .1
            .strand_id()
            .expect("strand-scoped event")
            .to_string();
        // Use the already-grouped per-strand events (O(events_on_strand)), not a
        // full-journal rescan per strand (would be O(S×E) and superlinear at scale).
        let (state, state_marker, state_offset) = compute_state_from_strand_events(&node_events);
        if !include_hidden && hidden {
            continue;
        }
        nodes.push(ProjectedStrand {
            id: strand_id_str,
            slug,
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
                content, effect, ..
            } => TimelineEventKind::LogAppended {
                content: content.clone(),
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
                observed, action, ..
            } => TimelineEventKind::CheckpointCreated {
                observed: observed.clone(),
                action: action.clone(),
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

// ── Event-time SubtreeScope for incremental --under ─────────

/// Live belongs-to edge instance used only for event-time scope folds.
struct BelongInst {
    source: String,
    target: String,
    id: Option<String>,
    live: bool,
}

fn belongs_children_map(
    insts: &[BelongInst],
) -> std::collections::HashMap<String, HashSet<String>> {
    let mut children: std::collections::HashMap<String, HashSet<String>> =
        std::collections::HashMap::new();
    for inst in insts.iter().filter(|i| i.live) {
        children
            .entry(inst.target.clone())
            .or_default()
            .insert(inst.source.clone());
    }
    children
}

fn apply_belongs_event(insts: &mut Vec<BelongInst>, created: &mut HashSet<String>, event: &Event) {
    match event {
        Event::StrandCreated { id, .. } => {
            created.insert(id.clone());
        }
        Event::LogAppended {
            id,
            entry_id,
            effect: Some(EntryEffect::Link { target, edge_type }),
            ..
        } if edge_type == "belongs-to" => {
            insts.push(BelongInst {
                source: id.clone(),
                target: target.clone(),
                id: entry_id.clone(),
                live: true,
            });
        }
        Event::EdgeLinked {
            id,
            to,
            edge_type: Some(et),
            ..
        } if et == "belongs-to" => {
            insts.push(BelongInst {
                source: id.clone(),
                target: to.clone(),
                id: None,
                live: true,
            });
        }
        Event::LogAppended {
            id,
            effect:
                Some(EntryEffect::Unlink {
                    target,
                    edge_type,
                    link_entry_id,
                }),
            ..
        } if edge_type == "belongs-to" => match link_entry_id {
            Some(cancel) => {
                if let Some(inst) = insts
                    .iter_mut()
                    .find(|i| i.live && i.id.as_deref() == Some(cancel.as_str()))
                {
                    inst.live = false;
                }
            }
            None => {
                // Legacy key tombstone: drop every live belongs-to from source→target.
                for inst in insts
                    .iter_mut()
                    .filter(|i| i.live && i.source == *id && i.target == *target)
                {
                    inst.live = false;
                }
            }
        },
        Event::EdgeUnlinked {
            id,
            to,
            edge_type: Some(et),
            ..
        } if et == "belongs-to" => {
            for inst in insts
                .iter_mut()
                .filter(|i| i.live && i.source == *id && i.target == *to)
            {
                inst.live = false;
            }
        }
        Event::EdgeUnlinked {
            id,
            to,
            edge_type: None,
            ..
        } => {
            // Untyped legacy unlink: drop matching belongs-to instances.
            for inst in insts
                .iter_mut()
                .filter(|i| i.live && i.source == *id && i.target == *to)
            {
                inst.live = false;
            }
        }
        _ => {}
    }
}

/// Journal offsets whose events fall inside `Subtree(root_id)` under
/// **event-time** membership.
///
/// An event at offset O on strand S is included when S is in the belongs-to
/// subtree of `root_id` either immediately before applying the event or
/// immediately after. That captures:
/// - authored entries and effects while a strand is a member;
/// - the join link that brings a strand into the tree;
/// - the leave unlink that removes it;
/// and excludes pre-join / post-leave activity that query-time membership
/// would incorrectly include or drop when composing `--under` with a cursor.
///
/// Used by `timeline --under X --scope-at-event`; a temporal lower bound may
/// then select the incremental suffix without changing scope semantics.
pub fn event_time_subtree_offsets(events: &[(usize, Event)], root_id: &str) -> HashSet<usize> {
    let mut created: HashSet<String> = HashSet::new();
    let mut insts: Vec<BelongInst> = Vec::new();
    let mut out: HashSet<usize> = HashSet::new();
    let mut in_scope: HashSet<String> = HashSet::new();

    for (offset, event) in events {
        if matches!(event, Event::JournalAnchored { .. }) {
            continue;
        }
        let strand_id = match event {
            Event::SubjectBound { strand_id, .. } => strand_id.as_str(),
            _ => match event.strand_id() {
                Some(id) => id,
                None => continue,
            },
        };

        let before = in_scope.contains(strand_id);

        let changes_membership = matches!(event, Event::StrandCreated { .. })
            || matches!(
                event,
                Event::LogAppended {
                    effect: Some(EntryEffect::Link { edge_type, .. }),
                    ..
                } if edge_type == "belongs-to"
            )
            || matches!(
                event,
                Event::LogAppended {
                    effect: Some(EntryEffect::Unlink { edge_type, .. }),
                    ..
                } if edge_type == "belongs-to"
            )
            || matches!(
                event,
                Event::EdgeLinked { edge_type: Some(edge_type), .. }
                    | Event::EdgeUnlinked { edge_type: Some(edge_type), .. }
                    if edge_type == "belongs-to"
            )
            || matches!(
                event,
                Event::EdgeUnlinked {
                    edge_type: None,
                    ..
                }
            );

        apply_belongs_event(&mut insts, &mut created, event);

        if changes_membership {
            let children = belongs_children_map(&insts);
            in_scope.clear();
            if created.contains(root_id) {
                let mut stack = vec![root_id.to_string()];
                while let Some(id) = stack.pop() {
                    if in_scope.insert(id.clone()) {
                        if let Some(children) = children.get(&id) {
                            stack.extend(children.iter().cloned());
                        }
                    }
                }
            }
        }
        let after = in_scope.contains(strand_id);

        if before || after {
            out.insert(*offset);
        }
    }
    out
}

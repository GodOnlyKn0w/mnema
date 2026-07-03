use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::process::Command;
use std::sync::atomic::{AtomicU16, Ordering};

static ID_COUNTER: AtomicU16 = AtomicU16::new(0);

/// Git context captured at append time — optional, never blocks append.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitContext {
    pub head: String,
    pub branch: String,
    pub status: String,
}
/// One strand head captured by a journal integrity anchor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JournalAnchorHead {
    pub strand_id: String,
    pub entry_id: String,
}

pub fn get_git_context() -> Option<GitContext> {
    let head = run_cmd(&["git", "rev-parse", "--short", "HEAD"]);
    let branch =
        run_cmd(&["git", "branch", "--show-current"]).unwrap_or_else(|_| "detached".to_string());
    let status = run_cmd(&["git", "status", "--porcelain"])
        .map(|s| {
            if s.trim().is_empty() {
                "clean".to_string()
            } else {
                "dirty".to_string()
            }
        })
        .unwrap_or_else(|_| "unknown".to_string());
    head.map(|h| GitContext {
        head: h,
        branch,
        status,
    })
    .ok()
}

fn run_cmd(args: &[&str]) -> Result<String, String> {
    let output = Command::new(args[0])
        .args(&args[1..])
        .output()
        .map_err(|e| format!("cannot run git: {}", e))?;
    if !output.status.success() {
        return Err("git command failed".to_string());
    }
    String::from_utf8(output.stdout)
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("invalid utf-8: {}", e))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntryEffect {
    Close { disposition: String },
    Reopen,
    Link { target: String, edge_type: String },
    Unlink { target: String, edge_type: String },
    Hide,
    Unhide,
}

impl EntryEffect {
    pub(crate) fn close(disposition: &str) -> Self {
        EntryEffect::Close {
            disposition: disposition.to_string(),
        }
    }

    pub(crate) fn link(target: &str, edge_type: &str) -> Self {
        EntryEffect::Link {
            target: target.to_string(),
            edge_type: edge_type.to_string(),
        }
    }

    pub(crate) fn unlink(target: &str, edge_type: &str) -> Self {
        EntryEffect::Unlink {
            target: target.to_string(),
            edge_type: edge_type.to_string(),
        }
    }
}
/// Event types for the append-only journal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
    #[serde(rename = "strand_created", alias = "node_created")]
    StrandCreated {
        id: String,
        ts: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        strand_type: Option<String>,
    },
    #[serde(rename = "log_appended")]
    LogAppended {
        id: String,
        ts: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        effect: Option<EntryEffect>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        prev_entry_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        entry_id: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        refs: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ref_: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        append_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        git: Option<GitContext>,
        /// Optional structured provenance attached to the entry.
        /// Persists alongside the entry as metadata, not content. New
        /// in this schema; older journals and entries simply omit the
        /// field. The shape is producer-defined; this crate only
        /// requires it to be a JSON object when present.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        provenance: Option<serde_json::Value>,
    },
    #[serde(rename = "edge_linked")]
    EdgeLinked {
        id: String,
        ts: String,
        to: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        edge_type: Option<String>,
        /// Optional structured provenance. Same contract as on `LogAppended`.
        /// New in this schema; older journals simply omit the field.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        provenance: Option<serde_json::Value>,
    },
    #[serde(rename = "edge_unlinked")]
    EdgeUnlinked {
        id: String,
        ts: String,
        to: String,
        /// Which typed edge to remove. Matches the EdgeLinked edge_type so an
        /// unlink can target exactly one of several edges between the same pair
        /// (F5). New field; no legacy events exist (unlink was never producible),
        /// so this is a zero-migration addition.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        edge_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        provenance: Option<serde_json::Value>,
    },
    #[serde(rename = "strand_hidden", alias = "node_hidden")]
    StrandHidden { id: String, ts: String },
    #[serde(rename = "strand_unhidden", alias = "node_unhidden")]
    StrandUnhidden { id: String, ts: String },
    #[serde(rename = "checkpoint")]
    CheckpointCreated {
        id: String,
        ts: String,
        observed: String,
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        append_id: Option<String>,
        /// Optional structured provenance shared with `LogAppended`.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        provenance: Option<serde_json::Value>,
    },
    /// Explicit close event written by `tasktree close`.
    /// This is the only event type that changes a strand's lifecycle state to closed.
    /// `disposition` is one of: done, failed, cancelled, merged, verified.
    #[serde(rename = "strand_closed")]
    StrandClosed {
        id: String,
        ts: String,
        disposition: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        provenance: Option<serde_json::Value>,
    },
    /// Explicit reopen event written by `tasktree reopen`.
    /// Moves the strand back to open/registered state.
    #[serde(rename = "strand_reopened")]
    StrandReopened {
        id: String,
        ts: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        provenance: Option<serde_json::Value>,
    },
    /// Journal-level integrity anchor. It captures the replayed set of current
    /// strand heads and a digest over that set plus the previous anchor digest.
    #[serde(rename = "journal_anchored")]
    JournalAnchored {
        ts: String,
        covered_event_count: usize,
        heads: Vec<JournalAnchorHead>,
        digest: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        previous_anchor: Option<String>,
    },
    /// Subject binding fact. Generic record of `subject_type + subject_id -> strand_id`.
    /// Consumers (e.g. `pi-strand`) decide what subject types mean; this crate only
    /// stores the binding, indexes it, and exposes it through `bind` / `current`.
    #[serde(rename = "subject_bound")]
    SubjectBound {
        /// Binding's own event id (24 hex). Distinct from the bound strand id.
        id: String,
        ts: String,
        subject_type: String,
        subject_id: String,
        strand_id: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        provenance: Option<serde_json::Value>,
    },
}

impl Event {
    pub fn strand_id(&self) -> Option<&str> {
        match self {
            Event::StrandCreated { id, .. }
            | Event::LogAppended { id, .. }
            | Event::EdgeLinked { id, .. }
            | Event::EdgeUnlinked { id, .. }
            | Event::StrandHidden { id, .. }
            | Event::StrandUnhidden { id, .. }
            | Event::CheckpointCreated { id, .. }
            | Event::StrandClosed { id, .. }
            | Event::StrandReopened { id, .. } => Some(id),
            Event::JournalAnchored { .. } | Event::SubjectBound { .. } => None,
        }
    }
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

/// Legacy non-strand ID format: 24 hex digits = microsecond timestamp (16) + PID (4) + counter (4).
/// PID prevents collision across processes on a single machine; counter prevents
/// collision within a process in the same microsecond.
///
/// Collision probability per microsecond per machine: ~2^-64 (negligible).
/// Sufficient for single-machine use. Insufficient for distributed deployment:
/// two machines may independently assign the same PID in the same microsecond.
/// If per-agent sub-journals on different machines are introduced, switch to a
/// random nonce or machine-level unique identifier.
fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;
    let pid = std::process::id() as u16;
    let counter = ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{:016x}{:04x}{:04x}", ts, pid, counter)
}

/// Legacy content-derived ID for a log append.
/// sha256(strand_id + ts + content) — deterministic, survives journal repairs.
pub fn compute_append_id(strand_id: &str, ts: &str, content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(strand_id.as_bytes());
    hasher.update(ts.as_bytes());
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

/// v2 entry identity: a strand-local hash chain.
///
/// The event's own `entry_id` and journal offset are excluded. Refs are entry
/// hashes and participate in identity, because changing an entry's cited basis
/// changes the entry itself.
pub fn compute_entry_id(
    prev_entry_id: Option<&str>,
    ts: &str,
    content: &str,
    refs: &[String],
    effect: Option<&EntryEffect>,
    provenance: Option<&serde_json::Value>,
    git: Option<&GitContext>,
) -> String {
    let payload = json!({
        "prev": prev_entry_id,
        "ts": ts,
        "content": content,
        "refs": refs,
        "effect": effect,
        "provenance": provenance,
        "git": git,
    });
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(&payload).expect("entry hash payload serializes"));
    hex::encode(hasher.finalize())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryChainMode {
    /// Read-side identity: stored v2 entry_id wins, retained v1 rows get virtual ids.
    Effective,
    /// Anchor identity: replay canonical heads from journal contents.
    Anchor,
    /// Audit identity: expose expected prev while hashing stored v2 rows by their declared prev.
    Integrity,
}

#[derive(Debug, Clone)]
pub(crate) struct EntryChainStep {
    pub(crate) strand_id: String,
    pub(crate) expected_prev_entry_id: Option<String>,
    pub(crate) prev_entry_id: Option<String>,
    pub(crate) stored_entry_id: Option<String>,
    pub(crate) computed_entry_id: String,
    pub(crate) folded_entry_id: String,
}

#[derive(Debug, Clone)]
pub(crate) struct EntryChainFold {
    mode: EntryChainMode,
    heads: std::collections::BTreeMap<String, String>,
}

impl EntryChainFold {
    pub(crate) fn new(mode: EntryChainMode) -> Self {
        Self {
            mode,
            heads: std::collections::BTreeMap::new(),
        }
    }

    pub(crate) fn apply(&mut self, event: &Event) -> Option<EntryChainStep> {
        let Event::LogAppended {
            id,
            ts,
            content,
            prev_entry_id,
            entry_id,
            refs,
            effect,
            provenance,
            git,
            ..
        } = event
        else {
            return None;
        };

        let expected_prev_entry_id = self.heads.get(id).cloned();
        let prev_entry_id = match self.mode {
            EntryChainMode::Effective => prev_entry_id
                .clone()
                .or_else(|| expected_prev_entry_id.clone()),
            EntryChainMode::Anchor => expected_prev_entry_id.clone(),
            EntryChainMode::Integrity => {
                if entry_id.is_some() {
                    prev_entry_id.clone()
                } else {
                    expected_prev_entry_id.clone()
                }
            }
        };
        let computed_entry_id = compute_entry_id(
            prev_entry_id.as_deref(),
            ts,
            content,
            refs,
            effect.as_ref(),
            provenance.as_ref(),
            git.as_ref(),
        );
        let folded_entry_id = match self.mode {
            EntryChainMode::Effective => entry_id
                .clone()
                .unwrap_or_else(|| computed_entry_id.clone()),
            EntryChainMode::Anchor | EntryChainMode::Integrity => computed_entry_id.clone(),
        };
        self.heads.insert(id.clone(), folded_entry_id.clone());

        Some(EntryChainStep {
            strand_id: id.clone(),
            expected_prev_entry_id,
            prev_entry_id,
            stored_entry_id: entry_id.clone(),
            computed_entry_id,
            folded_entry_id,
        })
    }

    pub(crate) fn head(&self, strand_id: &str) -> Option<String> {
        self.heads.get(strand_id).cloned()
    }

    pub(crate) fn anchor_heads(&self) -> Vec<JournalAnchorHead> {
        self.heads
            .iter()
            .map(|(strand_id, entry_id)| JournalAnchorHead {
                strand_id: strand_id.clone(),
                entry_id: entry_id.clone(),
            })
            .collect()
    }
}

pub fn journal_anchor_heads(events: &[Event]) -> Vec<JournalAnchorHead> {
    let mut fold = EntryChainFold::new(EntryChainMode::Anchor);
    for event in events {
        fold.apply(event);
    }
    fold.anchor_heads()
}

pub fn compute_journal_anchor_digest(
    heads: &[JournalAnchorHead],
    previous_anchor: Option<&str>,
    covered_event_count: usize,
) -> String {
    let payload = json!({
        "covered_event_count": covered_event_count,
        "previous_anchor": previous_anchor,
        "heads": heads,
    });
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(&payload).expect("journal anchor payload serializes"));
    hex::encode(hasher.finalize())
}

pub fn latest_journal_anchor_digest(events: &[Event]) -> Option<String> {
    events.iter().rev().find_map(|event| {
        if let Event::JournalAnchored { digest, .. } = event {
            Some(digest.clone())
        } else {
            None
        }
    })
}

pub fn make_journal_anchor(events: &[Event]) -> Event {
    let previous_anchor = latest_journal_anchor_digest(events);
    let heads = journal_anchor_heads(events);
    let covered_event_count = events.len();
    let digest =
        compute_journal_anchor_digest(&heads, previous_anchor.as_deref(), covered_event_count);
    Event::JournalAnchored {
        ts: now(),
        covered_event_count,
        heads,
        digest,
        previous_anchor,
    }
}

pub fn make_strand_created(content: &str, strand_type: Option<&str>) -> (Event, Event) {
    make_strand_created_with_provenance(content, strand_type, None)
}

pub fn make_strand_created_with_provenance(
    content: &str,
    strand_type: Option<&str>,
    provenance: Option<serde_json::Value>,
) -> (Event, Event) {
    make_strand_created_with_refs(content, strand_type, Vec::new(), None, provenance)
}

/// Like `make_strand_created_with_provenance`, but the first entry carries
/// source refs (`add --from`): the refs participate in the first entry's
/// hash and therefore in the strand id itself — a line's identity includes
/// where it came from.
pub fn make_strand_created_with_refs(
    content: &str,
    strand_type: Option<&str>,
    refs: Vec<String>,
    legacy_ref: Option<&str>,
    provenance: Option<serde_json::Value>,
) -> (Event, Event) {
    let ts = now();
    let git = get_git_context();
    let entry_id = compute_entry_id(
        None,
        &ts,
        content,
        &refs,
        None,
        provenance.as_ref(),
        git.as_ref(),
    );
    let id = entry_id.clone();
    let append_id = compute_append_id(&id, &ts, content);
    let created = Event::StrandCreated {
        id: id.clone(),
        ts: ts.clone(),
        strand_type: strand_type.map(|s| s.to_string()),
    };
    let appended = Event::LogAppended {
        id,
        ts,
        content: content.to_string(),
        effect: None,
        prev_entry_id: None,
        entry_id: Some(entry_id),
        refs,
        ref_: legacy_ref.map(str::to_string),
        append_id: Some(append_id),
        git,
        provenance,
    };
    (created, appended)
}
/// Build a `LogAppended` event. `provenance` is the optional structured
/// metadata blob attached to the entry. `None` produces an event identical
/// to the pre-provenance schema; older consumers see the same JSON shape
/// thanks to `skip_serializing_if`.
pub fn make_log_appended(id: &str, content: &str, provenance: Option<serde_json::Value>) -> Event {
    make_log_appended_entry(id, None, content, Vec::new(), None, provenance)
}

/// Like `make_log_appended` but sets the entry's legacy `ref_` rationale
/// pointer. New callers should prefer `make_log_appended_entry` with `refs`.
pub fn make_log_appended_with_ref(
    id: &str,
    content: &str,
    ref_: Option<&str>,
    provenance: Option<serde_json::Value>,
) -> Event {
    make_log_appended_entry(id, None, content, Vec::new(), ref_, provenance)
}

/// Build a v2 chained log entry. `refs` are entry hashes; `legacy_ref` is the
/// transitional v1 strand@offset pin kept while callers migrate.
pub fn make_log_appended_entry(
    id: &str,
    prev_entry_id: Option<&str>,
    content: &str,
    refs: Vec<String>,
    legacy_ref: Option<&str>,
    provenance: Option<serde_json::Value>,
) -> Event {
    make_log_appended_entry_with_effect(
        id,
        prev_entry_id,
        content,
        refs,
        legacy_ref,
        None,
        provenance,
    )
}

pub fn make_log_appended_entry_with_effect(
    id: &str,
    prev_entry_id: Option<&str>,
    content: &str,
    refs: Vec<String>,
    legacy_ref: Option<&str>,
    effect: Option<EntryEffect>,
    provenance: Option<serde_json::Value>,
) -> Event {
    let ts = now();
    let append_id = compute_append_id(id, &ts, content);
    let git = get_git_context();
    let entry_id = compute_entry_id(
        prev_entry_id,
        &ts,
        content,
        &refs,
        effect.as_ref(),
        provenance.as_ref(),
        git.as_ref(),
    );
    Event::LogAppended {
        id: id.to_string(),
        ts,
        content: content.to_string(),
        effect,
        prev_entry_id: prev_entry_id.map(|s| s.to_string()),
        entry_id: Some(entry_id),
        refs,
        ref_: legacy_ref.map(|s| s.to_string()),
        append_id: Some(append_id),
        git,
        provenance,
    }
}
/// Build a `CheckpointCreated` event. `provenance` follows the same
/// contract as on `LogAppended`; pass `None` for the original behaviour.
pub fn make_checkpoint(
    id: &str,
    observed: &str,
    action: &str,
    provenance: Option<serde_json::Value>,
) -> Event {
    let ts = now();
    let content = format!("observed={} action={}", observed, action);
    let append_id = compute_append_id(id, &ts, &content);
    Event::CheckpointCreated {
        id: id.to_string(),
        ts,
        observed: observed.to_string(),
        action: action.to_string(),
        append_id: Some(append_id),
        provenance,
    }
}

// ── effect entry content/effect pairs ──────────────────────
// An effect entry's durable content mirrors its machine effect ("link
// belongs-to <id>", "close disposition=done", ...). That pairing is event
// construction knowledge with a single owner: these constructors. Both the
// Event factories below and command-layer append requests must go through
// them; nothing else spells the content templates.

/// Valid close dispositions accepted by `tasktree close --as <DISPOSITION>`.
pub const CLOSE_DISPOSITIONS: &[&str] = &["done", "failed", "cancelled", "merged", "verified"];

pub(crate) fn link_entry_parts(target_id: &str, edge_type: &str) -> (String, EntryEffect) {
    (
        format!("link {} {}", edge_type, target_id),
        EntryEffect::link(target_id, edge_type),
    )
}

pub(crate) fn unlink_entry_parts(target_id: &str, edge_type: &str) -> (String, EntryEffect) {
    (
        format!("unlink {} {}", edge_type, target_id),
        EntryEffect::unlink(target_id, edge_type),
    )
}

/// `disposition` must be in `CLOSE_DISPOSITIONS`; callers validate before building.
pub(crate) fn close_entry_parts(disposition: &str) -> (String, EntryEffect) {
    (
        format!("close disposition={}", disposition),
        EntryEffect::close(disposition),
    )
}

pub(crate) fn reopen_entry_parts() -> (String, EntryEffect) {
    ("reopen erroneous close".to_string(), EntryEffect::Reopen)
}

pub(crate) fn hide_entry_parts(reason: Option<&str>) -> (String, EntryEffect) {
    (
        reason
            .map(|r| format!("[hidden] {}", r))
            .unwrap_or_else(|| "hide".to_string()),
        EntryEffect::Hide,
    )
}

pub(crate) fn unhide_entry_parts() -> (String, EntryEffect) {
    ("unhide".to_string(), EntryEffect::Unhide)
}

pub fn make_edge_linked(
    source_id: &str,
    prev_entry_id: Option<&str>,
    target_id: &str,
    edge_type: Option<&str>,
    provenance: Option<serde_json::Value>,
) -> Event {
    let edge_type = edge_type.unwrap_or("depends-on");
    let (content, effect) = link_entry_parts(target_id, edge_type);
    make_log_appended_entry_with_effect(
        source_id,
        prev_entry_id,
        &content,
        Vec::new(),
        None,
        Some(effect),
        provenance,
    )
}

/// Build an unlink effect entry (F5). Symmetric with `make_edge_linked`:
/// carries edge_type (which typed edge to remove) and provenance.
pub fn make_edge_unlinked(
    source_id: &str,
    prev_entry_id: Option<&str>,
    target_id: &str,
    edge_type: Option<&str>,
    provenance: Option<serde_json::Value>,
) -> Event {
    let edge_type = edge_type.unwrap_or("depends-on");
    let (content, effect) = unlink_entry_parts(target_id, edge_type);
    make_log_appended_entry_with_effect(
        source_id,
        prev_entry_id,
        &content,
        Vec::new(),
        None,
        Some(effect),
        provenance,
    )
}

/// Build a close effect entry.
/// `disposition` must be in `CLOSE_DISPOSITIONS`.
pub fn make_strand_closed(
    id: &str,
    prev_entry_id: Option<&str>,
    disposition: &str,
    provenance: Option<serde_json::Value>,
) -> Event {
    let (content, effect) = close_entry_parts(disposition);
    make_log_appended_entry_with_effect(
        id,
        prev_entry_id,
        &content,
        Vec::new(),
        None,
        Some(effect),
        provenance,
    )
}

/// Build a reopen effect entry.
pub fn make_strand_reopened(
    id: &str,
    prev_entry_id: Option<&str>,
    provenance: Option<serde_json::Value>,
) -> Event {
    let (content, effect) = reopen_entry_parts();
    make_log_appended_entry_with_effect(
        id,
        prev_entry_id,
        &content,
        Vec::new(),
        None,
        Some(effect),
        provenance,
    )
}

pub fn make_strand_hidden(
    id: &str,
    prev_entry_id: Option<&str>,
    reason: Option<&str>,
    provenance: Option<serde_json::Value>,
) -> Event {
    let (content, effect) = hide_entry_parts(reason);
    make_log_appended_entry_with_effect(
        id,
        prev_entry_id,
        &content,
        Vec::new(),
        None,
        Some(effect),
        provenance,
    )
}

pub fn make_strand_unhidden(
    id: &str,
    prev_entry_id: Option<&str>,
    provenance: Option<serde_json::Value>,
) -> Event {
    let (content, effect) = unhide_entry_parts();
    make_log_appended_entry_with_effect(
        id,
        prev_entry_id,
        &content,
        Vec::new(),
        None,
        Some(effect),
        provenance,
    )
}

/// Build a `SubjectBound` event. The `id` is the binding's own event id;
/// `strand_id` is the target strand the subject is bound to. `bind` is
/// append-only — newer bindings supersede older ones for the same
/// `(subject_type, subject_id)` pair; there is no unbind event in v1.
pub fn make_subject_bound(
    subject_type: &str,
    subject_id: &str,
    strand_id: &str,
    provenance: Option<serde_json::Value>,
) -> Event {
    Event::SubjectBound {
        id: generate_id(),
        ts: now(),
        subject_type: subject_type.to_string(),
        subject_id: subject_id.to_string(),
        strand_id: strand_id.to_string(),
        provenance,
    }
}

/// Resolve a strand-id prefix to the first matching full strand id, scanning
/// `StrandCreated` events in order. Lives here (not util.rs) because it is the
/// one resolver that depends on the `Event` type. Moved from main.rs in the
/// Layer 5-shape refactor.
pub(crate) fn find_strand(events: &[(usize, Event)], id: &str) -> Option<String> {
    // Empty/whitespace id must not resolve: starts_with("") matches every
    // strand, which would silently target the first one (data-integrity footgun).
    if id.trim().is_empty() {
        return None;
    }
    // Prefix match: first strand whose id starts with the given string
    events
        .iter()
        .filter_map(|(_, e)| {
            if let Event::StrandCreated { id: nid, .. } = e {
                Some(nid.clone())
            } else {
                None
            }
        })
        .find(|nid| nid.starts_with(id))
}

/// Resolve a strand ID prefix to a full strand ID, returning Result.
pub(crate) fn resolve_id(events: &[(usize, Event)], id: &str) -> Result<String, String> {
    find_strand(events, id).ok_or_else(|| format!("strand {} not found", id))
}

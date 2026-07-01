use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU16, Ordering};

static ID_COUNTER: AtomicU16 = AtomicU16::new(0);

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
        #[serde(skip_serializing_if = "Option::is_none")]
        ref_: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        append_id: Option<String>,
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
    /// Subject binding fact. Generic record of `subject_type + subject_id -> strand_id`.
    /// Consumers (e.g. `pi-strand`) decide what subject types mean; this crate only
    /// stores the binding and renders it in the timeline. Retained for append-only
    /// compatibility with historic journals; no command currently emits it.
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
    pub fn strand_id(&self) -> &str {
        match self {
            Event::StrandCreated { id, .. }
            | Event::LogAppended { id, .. }
            | Event::EdgeLinked { id, .. }
            | Event::EdgeUnlinked { id, .. }
            | Event::StrandHidden { id, .. }
            | Event::StrandUnhidden { id, .. }
            | Event::CheckpointCreated { id, .. }
            | Event::StrandClosed { id, .. }
            | Event::StrandReopened { id, .. } => id,
            // Binding events reference a strand but are not strand events.
            // Group them under the target strand so projection ignores them
            // (no StrandCreated match → filtered out by has_created gate).
            Event::SubjectBound { strand_id, .. } => strand_id,
        }
    }
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

/// ID format: 24 hex digits = microsecond timestamp (16) + PID (4) + counter (4).
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

/// Stable content-derived ID for a log append.
/// sha256(strand_id + ts + content) — deterministic, survives journal repairs.
pub fn compute_append_id(strand_id: &str, ts: &str, content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(strand_id.as_bytes());
    hasher.update(ts.as_bytes());
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn make_strand_created(
    content: &str,
    strand_type: Option<&str>,
    provenance: Option<serde_json::Value>,
) -> (Event, Event) {
    let ts = now();
    let id = generate_id();
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
        ref_: None,
        append_id: Some(append_id),
        provenance,
    };
    (created, appended)
}

/// Build a `LogAppended` event. `provenance` is the optional structured
/// metadata blob attached to the entry. `None` produces an event identical
/// to the pre-provenance schema; older consumers see the same JSON shape
/// thanks to `skip_serializing_if`.
pub fn make_log_appended(id: &str, content: &str, provenance: Option<serde_json::Value>) -> Event {
    make_log_appended_with_ref(id, content, None, provenance)
}

/// Like `make_log_appended` but sets the entry's `ref_` rationale pointer
/// (W1/F4-pin). `ref_` is a pinned reference `<target_id>@<offset>` so the
/// why-staleness clerk can later detect when the cited basis has evolved.
pub fn make_log_appended_with_ref(
    id: &str,
    content: &str,
    ref_: Option<&str>,
    provenance: Option<serde_json::Value>,
) -> Event {
    let ts = now();
    let append_id = compute_append_id(id, &ts, content);
    Event::LogAppended {
        id: id.to_string(),
        ts,
        content: content.to_string(),
        ref_: ref_.map(|s| s.to_string()),
        append_id: Some(append_id),
        provenance,
    }
}

pub fn make_edge_linked(
    source_id: &str,
    target_id: &str,
    edge_type: Option<&str>,
    provenance: Option<serde_json::Value>,
) -> Event {
    Event::EdgeLinked {
        id: source_id.to_string(),
        ts: now(),
        to: target_id.to_string(),
        edge_type: edge_type.map(|s| s.to_string()),
        provenance,
    }
}

/// Build an `EdgeUnlinked` event (F5). Symmetric with `make_edge_linked`:
/// carries edge_type (which typed edge to remove) and provenance.
pub fn make_edge_unlinked(
    source_id: &str,
    target_id: &str,
    edge_type: Option<&str>,
    provenance: Option<serde_json::Value>,
) -> Event {
    Event::EdgeUnlinked {
        id: source_id.to_string(),
        ts: now(),
        to: target_id.to_string(),
        edge_type: edge_type.map(|s| s.to_string()),
        provenance,
    }
}

/// Build a `StrandClosed` event.
/// `disposition` must be one of: done, failed, cancelled, merged, verified.
pub fn make_strand_closed(
    id: &str,
    disposition: &str,
    provenance: Option<serde_json::Value>,
) -> Event {
    Event::StrandClosed {
        id: id.to_string(),
        ts: now(),
        disposition: disposition.to_string(),
        provenance,
    }
}

/// Build a `StrandReopened` event.
pub fn make_strand_reopened(id: &str, provenance: Option<serde_json::Value>) -> Event {
    Event::StrandReopened {
        id: id.to_string(),
        ts: now(),
        provenance,
    }
}

pub fn make_strand_hidden(id: &str) -> Event {
    Event::StrandHidden {
        id: id.to_string(),
        ts: now(),
    }
}

pub fn make_strand_unhidden(id: &str) -> Event {
    Event::StrandUnhidden {
        id: id.to_string(),
        ts: now(),
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

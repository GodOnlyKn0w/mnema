use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
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

pub fn get_git_context() -> Option<GitContext> {
    let head = run_cmd(&["git", "rev-parse", "--short", "HEAD"]);
    let branch = run_cmd(&["git", "branch", "--show-current"])
        .unwrap_or_else(|_| "detached".to_string());
    let status = run_cmd(&["git", "status", "--porcelain"])
        .map(|s| if s.trim().is_empty() { "clean".to_string() } else { "dirty".to_string() })
        .unwrap_or_else(|_| "unknown".to_string());
    head.map(|h| GitContext { head: h, branch, status }).ok()
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
    },
    #[serde(rename = "strand_hidden", alias = "node_hidden")]
    StrandHidden {
        id: String,
        ts: String,
    },
    #[serde(rename = "strand_unhidden", alias = "node_unhidden")]
    StrandUnhidden {
        id: String,
        ts: String,
    },
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
    /// stores the binding, indexes it, and exposes it through `bind` / `current`.
    #[serde(rename = "subject_bound")]
    SubjectBound {
        /// Binding's own event id (24 hex). Distinct from the bound strand id.
        id: String,
        ts: String,
        subject_type: String,
        subject_id: String,
        strand_id: String,
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

pub fn make_strand_created(content: &str, strand_type: Option<&str>) -> (Event, Event) {
    let ts = now();
    let id = generate_id();
    let append_id = compute_append_id(&id, &ts, content);
    let git = get_git_context();
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
        git,
        provenance: None,
    };
    (created, appended)
}

/// Build a `LogAppended` event. `provenance` is the optional structured
/// metadata blob attached to the entry. `None` produces an event identical
/// to the pre-provenance schema; older consumers see the same JSON shape
/// thanks to `skip_serializing_if`.
pub fn make_log_appended(
    id: &str,
    content: &str,
    provenance: Option<serde_json::Value>,
) -> Event {
    let ts = now();
    let append_id = compute_append_id(id, &ts, content);
    let git = get_git_context();
    Event::LogAppended {
        id: id.to_string(),
        ts,
        content: content.to_string(),
        ref_: None,
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

/// Build a `SubjectBound` event. The `id` is the binding's own event id;
/// `strand_id` is the target strand the subject is bound to. `bind` is
/// append-only — newer bindings supersede older ones for the same
/// `(subject_type, subject_id)` pair; there is no unbind event in v1.
pub fn make_subject_bound(
    subject_type: &str,
    subject_id: &str,
    strand_id: &str,
) -> Event {
    Event::SubjectBound {
        id: generate_id(),
        ts: now(),
        subject_type: subject_type.to_string(),
        subject_id: subject_id.to_string(),
        strand_id: strand_id.to_string(),
    }
}

// ── Timeline Projection types ────────────────────────────

/// A single event in timeline projection.
///
/// Data model only — serialization lives in `output.rs` DTOs.
#[derive(Debug, Clone, Serialize)]
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
#[derive(Debug, Clone, Serialize)]
pub enum TimelineEventKind {
    StrandCreated {
        summary: Option<String>,
    },
    LogAppended {
        content: String,
        append_id: Option<String>,
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

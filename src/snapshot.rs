//! Snapshot generation layer.
//!
//! Produces SnapshotPayload from projected strands.
//! Called by `shuttle gate commit --share`; not aware of git or CLI.
//!
//! ## Ownership
//!
//! This module is the **only** place snapshot generation logic lives.
//! Implementations in `shuttle/` must call `mnema snapshot` (CLI) or
//! `mnema_core::snapshot::generate_snapshot` (if lib crate) — they must
//! NOT reimplement the projection-to-snapshot mapping themselves.
//!
//! ## Dependencies
//!
//! - `crate::projection::project_strands` — for strand state
//! - `crate::output::SnapshotPayload` — DTO defined in output.rs
//!
//! This module does NOT depend on `crate::event` directly — it consumes
//! projected strands, not raw events.
//!
//! See `protocols/snapshot-protocol.md` for the full spec.

use crate::output::SnapshotPayload;

/// Generate a strand snapshot from raw journal events.
///
/// The caller is responsible for:
/// - reading the journal (`read_events_lossy`)
/// - embedding the result into git commit message
/// - filtering by `--share` flag
///
/// This function only does the projection → DTO mapping.
pub fn generate_snapshot(_events: &[(usize, crate::event::Event)]) -> SnapshotPayload {
    todo!("snapshot generation — awaiting strand status DAG traversal")
}

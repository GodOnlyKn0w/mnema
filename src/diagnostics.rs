//! Unified diagnostic catalog — single source of truth for all diagnostic codes.
//!
//! Every code emitted by any producer (currently: lifecycle, health) MUST
//! have an entry here. The `tasktree explain` command queries this catalog.
//!
//! # Catalog closure contract
//!
//! Adding a new diagnostic code without a corresponding catalog entry is a bug.
//! Closure is two-way:
//!   1. Every emitted code must resolve via `tasktree explain --json <code>`
//!      with `ok: true` (no orphan emissions).
//!   2. Every catalog entry should have a live producer (no dead codes lying
//!      about checks that no longer run).
//!
//! # Code permanence
//!
//! Codes are permanent vocabulary: once a code has shipped, its number is
//! never reused for a different meaning (journals reference codes; reuse
//! makes history lie). 2026-06: 16 codes belonging to an external workflow
//! (gate/shuttle/covers/DAG/story — producers outside this repo) were
//! removed; see git history and `test_removed_workflow_codes_stay_removed`.

use serde::Serialize;

// ── Data model ──────────────────────────────────────────────

/// Fixed recovery kinds. Each diagnostic must use one of these.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryKind {
    /// Verify a task's completion.
    Verify,
    /// Modify existing code or documentation.
    Edit,
    /// Structural reorganisation or rename.
    MoveOrRename,
    /// Create a [covers] strand for a protocol file.
    CreateCoverStrand,
    /// Append a marker entry to an existing strand.
    AppendMarker,
    /// Dispatch a registered task.
    Dispatch,
    /// Cancel a stale task.
    Cancel,
    /// No mechanical recovery exists — human must decide.
    Manual,
}

/// Machine-readable recovery action (catalog — &'static str).
#[derive(Debug, Clone)]
pub struct RecoveryInfo {
    pub kind: RecoveryKind,
    pub command_str: &'static str,
    pub executable: bool,
    pub requires_human: bool,
}

/// Serializable recovery info for JSON output.
#[derive(Debug, Serialize)]
pub struct RecoveryInfoOutput {
    pub kind: RecoveryKind,
    pub command: String,
    pub executable: bool,
    pub requires_human: bool,
}

impl RecoveryInfo {
    fn to_output(&self) -> RecoveryInfoOutput {
        RecoveryInfoOutput {
            kind: self.kind.clone(),
            command: self.command_str.to_string(),
            executable: self.executable,
            requires_human: self.requires_human,
        }
    }
}

/// One diagnostic code in the catalog.
#[derive(Debug, Clone)]
pub struct DiagnosticInfo {
    pub code: &'static str,
    pub severity: Severity,
    pub category: &'static str,
    pub title: &'static str,
    pub finding: &'static str,
    pub impact: &'static str,
    pub recovery: RecoveryInfo,
    pub producer: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

// ── Explain output DTOs ─────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ExplainSuccessOutput {
    pub ok: bool,
    pub code: String,
    pub severity: String,
    pub category: String,
    pub title: String,
    pub finding: String,
    pub impact: String,
    pub recovery: RecoveryInfoOutput,
    pub producer: String,
}

#[derive(Debug, Serialize)]
pub struct ExplainErrorOutput {
    pub ok: bool,
    pub code: String,
    pub error: String,
}

impl From<&DiagnosticInfo> for ExplainSuccessOutput {
    fn from(d: &DiagnosticInfo) -> Self {
        ExplainSuccessOutput {
            ok: true,
            code: d.code.to_string(),
            severity: match d.severity {
                Severity::Error => "error".to_string(),
                Severity::Warning => "warning".to_string(),
            },
            category: d.category.to_string(),
            title: d.title.to_string(),
            finding: d.finding.to_string(),
            impact: d.impact.to_string(),
            recovery: d.recovery.to_output(),
            producer: d.producer.to_string(),
        }
    }
}

// ── Catalog ─────────────────────────────────────────────────

static CATALOG: &[DiagnosticInfo] = &[
    // ── Lifecycle: E053/E056 reserved, not removed ──────
    // Completion-pair checks (done↔verified) are parked until the marker
    // vocabulary stabilises — paired markers are coming, and these two
    // numbers stay reserved for that semantics. Their old recovery
    // commands referenced shuttle and must be rewritten on revival.
    //
    // E053  done without verified   (pair check, fire only if the strand
    //                                ever used [verified])
    // E056  verified without done   (inverse pair check)
    //
    // E055/E057/E058 (dispatch artifact / dispatched stale / registered
    // stale) were removed 2026-06 with the external workflow codes — the
    // dispatch concept belongs to that workflow, not to the journal.

    // ── Lifecycle (W codes) ─────────────────────────────
    DiagnosticInfo {
        code: "W068",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "deadline overdue",
        finding: "A task has a [deadline] entry whose by= time has passed, and the strand carries no closing marker ([verified] [done] [cancelled] [failed] [merged] [ended]).",
        impact: "The task is overdue; downstream schedule assumptions are invalid.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "verify or cancel the task; update the deadline if re-planned",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },
    DiagnosticInfo {
        code: "W069",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "concurrent marker write",
        finding: "The same marker type was written by two or more different agents on the same task.",
        impact: "Concurrent state transitions may conflict — the task's true state is ambiguous.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "review agents' actions and decide which one should continue",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },

    // ── Health (W062) ───────────────────────────────────
    DiagnosticInfo {
        code: "W062",
        severity: Severity::Warning,
        category: "health",
        title: "contradictory decision/constraint",
        finding: "A [decision] and [constraint] with the same keyword were written within 10 minutes from different strands.",
        impact: "The decision and constraint may conflict — the governance signal is ambiguous.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "review both entries and resolve the contradiction; append a clarifying entry",
            executable: false,
            requires_human: true,
        },
        producer: "health",
    },
];

// ── Lookup ──────────────────────────────────────────────────

pub fn lookup(code: &str) -> Option<&'static DiagnosticInfo> {
    CATALOG.iter().find(|d| d.code.eq_ignore_ascii_case(code))
}

/// Full catalog access for closure checks (examples-as-contract CI and
/// the two-way closure tests: every emitted code resolves, every entry
/// has a live producer).
pub fn catalog() -> &'static [DiagnosticInfo] {
    CATALOG
}

pub fn cmd_explain(code: &str, format_json: bool) -> String {
    match lookup(code) {
        Some(info) => {
            let output = ExplainSuccessOutput::from(info);
            if format_json {
                serde_json::to_string_pretty(&output).unwrap_or_else(|e| {
                    format!(r#"{{"ok":false,"code":"{}","error":"serialization failed: {}"}}"#, code, e)
                })
            } else {
                format!(
                    "{}\n  severity: {}\n  category: {}\n  title: {}\n\n  finding: {}\n\n  impact: {}\n\n  recovery:\n    kind: {:?}\n    command: {}\n    executable: {}\n    requires_human: {}\n\n  producer: {}",
                    info.code,
                    match info.severity { Severity::Error => "error", Severity::Warning => "warning" },
                    info.category,
                    info.title,
                    info.finding,
                    info.impact,
                    info.recovery.kind,
                    info.recovery.command_str,
                    info.recovery.executable,
                    info.recovery.requires_human,
                    info.producer,
                )
            }
        }
        None => {
            let output = ExplainErrorOutput {
                ok: false,
                code: code.to_string(),
                error: "unknown diagnostic code".to_string(),
            };
            if format_json {
                serde_json::to_string_pretty(&output).unwrap_or_else(|e| {
                    format!(r#"{{"ok":false,"code":"{}","error":"serialization failed: {}"}}"#, code, e)
                })
            } else {
                format!("unknown diagnostic code: {}", code)
            }
        }
    }
}

pub fn all_codes() -> Vec<&'static str> {
    CATALOG.iter().map(|d| d.code).collect()
}

pub fn catalog_size() -> usize {
    CATALOG.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lookup_known_code() {
        let info = lookup("W068").expect("W068 should be known");
        assert_eq!(info.code, "W068");
        assert_eq!(info.title, "deadline overdue");
        assert!(matches!(info.severity, Severity::Warning));
    }

    #[test]
    fn test_lookup_case_insensitive() {
        let info = lookup("w068").expect("w068 should be known");
        assert_eq!(info.code, "W068");
    }

    #[test]
    fn test_lookup_unknown_code() {
        assert!(lookup("E999").is_none());
    }

    #[test]
    fn test_explain_json_known() {
        let output = cmd_explain("W069", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(v["ok"], true);
        assert_eq!(v["code"], "W069");
        assert!(v["recovery"]["kind"].as_str().is_some());
        assert!(v["recovery"]["command"].as_str().is_some());
    }

    #[test]
    fn test_explain_json_unknown() {
        let output = cmd_explain("E999", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"], "unknown diagnostic code");
    }

    #[test]
    fn test_explain_text_known() {
        let output = cmd_explain("W062", false);
        assert!(output.contains("W062"));
        assert!(output.contains("contradictory"));
    }

    #[test]
    fn test_explain_text_unknown() {
        let output = cmd_explain("XYZ", false);
        assert!(output.contains("unknown diagnostic code"));
    }

    #[test]
    fn test_all_codes_present() {
        let codes = all_codes();
        assert!(codes.contains(&"W062"));
        assert!(codes.contains(&"W068"));
        assert!(codes.contains(&"W069"));
        assert_eq!(codes.len(), 3, "catalog size changed — update this test deliberately");
    }

    #[test]
    fn test_removed_workflow_codes_stay_removed() {
        // 20 codes were removed 2026-06 — they live in git history. Their
        // numbers must never be reused for new meanings:
        //   16 external-workflow codes (gate/shuttle/covers/DAG/story),
        //   E055/E057/E058 (dispatch concept left with that workflow),
        //   W066 (v0 migration finished — journal scan found no residue).
        // E053/E056 are NOT in this list: reserved (commented out in the
        // catalog) for completion-pair semantics once markers stabilise.
        for code in ["E047", "W058", "W065", "W067", "W070", "W071", "W072",
                     "W073", "E081", "W081", "E082", "W082", "E083", "W083",
                     "E084", "W085", "E055", "E057", "E058", "W066"] {
            assert!(lookup(code).is_none(), "removed code {} reappeared", code);
        }
    }

    #[test]
    fn test_reserved_codes_not_yet_revived() {
        // E053/E056 are parked until paired completion markers stabilise.
        // When they come back, delete this test and re-add them to
        // test_all_codes_present.
        assert!(lookup("E053").is_none());
        assert!(lookup("E056").is_none());
    }

    #[test]
    fn test_explain_json_recovery_fields() {
        let output = cmd_explain("W062", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        let recovery = &v["recovery"];
        assert_eq!(recovery["executable"], false);
        assert_eq!(recovery["requires_human"], true);
        assert!(recovery["command"].as_str().unwrap().contains("contradiction"));
    }

    #[test]
    fn test_no_duplicate_codes() {
        use std::collections::HashSet;
        let codes: Vec<&str> = CATALOG.iter().map(|d| d.code).collect();
        let unique: HashSet<&str> = codes.iter().copied().collect();
        assert_eq!(codes.len(), unique.len(), "duplicate diagnostic codes found");
    }
}

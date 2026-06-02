//! Unified diagnostic catalog — single source of truth for all diagnostic codes.
//!
//! Every code emitted by the checker (lifecycle, health, arch-boundary, gate)
//! MUST have an entry here. The `tasktree explain` command queries this catalog.
//!
//! # Catalog closure contract
//!
//! Adding a new diagnostic code without a corresponding catalog entry is a bug.
//! The closure check (see shuttle gate pre-check) enforces this:
//!   1. Collect all emitted codes from lifecycle/health/arch-boundary
//!   2. For each code, `tasktree explain --json <code>` must return `ok: true`
//!
//! # Relationship to docs
//!
//! `protocols/diagnostic-codes.md` is human-readable documentation.
//! This module is the machine-readable catalog. They must stay in sync.
//! Future: generate diagnostic-codes.md from this catalog.

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
    // ── Gate (E047) ─────────────────────────────────────
    DiagnosticInfo {
        code: "E047",
        severity: Severity::Error,
        category: "gate",
        title: "staged protocol without covers strand",
        finding: "A staged protocols/*.md file has no corresponding [covers] prompt-strand.",
        impact: "The protocol change is not linked to the journal's causal graph. Gate commit is blocked.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::CreateCoverStrand,
            command_str: "tasktree append \"[covers] protocols/<file>\" --new",
            executable: true,
            requires_human: false,
        },
        producer: "gate pre-check",
    },

    // ── Lifecycle (E053-E058) ───────────────────────────
    DiagnosticInfo {
        code: "E053",
        severity: Severity::Error,
        category: "lifecycle",
        title: "done without verified",
        finding: "done marker exists without verified marker — completion cannot be trusted.",
        impact: "Downstream dependency satisfaction is not trustworthy; do not auto-unlock.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Verify,
            command_str: "shuttle para verify <id> --gate-commit",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },
    DiagnosticInfo {
        code: "E055",
        severity: Severity::Error,
        category: "lifecycle",
        title: "dispatched without artifact",
        finding: "A task has a [dispatched] marker but no dispatch artifact file exists.",
        impact: "The task cannot be executed — the handoff artifact is missing.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Dispatch,
            command_str: "shuttle para dispatch <id> — or cancel the task",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },
    DiagnosticInfo {
        code: "E056",
        severity: Severity::Error,
        category: "lifecycle",
        title: "verified without done",
        finding: "A [verified] marker exists but no [done] marker — completion state inconsistent.",
        impact: "The task appears verified but never declared done; state machine is in an impossible state.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::AppendMarker,
            command_str: "shuttle task done <id> \"<msg>\"",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },
    DiagnosticInfo {
        code: "E057",
        severity: Severity::Error,
        category: "lifecycle",
        title: "dispatched stale",
        finding: "A task has been in dispatched state for more than 24 hours without progress.",
        impact: "The task may be abandoned; downstream tasks waiting on it will stall.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "progress the task or cancel it: shuttle para cancel <id>",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },
    DiagnosticInfo {
        code: "E058",
        severity: Severity::Error,
        category: "lifecycle",
        title: "registered stale",
        finding: "A task has been registered for more than 7 days without being dispatched.",
        impact: "The task may have been forgotten; capacity planning is inaccurate.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "dispatch or cancel: shuttle para dispatch <id>",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },

    // ── Lifecycle (W codes) ─────────────────────────────
    DiagnosticInfo {
        code: "W058",
        severity: Severity::Warning,
        category: "governance",
        title: "rule without decision entry",
        finding: "A lifecycle rule code has no corresponding [decision] entry in the journal.",
        impact: "The rule's rationale is undocumented — it may be arbitrary or accidental.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::AppendMarker,
            command_str: "tasktree append \"[decision] <CODE>: <reason>\" --new",
            executable: true,
            requires_human: false,
        },
        producer: "lifecycle",
    },
    DiagnosticInfo {
        code: "W065",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "DAG strand missing anchor",
        finding: "The DAG strand lacks an [anchor] git-head=<sha> entry for recovery cursor.",
        impact: "Without an anchor, recovery from a lost worktree is ambiguous — no known-good commit.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::AppendMarker,
            command_str: "re-run 'shuttle para group <name> --new' or append [anchor] git-head=<sha>",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },
    DiagnosticInfo {
        code: "W067",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "done without gate-commit",
        finding: "A task has [done] but no [gate-commit] entry — may have been marked done manually.",
        impact: "The gate seal process may not have run; the done marker may be unreliable.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::AppendMarker,
            command_str: "use 'shuttle task done <id> \"<msg>\"' which writes [gate-commit]",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },
    DiagnosticInfo {
        code: "W068",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "deadline overdue",
        finding: "A task has a [deadline] entry whose by_ts has passed, and no [verified] marker.",
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
    DiagnosticInfo {
        code: "W070",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "unscoped touches path",
        finding: "A task's [touches] paths are not covered by the DAG's declared [repo-scope].",
        impact: "The task may be touching files outside its authorised scope — security risk.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Edit,
            command_str: "update DAG's [repo-scope] or fix task's touches paths",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },
    DiagnosticInfo {
        code: "W071",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "prompt-strand zombie",
        finding: "A prompt-strand's [covers] path no longer exists on disk.",
        impact: "The strand's context is stale — it covers deleted or renamed files.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Edit,
            command_str: "update or hide the prompt strand — [covers] paths no longer exist",
            executable: true,
            requires_human: false,
        },
        producer: "lifecycle",
    },
    DiagnosticInfo {
        code: "W072",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "untracked .md files",
        finding: ".md files exist on disk but are not tracked by git — at risk of data loss.",
        impact: "These documents are invisible to git history and can be lost on checkout or reset.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Edit,
            command_str: "git add <files> && git commit -m \"docs: track untracked .md files\"",
            executable: true,
            requires_human: false,
        },
        producer: "lifecycle",
    },
    DiagnosticInfo {
        code: "W073",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "story index missing files",
        finding: "story/INDEX.md references .md files that don't exist on disk.",
        impact: "The story index is out of date — agents may follow dead links.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Edit,
            command_str: "update story/INDEX.md or create the referenced files",
            executable: true,
            requires_human: false,
        },
        producer: "lifecycle",
    },

    // ── Health (W062, W066) ─────────────────────────────
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
    DiagnosticInfo {
        code: "W066",
        severity: Severity::Warning,
        category: "health",
        title: "v0 format residue",
        finding: "A DAG strand contains task_created events without the v:1 version field — v0 format residue.",
        impact: "These tasks may not participate in current projections; format migration is needed.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "migrate v0 task_created events to v1 format with explicit task IDs",
            executable: false,
            requires_human: true,
        },
        producer: "health",
    },

    // ── Architecture Boundaries (E/W081-E/W085) ─────────
    DiagnosticInfo {
        code: "E081",
        severity: Severity::Error,
        category: "architecture",
        title: "shuttle reads journal directly (correctness path)",
        finding: "Shuttle code reads journal.jsonl directly on a correctness code path, bypassing tasktree CLI.",
        impact: "Creates a second journal access path that can drift from tasktree's projection logic. Trust chain broken.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Edit,
            command_str: "replace direct journal read with tasktree list/show --format json call",
            executable: false,
            requires_human: true,
        },
        producer: "check_arch_boundary",
    },
    DiagnosticInfo {
        code: "W081",
        severity: Severity::Warning,
        category: "architecture",
        title: "shuttle reads journal directly (diagnostic-only)",
        finding: "Shuttle code reads journal.jsonl directly on a diagnostic-only path (e.g. doctor.rs).",
        impact: "Lower risk than E081 — not on a correctness path — but still creates a second access pattern.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::MoveOrRename,
            command_str: "consider routing through tasktree CLI for consistency",
            executable: false,
            requires_human: true,
        },
        producer: "check_arch_boundary",
    },
    DiagnosticInfo {
        code: "E082",
        severity: Severity::Error,
        category: "architecture",
        title: "tasktree-core second-order terms (boundary error)",
        finding: "A second-order term (dispatch, agent, wave, lifecycle, handoff, gate) appears in a tasktree-core symbol, module, or public command.",
        impact: "Core boundary is eroding — application-domain terms in core will confuse future consumers.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::MoveOrRename,
            command_str: "move the function to shuttle or rename to structural terms",
            executable: false,
            requires_human: true,
        },
        producer: "check_arch_boundary",
    },
    DiagnosticInfo {
        code: "W082",
        severity: Severity::Warning,
        category: "architecture",
        title: "tasktree-core second-order terms (wording hygiene)",
        finding: "A second-order term appears only in comments, help text, or strings in tasktree-core.",
        impact: "Hygiene issue — not a boundary error, but may confuse readers about the core/shell split.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Edit,
            command_str: "rewrite the comment or string to use structural terms",
            executable: true,
            requires_human: false,
        },
        producer: "check_arch_boundary",
    },
    DiagnosticInfo {
        code: "E083",
        severity: Severity::Error,
        category: "architecture",
        title: "protocol missing explicit covers (tracked, old file)",
        finding: "A tracked protocols/*.md file older than the grace window lacks an explicit file-level [covers] strand.",
        impact: "The protocol is invisible to the causal graph — agents entering via strand search won't discover it.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::CreateCoverStrand,
            command_str: "tasktree append \"[covers] protocols/<file>\" --new",
            executable: true,
            requires_human: false,
        },
        producer: "check_arch_boundary",
    },
    DiagnosticInfo {
        code: "W083",
        severity: Severity::Warning,
        category: "architecture",
        title: "protocol missing explicit covers (new or unknown-age file)",
        finding: "An untracked, new, or unknown-age protocols/*.md file lacks an explicit file-level [covers] strand.",
        impact: "May be temporary — but the protocol is not yet linked into the causal graph.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::CreateCoverStrand,
            command_str: "tasktree append \"[covers] protocols/<file>\" --new",
            executable: true,
            requires_human: false,
        },
        producer: "check_arch_boundary",
    },
    DiagnosticInfo {
        code: "E084",
        severity: Severity::Error,
        category: "architecture",
        title: "prompt-strand non-whitelist marker",
        finding: "A prompt-strand entry uses a bracket-prefix marker not in the approved whitelist.",
        impact: "Unrestricted markers could inject commands or metadata that confuse agents or break context format.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Edit,
            command_str: "replace with a whitelisted marker or add the new marker to the whitelist first",
            executable: false,
            requires_human: true,
        },
        producer: "check_arch_boundary",
    },
    DiagnosticInfo {
        code: "W085",
        severity: Severity::Warning,
        category: "architecture",
        title: "cover strand may lag source",
        finding: "A prompt-strand covering a .md file has a last-entry timestamp older than the file's last git commit.",
        impact: "Agents reading the strand may see stale rules or decisions.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::AppendMarker,
            command_str: "append an [observed] entry summarizing the source file changes",
            executable: true,
            requires_human: false,
        },
        producer: "check_arch_boundary",
    },
];

// ── Lookup ──────────────────────────────────────────────────

pub fn lookup(code: &str) -> Option<&'static DiagnosticInfo> {
    CATALOG.iter().find(|d| d.code.eq_ignore_ascii_case(code))
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
        let info = lookup("E053").expect("E053 should be known");
        assert_eq!(info.code, "E053");
        assert_eq!(info.title, "done without verified");
        assert!(matches!(info.severity, Severity::Error));
    }

    #[test]
    fn test_lookup_case_insensitive() {
        let info = lookup("e053").expect("e053 should be known");
        assert_eq!(info.code, "E053");
    }

    #[test]
    fn test_lookup_unknown_code() {
        assert!(lookup("E999").is_none());
    }

    #[test]
    fn test_explain_json_known() {
        let output = cmd_explain("E053", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(v["ok"], true);
        assert_eq!(v["code"], "E053");
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
        assert!(codes.contains(&"E053"));
        assert!(codes.contains(&"E047"));
        assert!(codes.contains(&"W085"));
        assert!(codes.len() >= 20);
    }

    #[test]
    fn test_explain_json_recovery_fields() {
        let output = cmd_explain("E053", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        let recovery = &v["recovery"];
        assert_eq!(recovery["executable"], false);
        assert_eq!(recovery["requires_human"], true);
        assert!(recovery["command"].as_str().unwrap().contains("gate-commit"));
    }

    #[test]
    fn test_no_duplicate_codes() {
        use std::collections::HashSet;
        let codes: Vec<&str> = CATALOG.iter().map(|d| d.code).collect();
        let unique: HashSet<&str> = codes.iter().copied().collect();
        assert_eq!(codes.len(), unique.len(), "duplicate diagnostic codes found");
    }
}

/// Explain command: route a code or topic to the diagnostic catalog and
/// render it. Lives in the command layer so the catalog (diagnostics.rs)
/// stays pure data and never references the output DTO layer.
use crate::diagnostics::{Severity, lookup, topic_lookup, topics};

/// Routing order:
///   1. Diagnostic code lookup (case-insensitive; W062, w062, etc.)
///   2. Topic lookup (input lowercased; card, markers, retry, json, grammar)
///   3. Error with available-topics list and diagnostic-code hint
pub fn cmd_explain(input: &str, format_json: bool) -> String {
    // ── 1. Diagnostic code (case-insensitive) ──────────────
    if let Some(info) = lookup(input) {
        let output = crate::output::ExplainSuccessOutput::from(info);
        return if format_json {
            serde_json::to_string_pretty(&output).unwrap_or_else(|e| {
                format!(
                    r#"{{"ok":false,"code":"{}","error":"serialization failed: {}"}}"#,
                    input, e
                )
            })
        } else {
            format!(
                "{}\n  severity: {}\n  category: {}\n  title: {}\n\n  finding: {}\n\n  impact: {}\n\n  recovery:\n    kind: {:?}\n    command: {}\n    executable: {}\n    requires_human: {}\n\n  producer: {}",
                info.code,
                match info.severity {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                },
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
        };
    }

    // ── 2. Topic (exact lowercase match) ───────────────────
    let lowered = input.to_lowercase();
    if let Some(topic) = topic_lookup(&lowered) {
        let output = crate::output::ExplainTopicOutput::from(topic);
        return if format_json {
            serde_json::to_string_pretty(&output).unwrap_or_else(|e| {
                format!(
                    r#"{{"ok":false,"topic":"{}","error":"serialization failed: {}"}}"#,
                    input, e
                )
            })
        } else {
            format!("{}\n\n{}", topic.title, topic.body)
        };
    }

    // ── 3. Unknown ─────────────────────────────────────────
    let available_topics: Vec<&str> = topics().iter().map(|t| t.name).collect();
    if format_json {
        let error_output = crate::output::ExplainUnknownOutput::new(input, available_topics);
        serde_json::to_string_pretty(&error_output).unwrap_or_else(|_| {
            format!(
                r#"{{"ok":false,"input":"{}","error":"unknown code or topic"}}"#,
                input
            )
        })
    } else {
        format!(
            "unknown code or topic: {}\n  topics: {}\n  diagnostic codes: mnema explain W062 etc",
            input,
            available_topics.join(", "),
        )
    }
}

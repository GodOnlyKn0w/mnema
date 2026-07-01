//! Leaf utility helpers with no crate-internal type dependencies.
//!
//! Moved out of main.rs (Layer 5-shape refactor) so the lower/sibling modules
//! (commands/*, render, diagnostics) depend *down* on a leaf module instead of
//! reaching *up* into the crate root for these helpers. Pure functions over
//! std/chrono/serde/atty only — strand resolution over `Event` lives in
//! event.rs, not here.

use std::io::Read;

pub(crate) fn parse_event_ts(ts: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc))
}

/// Parse the `by=` token of a [deadline] entry. Accepts RFC3339 or a bare
/// date (YYYY-MM-DD, overdue after that day ends, UTC). Unparseable values
/// emit nothing — don't guess.
pub(crate) fn parse_deadline_by(content: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let by_val = content
        .split_whitespace()
        .find_map(|tok| tok.strip_prefix("by="))?;
    if let Some(ts) = parse_event_ts(by_val) {
        return Some(ts);
    }
    chrono::NaiveDate::parse_from_str(by_val, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(23, 59, 59))
        .map(|dt| chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc))
}

/// Default provenance from the `TASKTREE_PRODUCER` env var (per-session agent
/// identity), applied when no explicit `--provenance` is passed. Returns `None`
/// if the var is unset or blank. Explicit `--provenance` always wins.
pub(crate) fn env_producer_provenance() -> Option<serde_json::Value> {
    let producer = std::env::var("TASKTREE_PRODUCER").ok()?;
    let producer = producer.trim();
    if producer.is_empty() {
        None
    } else {
        Some(serde_json::json!({ "producer": producer }))
    }
}

/// Parse a `--provenance` argument. Must be a JSON object when present.
/// Returns the `TASKTREE_PRODUCER` env default for `None` input (or `None` if
/// unset); `Err` for malformed JSON or non-object shapes.
pub(crate) fn parse_provenance_arg(raw: Option<&str>) -> Result<Option<serde_json::Value>, String> {
    match raw {
        None => Ok(env_producer_provenance()),
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err("--provenance must be a non-empty JSON object".to_string());
            }
            let v: serde_json::Value = serde_json::from_str(trimmed)
                .map_err(|e| format!("--provenance is not valid JSON: {}", e))?;
            if !v.is_object() {
                return Err("--provenance must be a JSON object".to_string());
            }
            Ok(Some(v))
        }
    }
}

pub(crate) fn read_stdin_content() -> Result<String, String> {
    // Detect TTY: if stdin is a terminal, reject immediately to avoid agent hanging
    if atty::is(atty::Stream::Stdin) {
        return Err("--stdin requires piped input".to_string());
    }
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("failed to read stdin: {}", e))?;
    Ok(buf)
}

pub(crate) fn read_file_content(path: &str) -> Result<String, String> {
    let p = std::path::Path::new(path);
    if !p.exists() {
        return Err(format!("file not found: {}", path));
    }
    if p.is_dir() {
        return Err(format!("expected file path, got directory: {}", path));
    }
    let buf = std::fs::read_to_string(p).map_err(|e| format!("failed to read file: {}", e))?;
    Ok(buf)
}

pub(crate) fn looks_like_strand_id(value: &str) -> bool {
    let len = value.len();
    (6..=32).contains(&len) && value.chars().all(|c| c.is_ascii_hexdigit())
}

pub(crate) fn parse_duration(s: &str) -> Result<usize, String> {
    if s.is_empty() {
        return Err("empty duration".to_string());
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: usize = num_str.parse().map_err(|_| format!("invalid duration: {}", s))?;
    match unit {
        "s" => Ok(num),
        "m" => Ok(num * 60),
        "h" => Ok(num * 3600),
        "d" => Ok(num * 86400),
        _ => Err(format!("unknown duration unit '{}'. Use s/m/h/d (e.g. 2h)", unit)),
    }
}

pub(crate) fn shorten(id: &str) -> String {
    if id.len() > 12 {
        id[..12].to_string()
    } else {
        id.to_string()
    }
}

/// Collapse prose to a single-line preview, char-bounded by `max`.
///
/// 散文预览统一走这里：先在首个换行处截断（多行 entry/brief 只露首行，
/// orient/list/show 的「一眼扫」不被多行 blob 刷爆），再按字符数截断。
/// 任一处被截断都加 "..." 提示后面还有内容。完整正文仍可经 show 读取，
/// JSON 全保真字段（list 的 first_summary）不走本函数，故契约不变。
pub(crate) fn truncate(s: &str, max: usize) -> String {
    // First line only: a multi-line first entry must not flood one-line views.
    let first_line = s.split('\n').next().unwrap_or("");
    let has_more_lines = first_line.len() < s.len();
    let chars: Vec<char> = first_line.chars().collect();
    if chars.len() <= max {
        if has_more_lines {
            format!("{}...", first_line)
        } else {
            first_line.to_string()
        }
    } else {
        format!("{}...", chars[..max].iter().collect::<String>())
    }
}

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

/// Default provenance from the `MNEMA_PRODUCER` env var (per-session agent
/// identity), applied when no explicit `--provenance` is passed. Returns `None`
/// if the var is unset or blank. Explicit `--provenance` always wins.
pub(crate) fn env_producer_provenance() -> Option<serde_json::Value> {
    let producer = std::env::var("MNEMA_PRODUCER").ok()?;
    let producer = producer.trim();
    if producer.is_empty() {
        None
    } else {
        Some(serde_json::json!({ "producer": producer }))
    }
}

/// Parse a `--provenance` argument. Must be a JSON object when present.
/// Returns the `MNEMA_PRODUCER` env default for `None` input (or `None` if
/// unset); `Err` for malformed JSON or non-object shapes.
pub(crate) fn parse_provenance_arg(raw: Option<&str>) -> Result<Option<serde_json::Value>, String> {
    match raw {
        None => Ok(env_producer_provenance()),
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err("--provenance must be a non-empty JSON object".to_string());
            }
            let v: serde_json::Value = serde_json::from_str(trimmed).map_err(|e| {
                format!(
                    "--provenance is not valid JSON: {}\n  example: --provenance '{{\"key\":\"value\"}}'",
                    e
                )
            })?;
            if !v.is_object() {
                return Err(
                    "--provenance must be a JSON object\n  example: --provenance '{\"key\":\"value\"}'"
                        .to_string(),
                );
            }
            Ok(Some(v))
        }
    }
}

pub(crate) fn read_stdin_content() -> Result<String, String> {
    // Detect TTY: if stdin is a terminal, reject immediately to avoid agent hanging
    if atty::is(atty::Stream::Stdin) {
        return Err("stdin requires piped input".to_string());
    }
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("failed to read stdin: {}", e))?;
    Ok(buf)
}

/// Read an optional note from stdin: `None` when stdin is a terminal (no pipe)
/// or the piped content is blank. Used for close/reopen reasons, which are
/// optional — a bare `mnema close --id X` (no pipe) must still work.
pub(crate) fn read_stdin_if_piped() -> Option<String> {
    if atty::is(atty::Stream::Stdin) {
        return None;
    }
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() {
        return None;
    }
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
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

/// Render a duration in seconds as a human-readable string.
/// < 60s → "just now"; < 3600s → "<N>m"; < 86400s → "<N>h"; else "<N>d".
/// No external dependencies — purely arithmetic.
pub(crate) fn humanize_duration(secs: i64) -> String {
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

pub(crate) fn parse_duration(s: &str) -> Result<usize, String> {
    if s.is_empty() {
        return Err("empty duration".to_string());
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: usize = num_str
        .parse()
        .map_err(|_| format!("invalid duration: {}", s))?;
    match unit {
        "s" => Ok(num),
        "m" => Ok(num * 60),
        "h" => Ok(num * 3600),
        "d" => Ok(num * 86400),
        _ => Err(format!(
            "unknown duration unit '{}'. Use s/m/h/d (e.g. 2h)",
            unit
        )),
    }
}

pub(crate) fn shorten(id: &str) -> String {
    if id.len() > 12 {
        id[..12].to_string()
    } else {
        id.to_string()
    }
}

/// Compact display form of an RFC3339 timestamp: "MM-DD HH:MM".
/// Display-layer only — storage keeps the full timestamp (CORPUS §8:
/// raw high-precision timestamps are machine fields, not reader fields).
/// Falls back to the raw string when it is too short to slice.
pub(crate) fn compact_ts(ts: &str) -> String {
    if ts.len() >= 16 && ts.is_char_boundary(5) && ts.is_char_boundary(16) {
        format!("{} {}", &ts[5..10], &ts[11..16])
    } else {
        ts.to_string()
    }
}

/// Reader-facing time: relative and absolute together — "3d ago(06-29 13:21)".
/// Pure relative wording expires inside a long conversation; a bare timestamp
/// means nothing to a clock-less reader, so both travel together (CORPUS §8).
/// `now` is injected by the command layer so rendering is deterministic under
/// test; unparseable or future timestamps fall back to the absolute form
/// (clock skew: assert nothing).
pub(crate) fn display_ts(ts: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let compact = compact_ts(ts);
    let parsed = match chrono::DateTime::parse_from_rfc3339(ts) {
        Ok(t) => t.with_timezone(&chrono::Utc),
        Err(_) => return compact,
    };
    let secs = (now - parsed).num_seconds();
    if secs < 0 {
        return compact;
    }
    let rel = humanize_duration(secs);
    if rel == "just now" {
        format!("just now({})", compact)
    } else {
        format!("{} ago({})", rel, compact)
    }
}

/// Seconds between two RFC3339 timestamps (later minus earlier), if both
/// parse. Used for the in-line long-gap annotation ("gap: 19d").
pub(crate) fn ts_gap_seconds(earlier: &str, later: &str) -> Option<i64> {
    let e = chrono::DateTime::parse_from_rfc3339(earlier).ok()?;
    let l = chrono::DateTime::parse_from_rfc3339(later).ok()?;
    Some((l - e).num_seconds())
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

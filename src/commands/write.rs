/// Write-command family: cmd_add, cmd_append, cmd_close, cmd_reopen, cmd_checkpoint.
/// Moved from main.rs (Layer 4a refactor); function bodies are byte-identical to
/// the originals (only cross-module path qualification added where required).
///
/// Dependency direction: write → journal, event, projection, diagnostics, render (via crate::*)
/// write ← main.rs (mod commands; pub(crate) use commands::write::*)
use crate::diagnostics;
use crate::event::{self, Event, find_strand, resolve_id};
use crate::journal::{ensure_journal, read_events_lossy, read_events_strict,
                     with_journal_write_lock, append_event_unlocked};
use crate::projection;
use crate::{strand_card_fresh, strand_card_fresh_with_state,
            print_card_with_state, print_handle_line};
use crate::util::{shorten, read_stdin_content, read_file_content,
                  looks_like_strand_id, parse_provenance_arg, humanize_duration,
                  parse_event_ts};
use serde_json::json;

/// Strip at most one trailing newline (\n or \r\n).
/// Preserves leading whitespace, interior newlines, code blocks.
pub(crate) fn normalize_content(raw: &str) -> String {
    if raw.ends_with("\r\n") {
        raw[..raw.len() - 2].to_string()
    } else if raw.ends_with('\n') {
        raw[..raw.len() - 1].to_string()
    } else {
        raw.to_string()
    }
}

/// Single source of truth for all known append markers.
/// Used by validate_lifecycle_marker, suggest_marker, and CI closure tests.
pub fn known_markers() -> &'static [&'static str] {
    &[
        // judgment
        "[decision]", "[constraint]", "[friction]", "[fixed]", "[lesson]", "[insight]",
        // observation
        "[observed]", "[check]", "[progress]", "[deliverable]", "[metric]",
        // planning
        "[deadline]",
        // structure
        "[covers]", "[guide]", "[skill]", "[task]", "[session]",
        // closing
        "[done]", "[verified]", "[cancelled]", "[failed]", "[merged]", "[ended]",
        "[dispatched]", "[registered]",
        // system
        "[checkpoint]", "[hidden]", "[waiting:human]", "[grill]",
    ]
}

pub(crate) fn is_known_marker_str(marker: &str) -> bool {
    known_markers().contains(&marker)
}

/// Compute Levenshtein edit distance between two strings.
/// No external dependencies — pure Rust, O(m*n).
pub(crate) fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();
    if m == 0 { return n; }
    if n == 0 { return m; }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

/// If `word` (a bracket-stripped lowercase marker candidate) is close to a
/// known marker, return the best suggestion.  Returns None for exact matches,
/// unknown-but-distant words, and non-alphabetic tags like [W062] or [2026-06].
pub(crate) fn suggest_marker(marker: &str) -> Option<&'static str> {
    // Strip brackets: "[freiction]" → "freiction"
    let inner = marker.trim_start_matches('[').trim_end_matches(']');
    // Reject if it contains non-alphabetic chars (handles [W062], [2026-06], [my-tag], etc.)
    if inner.chars().any(|c| !c.is_alphabetic() && c != ':') {
        return None;
    }
    // Find closest known marker by edit distance on inner word vs known inner word
    let mut best_dist = usize::MAX;
    let mut best_marker: Option<&'static str> = None;
    for &km in known_markers() {
        let km_inner = km.trim_start_matches('[').trim_end_matches(']');
        let dist = levenshtein(inner, km_inner);
        if dist == 0 {
            return None; // exact match — not a typo
        }
        if dist < best_dist {
            best_dist = dist;
            best_marker = Some(km);
        }
    }
    if best_dist <= 2 {
        best_marker
    } else {
        None
    }
}

pub(crate) fn validate_lifecycle_marker(content: &str) -> Result<(), String> {
    // All bracket-prefixed content is accepted — unknown markers are handled
    // by the W073 warning path in cmd_append (post-write, stderr only).
    // This function is kept for future hard-rejection cases (none currently).
    let _ = content;
    Ok(())
}

pub(crate) fn cmd_add(
    content: Option<&str>,
    stdin: bool,
    file: Option<&str>,
    format_json: bool,
    strand_type: Option<&str>,
    provenance_raw: Option<&str>,
) -> Result<(), String> {
    // ---- Content Source Resolution (mirrors append) ----
    let source_kind = match (content.is_some(), stdin, file.is_some()) {
        (false, false, false) => {
            return Err(
                "choose a content source: positional content, --stdin, or --file <path>"
                    .to_string(),
            );
        }
        (true, false, false) => "positional",
        (false, true, false) => "stdin",
        (false, false, true) => "file",
        _ => {
            let mut sources = Vec::new();
            if content.is_some() { sources.push("positional content"); }
            if stdin { sources.push("--stdin"); }
            if file.is_some() { sources.push("--file"); }
            return Err(format!(
                "choose only one content source, got: {}",
                sources.join(", ")
            ));
        }
    };

    let raw = match source_kind {
        "positional" => content.unwrap().to_string(),
        "stdin" => read_stdin_content()?,
        "file" => read_file_content(file.unwrap())?,
        _ => unreachable!(),
    };

    if raw.trim().is_empty() {
        let hint = match source_kind {
            "stdin" => "stdin content is empty",
            "file" => return Err(format!("file content is empty: {}", file.unwrap())),
            _ => "content is empty",
        };
        return Err(hint.to_string());
    }

    // Strip trailing newline (same as append), preserve other whitespace
    let stored = normalize_content(&raw);

    // Auto-detect strand type from content if not provided
    let resolved_type = strand_type.or_else(|| {
        if stored.starts_with("para group ") { Some("dag") }
        else if stored.starts_with('[') && stored.len() > 2
            && stored[1..].chars().next().map_or(false, |c| c.is_ascii_digit())
        { Some("task") }
        else { None }
    });

    let provenance = parse_provenance_arg(provenance_raw)?;

    // acquire lock once, write both events atomically
    let result = with_journal_write_lock(|journal| {
        let (created, mut appended) = event::make_strand_created(&stored, resolved_type);
        // Attach provenance to the initial LogAppended event
        if let Event::LogAppended { provenance: ref mut prov_field, .. } = appended {
            *prov_field = provenance.clone();
        }
        let id = created.strand_id().to_string();
        append_event_unlocked(journal, &created)?;
        append_event_unlocked(journal, &appended)?;
        Ok(id)
    });
    let id = match result {
        Ok(id) => id,
        Err(e) => return Err(e),
    };
    if format_json {
        let card = strand_card_fresh(&id);
        let card_val = card.as_ref().map(|c| serde_json::to_value(c).ok()).flatten();
        println!("{}", json!({"id": id, "status": "ok", "provenance": provenance, "result": card_val}));
    } else {
        println!("{}", id);
        if let Some((card, state)) = strand_card_fresh_with_state(&id) {
            print_card_with_state(&card, &state);
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn cmd_append(
    content: Option<&str>,
    legacy_id: Option<&str>,
    new: bool,
    stdin: bool,
    file: Option<&str>,
    explicit_id: Option<&str>,
    format: Option<&str>,
    provenance_raw: Option<&str>,
) -> Result<(), String> {
    cmd_append_with_seen_offset(
        content,
        legacy_id,
        new,
        stdin,
        file,
        explicit_id,
        format,
        provenance_raw,
        None,
    )
}

pub(crate) fn cmd_append_with_seen_offset(
    content: Option<&str>,
    legacy_id: Option<&str>,
    new: bool,
    stdin: bool,
    file: Option<&str>,
    explicit_id: Option<&str>,
    format: Option<&str>,
    provenance_raw: Option<&str>,
    seen_offset: Option<usize>,
) -> Result<(), String> {
    // ---- Content Source Resolution ----
    if (stdin || file.is_some())
        && legacy_id.is_none()
        && content.map(looks_like_strand_id).unwrap_or(false)
    {
        return Err(
            "warn: stdin and --file require --id to specify target; positional strand id is not supported with this content source".to_string()
        );
    }

    let source_kind = match (content.is_some(), stdin, file.is_some()) {
        (false, false, false) => {
            return Err(
                "choose a content source: positional content, --stdin, or --file <path>"
                    .to_string(),
            );
        }
        (true, false, false) => "positional",
        (false, true, false) => "stdin",
        (false, false, true) => "file",
        _ => {
            let mut sources = Vec::new();
            if content.is_some() {
                sources.push("positional content");
            }
            if stdin {
                sources.push("--stdin");
            }
            if file.is_some() {
                sources.push("--file");
            }
            return Err(format!(
                "choose only one content source, got: {}",
                sources.join(", ")
            ));
        }
    };

    // Read raw content
    let raw = match source_kind {
        "positional" => content.unwrap().to_string(),
        "stdin" => read_stdin_content()?,
        "file" => read_file_content(file.unwrap())?,
        _ => unreachable!(),
    };

    // Empty check (trimmed for detection, but we don't trim for storage)
    if raw.trim().is_empty() {
        let hint = match source_kind {
            "stdin" => "stdin content is empty",
            "file" => return Err(format!("file content is empty: {}", file.unwrap())),
            _ => "content is empty",
        };
        return Err(hint.to_string());
    }

    // Normalize: strip at most one trailing newline/CRLF, preserve leading whitespace
    let stored = normalize_content(&raw);
    validate_lifecycle_marker(&stored)?;

    // Load journal for target resolution
    let path = ensure_journal()?;
    let (events, _) = read_events_lossy(&path);

    // ---- Target Resolution ----
    if let (Some(first), Some(second)) = (content, legacy_id) {
        if find_strand(&events, first).is_some() && find_strand(&events, second).is_none() {
            return Err(format!(
                "positional append arguments look reversed. Use:\n  tasktree append --id {} \"{}\"",
                first,
                second.replace('"', "\\\"")
            ));
        }
    }

    let target_count = [new, explicit_id.is_some(), legacy_id.is_some()]
        .iter()
        .filter(|&&x| x)
        .count();

    if target_count > 1 {
        return Err("choose only one target: --new, --id, or positional strand id".to_string());
    }

    // Legacy positional id only valid with positional content source
    if legacy_id.is_some() && source_kind != "positional" {
        return Err(
            "warn: stdin and --file require --id to specify target; positional strand id is not supported with this content source".to_string()
        );
    }

    if new {
        // Create new strand — same as Add but using stored content
        let (created, appended) = event::make_strand_created(&stored, Some("session"));
        let new_id = created.strand_id().to_string();
        with_journal_write_lock(|journal| {
            append_event_unlocked(journal, &created)?;
            append_event_unlocked(journal, &appended)?;
            Ok(())
        })?;
        println!("{}", new_id);
        if let Some((card, state)) = strand_card_fresh_with_state(&new_id) {
            print_card_with_state(&card, &state);
        }
        return Ok(());
    }

    // Resolve target strand
    let target_id = explicit_id.or(legacy_id);
    let full_id = if let Some(id) = target_id {
        find_strand(&events, id).ok_or_else(|| {
            let mut msg = format!("strand {} not found", id);
            if id == "-" {
                msg.push_str(
                    ". If you meant to pipe content from stdin, use:\n  echo \"...\" | tasktree append --stdin --id <id>",
                );
            }
            msg
        })?
    } else {
        // Append to most recently active strand (last-append ordering)
        let strands = projection::project_strands(&events, false);
        let mut sorted: Vec<_> = strands.iter().collect();
        sorted.sort_by(|a, b| b.last_ts().cmp(&a.last_ts()));
        let recent = sorted
            .first()
            .ok_or("no strands found — use 'add' or 'append --new' first")?;
        recent.id.clone()
    };

    let strand_last_offset = projection::project_strands(&events, true)
        .iter()
        .find(|s| s.id == full_id)
        .map(|s| s.last_offset())
        .unwrap_or(0);
    let w076 = diagnostics::check_w076_seen_offset(&full_id, seen_offset, strand_last_offset);
    let provenance = parse_provenance_arg(provenance_raw)?;
    let event = event::make_log_appended(&full_id, &stored, provenance.clone());
    let append_id = match &event {
        Event::LogAppended { append_id, .. } => append_id.clone(),
        _ => None,
    };
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &event)
    })?;

    // W073: after successful write, check for possible marker typo (stderr only).
    {
        let trimmed = stored.trim_start();
        if trimmed.starts_with('[') {
            if let Some(end) = trimmed.find(']') {
                let marker = &trimmed[..=end];
                if !is_known_marker_str(marker) {
                    if let Some(suggestion) = suggest_marker(marker) {
                        eprintln!(
                            "W073: unknown marker {} — did you mean {}? (tasktree explain markers)",
                            marker, suggestion
                        );
                    }
                }
            }
        }
    }

    // W074: nudge when a closing marker is appended — markers are now annotations only.
    // Fires only on exact closing-marker prefix matches (precision-first: no false positives).
    if diagnostics::is_closing_annotation_marker(&stored) {
        eprintln!(
            "W074: [done]/[failed]/[cancelled]/[merged]/[verified] are annotations — \
            they no longer close the strand. Use: tasktree close --id {} (tasktree explain W074)",
            shorten(&full_id)
        );
    }

    if let Some(w) = &w076 {
        eprintln!("{}: {} (tasktree explain {})", w.code, w.detail, w.code);
    }

    if format == Some("json") {
        let card = strand_card_fresh(&full_id);
        let card_val = card.as_ref().map(|c| serde_json::to_value(c).ok()).flatten();
        let warnings_json: Vec<serde_json::Value> = w076
            .iter()
            .map(|w| json!({
                "code": w.code,
                "detail": w.detail,
                "seen_offset": w.seen_offset,
                "strand_last_offset": w.strand_last_offset,
                "seen_gap": w.seen_gap,
                "catch_up": w.catch_up,
            }))
            .collect();
        println!("{}", serde_json::to_string(&serde_json::json!({
            "strand_id": full_id,
            "append_id": append_id,
            "content_preview": stored.chars().take(120).collect::<String>(),
            "provenance": provenance,
            "seen_offset": seen_offset,
            "seen_gap": w076.as_ref().map(|w| w.seen_gap),
            "warnings": warnings_json,
            "result": card_val,
        })).unwrap());
    } else {
        let prod = provenance
            .as_ref()
            .and_then(|p| p.get("producer"))
            .and_then(|v| v.as_str())
            .map(|p| format!(" producer={}", p))
            .unwrap_or_default();
        if let Some((card, state)) = strand_card_fresh_with_state(&full_id) {
            println!("appended to {} (offset {}){}", shorten(&full_id), card.last_offset, prod);
            print_card_with_state(&card, &state);
        } else {
            println!("appended to {}{}", shorten(&full_id), prod);
        }
    }
    Ok(())
}

/// Close a strand by writing a StrandClosed lifecycle event.
/// `disposition` defaults to "done" when not specified.
pub(crate) fn cmd_close(id: &str, disposition: Option<&str>, format_json: bool) -> Result<(), String> {
    let disp = disposition.unwrap_or("done");
    if !projection::CLOSE_DISPOSITIONS.contains(&disp) {
        let valid = projection::CLOSE_DISPOSITIONS.join(", ");
        return Err(format!(
            "invalid disposition {:?}; valid values: {}",
            disp, valid
        ));
    }
    let strand_id = resolve_id(&read_events_strict(&ensure_journal()?)?, id)?;
    // Check current state before writing (readable feedback, not a gate)
    let path = ensure_journal()?;
    let (events, _) = read_events_lossy(&path);
    let (current_state, _, _) =
        projection::compute_state_from_events(&events, &strand_id);
    if current_state.starts_with("closed:") {
        return Err(format!(
            "strand {} is already {}; use reopen first",
            shorten(&strand_id), current_state
        ));
    }
    let close_event = event::make_strand_closed(&strand_id, disp, None);
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &close_event)
    })?;
    if format_json {
        let card_val = strand_card_fresh(&strand_id)
            .as_ref()
            .map(|c| serde_json::to_value(c).ok())
            .flatten();
        println!("{}", serde_json::to_string(&serde_json::json!({
            "strand_id": strand_id,
            "disposition": disp,
            "lifecycle": format!("closed:{}", disp),
            "status": "ok",
            "result": card_val,
        })).unwrap());
    } else {
        let lifecycle = format!("closed:{}", disp);
        if let Some((card, _)) = strand_card_fresh_with_state(&strand_id) {
            print_handle_line(&card, &lifecycle);
            eprintln!("    {}", card.summary);
        } else {
            eprintln!("  closed {}", shorten(&strand_id));
        }
    }
    Ok(())
}

/// Reopen a closed strand by writing a StrandReopened lifecycle event.
pub(crate) fn cmd_reopen(id: &str, format_json: bool) -> Result<(), String> {
    let strand_id = resolve_id(&read_events_strict(&ensure_journal()?)?, id)?;
    let path = ensure_journal()?;
    let (events, _) = read_events_lossy(&path);
    let (current_state, _, _) =
        projection::compute_state_from_events(&events, &strand_id);
    if current_state == "registered" {
        return Err(format!(
            "strand {} is already open (registered); nothing to reopen",
            shorten(&strand_id)
        ));
    }
    let reopen_event = event::make_strand_reopened(&strand_id, None);
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &reopen_event)
    })?;
    if format_json {
        let card_val = strand_card_fresh(&strand_id)
            .as_ref()
            .map(|c| serde_json::to_value(c).ok())
            .flatten();
        println!("{}", serde_json::to_string(&serde_json::json!({
            "strand_id": strand_id,
            "lifecycle": "registered",
            "status": "ok",
            "result": card_val,
        })).unwrap());
    } else {
        if let Some((card, state)) = strand_card_fresh_with_state(&strand_id) {
            print_handle_line(&card, &state);
            eprintln!("    {}", card.summary);
        } else {
            eprintln!("  reopened {}", shorten(&strand_id));
        }
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct CheckpointFailure {
    pub(crate) code: i32,
    pub(crate) message: String,
    pub(crate) requested_strand: Option<String>,
    pub(crate) resolved_strand: Option<String>,
    pub(crate) journal_appended: bool,
}

pub(crate) fn checkpoint_error_json(failure: &CheckpointFailure) {
    println!(
        "{}",
        json!({
            "ok": false,
            "error": failure.message,
            "requested_strand": failure.requested_strand,
            "resolved_strand": failure.resolved_strand,
            "journal_appended": failure.journal_appended,
        })
    );
}

pub(crate) fn resolve_most_recent_strand(strands: &[projection::ProjectedStrand]) -> Option<&projection::ProjectedStrand> {
    let mut sorted: Vec<_> = strands.iter().collect();
    sorted.sort_by(|a, b| b.last_ts().cmp(a.last_ts()));
    sorted.into_iter().next()
}

pub(crate) fn escape_checkpoint_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
pub(crate) fn cmd_checkpoint(
    requested_id: Option<&str>,
    action: &str,
    tail: Option<usize>,
    format_json: bool,
    include_hidden: bool,
    provenance_raw: Option<&str>,
) -> Result<(), CheckpointFailure> {
    cmd_checkpoint_with_seen_offset(
        requested_id,
        action,
        tail,
        format_json,
        include_hidden,
        provenance_raw,
        None,
    )
}

pub(crate) fn cmd_checkpoint_with_seen_offset(
    requested_id: Option<&str>,
    action: &str,
    tail: Option<usize>,
    format_json: bool,
    include_hidden: bool,
    provenance_raw: Option<&str>,
    seen_offset: Option<usize>,
) -> Result<(), CheckpointFailure> {
    if action.trim().is_empty() {
        return Err(CheckpointFailure {
            code: 3,
            message: "invalid arguments: --action cannot be empty".to_string(),
            requested_strand: requested_id.map(str::to_string),
            resolved_strand: None,
            journal_appended: false,
        });
    }

    let path = ensure_journal().map_err(|e| CheckpointFailure {
        code: 1,
        message: format!("strand resolve/show failed: {}", e),
        requested_strand: requested_id.map(str::to_string),
        resolved_strand: None,
        journal_appended: false,
    })?;
    let events = read_events_strict(&path).map_err(|e| CheckpointFailure {
        code: 1,
        message: format!("strand resolve/show failed: {}", e),
        requested_strand: requested_id.map(str::to_string),
        resolved_strand: None,
        journal_appended: false,
    })?;
    // Two projection views:
    //   - `all_strands` includes hidden strands: used to resolve an explicit
    //     --id lookup, because the user named the strand directly and we
    //     should not silently refuse to checkpoint a hidden one.
    //   - `visible_strands` honours the include-hidden flag: used to pick
    //     the most-recent active strand, which is the only place a default
    //     checkpoint would otherwise pick a hidden strand by accident.
    let all_strands = projection::project_strands(&events, true);
    let visible_strands = projection::project_strands(&events, include_hidden);

    let (strand, resolved_by) = if let Some(id) = requested_id {
        let full = find_strand(&events, id).ok_or_else(|| CheckpointFailure {
            code: 1,
            message: format!("strand resolve/show failed: strand {} not found", id),
            requested_strand: Some(id.to_string()),
            resolved_strand: None,
            journal_appended: false,
        })?;
        let strand = all_strands
            .iter()
            .find(|s| s.id == full)
            .ok_or_else(|| CheckpointFailure {
                code: 1,
                message: format!("strand resolve/show failed: strand {} not found", id),
                requested_strand: Some(id.to_string()),
                resolved_strand: None,
                journal_appended: false,
            })?;
        (strand, "explicit --id")
    } else {
        let strand = resolve_most_recent_strand(&visible_strands).ok_or_else(|| CheckpointFailure {
            code: 1,
            message: "strand resolve/show failed: no strands found".to_string(),
            requested_strand: None,
            resolved_strand: None,
            journal_appended: false,
        })?;
        (strand, "most_recent_active_strand")
    };

    // ── Staleness snapshot (before append) ───────────────────────────────
    // Compute before the write so the delta reflects pre-checkpoint state.
    let strand_last_offset = strand.last_offset();
    let max_offset_before = events.last().map(|(o, _)| *o).unwrap_or(0);
    let journal_delta = max_offset_before.saturating_sub(strand_last_offset);

    // Parse strand's last ts for "last touched N ago" display.
    let staleness_seconds: Option<i64> = if strand.last_ts().is_empty() {
        None
    } else {
        parse_event_ts(strand.last_ts()).map(|ts| (chrono::Utc::now() - ts).num_seconds())
    };

    // ── Gate warnings (W070 / W071) — evaluated before write ─────────────
    let provenance_val = parse_provenance_arg(provenance_raw).map_err(|message| CheckpointFailure {
        code: 3,
        message,
        requested_strand: requested_id.map(str::to_string),
        resolved_strand: Some(strand.id.clone()),
        journal_appended: false,
    })?;
    let checkpoint_producer: Option<&str> = provenance_val
        .as_ref()
        .and_then(|p| p.get("producer"))
        .and_then(|v| v.as_str());
    let w070 = diagnostics::check_w070_strand_moved(&events, &strand.id, checkpoint_producer);
    let w071 = diagnostics::check_w071_closed_strand(strand);
    let w076 = diagnostics::check_w076_seen_offset(&strand.id, seen_offset, strand_last_offset);

    let observed_entries_before_append = strand.log_count();
    let escaped_action = escape_checkpoint_value(action);
    let content = format!(
        "[checkpoint] ok resolved_by=\"{}\" observed_entries_before_append={} action=\"{}\"",
        resolved_by, observed_entries_before_append, escaped_action
    );
    let event = event::make_log_appended(&strand.id, &content, provenance_val);
    let append_id = match &event {
        Event::LogAppended { append_id, .. } => append_id.clone(),
        _ => None,
    };
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &event)
    }).map_err(|e| CheckpointFailure {
        code: 2,
        message: format!("journal append failed: {}", e),
        requested_strand: requested_id.map(str::to_string),
        resolved_strand: Some(strand.id.clone()),
        journal_appended: false,
    })?;

    let shown_entries: Vec<_> = if let Some(n) = tail {
        let skip = strand.log.len().saturating_sub(n);
        strand.log[skip..].iter().collect()
    } else {
        strand.log.iter().collect()
    };

    // Run diagnostics on the pre-append events (checkpoint itself is not a
    // diagnostic target; re-reading after append would be equivalent here).
    let raw_events: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
    let diags = diagnostics::run_journal_diagnostics(&raw_events, chrono::Utc::now());
    let diag_count = diags.len();

    // Build warning list (W070/W071/W076) for output.
    let mut cp_warnings: Vec<serde_json::Value> = Vec::new();
    if let Some((code, detail)) = w070 {
        cp_warnings.push(json!({"code": code, "detail": detail}));
    }
    if let Some((code, detail)) = w071 {
        cp_warnings.push(json!({"code": code, "detail": detail}));
    }
    if let Some(w) = &w076 {
        cp_warnings.push(json!({
            "code": w.code,
            "detail": w.detail,
            "seen_offset": w.seen_offset,
            "strand_last_offset": w.strand_last_offset,
            "seen_gap": w.seen_gap,
            "catch_up": w.catch_up,
        }));
    }

    if format_json {
        let card = strand_card_fresh(&strand.id);
        let card_val = card.as_ref().map(|c| serde_json::to_value(c).ok()).flatten();
        let catch_up_val: serde_json::Value = if journal_delta > 0 {
            json!(format!(
                "tasktree timeline --since-offset {} --links {}",
                strand_last_offset, shorten(&strand.id)
            ))
        } else {
            serde_json::Value::Null
        };
        println!(
            "{}",
            json!({
                "ok": true,
                "strand": shorten(&strand.id),
                "resolved_strand": strand.id,
                "resolved_by": resolved_by,
                "observed_entries_before_append": observed_entries_before_append,
                "shown_entries": shown_entries.len(),
                "action": action,
                "append_id": append_id,
                "journal_appended": true,
                "diagnostics_count": diag_count,
                "result": card_val,
                "staleness_seconds": staleness_seconds,
                "journal_delta": journal_delta,
                "seen_offset": seen_offset,
                "seen_gap": w076.as_ref().map(|w| w.seen_gap),
                "catch_up": catch_up_val,
                "warnings": cp_warnings,
            })
        );
    } else {
        println!("checkpoint ok");
        println!("  strand: {} | {} entries | {}", shorten(&strand.id), strand.log_count() + 1, strand.state());
        println!("  resolved_by: {}", resolved_by);

        // Staleness line — always printed after strand line.
        let staleness_part = staleness_seconds.map(|s| {
            let d = humanize_duration(s);
            if d == "just now" {
                "last touched just now | ".to_string()
            } else {
                format!("last touched {} ago | ", d)
            }
        }).unwrap_or_default();
        println!(
            "  staleness: {}journal +{} entries since (offset {} → {})",
            staleness_part, journal_delta, strand_last_offset, max_offset_before
        );

        // Catch-up line — only when delta > 0.
        if journal_delta > 0 {
            println!(
                "  catch-up: tasktree timeline --since-offset {} --links {}",
                strand_last_offset, shorten(&strand.id)
            );
        }

        println!(
            "  observed_entries_before_append: {}",
            observed_entries_before_append
        );
        println!("  action: {}", action);
        if let Some(id) = append_id {
            println!("  append_id: {}", id);
        }
        println!("  appended to journal");
        println!("log:");
        for entry in shown_entries {
            let id_str = entry
                .append_id
                .as_ref()
                .map(|a| format!(" [{}]", &a[..12]))
                .unwrap_or_default();
            println!("  [{}]{} {}", &entry.ts[..19], id_str, entry.content);
        }
        // W-code scar lines — printed before the general diagnostics count.
        for warning in &cp_warnings {
            let code = warning["code"].as_str().unwrap_or("W");
            let detail = warning["detail"].as_str().unwrap_or("");
            println!("  {} {}  (tasktree explain {})", code, detail, code);
        }
        if diag_count > 0 {
            println!("diagnostics: {} warning(s) — run tasktree doctor journal", diag_count);
        }
    }

    Ok(())
}

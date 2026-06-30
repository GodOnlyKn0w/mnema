/// Write-command family: cmd_add, cmd_append, cmd_close, cmd_reopen, cmd_checkpoint.
/// Moved from main.rs (Layer 4a refactor); function bodies are byte-identical to
/// the originals (only cross-module path qualification added where required).
///
/// Dependency direction: write → journal, event, projection, diagnostics, render (via crate::*)
/// write ← main.rs (mod commands; pub(crate) use commands::write::*)
use crate::diagnostics;
use crate::event::{self, Event, find_strand, resolve_id};
use crate::journal::{
    append_event_unlocked, ensure_journal, read_events_lossy, read_events_strict,
    with_journal_write_lock,
};
use crate::markers::{
    is_closing_annotation_marker, is_known_marker_str, suggest_marker, validate_lifecycle_marker,
};
use crate::output::{self, OrientStrand};
use crate::projection;
use crate::util::{
    humanize_duration, looks_like_strand_id, parse_event_ts, parse_provenance_arg,
    read_file_content, read_stdin_content, shorten,
};
use crate::{
    print_card_with_state, print_handle_line, strand_card_fresh, strand_card_fresh_with_state,
};

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
        if stored.starts_with("para group ") {
            Some("dag")
        } else if stored.starts_with('[')
            && stored.len() > 2
            && stored[1..]
                .chars()
                .next()
                .map_or(false, |c| c.is_ascii_digit())
        {
            Some("task")
        } else {
            None
        }
    });

    let provenance = parse_provenance_arg(provenance_raw)?;

    // acquire lock once, write both events atomically
    let result = with_journal_write_lock(|journal| {
        let (created, mut appended) = event::make_strand_created(&stored, resolved_type);
        // Attach provenance to the initial LogAppended event
        if let Event::LogAppended {
            provenance: ref mut prov_field,
            ..
        } = appended
        {
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
        let output = output::AddOutput {
            id: id.clone(),
            status: "ok",
            provenance: provenance.as_ref(),
            result: strand_card_fresh(&id),
        };
        println!("{}", serde_json::to_string(&output).unwrap());
    } else {
        println!("{}", id);
        if let Some((card, state)) = strand_card_fresh_with_state(&id) {
            print_card_with_state(&card, &state);
        }
    }
    Ok(())
}

pub(crate) struct AppendRequest<'a> {
    pub(crate) content: Option<&'a str>,
    pub(crate) legacy_id: Option<&'a str>,
    pub(crate) new: bool,
    pub(crate) stdin: bool,
    pub(crate) file: Option<&'a str>,
    pub(crate) explicit_id: Option<&'a str>,
    pub(crate) provenance_raw: Option<&'a str>,
    pub(crate) seen_offset: Option<usize>,
    pub(crate) why: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppendOutcomeKind {
    CreatedNew,
    AppendedExisting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppendMarkerWarning {
    pub(crate) marker: String,
    pub(crate) suggestion: &'static str,
}

#[derive(Debug, Clone)]
pub(crate) struct AppendOutcome {
    pub(crate) kind: AppendOutcomeKind,
    pub(crate) strand_id: String,
    pub(crate) append_id: Option<String>,
    pub(crate) stored_content: String,
    pub(crate) provenance: Option<serde_json::Value>,
    pub(crate) seen_offset: Option<usize>,
    pub(crate) seen_warning: Option<diagnostics::SeenOffsetWarning>,
    pub(crate) marker_warning: Option<AppendMarkerWarning>,
    pub(crate) closing_marker_warning: bool,
    pub(crate) card_state: Option<(OrientStrand, String)>,
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
    why: Option<&str>,
) -> Result<(), String> {
    let outcome = execute_append(AppendRequest {
        content,
        legacy_id,
        new,
        stdin,
        file,
        explicit_id,
        provenance_raw,
        seen_offset,
        why,
    })?;
    render_append_outcome(&outcome, format);
    Ok(())
}

pub(crate) fn execute_append(req: AppendRequest<'_>) -> Result<AppendOutcome, String> {
    if (req.stdin || req.file.is_some())
        && req.legacy_id.is_none()
        && req.content.map(looks_like_strand_id).unwrap_or(false)
    {
        return Err(
            "warn: stdin and --file require --id to specify target; positional strand id is not supported with this content source".to_string()
        );
    }

    let source_kind = match (req.content.is_some(), req.stdin, req.file.is_some()) {
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
            if req.content.is_some() {
                sources.push("positional content");
            }
            if req.stdin {
                sources.push("--stdin");
            }
            if req.file.is_some() {
                sources.push("--file");
            }
            return Err(format!(
                "choose only one content source, got: {}",
                sources.join(", ")
            ));
        }
    };

    let raw = match source_kind {
        "positional" => req.content.unwrap().to_string(),
        "stdin" => read_stdin_content()?,
        "file" => read_file_content(req.file.unwrap())?,
        _ => unreachable!(),
    };

    if raw.trim().is_empty() {
        let hint = match source_kind {
            "stdin" => "stdin content is empty",
            "file" => return Err(format!("file content is empty: {}", req.file.unwrap())),
            _ => "content is empty",
        };
        return Err(hint.to_string());
    }

    let stored = normalize_content(&raw);
    validate_lifecycle_marker(&stored)?;

    let path = ensure_journal()?;
    let (events, _) = read_events_lossy(&path);

    if let (Some(first), Some(second)) = (req.content, req.legacy_id) {
        if find_strand(&events, first).is_some() && find_strand(&events, second).is_none() {
            return Err(format!(
                "positional append arguments look reversed. Use:\n  tasktree append --id {} \"{}\"",
                first,
                second.replace('"', "\\\"")
            ));
        }
    }

    let target_count = [req.new, req.explicit_id.is_some(), req.legacy_id.is_some()]
        .iter()
        .filter(|&&x| x)
        .count();

    if target_count > 1 {
        return Err("choose only one target: --new, --id, or positional strand id".to_string());
    }

    if req.legacy_id.is_some() && source_kind != "positional" {
        return Err(
            "warn: stdin and --file require --id to specify target; positional strand id is not supported with this content source".to_string()
        );
    }

    if req.new {
        let (created, appended) = event::make_strand_created(&stored, Some("session"));
        let new_id = created.strand_id().to_string();
        with_journal_write_lock(|journal| {
            append_event_unlocked(journal, &created)?;
            append_event_unlocked(journal, &appended)?;
            Ok(())
        })?;
        return Ok(AppendOutcome {
            kind: AppendOutcomeKind::CreatedNew,
            strand_id: new_id.clone(),
            append_id: None,
            stored_content: stored,
            provenance: None,
            seen_offset: req.seen_offset,
            seen_warning: None,
            marker_warning: None,
            closing_marker_warning: false,
            card_state: strand_card_fresh_with_state(&new_id),
        });
    }

    let target_id = req.explicit_id.or(req.legacy_id);
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
    let seen_warning =
        diagnostics::check_w076_seen_offset(&full_id, req.seen_offset, strand_last_offset);
    let provenance = parse_provenance_arg(req.provenance_raw)?;
    let pinned_ref: Option<String> = match req.why {
        Some(w) => {
            let tgt = find_strand(&events, w)
                .ok_or_else(|| format!("--why target strand {} not found", w))?;
            let tgt_offset = projection::project_strands(&events, true)
                .iter()
                .find(|s| s.id == tgt)
                .map(|s| s.last_offset())
                .unwrap_or(0);
            Some(format!("{}@{}", tgt, tgt_offset))
        }
        None => None,
    };
    let event = event::make_log_appended_with_ref(
        &full_id,
        &stored,
        pinned_ref.as_deref(),
        provenance.clone(),
    );
    let append_id = match &event {
        Event::LogAppended { append_id, .. } => append_id.clone(),
        _ => None,
    };
    with_journal_write_lock(|journal| append_event_unlocked(journal, &event))?;

    let marker_warning = possible_marker_warning(&stored);
    let closing_marker_warning = is_closing_annotation_marker(&stored);
    let card_state = strand_card_fresh_with_state(&full_id);

    Ok(AppendOutcome {
        kind: AppendOutcomeKind::AppendedExisting,
        strand_id: full_id,
        append_id,
        stored_content: stored,
        provenance,
        seen_offset: req.seen_offset,
        seen_warning,
        marker_warning,
        closing_marker_warning,
        card_state,
    })
}

fn possible_marker_warning(stored: &str) -> Option<AppendMarkerWarning> {
    let trimmed = stored.trim_start();
    if !trimmed.starts_with('[') {
        return None;
    }
    let end = trimmed.find(']')?;
    let marker = &trimmed[..=end];
    if is_known_marker_str(marker) {
        return None;
    }
    suggest_marker(marker).map(|suggestion| AppendMarkerWarning {
        marker: marker.to_string(),
        suggestion,
    })
}

fn render_append_outcome(outcome: &AppendOutcome, format: Option<&str>) {
    if outcome.kind == AppendOutcomeKind::CreatedNew {
        println!("{}", outcome.strand_id);
        if let Some((card, state)) = &outcome.card_state {
            print_card_with_state(card, state);
        }
        return;
    }

    if let Some(warning) = &outcome.marker_warning {
        eprintln!(
            "W073: unknown marker {} — did you mean {}? (tasktree explain markers)",
            warning.marker, warning.suggestion
        );
    }

    if outcome.closing_marker_warning {
        eprintln!(
            "W074: [done]/[failed]/[cancelled]/[merged]/[verified] are annotations — \
            they no longer close the strand. Use: tasktree close --id {} (tasktree explain W074)",
            shorten(&outcome.strand_id)
        );
    }

    if let Some(w) = &outcome.seen_warning {
        eprintln!("{}: {} (tasktree explain {})", w.code, w.detail, w.code);
    }

    if format == Some("json") {
        let warnings: Vec<output::SeenOffsetWarningOutput<'_>> = outcome
            .seen_warning
            .iter()
            .map(output::SeenOffsetWarningOutput::from)
            .collect();
        let output = output::AppendOutput {
            strand_id: &outcome.strand_id,
            append_id: &outcome.append_id,
            content_preview: outcome.stored_content.chars().take(120).collect::<String>(),
            provenance: &outcome.provenance,
            seen_offset: outcome.seen_offset,
            seen_gap: outcome.seen_warning.as_ref().map(|w| w.seen_gap),
            warnings,
            result: outcome.card_state.as_ref().map(|(card, _)| card.clone()),
        };
        println!("{}", serde_json::to_string(&output).unwrap());
    } else {
        let prod = outcome
            .provenance
            .as_ref()
            .and_then(|p| p.get("producer"))
            .and_then(|v| v.as_str())
            .map(|p| format!(" producer={}", p))
            .unwrap_or_default();
        if let Some((card, state)) = &outcome.card_state {
            println!(
                "appended to {} (offset {}){}",
                shorten(&outcome.strand_id),
                card.last_offset,
                prod
            );
            print_card_with_state(card, state);
        } else {
            println!("appended to {}{}", shorten(&outcome.strand_id), prod);
        }
    }
}

/// Close a strand by writing a StrandClosed lifecycle event.
/// `disposition` defaults to "done" when not specified.
pub(crate) fn cmd_close(
    id: &str,
    disposition: Option<&str>,
    format_json: bool,
) -> Result<(), String> {
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
    let (current_state, _, _) = projection::compute_state_from_events(&events, &strand_id);
    if current_state.starts_with("closed:") {
        return Err(format!(
            "strand {} is already {}; use reopen first",
            shorten(&strand_id),
            current_state
        ));
    }
    let close_event = event::make_strand_closed(&strand_id, disp, None);
    with_journal_write_lock(|journal| append_event_unlocked(journal, &close_event))?;
    if format_json {
        let output = output::LifecycleOutput {
            strand_id: strand_id.clone(),
            disposition: Some(disp.to_string()),
            lifecycle: format!("closed:{}", disp),
            status: "ok",
            result: strand_card_fresh(&strand_id),
        };
        println!("{}", serde_json::to_string(&output).unwrap());
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
    let (current_state, _, _) = projection::compute_state_from_events(&events, &strand_id);
    if current_state == "registered" {
        return Err(format!(
            "strand {} is already open (registered); nothing to reopen",
            shorten(&strand_id)
        ));
    }
    let reopen_event = event::make_strand_reopened(&strand_id, None);
    with_journal_write_lock(|journal| append_event_unlocked(journal, &reopen_event))?;
    if format_json {
        let output = output::LifecycleOutput {
            strand_id: strand_id.clone(),
            disposition: None,
            lifecycle: "registered".to_string(),
            status: "ok",
            result: strand_card_fresh(&strand_id),
        };
        println!("{}", serde_json::to_string(&output).unwrap());
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
    let output = output::CheckpointErrorOutput {
        ok: false,
        error: &failure.message,
        requested_strand: &failure.requested_strand,
        resolved_strand: &failure.resolved_strand,
        journal_appended: failure.journal_appended,
    };
    println!("{}", serde_json::to_string(&output).unwrap());
}

pub(crate) fn resolve_most_recent_strand(
    strands: &[projection::ProjectedStrand],
) -> Option<&projection::ProjectedStrand> {
    let mut sorted: Vec<_> = strands.iter().collect();
    sorted.sort_by(|a, b| b.last_ts().cmp(a.last_ts()));
    sorted.into_iter().next()
}

pub(crate) fn escape_checkpoint_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(crate) struct CheckpointRequest<'a> {
    pub(crate) requested_id: Option<&'a str>,
    pub(crate) action: &'a str,
    pub(crate) tail: Option<usize>,
    pub(crate) include_hidden: bool,
    pub(crate) provenance_raw: Option<&'a str>,
    pub(crate) seen_offset: Option<usize>,
}

#[derive(Debug, Clone)]
pub(crate) struct CheckpointShownEntry {
    pub(crate) ts: String,
    pub(crate) content: String,
    pub(crate) append_id: Option<String>,
}

#[derive(Debug)]
pub(crate) struct CheckpointPlan {
    pub(crate) requested_strand: Option<String>,
    pub(crate) strand_id: String,
    pub(crate) strand_state: String,
    pub(crate) resolved_by: &'static str,
    pub(crate) action: String,
    pub(crate) event: Event,
    pub(crate) append_id: Option<String>,
    pub(crate) observed_entries_before_append: usize,
    pub(crate) shown_entries: Vec<CheckpointShownEntry>,
    pub(crate) staleness_seconds: Option<i64>,
    pub(crate) journal_delta: usize,
    pub(crate) strand_last_offset: usize,
    pub(crate) max_offset_before: usize,
    pub(crate) seen_offset: Option<usize>,
    pub(crate) seen_gap: Option<usize>,
    pub(crate) catch_up: Option<String>,
    pub(crate) warnings: Vec<output::CheckpointWarningOutput>,
    pub(crate) warning_lines: Vec<(&'static str, String)>,
    pub(crate) diagnostics_count: usize,
}

pub(crate) struct CheckpointOutcome {
    pub(crate) plan: CheckpointPlan,
    pub(crate) card: Option<OrientStrand>,
}

fn checkpoint_failure(
    code: i32,
    message: String,
    requested_strand: Option<String>,
    resolved_strand: Option<String>,
    journal_appended: bool,
) -> CheckpointFailure {
    CheckpointFailure {
        code,
        message,
        requested_strand,
        resolved_strand,
        journal_appended,
    }
}

pub(crate) fn plan_checkpoint(
    events: &[(usize, Event)],
    req: CheckpointRequest<'_>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<CheckpointPlan, CheckpointFailure> {
    if req.action.trim().is_empty() {
        return Err(checkpoint_failure(
            3,
            "invalid arguments: --action cannot be empty".to_string(),
            req.requested_id.map(str::to_string),
            None,
            false,
        ));
    }

    let all_strands = projection::project_strands(events, true);
    let visible_strands = projection::project_strands(events, req.include_hidden);

    let (strand, resolved_by) = if let Some(id) = req.requested_id {
        let full = find_strand(events, id).ok_or_else(|| {
            checkpoint_failure(
                1,
                format!("strand resolve/show failed: strand {} not found", id),
                Some(id.to_string()),
                None,
                false,
            )
        })?;
        let strand = all_strands.iter().find(|s| s.id == full).ok_or_else(|| {
            checkpoint_failure(
                1,
                format!("strand resolve/show failed: strand {} not found", id),
                Some(id.to_string()),
                None,
                false,
            )
        })?;
        (strand, "explicit --id")
    } else {
        let strand = resolve_most_recent_strand(&visible_strands).ok_or_else(|| {
            checkpoint_failure(
                1,
                "strand resolve/show failed: no strands found".to_string(),
                None,
                None,
                false,
            )
        })?;
        (strand, "most_recent_active_strand")
    };

    let strand_last_offset = strand.last_offset();
    let max_offset_before = events.last().map(|(o, _)| *o).unwrap_or(0);
    let journal_delta = max_offset_before.saturating_sub(strand_last_offset);
    let staleness_seconds = if strand.last_ts().is_empty() {
        None
    } else {
        parse_event_ts(strand.last_ts()).map(|ts| (now - ts).num_seconds())
    };

    let provenance_val = parse_provenance_arg(req.provenance_raw).map_err(|message| {
        checkpoint_failure(
            3,
            message,
            req.requested_id.map(str::to_string),
            Some(strand.id.clone()),
            false,
        )
    })?;
    let checkpoint_producer = provenance_val
        .as_ref()
        .and_then(|p| p.get("producer"))
        .and_then(|v| v.as_str());
    let w070 = diagnostics::check_w070_strand_moved(events, &strand.id, checkpoint_producer);
    let w071 = diagnostics::check_w071_closed_strand(strand);
    let w076 = diagnostics::check_w076_seen_offset(&strand.id, req.seen_offset, strand_last_offset);

    let observed_entries_before_append = strand.log_count();
    let escaped_action = escape_checkpoint_value(req.action);
    let content = format!(
        "[checkpoint] ok resolved_by=\"{}\" observed_entries_before_append={} action=\"{}\"",
        resolved_by, observed_entries_before_append, escaped_action
    );
    let event = event::make_log_appended(&strand.id, &content, provenance_val);
    let append_id = match &event {
        Event::LogAppended { append_id, .. } => append_id.clone(),
        _ => None,
    };

    let shown_entries: Vec<CheckpointShownEntry> = if let Some(n) = req.tail {
        let skip = strand.log.len().saturating_sub(n);
        strand.log[skip..].iter().collect::<Vec<_>>()
    } else {
        strand.log.iter().collect::<Vec<_>>()
    }
    .into_iter()
    .map(|entry| CheckpointShownEntry {
        ts: entry.ts.clone(),
        content: entry.content.clone(),
        append_id: entry.append_id.clone(),
    })
    .collect();

    let raw_events: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
    let diagnostics_count = diagnostics::run_journal_diagnostics(&raw_events, now).len();

    let mut warnings = Vec::new();
    let mut warning_lines = Vec::new();
    if let Some((code, detail)) = w070 {
        warnings.push(output::CheckpointWarningOutput {
            code: code.to_string(),
            detail: detail.clone(),
            seen_offset: None,
            strand_last_offset: None,
            seen_gap: None,
            catch_up: None,
        });
        warning_lines.push((code, detail));
    }
    if let Some((code, detail)) = w071 {
        warnings.push(output::CheckpointWarningOutput {
            code: code.to_string(),
            detail: detail.clone(),
            seen_offset: None,
            strand_last_offset: None,
            seen_gap: None,
            catch_up: None,
        });
        warning_lines.push((code, detail));
    }
    if let Some(w) = &w076 {
        warnings.push(output::CheckpointWarningOutput {
            code: w.code.to_string(),
            detail: w.detail.clone(),
            seen_offset: Some(w.seen_offset),
            strand_last_offset: Some(w.strand_last_offset),
            seen_gap: Some(w.seen_gap),
            catch_up: Some(w.catch_up.clone()),
        });
        warning_lines.push((w.code, w.detail.clone()));
    }

    let catch_up = if journal_delta > 0 {
        Some(format!(
            "tasktree timeline --since-offset {} --links {}",
            strand_last_offset,
            shorten(&strand.id)
        ))
    } else {
        None
    };

    Ok(CheckpointPlan {
        requested_strand: req.requested_id.map(str::to_string),
        strand_id: strand.id.clone(),
        strand_state: strand.state().to_string(),
        resolved_by,
        action: req.action.to_string(),
        event,
        append_id,
        observed_entries_before_append,
        shown_entries,
        staleness_seconds,
        journal_delta,
        strand_last_offset,
        max_offset_before,
        seen_offset: req.seen_offset,
        seen_gap: w076.as_ref().map(|w| w.seen_gap),
        catch_up,
        warnings,
        warning_lines,
        diagnostics_count,
    })
}

pub(crate) fn commit_checkpoint(plan: &CheckpointPlan) -> Result<(), CheckpointFailure> {
    with_journal_write_lock(|journal| append_event_unlocked(journal, &plan.event)).map_err(|e| {
        checkpoint_failure(
            2,
            format!("journal append failed: {}", e),
            plan.requested_strand.clone(),
            Some(plan.strand_id.clone()),
            false,
        )
    })
}

pub(crate) fn checkpoint_outcome(plan: CheckpointPlan) -> CheckpointOutcome {
    let card = strand_card_fresh(&plan.strand_id);
    CheckpointOutcome { plan, card }
}

pub(crate) fn render_checkpoint_outcome(outcome: &CheckpointOutcome, format_json: bool) {
    let plan = &outcome.plan;
    if format_json {
        let output = output::CheckpointOutput {
            ok: true,
            strand: shorten(&plan.strand_id),
            resolved_strand: &plan.strand_id,
            resolved_by: plan.resolved_by,
            observed_entries_before_append: plan.observed_entries_before_append,
            shown_entries: plan.shown_entries.len(),
            action: &plan.action,
            append_id: &plan.append_id,
            journal_appended: true,
            diagnostics_count: plan.diagnostics_count,
            result: outcome.card.clone(),
            staleness_seconds: plan.staleness_seconds,
            journal_delta: plan.journal_delta,
            seen_offset: plan.seen_offset,
            seen_gap: plan.seen_gap,
            catch_up: plan.catch_up.as_deref(),
            warnings: &plan.warnings,
        };
        println!("{}", serde_json::to_string(&output).unwrap());
        return;
    }

    println!("checkpoint ok");
    println!(
        "  strand: {} | {} entries | {}",
        shorten(&plan.strand_id),
        plan.observed_entries_before_append + 1,
        plan.strand_state
    );
    println!("  resolved_by: {}", plan.resolved_by);

    let staleness_part = plan
        .staleness_seconds
        .map(|s| {
            let d = humanize_duration(s);
            if d == "just now" {
                "last touched just now | ".to_string()
            } else {
                format!("last touched {} ago | ", d)
            }
        })
        .unwrap_or_default();
    println!(
        "  staleness: {}journal +{} entries since (offset {} → {})",
        staleness_part, plan.journal_delta, plan.strand_last_offset, plan.max_offset_before
    );

    if let Some(catch_up) = &plan.catch_up {
        println!("  catch-up: {}", catch_up);
    }

    println!(
        "  observed_entries_before_append: {}",
        plan.observed_entries_before_append
    );
    println!("  action: {}", plan.action);
    if let Some(id) = &plan.append_id {
        println!("  append_id: {}", id);
    }
    println!("  appended to journal");
    println!("log:");
    for entry in &plan.shown_entries {
        let id_str = entry
            .append_id
            .as_ref()
            .map(|a| format!(" [{}]", &a[..12]))
            .unwrap_or_default();
        println!("  [{}]{} {}", &entry.ts[..19], id_str, entry.content);
    }
    for (code, detail) in &plan.warning_lines {
        println!("  {} {}  (tasktree explain {})", code, detail, code);
    }
    if plan.diagnostics_count > 0 {
        println!(
            "diagnostics: {} warning(s) — run tasktree doctor journal",
            plan.diagnostics_count
        );
    }
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
    let path = ensure_journal().map_err(|e| {
        checkpoint_failure(
            1,
            format!("strand resolve/show failed: {}", e),
            requested_id.map(str::to_string),
            None,
            false,
        )
    })?;
    let events = read_events_strict(&path).map_err(|e| {
        checkpoint_failure(
            1,
            format!("strand resolve/show failed: {}", e),
            requested_id.map(str::to_string),
            None,
            false,
        )
    })?;
    let request = CheckpointRequest {
        requested_id,
        action,
        tail,
        include_hidden,
        provenance_raw,
        seen_offset,
    };
    let plan = plan_checkpoint(&events, request, chrono::Utc::now())?;
    commit_checkpoint(&plan)?;
    let outcome = checkpoint_outcome(plan);
    render_checkpoint_outcome(&outcome, format_json);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_warning_classification_is_adapter_independent() {
        let marker =
            possible_marker_warning("[freiction] typo marker").expect("typo marker warning");
        assert_eq!(marker.marker, "[freiction]");
        assert_eq!(marker.suggestion, "[friction]");
        assert!(possible_marker_warning("[friction] exact marker").is_none());
        assert!(crate::markers::is_closing_annotation_marker(
            "[done] annotated close"
        ));
    }
}

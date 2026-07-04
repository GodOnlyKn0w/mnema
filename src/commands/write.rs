/// Write-command family: cmd_add, cmd_append, cmd_close, cmd_reopen, cmd_checkpoint.
/// Moved from main.rs (Layer 4a refactor); function bodies are byte-identical to
/// the originals (only cross-module path qualification added where required).
///
/// Dependency direction: write → journal, event, projection, diagnostics, render (via crate::*)
/// write ← main.rs (mod commands; pub(crate) use commands::write::*)
use crate::diagnostics;
use crate::event::{self, Event, find_strand, resolve_id};
use crate::journal::{
    JournalEntryAppendRequest, append_entry_to_strand, append_entry_to_strand_checked,
    append_events, ensure_journal, read_events_lossy, read_events_strict,
};
use crate::markers::{
    is_closing_annotation_marker, is_known_marker_str, suggest_marker, validate_lifecycle_marker,
};
use crate::output::{self, OrientStrand};
use crate::projection;
use crate::util::{
    humanize_duration, parse_event_ts, parse_provenance_arg, read_file_content, read_stdin_content,
    shorten,
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

#[cfg(test)]
pub(crate) fn cmd_add(
    content: Option<&str>,
    stdin: bool,
    file: Option<&str>,
    format_json: bool,
    strand_type: Option<&str>,
    provenance_raw: Option<&str>,
) -> Result<(), String> {
    cmd_add_with_parent(
        content,
        stdin,
        file,
        format_json,
        None,
        None,
        strand_type,
        provenance_raw,
    )
}

/// Resolve a `--why`/`--from` rationale REF to the cited entry hash.
///
/// Resolution order: a strand id/prefix pins the target line's *latest*
/// entry (shorthand for "that line's current conclusion"); anything else
/// resolves as an entry-hash prefix and pins that exact entry. Staleness
/// needs no stored pin: journal offsets are globally monotonic, so
/// ref-target-advanced derives it from positions alone.
fn resolve_rationale_ref(
    events: &[(usize, Event)],
    all_strands: &[projection::ProjectedStrand],
    input: &str,
) -> Result<String, String> {
    if let Some(tgt) = find_strand(events, input) {
        let target_basis = all_strands
            .iter()
            .find(|s| s.id == tgt)
            .ok_or_else(|| format!("rationale target strand {} not found", input))?;
        return target_basis
            .log
            .last()
            .and_then(|entry| entry.entry_id.clone())
            .ok_or_else(|| format!("rationale target strand {} has no entry hash", input));
    }
    match projection::find_entry(all_strands, input) {
        projection::EntryLookup::One { entry, .. } => Ok(entry
            .entry_id
            .clone()
            .expect("find_entry only matches entries with ids")),
        projection::EntryLookup::None => Err(format!(
            "rationale target {} matches no strand or entry",
            input
        )),
        projection::EntryLookup::Ambiguous(candidates) => {
            let sample: Vec<String> = candidates.iter().take(4).map(|c| shorten(c)).collect();
            Err(format!(
                "rationale prefix {} is ambiguous: {} entries match (e.g. {})",
                input,
                candidates.len(),
                sample.join(", ")
            ))
        }
    }
}

pub(crate) fn cmd_add_from_stdin(
    format_json: bool,
    parent: Option<&str>,
    from: Option<&str>,
    strand_type: Option<&str>,
    provenance_raw: Option<&str>,
) -> Result<(), String> {
    let raw = read_stdin_content()?;
    cmd_add_with_parent(
        Some(&raw),
        false,
        None,
        format_json,
        parent,
        from,
        strand_type,
        provenance_raw,
    )
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn cmd_add_with_parent(
    content: Option<&str>,
    stdin: bool,
    file: Option<&str>,
    format_json: bool,
    parent: Option<&str>,
    from: Option<&str>,
    strand_type: Option<&str>,
    provenance_raw: Option<&str>,
) -> Result<(), String> {
    // CLI v2 carries entry content on stdin. The content/file branches are kept
    // for unit tests and internal callers that bypass clap.
    let (raw, source_kind) = match (content, stdin, file) {
        (None, false, None) | (None, true, None) => (read_stdin_content()?, "stdin"),
        (Some(c), false, None) => (c.to_string(), "direct"),
        (None, false, Some(path)) => (read_file_content(path)?, "file"),
        _ => {
            let mut sources = Vec::new();
            if content.is_some() {
                sources.push("direct content");
            }
            if stdin {
                sources.push("stdin");
            }
            if file.is_some() {
                sources.push("file");
            }
            return Err(format!(
                "entry content is read from exactly one stdin stream; got: {}",
                sources.join(", ")
            ));
        }
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
    // --parent (belongs-to) and --from (source ref) are orthogonal: a child
    // can carry either, both, or neither (CORPUS §6 归属与来源正交).
    let needs_events = parent.is_some() || from.is_some();
    let events = if needs_events {
        let path = ensure_journal()?;
        read_events_lossy(&path).0
    } else {
        Vec::new()
    };
    let parent_id = if let Some(parent_raw) = parent {
        let parent_raw = parent_raw.trim();
        if parent_raw.is_empty() {
            return Err("--parent cannot be empty".to_string());
        }
        Some(
            find_strand(&events, parent_raw)
                .ok_or_else(|| format!("parent strand {} not found", parent_raw))?,
        )
    } else {
        None
    };
    let from_refs = if let Some(from_raw) = from {
        let from_raw = from_raw.trim();
        if from_raw.is_empty() {
            return Err("--from cannot be empty".to_string());
        }
        let all_strands = projection::project_strands(&events, true);
        vec![resolve_rationale_ref(&events, &all_strands, from_raw)?]
    } else {
        Vec::new()
    };

    let (created, appended) = event::make_strand_created_with_refs(
        &stored,
        resolved_type,
        from_refs,
        None,
        provenance.clone(),
    );
    let id = created
        .strand_id()
        .expect("strand-scoped event")
        .to_string();
    let first_entry_id = match &appended {
        Event::LogAppended { entry_id, .. } => entry_id.clone(),
        _ => None,
    };
    let mut events_to_append = vec![created, appended];
    if let Some(parent_id) = &parent_id {
        events_to_append.push(event::make_edge_linked(
            &id,
            first_entry_id.as_deref(),
            parent_id,
            Some("belongs-to"),
            provenance.clone(),
        ));
    }
    append_events(&events_to_append)?;
    if format_json {
        let output = output::AddOutput {
            id: id.clone(),
            status: "ok",
            provenance: provenance.as_ref(),
            parent_id: parent_id.clone(),
            edge_type: parent_id.as_ref().map(|_| "belongs-to"),
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
    pub(crate) entry_id: Option<String>,
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

pub(crate) fn cmd_append_from_stdin(
    new: bool,
    explicit_id: Option<&str>,
    format: Option<&str>,
    provenance_raw: Option<&str>,
    seen_offset: Option<usize>,
    why: Option<&str>,
) -> Result<(), String> {
    let raw = read_stdin_content()?;
    cmd_append_with_seen_offset(
        Some(&raw),
        None,
        new,
        false,
        None,
        explicit_id,
        format,
        provenance_raw,
        seen_offset,
        why,
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
    if req.legacy_id.is_some() {
        return Err("legacy positional strand id was removed; use --id <ID>".to_string());
    }

    // CLI v2 carries entry content on stdin. The content/file branches are kept
    // for unit tests and internal callers that bypass clap.
    let (raw, source_kind) = match (req.content, req.stdin, req.file) {
        (None, false, None) | (None, true, None) => (read_stdin_content()?, "stdin"),
        (Some(c), false, None) => (c.to_string(), "direct"),
        (None, false, Some(path)) => (read_file_content(path)?, "file"),
        _ => {
            let mut sources = Vec::new();
            if req.content.is_some() {
                sources.push("direct content");
            }
            if req.stdin {
                sources.push("stdin");
            }
            if req.file.is_some() {
                sources.push("file");
            }
            return Err(format!(
                "entry content is read from exactly one stdin stream; got: {}",
                sources.join(", ")
            ));
        }
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

    if req.explicit_id.map_or(false, |id| id.trim().is_empty()) {
        return Err("explicit --id cannot be empty".to_string());
    }

    let target_count = [req.new, req.explicit_id.is_some()]
        .iter()
        .filter(|&&x| x)
        .count();

    if target_count > 1 {
        return Err("choose only one target: --new or --id".to_string());
    }

    let provenance = parse_provenance_arg(req.provenance_raw)?;

    if req.new {
        let (created, appended) = event::make_strand_created_with_provenance(
            &stored,
            Some("session"),
            provenance.clone(),
        );
        let new_id = created
            .strand_id()
            .expect("strand-scoped event")
            .to_string();
        let entry_id = match &appended {
            Event::LogAppended { entry_id, .. } => entry_id.clone(),
            _ => None,
        };
        append_events(&[created, appended])?;
        return Ok(AppendOutcome {
            kind: AppendOutcomeKind::CreatedNew,
            strand_id: new_id.clone(),
            entry_id,
            stored_content: stored,
            provenance: provenance.clone(),
            seen_offset: req.seen_offset,
            seen_warning: None,
            marker_warning: None,
            closing_marker_warning: false,
            card_state: strand_card_fresh_with_state(&new_id),
        });
    }

    let all_strands = projection::project_strands(&events, true);
    let target_id = req.explicit_id.or(req.legacy_id);
    let full_id = if let Some(id) = target_id {
        find_strand(&events, id).ok_or_else(|| {
            let mut msg = format!("strand {} not found", id);
            if id == "-" {
                msg.push_str(
                    ". If you meant to pipe content from stdin, use:\n  echo \"...\" | tasktree append --id <id>",
                );
            }
            msg
        })?
    } else {
        let visible_strands = projection::project_strands(&events, false);
        let mut sorted: Vec<_> = visible_strands.iter().collect();
        sorted.sort_by(|a, b| b.last_ts().cmp(&a.last_ts()));
        let recent = sorted
            .first()
            .ok_or("no strands found — use 'add' or 'append --new' first")?;
        recent.id.clone()
    };

    let target_strand = all_strands
        .iter()
        .find(|s| s.id == full_id)
        .ok_or_else(|| format!("strand {} not found", full_id))?;
    let strand_last_offset = target_strand.last_offset();

    let seen_warning =
        diagnostics::check_w076_seen_offset(&full_id, req.seen_offset, strand_last_offset);
    let mut refs: Vec<String> = Vec::new();
    if let Some(w) = req.why {
        refs.push(resolve_rationale_ref(&events, &all_strands, w)?);
    }
    let appended = append_entry_to_strand(JournalEntryAppendRequest {
        strand_id: full_id.clone(),
        content: stored.clone(),
        refs,
        legacy_ref: None,
        effect: None,
        provenance: provenance.clone(),
    })?;
    let entry_id = appended.entry_id;

    let marker_warning = possible_marker_warning(&stored);
    let closing_marker_warning = is_closing_annotation_marker(&stored);
    let card_state = strand_card_fresh_with_state(&full_id);

    Ok(AppendOutcome {
        kind: AppendOutcomeKind::AppendedExisting,
        strand_id: full_id,
        entry_id,
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
    if outcome.kind == AppendOutcomeKind::CreatedNew && format != Some("json") {
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
            entry_id: &outcome.entry_id,
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

/// Close a strand by writing a close effect entry.
/// `disposition` defaults to "done" when not specified.
pub(crate) fn cmd_close(
    id: &str,
    disposition: Option<&str>,
    reason: Option<&str>,
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
    let validate_id = strand_id.clone();
    let (content, effect) = event::close_entry_parts(disp, reason);
    append_entry_to_strand_checked(
        JournalEntryAppendRequest {
            strand_id: strand_id.clone(),
            content,
            refs: Vec::new(),
            legacy_ref: None,
            effect: Some(effect),
            provenance: None,
        },
        move |events| {
            let (current_state, _, _) = projection::compute_state_from_events(events, &validate_id);
            if current_state.starts_with("closed:") {
                return Err(format!(
                    "strand {} is already {}; use reopen first",
                    shorten(&validate_id),
                    current_state
                ));
            }
            Ok(())
        },
    )?;
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

/// Reopen a closed strand by writing a reopen effect entry.
pub(crate) fn cmd_reopen(
    id: &str,
    reason: Option<&str>,
    format_json: bool,
) -> Result<(), String> {
    let strand_id = resolve_id(&read_events_strict(&ensure_journal()?)?, id)?;
    let validate_id = strand_id.clone();
    let (content, effect) = event::reopen_entry_parts(reason);
    append_entry_to_strand_checked(
        JournalEntryAppendRequest {
            strand_id: strand_id.clone(),
            content,
            refs: Vec::new(),
            legacy_ref: None,
            effect: Some(effect),
            provenance: None,
        },
        move |events| {
            let (current_state, _, _) = projection::compute_state_from_events(events, &validate_id);
            if current_state == "registered" {
                return Err(format!(
                    "strand {} is already open (registered); nothing to reopen",
                    shorten(&validate_id)
                ));
            }
            Ok(())
        },
    )?;
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
    pub(crate) entry_id: Option<String>,
}

#[derive(Debug)]
pub(crate) struct CheckpointPlan {
    pub(crate) requested_strand: Option<String>,
    pub(crate) strand_id: String,
    pub(crate) strand_state: String,
    pub(crate) resolved_by: &'static str,
    pub(crate) action: String,
    pub(crate) event: Event,
    pub(crate) entry_id: Option<String>,
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
    let prev_entry_id = strand
        .log
        .last()
        .and_then(|entry| entry.entry_id.as_deref());
    let event = event::make_log_appended_entry(
        &strand.id,
        prev_entry_id,
        &content,
        Vec::new(),
        None,
        provenance_val,
    );
    let entry_id = match &event {
        Event::LogAppended { entry_id, .. } => entry_id.clone(),
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
        entry_id: entry.entry_id.clone(),
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
        entry_id,
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

pub(crate) fn commit_checkpoint(plan: &mut CheckpointPlan) -> Result<(), CheckpointFailure> {
    let Event::LogAppended {
        content,
        refs,
        ref_,
        effect,
        provenance,
        prev_entry_id,
        ..
    } = &plan.event
    else {
        return Err(checkpoint_failure(
            2,
            "journal append failed: checkpoint plan did not contain a log entry".to_string(),
            plan.requested_strand.clone(),
            Some(plan.strand_id.clone()),
            false,
        ));
    };
    let planned_prev_entry_id = prev_entry_id.clone();
    let planned_max_offset = plan.max_offset_before;
    let validate_id = plan.strand_id.clone();
    let appended = append_entry_to_strand_checked(
        JournalEntryAppendRequest {
            strand_id: plan.strand_id.clone(),
            content: content.clone(),
            refs: refs.clone(),
            legacy_ref: ref_.clone(),
            effect: effect.clone(),
            provenance: provenance.clone(),
        },
        move |events| {
            let current_max_offset = events.last().map(|(offset, _)| *offset).unwrap_or(0);
            if current_max_offset != planned_max_offset {
                return Err(format!(
                    "checkpoint plan is stale: journal advanced from offset {} to {}; rerun checkpoint",
                    planned_max_offset, current_max_offset
                ));
            }
            let current_head = crate::journal::current_entry_head(events, &validate_id);
            if current_head != planned_prev_entry_id {
                return Err(format!(
                    "checkpoint plan is stale: strand {} head changed; rerun checkpoint",
                    shorten(&validate_id)
                ));
            }
            Ok(())
        },
    )
    .map_err(|e| {
        checkpoint_failure(
            2,
            format!("journal append failed: {}", e),
            plan.requested_strand.clone(),
            Some(plan.strand_id.clone()),
            false,
        )
    })?;
    plan.event = appended.event;
    plan.entry_id = appended.entry_id;
    Ok(())
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
            entry_id: &plan.entry_id,
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
    if let Some(id) = &plan.entry_id {
        println!("  entry_id: {}", id);
    }
    println!("  appended to journal");
    println!("log:");
    for entry in &plan.shown_entries {
        let id_str = entry
            .entry_id
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
    let mut plan = plan_checkpoint(&events, request, chrono::Utc::now())?;
    commit_checkpoint(&mut plan)?;
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

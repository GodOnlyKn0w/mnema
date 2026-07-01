/// Write-command family: cmd_add, cmd_append, cmd_close, cmd_reopen.
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
    looks_like_strand_id, parse_provenance_arg, read_file_content, read_stdin_content, shorten,
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

/// Parsed inputs for `tasktree add`, sitting between CLI grammar and the
/// command workflow (see ARCHITECTURE.md Conventions).
#[derive(Default)]
pub(crate) struct AddRequest<'a> {
    pub(crate) content: Option<&'a str>,
    pub(crate) stdin: bool,
    pub(crate) file: Option<&'a str>,
    pub(crate) format_json: bool,
    pub(crate) parent: Option<&'a str>,
    pub(crate) strand_type: Option<&'a str>,
    pub(crate) provenance_raw: Option<&'a str>,
}

pub(crate) fn cmd_add(req: AddRequest<'_>) -> Result<(), String> {
    let AddRequest {
        content,
        stdin,
        file,
        format_json,
        parent,
        strand_type,
        provenance_raw,
    } = req;
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
    let parent_id = if let Some(parent_raw) = parent {
        let parent_raw = parent_raw.trim();
        if parent_raw.is_empty() {
            return Err("--parent cannot be empty".to_string());
        }
        let path = ensure_journal()?;
        let (events, _) = read_events_lossy(&path);
        Some(
            find_strand(&events, parent_raw)
                .ok_or_else(|| format!("parent strand {} not found", parent_raw))?,
        )
    } else {
        None
    };

    // acquire lock once, write all events atomically
    let result = with_journal_write_lock(|journal| {
        let (created, appended) =
            event::make_strand_created(&stored, resolved_type, provenance.clone());
        let id = created.strand_id().to_string();
        append_event_unlocked(journal, &created)?;
        append_event_unlocked(journal, &appended)?;
        if let Some(parent_id) = &parent_id {
            let edge =
                event::make_edge_linked(&id, parent_id, Some("belongs-to"), provenance.clone());
            append_event_unlocked(journal, &edge)?;
        }
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
    pub(crate) stdin: bool,
    pub(crate) file: Option<&'a str>,
    pub(crate) explicit_id: Option<&'a str>,
    pub(crate) provenance_raw: Option<&'a str>,
    pub(crate) seen_offset: Option<usize>,
    pub(crate) why: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppendMarkerWarning {
    pub(crate) marker: String,
    pub(crate) suggestion: &'static str,
}

#[derive(Debug, Clone)]
pub(crate) struct AppendOutcome {
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
    stdin: bool,
    file: Option<&str>,
    explicit_id: Option<&str>,
    format: Option<&str>,
    provenance_raw: Option<&str>,
) -> Result<(), String> {
    cmd_append_with_seen_offset(
        content,
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

    if req.explicit_id.map_or(false, |id| id.trim().is_empty()) {
        return Err("explicit --id cannot be empty".to_string());
    }

    let provenance = parse_provenance_arg(req.provenance_raw)?;

    let full_id = if let Some(id) = req.explicit_id {
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
            .ok_or("no strands found — use 'add' to create a strand first")?;
        recent.id.clone()
    };

    let strand_last_offset = projection::project_strands(&events, true)
        .iter()
        .find(|s| s.id == full_id)
        .map(|s| s.last_offset())
        .unwrap_or(0);
    let seen_warning =
        diagnostics::check_w076_seen_offset(&full_id, req.seen_offset, strand_last_offset);
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

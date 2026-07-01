/// Manage/metadata command family: cmd_link/cmd_unlink, cmd_hide/cmd_unhide,
/// cmd_export.
/// Moved from main.rs (Layer 4d-manage refactor).
use crate::event::{self, resolve_id};
use crate::journal::*;
use crate::output;
use crate::projection;
use crate::util::{parse_provenance_arg, shorten};
use crate::{
    print_handle_line, print_visibility_ledger, strand_card_fresh, strand_card_fresh_with_state,
    visibility_ledger_json,
};
use serde_json::json;
use std::io::Write;
use std::path::PathBuf;

pub(crate) fn cmd_link(
    source: &str,
    target: &str,
    edge_type: Option<&str>,
    format_json: bool,
    provenance_raw: Option<&str>,
) -> Result<(), String> {
    // Default edge type: depends-on
    let etype = edge_type.unwrap_or("depends-on");
    // F2: validate edge_type at the write entrance. Only the two real edges
    // survive (D2: `why` left the edge system → it is now an entry rationale
    // field, not a link). A free-string edge_type silently became an
    // un-projected, un-queryable label sitting in the journal — seal it here so
    // every edge entering the typed projections is clean. ("why" gets a pointed
    // message because it used to be accepted.)
    match etype {
        "belongs-to" | "depends-on" => {}
        "why" => {
            return Err(
                "edge_type 'why' is no longer a link (D2): why is an entry rationale, \
                 not a strand->strand edge. Record the reason in the entry itself."
                    .to_string(),
            );
        }
        other => {
            return Err(format!(
                "unknown edge_type '{}'. Valid edge types: belongs-to, depends-on",
                other
            ));
        }
    }
    let events = read_events_strict(&ensure_journal()?)?;
    let src_id = resolve_id(&events, source)?;
    let tgt_id = resolve_id(&events, target)?;
    let provenance = parse_provenance_arg(provenance_raw)?;
    let event = event::make_edge_linked(&src_id, &tgt_id, Some(etype), provenance);
    with_journal_write_lock(|journal| append_event_unlocked(journal, &event))?;
    if format_json {
        let output = output::LinkOutput {
            source_id: src_id.clone(),
            target_id: tgt_id.clone(),
            edge_type: etype.to_string(),
            status: "ok",
            result: output::LinkResultOutput {
                source: strand_card_fresh(&src_id),
                target: strand_card_fresh(&tgt_id),
            },
        };
        println!("{}", serde_json::to_string(&output).unwrap());
    } else {
        println!(
            "linked {} -> {} ({})",
            shorten(&src_id),
            shorten(&tgt_id),
            etype
        );
        if let Some((card, state)) = strand_card_fresh_with_state(&src_id) {
            print_handle_line(&card, &state);
        }
        if let Some((card, state)) = strand_card_fresh_with_state(&tgt_id) {
            print_handle_line(&card, &state);
        }
        println!("{} --{}--> {}", shorten(&src_id), etype, shorten(&tgt_id));
    }
    Ok(())
}

/// Remove a typed edge (F5). Symmetric with `cmd_link`: validates edge_type,
/// resolves both ids, appends an `EdgeUnlinked` carrying edge_type so the
/// projection's last-write-wins fold drops exactly that edge. Append-only — the
/// original EdgeLinked stays in the journal; only the read projection changes.
pub(crate) fn cmd_unlink(
    source: &str,
    target: &str,
    edge_type: Option<&str>,
    format_json: bool,
    provenance_raw: Option<&str>,
) -> Result<(), String> {
    let etype = edge_type.unwrap_or("depends-on");
    match etype {
        "belongs-to" | "depends-on" => {}
        "why" => {
            return Err(
                "edge_type 'why' is not a link (D2) — there is nothing to unlink.".to_string(),
            );
        }
        other => {
            return Err(format!(
                "unknown edge_type '{}'. Valid edge types: belongs-to, depends-on",
                other
            ));
        }
    }
    let events = read_events_strict(&ensure_journal()?)?;
    let src_id = resolve_id(&events, source)?;
    let tgt_id = resolve_id(&events, target)?;
    let provenance = parse_provenance_arg(provenance_raw)?;
    let event = event::make_edge_unlinked(&src_id, &tgt_id, Some(etype), provenance);
    with_journal_write_lock(|journal| append_event_unlocked(journal, &event))?;
    if format_json {
        let output = output::UnlinkOutput {
            source_id: src_id.clone(),
            target_id: tgt_id.clone(),
            edge_type: etype.to_string(),
            status: "ok",
            unlinked: true,
        };
        println!("{}", serde_json::to_string(&output).unwrap());
    } else {
        println!(
            "unlinked {} -x-> {} ({})",
            shorten(&src_id),
            shorten(&tgt_id),
            etype
        );
    }
    Ok(())
}

/// Hide a strand. Idempotent: if the strand is already hidden (hide_count > 0),
/// no event is written. The current state read and the append happen inside the
/// same journal write lock so concurrent hide/unhide calls are serialised.
///
/// `provenance_raw` is forwarded to the `[hidden] <reason>` LogAppended entry
/// when `reason` is given. Without a reason no content entry is written and
/// the StrandHidden event carries no provenance field (the event schema has none).
pub(crate) fn cmd_hide(
    id: &str,
    reason: Option<&str>,
    format_json: bool,
    provenance_raw: Option<&str>,
) -> Result<(), String> {
    let strand_id = resolve_id(&read_events_strict(&ensure_journal()?)?, id)?;
    let provenance = parse_provenance_arg(provenance_raw)?;
    // Both the read (to compute current state) and the append must be inside
    // the same write lock. Otherwise two concurrent `cmd_hide` calls would each
    // see hide_count=0 and both append a StrandHidden event.
    let outcome = with_journal_write_lock(|journal| {
        // Re-read events under the lock. The journal file is already open
        // for append, so we use a fresh read of the on-disk file via the
        // shared reader for consistency.
        let path = ensure_journal()?;
        let (events, _) = read_events_lossy(&path);
        let current = projection::hide_balance(&events, &strand_id);
        if current > 0 {
            return Ok(false); // already hidden: no-op
        }
        let hide_event = event::make_strand_hidden(&strand_id);
        if let Some(r) = reason {
            let log_event = event::make_log_appended(
                &strand_id,
                &format!("[hidden] {}", r),
                provenance.clone(),
            );
            append_events_unlocked(journal, &[hide_event, log_event])?;
        } else {
            append_event_unlocked(journal, &hide_event)?;
        }
        Ok(true)
    })?;
    if format_json {
        println!("{}", visibility_ledger_json(&strand_id, !outcome));
    } else {
        if outcome {
            println!("hidden {}", shorten(&strand_id));
        } else {
            println!("hidden {} (already hidden, no-op)", shorten(&strand_id));
        }
        // Handle line (abbreviated card) + visibility ledger after both branches.
        if let Some((card, state)) = strand_card_fresh_with_state(&strand_id) {
            print_handle_line(&card, &state);
        }
        print_visibility_ledger();
    }
    Ok(())
}

/// Unhide a strand. Idempotent: if the strand is not hidden (hide_count <= 0),
/// no event is written. The current state read and the append happen inside the
/// same journal write lock so concurrent hide/unhide calls are serialised.
pub(crate) fn cmd_unhide(id: &str, format_json: bool) -> Result<(), String> {
    let strand_id = resolve_id(&read_events_strict(&ensure_journal()?)?, id)?;
    let outcome = with_journal_write_lock(|journal| {
        let path = ensure_journal()?;
        let (events, _) = read_events_lossy(&path);
        let current = projection::hide_balance(&events, &strand_id);
        if current <= 0 {
            return Ok(false); // already visible: no-op
        }
        let event = event::make_strand_unhidden(&strand_id);
        append_event_unlocked(journal, &event)?;
        Ok(true)
    })?;
    if format_json {
        println!("{}", visibility_ledger_json(&strand_id, !outcome));
    } else {
        if outcome {
            println!("unhidden {}", shorten(&strand_id));
        } else {
            println!("unhidden {} (already visible, no-op)", shorten(&strand_id));
        }
        // Handle line + visibility ledger after both branches.
        if let Some((card, state)) = strand_card_fresh_with_state(&strand_id) {
            print_handle_line(&card, &state);
        }
        print_visibility_ledger();
    }
    Ok(())
}

pub(crate) fn cmd_export(out: &str) -> Result<(), String> {
    let journal_path = resolve_journal_dir()?.join("journal.jsonl");

    let out_path = PathBuf::from(out);
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create output directory: {}", e))?;
        }
    }

    let journal_bytes =
        std::fs::read(&journal_path).map_err(|e| format!("cannot read journal: {}", e))?;
    let journal_text = String::from_utf8_lossy(&journal_bytes);
    let line_count = journal_text.lines().count();

    let metadata = json!({
        "type": "export_metadata",
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "journal_lines": line_count,
        "head_at_export": "",
        "source": "tasktree export"
    });

    let mut file = std::fs::File::create(&out_path)
        .map_err(|e| format!("cannot create output file '{}': {}", out, e))?;
    let metadata_line = serde_json::to_string(&metadata)
        .map_err(|e| format!("metadata serialization failed: {}", e))?;
    writeln!(file, "{}", metadata_line)
        .map_err(|e| format!("cannot write metadata to output: {}", e))?;
    file.write_all(&journal_bytes)
        .map_err(|e| format!("cannot write journal to output: {}", e))?;

    let export_lines = line_count + 1;
    println!(
        "Exported {} lines (1 metadata + {} journal) to {}",
        export_lines, line_count, out
    );
    Ok(())
}

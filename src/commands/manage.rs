/// Manage/metadata command family: cmd_find, cmd_link, cmd_hide/cmd_unhide,
/// cmd_export.
/// Moved from main.rs (Layer 4d-manage refactor).
use crate::event::{self, Event, find_strand, resolve_id};
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

pub(crate) fn cmd_find(id: &str, format_json: bool) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _) = read_events_lossy(&path);
    match find_strand(&events, id) {
        Some(full_id) => {
            if format_json {
                println!(
                    "{}",
                    serde_json::to_string(&output::FindOutput { id: full_id }).unwrap()
                );
            } else {
                println!("{}", full_id);
            }
        }
        None => return Err(format!("strand {} not found", id)),
    }
    Ok(())
}

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
    let (content, effect) = event::link_entry_parts(&tgt_id, etype);
    append_entry_to_strand(JournalEntryAppendRequest {
        strand_id: src_id.clone(),
        content,
        refs: Vec::new(),
        legacy_ref: None,
        effect: Some(effect),
        provenance,
    })?;
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
/// resolves both ids, appends an unlink effect entry carrying edge_type so the
/// projection's last-write-wins fold drops exactly that edge. Append-only — the
/// original link entry stays in the journal; only the read projection changes.
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
    // Name the specific Link entry being reversed (CORPUS §4): the most recent
    // *live* Link on the source strand to (target, edge_type). None → a legacy
    // key-tombstone unlink (no live link found).
    let link_entry_id = projection::project_strands(&events, true)
        .iter()
        .find(|s| s.id == src_id)
        .and_then(|s| {
            projection::live_link_entry_ids(s, &tgt_id, etype)
                .pop()
        });
    let (content, effect) = event::unlink_entry_parts(&tgt_id, etype, link_entry_id);
    append_entry_to_strand(JournalEntryAppendRequest {
        strand_id: src_id.clone(),
        content,
        refs: Vec::new(),
        legacy_ref: None,
        effect: Some(effect),
        provenance,
    })?;
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
/// `provenance_raw` is stored on the hide effect entry.
/// When `reason` is given, the entry content keeps the transition
/// `[hidden] <reason>` spelling.
pub(crate) fn cmd_hide(
    id: &str,
    reason: Option<&str>,
    format_json: bool,
    provenance_raw: Option<&str>,
) -> Result<(), String> {
    let strand_id = resolve_id(&read_events_strict(&ensure_journal()?)?, id)?;
    let provenance = parse_provenance_arg(provenance_raw)?;
    let (content, effect) = event::hide_entry_parts(reason);
    // The gate reads current state and the append happens under the same
    // journal write lock, so concurrent hide/unhide calls are serialised.
    let outcome = append_entry_to_strand_gated(
        JournalEntryAppendRequest {
            strand_id: strand_id.clone(),
            content,
            refs: Vec::new(),
            legacy_ref: None,
            effect: Some(effect),
            provenance,
        },
        |events| {
            if projection::hide_balance(events, &strand_id) > 0 {
                Ok(AppendGate::Skip) // already hidden: no-op
            } else {
                Ok(AppendGate::Proceed)
            }
        },
    )?
    .is_some();
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
    let (content, effect) = event::unhide_entry_parts();
    let outcome = append_entry_to_strand_gated(
        JournalEntryAppendRequest {
            strand_id: strand_id.clone(),
            content,
            refs: Vec::new(),
            legacy_ref: None,
            effect: Some(effect),
            provenance: None,
        },
        |events| {
            if projection::hide_balance(events, &strand_id) <= 0 {
                Ok(AppendGate::Skip) // already visible: no-op
            } else {
                Ok(AppendGate::Proceed)
            }
        },
    )?
    .is_some();
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

pub(crate) fn cmd_cutover_v2(
    apply: bool,
    archive: Option<&str>,
    map: Option<&str>,
    format_json: bool,
) -> Result<(), String> {
    let journal_dir = resolve_journal_dir()?;
    let journal_path = ensure_journal()?;
    let archive_path = archive
        .map(PathBuf::from)
        .unwrap_or_else(|| journal_dir.join("journal.v1.jsonl"));
    let map_path = map
        .map(PathBuf::from)
        .unwrap_or_else(|| journal_dir.join("migration-v1-to-v2.json"));

    let read = read_journal_lossy(&journal_path);
    if let Some(error) = read.read_error {
        return Err(error);
    }
    if !read.diagnostics.is_empty() {
        return Err(format!(
            "cannot cut over: journal has {} parse error(s); run doctor first",
            read.diagnostics.len()
        ));
    }

    let source_event_count = read.events.len();
    let plan = build_cutover_v2_plan(&read.events)?;
    let report = output::CutoverV2ReportOutput {
        applied: apply,
        source_journal: journal_path.display().to_string(),
        archive_journal: archive_path.display().to_string(),
        map_path: map_path.display().to_string(),
        source_event_count,
        imported_event_count: plan.events.len(),
        strand_count: plan.map.strands.len(),
        entry_count: plan.map.entries.len(),
        anchor_count: plan
            .events
            .iter()
            .filter(|event| matches!(event, Event::JournalAnchored { .. }))
            .count(),
        unresolved_ref_count: plan.map.unresolved_refs.len(),
    };

    if apply {
        apply_cutover_v2(&journal_path, &archive_path, &map_path, &plan)?;
    }

    if format_json {
        println!(
            "{}",
            serde_json::to_string(&report).expect("serialize cutover report")
        );
    } else {
        println!("v2 cutover {}", if apply { "applied" } else { "dry-run" });
        println!("  source: {}", report.source_journal);
        println!("  archive: {}", report.archive_journal);
        println!("  map: {}", report.map_path);
        println!(
            "  events: {} -> {}",
            report.source_event_count, report.imported_event_count
        );
        println!("  strands: {}", report.strand_count);
        println!("  entries: {}", report.entry_count);
        println!("  anchors: {}", report.anchor_count);
        println!("  unresolved_refs: {}", report.unresolved_ref_count);
        if !apply {
            println!("  apply with: tasktree cutover-v2 --apply");
        }
    }

    Ok(())
}

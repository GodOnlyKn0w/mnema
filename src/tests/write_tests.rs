use super::*;

#[test]
fn add_requires_explicit_strand_type_and_never_infers_from_body() {
    let _env = setup();
    for content in [
        "[task] marker is prose",
        "[session] marker is prose",
        "para group old convention",
        "[12] old task convention",
    ] {
        cmd_add_with_parent(Some(content), false, None, false, None, None, None, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strand = projection::project_strands(&events, true)
            .into_iter()
            .find(|strand| strand.first_summary() == content)
            .expect("new marked strand is projected");
        assert_eq!(strand.strand_type, None);
    }

    cmd_add_with_parent(
        Some("plain explicit type"),
        false,
        None,
        false,
        None,
        None,
        Some("task"),
        None,
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let explicit = projection::project_strands(&events, true)
        .into_iter()
        .find(|strand| strand.first_summary() == "plain explicit type")
        .unwrap();
    assert_eq!(explicit.strand_type.as_deref(), Some("task"));
}

#[test]
fn append_explicit_empty_id_errors() {
    let _env = setup();
    create_strand("existing strand");
    let result = cmd_append(
        Some("must not fallback"),
        None,
        false,
        false,
        None,
        Some("  "),
        None,
        None,
    );
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .contains("explicit --id cannot be empty")
    );
}
#[test]
fn positional_append_most_recent() {
    let _env = setup();
    let _id1 = create_strand("first strand");
    let id2 = create_strand("second strand");
    // Positional content, no ID → most recent active strand
    let result = cmd_append(
        Some("hello world"),
        None,
        false,
        false,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_ok());
    // Verify content appears in most recent strand (id2)
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let has_content = events.iter().any(|(_, e)| {
        if let Event::LogAppended { id, content, .. } = e {
            id == &id2 && content == "hello world"
        } else {
            false
        }
    });
    assert!(has_content);
}

/// Default resolve among ≥2 active lines must surface a teaching disclosure
/// (silent pick is a landmine). Single active line stays quiet.
#[test]
fn append_default_multi_active_discloses_resolve() {
    let _env = setup();
    let _a = create_strand("line a");
    let b = create_strand("line b");

    let multi = execute_append(AppendRequest {
        content: Some("note multi"),
        legacy_id: None,
        new: false,
        stdin: false,
        file: None,
        explicit_id: None,
        provenance_raw: None,
        seen_offset: None,
        why: &[],
        allow_selection: false,
    })
    .expect("default append with 2 active must succeed");
    assert_eq!(multi.strand_id, b);
    assert_eq!(multi.resolved_by, Some("most_recent_active_strand"));
    assert_eq!(multi.active_count, Some(2));
    let (card, state) = multi
        .card_state
        .as_ref()
        .expect("append transaction must return a post-write card");
    assert_eq!(state, "registered");
    assert_eq!(card.entry_count, 2);
    assert_eq!(card.last_entry, "note multi");
    let disclosure = multi_active_resolve_disclosure(&multi.strand_id, multi.active_count.unwrap())
        .expect("2 active lines must produce resolve disclosure");
    assert!(
        disclosure.contains("resolved to"),
        "disclosure must name the pick: {disclosure}"
    );
    assert!(
        disclosure.contains("most recent of 2 active lines"),
        "disclosure must name the count: {disclosure}"
    );
    assert!(
        disclosure.contains("pass --id"),
        "disclosure must teach --id: {disclosure}"
    );
    assert!(
        disclosure.contains(&shorten(&b)),
        "disclosure must carry the chosen prefix: {disclosure}"
    );

    // Explicit --id: no default-resolve metadata (no silent-pick story).
    let explicit = execute_append(AppendRequest {
        content: Some("note explicit"),
        legacy_id: None,
        new: false,
        stdin: false,
        file: None,
        explicit_id: Some(&b),
        provenance_raw: None,
        seen_offset: None,
        why: &[],
        allow_selection: false,
    })
    .expect("explicit append must succeed");
    assert_eq!(explicit.resolved_by, None);
    assert_eq!(explicit.active_count, None);
}

#[test]
fn append_default_single_active_no_resolve_disclosure() {
    let _env = setup();
    let only = create_strand("solo line");

    let outcome = execute_append(AppendRequest {
        content: Some("note solo"),
        legacy_id: None,
        new: false,
        stdin: false,
        file: None,
        explicit_id: None,
        provenance_raw: None,
        seen_offset: None,
        why: &[],
        allow_selection: false,
    })
    .expect("default append with 1 active must succeed");
    assert_eq!(outcome.strand_id, only);
    assert_eq!(outcome.resolved_by, Some("most_recent_active_strand"));
    assert_eq!(outcome.active_count, Some(1));
    assert!(
        multi_active_resolve_disclosure(&outcome.strand_id, 1).is_none(),
        "single active line must not emit resolve disclosure"
    );
}

#[test]
fn append_transaction_preserves_v2_physical_offsets_with_blank_lines() {
    use std::io::Write as _;

    let _env = setup();
    let id = create_strand("v2 offset target");
    let path = ensure_journal().unwrap();
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
    )
    .unwrap();
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
    )
    .unwrap();

    let outcome = execute_append(AppendRequest {
        content: Some("after blank line"),
        legacy_id: None,
        new: false,
        stdin: false,
        file: None,
        explicit_id: Some(&id),
        provenance_raw: None,
        seen_offset: None,
        why: &[],
        allow_selection: false,
    })
    .expect("v2 append transaction must accept blank journal lines");
    let (card, _) = outcome.card_state.expect("post-write card");
    assert_eq!(card.last_offset, 5);
    assert_eq!(card.last_entry, "after blank line");
}

#[test]
fn checkpoint_default_multi_active_discloses_resolve() {
    let _env = setup();
    let _a = create_strand("cp line a");
    let b = create_strand("cp line b");

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let plan = plan_checkpoint(
        &events,
        CheckpointRequest {
            requested_id: None,
            action: "before risky step",
            tail: None,
            include_hidden: false,
            provenance_raw: None,
            seen_offset: None,
            allow_selection: false,
        },
        chrono::Utc::now(),
    )
    .expect("default checkpoint with 2 active must plan");
    assert_eq!(plan.strand_id, b);
    assert_eq!(plan.resolved_by, "most_recent_active_strand");
    assert_eq!(plan.active_count, 2);
    let disclosure = multi_active_resolve_disclosure(&plan.strand_id, plan.active_count)
        .expect("2 active lines must produce checkpoint resolve disclosure");
    assert!(disclosure.contains("most recent of 2 active lines"));
}

#[test]
fn checkpoint_default_single_active_no_resolve_disclosure() {
    let _env = setup();
    let solo = create_strand("cp solo");
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let plan = plan_checkpoint(
        &events,
        CheckpointRequest {
            requested_id: None,
            action: "solo action",
            tail: None,
            include_hidden: false,
            provenance_raw: None,
            seen_offset: None,
            allow_selection: false,
        },
        chrono::Utc::now(),
    )
    .expect("default checkpoint with 1 active must plan");
    assert_eq!(plan.strand_id, solo);
    assert_eq!(plan.resolved_by, "most_recent_active_strand");
    assert_eq!(plan.active_count, 1);
    assert!(multi_active_resolve_disclosure(&plan.strand_id, 1).is_none());
}

#[test]
fn append_default_skips_closed_strand() {
    // The most-recently-touched strand is closed; --last/default must fall
    // through to the newest OPEN strand, not land on the closed one.
    let _env = setup();
    let open_id = create_strand("still open");
    let closed_id = create_strand("about to close"); // newest by ts
    let closed = cmd_close(&closed_id, Some("done"), None, false);
    assert!(closed.is_ok(), "close failed: {:?}", closed);

    let result = cmd_append(
        Some("lands on open"),
        None,
        false,
        false,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_ok());

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let on_open = events.iter().any(|(_, e)| {
        matches!(e, Event::LogAppended { id, content, .. }
            if id == &open_id && content == "lands on open")
    });
    let on_closed = events.iter().any(|(_, e)| {
        matches!(e, Event::LogAppended { id, content, .. }
            if id == &closed_id && content == "lands on open")
    });
    assert!(on_open, "append should land on the newest open strand");
    assert!(!on_closed, "append must not land on the closed strand");
}

#[test]
fn legacy_positional_id_is_rejected() {
    let _env = setup();
    let id1 = create_strand("first strand");
    let result = cmd_append(
        Some("legacy id test"),
        Some(&id1),
        false,
        false,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("legacy positional strand id"));
}

#[test]
fn positional_with_explicit_id() {
    let _env = setup();
    let id1 = create_strand("first strand");
    let result = cmd_append(
        Some("explicit id test"),
        None,
        false,
        false,
        None,
        Some(&id1),
        None,
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn positional_empty_rejected() {
    let _env = setup();
    create_strand("first strand");
    let result = cmd_append(Some("   "), None, false, false, None, None, None, None);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("empty"));
}

#[test]
fn stdin_append() {
    let _env = setup();
    create_strand("first strand");
    // Simulate stdin by writing to a temp file and redirecting
    // Since we can't easily pipe in unit tests, we test read_stdin_content via a temp file approach.
    // Instead, test directly: create a file, read it with read_file_content, verify normalize_content
    let raw = "stdin simulated content\n";
    let stored = normalize_content(raw);
    assert_eq!(stored, "stdin simulated content");
}

// ── Content source: --file ──

#[test]
fn file_append_valid() {
    let _env = setup();
    let id = create_strand("first strand");
    let file_path = _env.path().join("note.md");
    fs::write(&file_path, "file content here").unwrap();
    let result = cmd_append(
        None,
        None,
        false,
        false,
        Some(file_path.to_str().unwrap()),
        Some(&id),
        None,
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn file_content_not_found() {
    let result = read_file_content("nonexistent_file_xyz.md");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("file not found"));
}

#[test]
fn file_content_is_directory() {
    let dir = tempfile::tempdir().unwrap();
    let result = read_file_content(dir.path().to_str().unwrap());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("directory"));
}

#[test]
fn file_content_empty() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("empty.md");
    fs::write(&file_path, "").unwrap();
    let result = read_file_content(file_path.to_str().unwrap());
    assert!(result.is_ok()); // read succeeds, empty check happens in cmd_append
    let _env = setup();
    create_strand("first strand");
    let result = cmd_append(
        None,
        None,
        false,
        false,
        Some(file_path.to_str().unwrap()),
        None,
        None,
        None,
    );
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("empty"));
    assert!(err.contains("empty.md"));
}

// ── Content source conflicts ──

#[test]
fn default_stdin_empty_or_unpiped_is_rejected() {
    let _env = setup();
    create_strand("first strand");
    let result = cmd_append(None, None, false, false, None, None, None, None);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("stdin"));
}

#[test]
fn content_source_conflict_positional_and_stdin() {
    let _env = setup();
    create_strand("first strand");
    let result = cmd_append(Some("content"), None, false, true, None, None, None, None);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("exactly one stdin stream"));
}

#[test]
fn content_source_conflict_positional_and_file() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("note.md");
    fs::write(&file_path, "test").unwrap();
    let _env = setup();
    create_strand("first strand");
    let result = cmd_append(
        Some("content"),
        None,
        false,
        false,
        Some(file_path.to_str().unwrap()),
        None,
        None,
        None,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("exactly one stdin stream"));
}

#[test]
fn direct_content_and_stdin_conflict() {
    let _env = setup();
    create_strand("first strand");
    let result = cmd_append(
        Some("0000019dd34b"),
        None,
        false,
        true,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("exactly one stdin stream"));
}

#[test]
fn direct_content_and_file_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("note.md");
    fs::write(&file_path, "test").unwrap();
    let _env = setup();
    create_strand("first strand");
    let result = cmd_append(
        Some("0000019dd34b"),
        None,
        false,
        false,
        Some(file_path.to_str().unwrap()),
        None,
        None,
        None,
    );
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("exactly one stdin stream"));
}

#[test]
fn content_source_conflict_stdin_and_file() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("note.md");
    fs::write(&file_path, "test").unwrap();
    let _env = setup();
    create_strand("first strand");
    let result = cmd_append(
        None,
        None,
        false,
        true,
        Some(file_path.to_str().unwrap()),
        None,
        None,
        None,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("exactly one stdin stream"));
}

#[test]
fn content_source_all_three() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("note.md");
    fs::write(&file_path, "test").unwrap();
    let _env = setup();
    create_strand("first strand");
    let result = cmd_append(
        Some("content"),
        None,
        false,
        true,
        Some(file_path.to_str().unwrap()),
        None,
        None,
        None,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("exactly one stdin stream"));
}

// ── Target source conflicts ──

#[test]
fn checkpoint_diagnostics_scar_fires_on_overdue_deadline() {
    // Strands with an overdue [deadline] must produce a W068 diagnostic.
    // Checkpoint runs diagnostics internally; this test verifies that the
    // same journal state run_journal_diagnostics sees is non-empty, which
    // is what drives the scar line printed by cmd_checkpoint.
    let _env = setup();
    let id = create_strand("deadline work");
    cmd_append(
        Some("[deadline] finish rollout by=2000-01-01"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();

    // cmd_checkpoint must succeed (overdue deadline is a warning, not fatal).
    let result = cmd_checkpoint(
        Some(&id),
        "checkpoint before close",
        None,
        false,
        false,
        None,
    );
    assert!(
        result.is_ok(),
        "checkpoint must succeed even with overdue deadline: {:?}",
        result
    );

    // Confirm the journal state produces a W068 — the same data checkpoint uses.
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
    let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
    assert!(
        diags.iter().any(|(c, _)| *c == "W068"),
        "expected W068 diagnostic for overdue deadline, got {:?}",
        diags
    );
}

#[test]
fn checkpoint_explicit_id_appends_structured_entry() {
    let _env = setup();
    let id = create_strand("checkpoint target");

    let result = cmd_checkpoint(
        Some(&id),
        "git commit checkpoint work",
        None,
        false,
        false,
        None,
    );
    assert!(result.is_ok());

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::LogAppended {
            id: event_id,
            content,
            entry_id,
            ..
        } = e
        {
            event_id == &id
                && content.contains("[checkpoint] ok")
                && content.contains("resolved_by=\"explicit --id\"")
                && content.contains("observed_entries_before_append=1")
                && content.contains("action=\"git commit checkpoint work\"")
                && entry_id.is_some()
        } else {
            false
        }
    });
    assert!(found);
}

#[test]
fn checkpoint_without_id_uses_most_recent_strand() {
    let _env = setup();
    let _old = create_strand("old strand");
    let recent = create_strand("recent strand");

    let result = cmd_checkpoint(None, "remove old build dirs", None, false, false, None);
    assert!(result.is_ok());

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::LogAppended { id, content, .. } = e {
            id == &recent
                && content.contains("[checkpoint] ok")
                && content.contains("resolved_by=\"most_recent_active_strand\"")
        } else {
            false
        }
    });
    assert!(found);
}

#[test]
fn checkpoint_tail_does_not_change_observed_entry_count() {
    let _env = setup();
    let id = create_strand("checkpoint target");
    cmd_append(
        Some("step one"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("step two"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();

    let result = cmd_checkpoint(
        Some(&id),
        "commit after three entries",
        Some(1),
        false,
        false,
        None,
    );
    assert!(result.is_ok());

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::LogAppended {
            id: event_id,
            content,
            ..
        } = e
        {
            event_id == &id
                && content.contains("[checkpoint] ok")
                && content.contains("observed_entries_before_append=3")
        } else {
            false
        }
    });
    assert!(found);
}

#[test]
fn checkpoint_bad_strand_returns_resolve_failure_without_append() {
    let _env = setup();
    create_strand("checkpoint target");
    let before = read_events_lossy(&ensure_journal().unwrap()).0.len();

    let result = cmd_checkpoint(
        Some("doesnotexist"),
        "bad checkpoint",
        None,
        false,
        false,
        None,
    );
    assert!(result.is_err());
    let failure = result.unwrap_err();
    assert_eq!(failure.code, 1);
    assert!(!failure.journal_appended);

    let after = read_events_lossy(&ensure_journal().unwrap()).0.len();
    assert_eq!(before, after);
}

#[test]
fn checkpoint_empty_action_returns_invalid_arguments() {
    let _env = setup();
    let id = create_strand("checkpoint target");
    let before = read_events_lossy(&ensure_journal().unwrap()).0.len();

    let result = cmd_checkpoint(Some(&id), "   ", None, false, false, None);
    assert!(result.is_err());
    let failure = result.unwrap_err();
    assert_eq!(failure.code, 3);
    assert!(!failure.journal_appended);

    let after = read_events_lossy(&ensure_journal().unwrap()).0.len();
    assert_eq!(before, after);
}

// ── exit_code_for (exit-code contract) ─────────────────────────────────

#[test]
fn checkpoint_on_closed_strand_still_succeeds() {
    let _env = setup();
    let id = create_strand("done work");
    cmd_close(&id, Some("done"), None, false).unwrap();
    // Checkpoint must still succeed — W071 is a warning, not a gate.
    let result = cmd_checkpoint(Some(&id), "tag the release", None, false, false, None);
    assert!(
        result.is_ok(),
        "checkpoint on closed strand must exit 0: {:?}",
        result
    );
}

// ── staleness / journal_delta helpers ─────────────────────────────────

#[test]
fn append_seen_offset_stale_still_writes() {
    let _env = setup();
    let id = create_strand("seen offset append target");
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let seen = strands.iter().find(|s| s.id == id).unwrap().last_offset();

    cmd_append(
        Some("[progress] moved after read"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    let result = cmd_append_with_seen_offset(
        Some("[progress] write with stale seen offset"),
        None,
        false,
        false,
        None,
        Some(&id),
        Some("json"),
        None,
        Some(seen),
        &[],
    );
    assert!(
        result.is_ok(),
        "stale --seen-offset is a warning, not a gate: {:?}",
        result
    );

    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let strand = strands.iter().find(|s| s.id == id).unwrap();
    assert!(
        strand
            .log
            .iter()
            .any(|e| e.content.contains("write with stale seen offset")),
        "append must still write the requested entry"
    );
}

#[test]
fn checkpoint_seen_offset_stale_still_writes() {
    let _env = setup();
    let id = create_strand("seen offset checkpoint target");
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let seen = strands.iter().find(|s| s.id == id).unwrap().last_offset();

    cmd_append(
        Some("[progress] moved after read"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    let result = cmd_checkpoint_with_seen_offset(
        Some(&id),
        "checkpoint with stale seen offset",
        None,
        true,
        false,
        None,
        Some(seen),
    );
    assert!(
        result.is_ok(),
        "stale --seen-offset is a warning, not a gate: {:?}",
        result
    );

    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::LogAppended {
            id: event_id,
            content,
            ..
        } = e
        {
            event_id == &id && content.contains("checkpoint with stale seen offset")
        } else {
            false
        }
    });
    assert!(found, "checkpoint must still append its journal entry");
}

#[test]
fn checkpoint_without_id_skips_hidden_when_explicit_id_missing() {
    let _env = setup();
    let old = create_strand("old visible strand");
    let recent = create_strand("recent will be hidden");
    cmd_hide(&recent, Some("noise"), false, None).unwrap();
    let result = cmd_checkpoint(None, "fall back to visible", None, false, false, None);
    assert!(result.is_ok());
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::LogAppended { id, content, .. } = e {
            id == &old && content.contains("resolved_by=\"most_recent_active_strand\"")
        } else {
            false
        }
    });
    assert!(
        found,
        "checkpoint must fall back to the visible strand when most-recent is hidden"
    );
}

// With --include-hidden / --all, cmd_checkpoint may pick a hidden strand.

#[test]
fn checkpoint_with_include_hidden_can_pick_hidden_strand() {
    let _env = setup();
    let _old = create_strand("old visible strand");
    let recent = create_strand("recent will be hidden");
    cmd_hide(&recent, Some("noise"), false, None).unwrap();
    let result = cmd_checkpoint(None, "allow hidden", None, false, true, None);
    assert!(result.is_ok());
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::LogAppended { id, content, .. } = e {
            id == &recent && content.contains("resolved_by=\"most_recent_active_strand\"")
        } else {
            false
        }
    });
    assert!(
        found,
        "with include_hidden=true, checkpoint must pick the most-recent hidden strand"
    );
}

// With an explicit --id that happens to be a hidden strand, the
// checkpoint must still find it (the user named it directly).

#[test]
fn checkpoint_explicit_id_finds_hidden_strand() {
    let _env = setup();
    let id = create_strand("explicit hidden");
    cmd_hide(&id, Some("noise"), false, None).unwrap();
    let result = cmd_checkpoint(Some(&id), "explicit id on hidden", None, false, false, None);
    assert!(result.is_ok(), "explicit --id must resolve a hidden strand");
}

#[test]
fn append_new_stores_provenance_on_initial_entry() {
    let _env = setup();
    cmd_append(
        Some("new with provenance"),
        None,
        true,
        false,
        None,
        None,
        None,
        Some(r#"{"producer":"tester"}"#),
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::LogAppended {
            content,
            provenance,
            ..
        } = e
        {
            content == "new with provenance"
                && provenance
                    .as_ref()
                    .and_then(|p| p.get("producer"))
                    .and_then(|p| p.as_str())
                    == Some("tester")
        } else {
            false
        }
    });
    assert!(
        found,
        "append --new must carry explicit provenance on the initial entry"
    );
}
#[test]
fn add_provenance_stored_on_first_log_entry() {
    let _env = setup();
    cmd_add(
        Some("add prov test"),
        false,
        None,
        false,
        None,
        Some(r#"{"producer":"tester"}"#),
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::LogAppended {
            content,
            provenance,
            ..
        } = e
        {
            content == "add prov test" && provenance.is_some()
        } else {
            false
        }
    });
    assert!(
        found,
        "LogAppended from add must carry provenance when --provenance given"
    );
}

#[test]
fn add_without_provenance_has_none() {
    let _env = setup();
    cmd_add(Some("add no prov"), false, None, false, None, None).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::LogAppended {
            content,
            provenance,
            ..
        } = e
        {
            content == "add no prov" && provenance.is_none()
        } else {
            false
        }
    });
    assert!(
        found,
        "LogAppended from add must have provenance=None when not given"
    );
}

#[test]
fn add_parent_creates_child_and_belongs_to_edge() {
    let _env = setup();
    let parent = create_strand("parent line");
    cmd_add_with_parent(
        Some("child line"),
        false,
        None,
        false,
        Some(&parent),
        None,
        None,
        None,
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let child = strands
        .iter()
        .find(|s| s.first_summary() == "child line")
        .expect("child strand must exist");
    assert_eq!(child.belongs_to_edges, vec![parent]);
}

#[test]
fn add_parent_missing_errors_without_creating_child() {
    let _env = setup();
    let result = cmd_add_with_parent(
        Some("orphan child"),
        false,
        None,
        false,
        Some("0000019dd34b"),
        None,
        None,
        None,
    );
    assert!(result.is_err(), "missing parent must error");
    assert!(result.unwrap_err().contains("parent strand"));
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    assert!(
        strands.iter().all(|s| s.first_summary() != "orphan child"),
        "child must not be created when parent resolution fails"
    );
}

#[test]
fn add_empty_parent_errors() {
    let _env = setup();
    let result = cmd_add_with_parent(
        Some("child"),
        false,
        None,
        false,
        Some("  "),
        None,
        None,
        None,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("--parent cannot be empty"));
}
// ── ③ --edge-type: renamed flag still resolves correctly ─────────────

#[test]
fn add_positional_content_creates_strand() {
    let _env = setup();
    // Positional content: existing path, now cmd_add(Some(..), false, None, ..)
    let result = cmd_add(Some("add positional"), false, None, false, None, None);
    assert!(
        result.is_ok(),
        "add with positional content must succeed: {:?}",
        result
    );
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    assert!(
        strands
            .iter()
            .any(|s| s.first_summary() == "add positional"),
        "strand with 'add positional' summary must exist"
    );
}

#[test]
fn add_success_next_step_hands_off_append_with_new_id() {
    // From-zero hand-off: first line just born → paste-ready append next.
    let id = "abcdef0123456789deadbeef";
    let line = add_success_next_step(id);
    let prefix = shorten(id);
    assert!(
        line.contains(&format!("mnema append --id {}", prefix)),
        "next step must carry new id prefix: {line}"
    );
    assert!(
        line.contains(r#"echo "<note>""#) || line.contains("echo \"<note>\""),
        "next step must be an echo|append pipe: {line}"
    );
    // The taught command itself must parse against real grammar (CI gate).
    try_parse_example(&line).unwrap_or_else(|e| panic!("add next step must parse: {e}"));
}

#[test]
fn add_file_content_creates_strand() {
    let env = setup();
    let file_path = env.path().join("brief.md");
    fs::write(&file_path, "add from file\n").unwrap();
    let path_str = file_path.to_str().unwrap();
    let result = cmd_add(None, false, Some(path_str), false, None, None);
    assert!(result.is_ok(), "add --file must succeed: {:?}", result);
    let jpath = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&jpath);
    let strands = projection::project_strands(&events, true);
    assert!(
        strands.iter().any(|s| s.first_summary() == "add from file"),
        "strand with 'add from file' summary must exist after add --file"
    );
}

#[test]
fn add_multiple_content_sources_errors() {
    let _env = setup();
    // positional + stdin both set → must error
    let result = cmd_add(Some("content"), true, None, false, None, None);
    assert!(result.is_err(), "add with two content sources must error");
}

#[test]
fn add_default_stdin_empty_or_unpiped_errors() {
    let _env = setup();
    let result = cmd_add(None, false, None, false, None, None);
    assert!(result.is_err(), "add without piped stdin must error");
    assert!(result.unwrap_err().contains("stdin"));
}

#[test]
fn add_empty_file_content_errors() {
    let env = setup();
    let file_path = env.path().join("empty.md");
    fs::write(&file_path, "").unwrap();
    let path_str = file_path.to_str().unwrap();
    let result = cmd_add(None, false, Some(path_str), false, None, None);
    assert!(result.is_err(), "add --file with empty file must error");
}

#[test]
fn add_nonexistent_file_errors() {
    let _env = setup();
    let result = cmd_add(
        None,
        false,
        Some("/nonexistent/path/to/file.txt"),
        false,
        None,
        None,
    );
    assert!(
        result.is_err(),
        "add --file with nonexistent file must error"
    );
}

// ── W073: typo marker suggestion ─────────────────────────────────────────

#[test]
fn v2_add_uses_first_entry_hash_as_strand_id() {
    let _env = setup();
    let id = create_strand("v2 first entry identity");
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let strand = strands.iter().find(|s| s.id == id).expect("strand exists");
    let first = strand.log.first().expect("first entry exists");

    assert_eq!(id.len(), 64);
    assert_eq!(first.entry_id.as_deref(), Some(id.as_str()));
    assert!(first.prev_entry_id.is_none());
}

#[test]
fn v2_append_chains_to_previous_entry_hash() {
    let _env = setup();
    let id = create_strand("v2 chain root");
    let path = ensure_journal().unwrap();
    let (before_events, _) = read_events_lossy(&path);
    let before_strands = projection::project_strands(&before_events, true);
    let first_entry_id = before_strands
        .iter()
        .find(|s| s.id == id)
        .and_then(|s| s.log.first())
        .and_then(|entry| entry.entry_id.clone())
        .expect("first entry hash exists");

    cmd_append(
        Some("v2 chained append"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();

    let (after_events, _) = read_events_lossy(&path);
    let after_strands = projection::project_strands(&after_events, true);
    let strand = after_strands
        .iter()
        .find(|s| s.id == id)
        .expect("strand exists after append");
    let second = strand.log.get(1).expect("second entry exists");

    assert_eq!(
        second.prev_entry_id.as_deref(),
        Some(first_entry_id.as_str())
    );
    assert_ne!(second.entry_id.as_deref(), Some(first_entry_id.as_str()));
}

#[test]
fn v2_why_writes_entry_hash_ref_only() {
    let _env = setup();
    let basis = create_strand("basis strand");
    cmd_append(
        Some("basis update"),
        None,
        false,
        false,
        None,
        Some(&basis),
        None,
        None,
    )
    .unwrap();
    let consumer = create_strand("consumer strand");

    let path = ensure_journal().unwrap();
    let (before_events, _) = read_events_lossy(&path);
    let basis_entry_id = projection::project_strands(&before_events, true)
        .iter()
        .find(|s| s.id == basis)
        .and_then(|s| s.log.last())
        .and_then(|entry| entry.entry_id.clone())
        .expect("basis entry hash exists");

    cmd_append_with_seen_offset(
        Some("[decision] cite basis"),
        None,
        false,
        false,
        None,
        Some(&consumer),
        None,
        None,
        None,
        &[&basis],
    )
    .unwrap();

    let (after_events, _) = read_events_lossy(&path);
    let cited = after_events
        .iter()
        .filter_map(|(_, event)| {
            if let Event::LogAppended {
                id,
                content,
                refs,
                ref_,
                ..
            } = event
            {
                if id == &consumer && content == "[decision] cite basis" {
                    return Some((refs, ref_));
                }
            }
            None
        })
        .next()
        .expect("citing entry exists");

    assert_eq!(cited.0, &vec![basis_entry_id]);
    assert!(
        cited.1.is_none(),
        "legacy ref pin retired 2026-07-04: hash refs are the only rationale storage"
    );
}

#[test]
fn v2_why_strand_shorthand_skips_latest_structural_effect() {
    let _env = setup();
    let basis = create_strand("basis strand");
    cmd_append(
        Some("[decision] authored rationale"),
        None,
        false,
        false,
        None,
        Some(&basis),
        None,
        None,
    )
    .unwrap();
    let structural_target = create_strand("structural target");
    cmd_link(&basis, &structural_target, Some("depends-on"), false, None).unwrap();
    let consumer = create_strand("consumer strand");

    let path = ensure_journal().unwrap();
    let (before_events, _) = read_events_lossy(&path);
    let authored_entry_id = projection::project_strands(&before_events, true)
        .iter()
        .find(|s| s.id == basis)
        .and_then(|s| s.log.iter().rev().find(|entry| entry.effect.is_none()))
        .and_then(|entry| entry.entry_id.clone())
        .expect("authored rationale hash exists");

    cmd_append_with_seen_offset(
        Some("[decision] cite the current conclusion"),
        None,
        false,
        false,
        None,
        Some(&consumer),
        None,
        None,
        None,
        &[&basis],
    )
    .unwrap();

    let (after_events, _) = read_events_lossy(&path);
    let refs = after_events
        .iter()
        .find_map(|(_, event)| match event {
            Event::LogAppended {
                id, content, refs, ..
            } if id == &consumer && content == "[decision] cite the current conclusion" => {
                Some(refs.clone())
            }
            _ => None,
        })
        .expect("citing entry exists");

    assert_eq!(
        refs,
        vec![authored_entry_id],
        "strand shorthand must cite authored content, not the latest link effect"
    );
}

#[test]
fn v2_why_pins_exact_entry_by_hash_prefix() {
    let _env = setup();
    let basis = create_strand("basis line first entry");
    for content in ["basis middle entry", "basis late entry"] {
        cmd_append(
            Some(content),
            None,
            false,
            false,
            None,
            Some(&basis),
            None,
            None,
        )
        .unwrap();
    }
    let consumer = create_strand("consumer line");

    let path = ensure_journal().unwrap();
    let (before_events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&before_events, true);
    let basis_strand = strands.iter().find(|s| s.id == basis).unwrap();
    // Pin the MIDDLE entry — not the head (a head prefix resolves as the
    // strand and falls back to latest-entry shorthand), not the latest.
    let middle_entry_id = basis_strand.log[1]
        .entry_id
        .clone()
        .expect("middle entry hash exists");

    cmd_append_with_seen_offset(
        Some("[decision] cite the middle entry exactly"),
        None,
        false,
        false,
        None,
        Some(&consumer),
        None,
        None,
        None,
        &[&middle_entry_id[..16]],
    )
    .unwrap();

    let (after_events, _) = read_events_lossy(&path);
    let cited = after_events
        .iter()
        .filter_map(|(_, event)| {
            if let Event::LogAppended {
                id,
                content,
                refs,
                ref_,
                ..
            } = event
            {
                if id == &consumer && content == "[decision] cite the middle entry exactly" {
                    return Some((refs.clone(), ref_.clone()));
                }
            }
            None
        })
        .next()
        .expect("citing entry exists");

    assert_eq!(
        cited.0,
        vec![middle_entry_id],
        "an entry-hash prefix pins that exact entry, not the line's latest"
    );
    assert!(
        cited.1.is_none(),
        "legacy ref pin retired 2026-07-04: staleness derives from positions, not pins"
    );
}

#[test]
fn add_from_pins_source_and_composes_with_parent() {
    let _env = setup();
    let mother = create_strand("mother line of work");
    cmd_append(
        Some("[decision] spawn a derived matter"),
        None,
        false,
        false,
        None,
        Some(&mother),
        None,
        None,
    )
    .unwrap();

    let path = ensure_journal().unwrap();
    let (before_events, _) = read_events_lossy(&path);
    let mother_latest_entry_id = projection::project_strands(&before_events, true)
        .iter()
        .find(|s| s.id == mother)
        .and_then(|s| s.log.last())
        .and_then(|entry| entry.entry_id.clone())
        .expect("mother entry hash exists");

    // --parent and --from together: one command writes first entry with the
    // source ref AND the belongs-to link (CORPUS §6 atomic derivation).
    cmd_add_with_parent(
        Some("derived line of work"),
        false,
        None,
        false,
        Some(&mother),
        Some(&mother),
        None,
        None,
    )
    .unwrap();

    let (after_events, _) = read_events_lossy(&path);
    let first_entry = after_events
        .iter()
        .filter_map(|(_, event)| {
            if let Event::LogAppended {
                id,
                content,
                refs,
                ref_,
                ..
            } = event
            {
                if content == "derived line of work" {
                    return Some((id.clone(), refs.clone(), ref_.clone()));
                }
            }
            None
        })
        .next()
        .expect("derived first entry exists");

    assert_eq!(
        first_entry.1,
        vec![mother_latest_entry_id],
        "--from stores the source ref on the derived line's first entry"
    );
    assert!(
        first_entry.2.is_none(),
        "legacy ref pin retired 2026-07-04: --from stores the hash ref only"
    );
    let has_belongs_to = after_events.iter().any(|(_, event)| {
        matches!(
            event,
            Event::LogAppended {
                id,
                effect: Some(event::EntryEffect::Link { target, edge_type }),
                ..
            } if id == &first_entry.0 && target == &mother && edge_type == "belongs-to"
        )
    });
    assert!(
        has_belongs_to,
        "--parent still writes the belongs-to link alongside --from"
    );
}

#[test]
fn add_from_zero_one_n_order_and_duplicate_rejection() {
    let _env = setup();
    let a = create_strand("source a");
    let b = create_strand("source b");
    let path = ensure_journal().unwrap();
    let events = read_events_lossy(&path).0;
    let strands = projection::project_strands(&events, true);
    let a_entry = strands
        .iter()
        .find(|s| s.id == a)
        .and_then(|s| s.log.last())
        .and_then(|e| e.entry_id.clone())
        .expect("a entry");
    let b_entry = strands
        .iter()
        .find(|s| s.id == b)
        .and_then(|s| s.log.last())
        .and_then(|e| e.entry_id.clone())
        .expect("b entry");

    // 0 refs
    cmd_add_with_parent_and_slug(
        Some("zero refs line"),
        false,
        None,
        false,
        None,
        &[],
        None,
        None,
        None,
    )
    .unwrap();
    let zero = find_log_refs(&path, "zero refs line");
    assert!(zero.is_empty(), "0 --from → empty refs: {zero:?}");

    // 1 ref (compat)
    cmd_add_with_parent_and_slug(
        Some("one ref line"),
        false,
        None,
        false,
        None,
        &[&a],
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(find_log_refs(&path, "one ref line"), vec![a_entry.clone()]);

    // N refs, authored order preserved
    cmd_add_with_parent_and_slug(
        Some("n refs line"),
        false,
        None,
        false,
        None,
        &[&b, &a],
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        find_log_refs(&path, "n refs line"),
        vec![b_entry.clone(), a_entry.clone()],
        "refs keep authored order"
    );

    // reverse order is a different identity payload
    cmd_add_with_parent_and_slug(
        Some("n refs reverse"),
        false,
        None,
        false,
        None,
        &[&a, &b],
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        find_log_refs(&path, "n refs reverse"),
        vec![a_entry.clone(), b_entry.clone()]
    );

    // duplicate resolved target rejected
    let dup = cmd_add_with_parent_and_slug(
        Some("dup refs"),
        false,
        None,
        false,
        None,
        &[&a, &a],
        None,
        None,
        None,
    );
    let err = dup.unwrap_err();
    assert!(err.contains("duplicate"), "{err}");
    assert!(err.contains("--ref"), "{err}");
}

#[test]
fn add_parent_plus_multi_from_batch_is_atomic_visible() {
    let _env = setup();
    let parent = create_strand("parent line");
    let evidence = create_strand("evidence line");
    let path = ensure_journal().unwrap();
    let events = read_events_lossy(&path).0;
    let evidence_entry = projection::project_strands(&events, true)
        .iter()
        .find(|s| s.id == evidence)
        .and_then(|s| s.log.last())
        .and_then(|e| e.entry_id.clone())
        .expect("evidence entry");
    let parent_entry = projection::project_strands(&events, true)
        .iter()
        .find(|s| s.id == parent)
        .and_then(|s| s.log.last())
        .and_then(|e| e.entry_id.clone())
        .expect("parent entry");

    cmd_add_with_parent_and_slug(
        Some("child with parent and multi refs"),
        false,
        None,
        false,
        Some(&parent),
        &[&evidence, &parent],
        None,
        None,
        None,
    )
    .unwrap();

    let (after, _) = read_events_lossy(&path);
    let mut found_first = None;
    let mut found_link = false;
    for (_, event) in &after {
        match event {
            Event::LogAppended {
                id,
                content,
                refs,
                effect: None,
                ..
            } if content == "child with parent and multi refs" => {
                assert_eq!(
                    refs,
                    &vec![evidence_entry.clone(), parent_entry.clone()],
                    "first entry carries both refs in order"
                );
                found_first = Some(id.clone());
            }
            Event::LogAppended {
                id,
                effect: Some(event::EntryEffect::Link { target, edge_type }),
                ..
            } if found_first.as_ref() == Some(id)
                && target == &parent
                && edge_type == "belongs-to" =>
            {
                found_link = true;
            }
            _ => {}
        }
    }
    assert!(found_first.is_some(), "child first entry visible");
    assert!(found_link, "belongs-to visible in same successful command");
}

#[test]
fn append_why_zero_one_n_and_new() {
    let _env = setup();
    let target = create_strand("append target");
    let r1 = create_strand("rationale one");
    let r2 = create_strand("rationale two");
    let path = ensure_journal().unwrap();
    let events = read_events_lossy(&path).0;
    let strands = projection::project_strands(&events, true);
    let r1_entry = strands
        .iter()
        .find(|s| s.id == r1)
        .and_then(|s| s.log.last())
        .and_then(|e| e.entry_id.clone())
        .expect("r1");
    let r2_entry = strands
        .iter()
        .find(|s| s.id == r2)
        .and_then(|s| s.log.last())
        .and_then(|e| e.entry_id.clone())
        .expect("r2");

    // 0 why
    execute_append(AppendRequest {
        content: Some("plain note"),
        legacy_id: None,
        new: false,
        stdin: false,
        file: None,
        explicit_id: Some(&target),
        provenance_raw: None,
        seen_offset: None,
        why: &[],
        allow_selection: false,
    })
    .unwrap();
    assert!(find_log_refs(&path, "plain note").is_empty());

    // N why on existing
    execute_append(AppendRequest {
        content: Some("[decision] multi why"),
        legacy_id: None,
        new: false,
        stdin: false,
        file: None,
        explicit_id: Some(&target),
        provenance_raw: None,
        seen_offset: None,
        why: &[&r1, &r2],
        allow_selection: false,
    })
    .unwrap();
    assert_eq!(
        find_log_refs(&path, "[decision] multi why"),
        vec![r1_entry.clone(), r2_entry.clone()]
    );

    // append --new honors why
    execute_append(AppendRequest {
        content: Some("brand new with why"),
        legacy_id: None,
        new: true,
        stdin: false,
        file: None,
        explicit_id: None,
        provenance_raw: None,
        seen_offset: None,
        why: &[&r2],
        allow_selection: false,
    })
    .unwrap();
    assert_eq!(
        find_log_refs(&path, "brand new with why"),
        vec![r2_entry.clone()]
    );

    let dup = execute_append(AppendRequest {
        content: Some("dup why"),
        legacy_id: None,
        new: false,
        stdin: false,
        file: None,
        explicit_id: Some(&target),
        provenance_raw: None,
        seen_offset: None,
        why: &[&r1, &r1],
        allow_selection: false,
    });
    assert!(dup.unwrap_err().contains("duplicate"));
}

fn find_log_refs(path: &std::path::PathBuf, content: &str) -> Vec<String> {
    let (events, _) = read_events_lossy(path);
    events
        .into_iter()
        .find_map(|(_, event)| match event {
            Event::LogAppended {
                content: c, refs, ..
            } if c == content => Some(refs),
            _ => None,
        })
        .unwrap_or_else(|| panic!("log entry with content {content:?} not found"))
}

#[test]
fn v2_close_and_reopen_write_lifecycle_effect_entries() {
    let _env = setup();
    let id = create_strand("v2 lifecycle target");
    let path = ensure_journal().unwrap();
    let (before_events, _) = read_events_lossy(&path);
    let first_entry_id = projection::project_strands(&before_events, true)
        .iter()
        .find(|s| s.id == id)
        .and_then(|s| s.log.last())
        .and_then(|entry| entry.entry_id.clone())
        .expect("first entry hash exists");

    cmd_close(&id, Some("done"), None, false).unwrap();
    let (closed_events, _) = read_events_lossy(&path);
    let close_prev = closed_events
        .iter()
        .filter_map(|(_, event)| {
            if let Event::LogAppended {
                id: event_id,
                effect: Some(event::EntryEffect::Close { disposition }),
                prev_entry_id,
                ..
            } = event
            {
                if event_id == &id && disposition == "done" {
                    return prev_entry_id.clone();
                }
            }
            None
        })
        .next()
        .expect("close effect entry exists");
    assert_eq!(close_prev, first_entry_id);
    let closed_strands = projection::project_strands(&closed_events, true);
    let closed = closed_strands.iter().find(|s| s.id == id).unwrap();
    assert_eq!(closed.state(), "closed:done");
    let close_entry_id = closed
        .log
        .last()
        .and_then(|entry| entry.entry_id.clone())
        .expect("close entry hash exists");

    cmd_reopen(&id, None, false).unwrap();
    let (reopened_events, _) = read_events_lossy(&path);
    let reopen_prev = reopened_events
        .iter()
        .filter_map(|(_, event)| {
            if let Event::LogAppended {
                id: event_id,
                effect: Some(event::EntryEffect::Reopen),
                prev_entry_id,
                ..
            } = event
            {
                if event_id == &id {
                    return prev_entry_id.clone();
                }
            }
            None
        })
        .next()
        .expect("reopen effect entry exists");
    assert_eq!(reopen_prev, close_entry_id);
    let reopened_strands = projection::project_strands(&reopened_events, true);
    let reopened = reopened_strands.iter().find(|s| s.id == id).unwrap();
    assert_eq!(reopened.state(), "registered");
}

#[test]
fn close_with_reason_stores_it_in_content() {
    let _env = setup();
    let id = create_strand("target for reasoned close");
    cmd_close(&id, Some("verified"), Some("checked in staging"), false).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let closed = events.iter().find_map(|(_, e)| {
        if let Event::LogAppended {
            id: eid,
            content,
            effect: Some(event::EntryEffect::Close { disposition }),
            ..
        } = e
        {
            if eid == &id {
                return Some((content.clone(), disposition.clone()));
            }
        }
        None
    });
    let (content, disp) = closed.expect("close effect entry exists");
    assert_eq!(disp, "verified", "effect carries the machine disposition");
    assert_eq!(
        content, "close disposition=verified: checked in staging",
        "author reason rides in content after the machine-mirror prefix"
    );

    // A reason-less close stays byte-identical to the pre-reason format.
    let id2 = create_strand("target for bare close");
    cmd_close(&id2, None, None, false).unwrap();
    let (events2, _) = read_events_lossy(&path);
    let bare = events2.iter().find_map(|(_, e)| {
        if let Event::LogAppended {
            id: eid,
            content,
            effect: Some(event::EntryEffect::Close { .. }),
            ..
        } = e
        {
            if eid == &id2 {
                return Some(content.clone());
            }
        }
        None
    });
    assert_eq!(bare.unwrap(), "close disposition=done");
}

#[test]
fn add_slug_persists_and_rejects_hex_or_duplicate() {
    let _env = setup();
    cmd_add_with_parent_and_slug(
        Some("slugged strand"),
        false,
        None,
        false,
        None,
        &[],
        Some("human-1"),
        None,
        None,
    )
    .unwrap();
    let events = read_events_lossy(&ensure_journal().unwrap()).0;
    let strands = projection::project_strands(&events, true);
    assert_eq!(strands[0].slug.as_deref(), Some("human-1"));

    let hex = cmd_add_with_parent_and_slug(
        Some("bad hex slug"),
        false,
        None,
        false,
        None,
        &[],
        Some("deadbeef"),
        None,
        None,
    );
    assert!(hex.unwrap_err().contains("pure hex"));

    let duplicate = cmd_add_with_parent_and_slug(
        Some("duplicate slug"),
        false,
        None,
        false,
        None,
        &[],
        Some("human-1"),
        None,
        None,
    );
    assert!(duplicate.unwrap_err().contains("already exists"));
}

#[test]
fn rationale_strand_ambiguity_does_not_fallback_to_entry_hash() {
    let _env = setup();
    let events = vec![
        Event::StrandCreated {
            id: "aa111".to_string(),
            ts: "2026-01-01T00:00:00Z".to_string(),
            strand_type: None,
            slug: None,
        },
        Event::LogAppended {
            id: "aa111".to_string(),
            ts: "2026-01-01T00:00:01Z".to_string(),
            content: "first ambiguous strand".to_string(),
            effect: None,
            prev_entry_id: None,
            entry_id: Some("aaentryfallback".to_string()),
            refs: Vec::new(),
            ref_: None,
            append_id: None,
            git: None,
            provenance: None,
        },
        Event::StrandCreated {
            id: "aa222".to_string(),
            ts: "2026-01-01T00:00:02Z".to_string(),
            strand_type: None,
            slug: None,
        },
        Event::LogAppended {
            id: "aa222".to_string(),
            ts: "2026-01-01T00:00:03Z".to_string(),
            content: "second ambiguous strand".to_string(),
            effect: None,
            prev_entry_id: None,
            entry_id: Some("otherentry".to_string()),
            refs: Vec::new(),
            ref_: None,
            append_id: None,
            git: None,
            provenance: None,
        },
    ];
    append_events(&events).unwrap();

    let result = cmd_add_with_parent_and_slug(
        Some("derived"),
        false,
        None,
        false,
        None,
        &["aa"],
        None,
        None,
        None,
    );
    let err = result.unwrap_err();
    assert!(err.contains("ambiguous"), "{err}");
    assert!(err.contains("aa111"));
    assert!(err.contains("aa222"));
}

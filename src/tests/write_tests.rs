use super::*;

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

#[test]
fn positional_with_legacy_id() {
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
    assert!(result.is_ok());
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
fn content_source_none() {
    let _env = setup();
    create_strand("first strand");
    let result = cmd_append(None, None, false, false, None, None, None, None);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("content source"));
}

#[test]
fn content_source_conflict_positional_and_stdin() {
    let _env = setup();
    create_strand("first strand");
    let result = cmd_append(Some("content"), None, false, true, None, None, None, None);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("only one content source"));
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
    assert!(result.unwrap_err().contains("only one content source"));
}

#[test]
fn stdin_positional_strand_id_warns_to_use_explicit_id() {
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
    assert!(err.starts_with("warn:"));
    assert!(err.contains("require --id"));
}

#[test]
fn file_positional_strand_id_warns_to_use_explicit_id() {
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
    assert!(err.starts_with("warn:"));
    assert!(err.contains("require --id"));
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
    assert!(result.unwrap_err().contains("only one content source"));
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
    assert!(result.unwrap_err().contains("only one content source"));
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
            append_id,
            ..
        } = e
        {
            event_id == &id
                && content.contains("[checkpoint] ok")
                && content.contains("resolved_by=\"explicit --id\"")
                && content.contains("observed_entries_before_append=1")
                && content.contains("action=\"git commit checkpoint work\"")
                && append_id.is_some()
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
        Some(&id),
        false,
        false,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("step two"),
        Some(&id),
        false,
        false,
        None,
        None,
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
    cmd_close(&id, Some("done"), false).unwrap();
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
        None,
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

// cmd_context default (include_hidden=false) MUST NOT surface hidden
// prompt-strands via the cmd_context call path. Regression for the
// 'flag plumbed but projection ignores it' bug caught during
// hygiene review of 66f668e.

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
fn add_no_content_source_errors() {
    let _env = setup();
    let result = cmd_add(None, false, None, false, None, None);
    assert!(result.is_err(), "add with no content source must error");
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

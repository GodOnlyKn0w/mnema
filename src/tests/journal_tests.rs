use super::*;

#[test]
fn provenance_defaults_to_env_producer_when_set() {
    let _lock = ENV_LOCK.lock().unwrap();
    let prev = std::env::var("MNEMA_PRODUCER").ok();
    unsafe { std::env::set_var("MNEMA_PRODUCER", "codex") };
    assert_eq!(
        parse_provenance_arg(None).unwrap(),
        Some(serde_json::json!({ "producer": "codex" }))
    );
    // Explicit --provenance always overrides the env default.
    assert_eq!(
        parse_provenance_arg(Some(r#"{"producer":"claude"}"#)).unwrap(),
        Some(serde_json::json!({ "producer": "claude" }))
    );
    // Blank env → no default (treated as unset).
    unsafe { std::env::set_var("MNEMA_PRODUCER", "   ") };
    assert_eq!(parse_provenance_arg(None).unwrap(), None);
    match prev {
        Some(v) => unsafe { std::env::set_var("MNEMA_PRODUCER", v) },
        None => unsafe { std::env::remove_var("MNEMA_PRODUCER") },
    }
}

#[test]
fn test_resolve_journal_walkup_finds_parent() {
    // TestEnv sets cwd to temp dir with .mnema/ (the "project root").
    // Create a subdir and verify walk-up still finds the project journal.
    let env = setup();
    let subdir = env.path().join("subdir");
    fs::create_dir(&subdir).unwrap();
    let prev_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&subdir).unwrap();
    let result = with_mnema_home(None, || resolve_journal_dir());
    std::env::set_current_dir(&prev_cwd).unwrap();
    let resolved = result.unwrap();
    // The resolved journal must be the project one, NOT a subdir one.
    assert!(resolved.is_dir(), "resolved path must be a directory");
    assert!(
        resolved.join("journal.jsonl").exists() || resolved.join("journal.lock").exists(),
        "resolved dir must look like a journal dir"
    );
}

#[test]
fn test_resolve_journal_no_journal_errors() {
    // Set cwd to a temp dir with NO .mnema/, no parent has one either.
    let _lock = CWD_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let prev_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    let result = with_mnema_home(None, || resolve_journal_dir());
    std::env::set_current_dir(&prev_cwd).unwrap();
    assert!(result.is_err(), "should error when no .mnema/ found");
    let err = result.unwrap_err();
    assert!(
        err.contains(".mnema/ not found"),
        "unexpected error: {}",
        err
    );
}

#[test]
fn test_resolve_journal_mnema_home_absolute() {
    // MNEMA_HOME pointing to a dir with .mnema/ must win over walk-up.
    let env = setup();
    with_mnema_home(Some(env.path().to_str().unwrap()), || {
        let resolved = resolve_journal_dir().unwrap();
        assert!(
            resolved.ends_with(JOURNAL_DIR),
            "resolved should end with .mnema, got {:?}",
            resolved
        );
    });
}

#[test]
fn test_resolve_journal_mnema_home_missing_dir_errors() {
    // MNEMA_HOME pointing to a dir WITHOUT .mnema/ must error.
    let _lock = CWD_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    with_mnema_home(Some(dir.path().to_str().unwrap()), || {
        let result = resolve_journal_dir();
        assert!(
            result.is_err(),
            "should error when MNEMA_HOME dir has no .mnema/"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("MNEMA_HOME"),
            "error must mention MNEMA_HOME: {}",
            err
        );
    });
}

#[test]
fn test_resolve_journal_mnema_home_relative() {
    // Relative MNEMA_HOME must resolve against cwd.
    let env = setup();
    let dir_name = env.path().file_name().unwrap().to_str().unwrap();
    let prev_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(env.path().parent().unwrap()).unwrap();
    let result = with_mnema_home(Some(dir_name), || resolve_journal_dir());
    std::env::set_current_dir(&prev_cwd).unwrap();
    assert!(
        result.is_ok(),
        "relative MNEMA_HOME should resolve: {:?}",
        result
    );
}

#[test]
fn test_resolve_journal_walkup_stops_at_root() {
    // Walk-up must terminate (not infinite loop) even when no .mnema/ exists.
    let _lock = CWD_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let prev_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    let result = with_mnema_home(None, || resolve_journal_dir());
    std::env::set_current_dir(&prev_cwd).unwrap();
    assert!(result.is_err(), "should error, not infinite loop");
}

// ─────────────────────────────────────────────────────────────────
// -C / --chdir global flag tests
// ─────────────────────────────────────────────────────────────────

// -C parses correctly from ['mnema', '-C', 'X', 'orient']

#[test]
fn test_ensure_journal_uses_resolver() {
    // After refactor, ensure_journal must go through resolve_journal_dir().
    // Smoke test: from a subdir, it returns a path inside the project .mnema/.
    let env = setup();
    let subdir = env.path().join("nested").join("deeper");
    fs::create_dir_all(&subdir).unwrap();
    let prev_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&subdir).unwrap();
    let path = with_mnema_home(None, || ensure_journal());
    std::env::set_current_dir(&prev_cwd).unwrap();
    let path = path.unwrap();
    assert!(
        path.ends_with("journal.jsonl"),
        "must end with journal.jsonl, got {:?}",
        path
    );
    assert!(
        path.parent().unwrap().file_name().unwrap() == ".mnema",
        "parent must be .mnema/, got {:?}",
        path.parent()
    );
}

#[test]
fn journal_delta_reflects_other_strand_entries() {
    let _env = setup();
    let id_a = create_strand("strand A");
    let id_b = create_strand("strand B");
    // Add two entries to B after A was last touched.
    cmd_append(
        Some("b-entry-1"),
        None,
        false,
        false,
        None,
        Some(&id_b),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("b-entry-2"),
        None,
        false,
        false,
        None,
        Some(&id_b),
        None,
        None,
    )
    .unwrap();

    // Compute delta for strand A (before any checkpoint write).
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let strand_a = strands.iter().find(|s| s.id == id_a).unwrap();
    let max_offset = events.last().map(|(o, _)| *o).unwrap_or(0);
    let delta = max_offset.saturating_sub(strand_a.last_offset());
    // The two entries on B occurred after A's last offset → delta >= 2.
    assert!(delta >= 2, "delta must be >= 2, got {}", delta);
}

#[test]
fn append_with_provenance_stores_it() {
    let _env = setup();
    let id = create_strand("target");
    let prov = Some(serde_json::json!({ "producer": "pi", "model": "gpt-5" }));
    let event = event::make_log_appended(&id, "provenance test", prov);
    with_journal_write_lock(|j| append_event_unlocked(j, &event)).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::LogAppended {
            content,
            provenance,
            ..
        } = e
        {
            content == "provenance test" && provenance.is_some()
        } else {
            false
        }
    });
    assert!(found, "provenance must be stored on the event");
}

#[test]
fn append_without_provenance_has_none() {
    let _env = setup();
    let id = create_strand("target");
    let event = event::make_log_appended(&id, "no provenance", None);
    with_journal_write_lock(|j| append_event_unlocked(j, &event)).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::LogAppended {
            content,
            provenance,
            ..
        } = e
        {
            content == "no provenance" && provenance.is_none()
        } else {
            false
        }
    });
    assert!(found, "append without provenance must have provenance=None");
}

#[test]
fn provenance_serializes_only_when_present() {
    // Verify that serialized JSON doesn't contain provenance key when None.
    let event = event::make_log_appended("test", "no prov", None);
    let json = serde_json::to_string(&event).unwrap();
    assert!(
        !json.contains("provenance"),
        "None provenance must not appear in JSON: {}",
        json
    );
    let with_prov =
        event::make_log_appended("test", "has prov", Some(serde_json::json!({ "k": "v" })));
    let json2 = serde_json::to_string(&with_prov).unwrap();
    assert!(
        json2.contains("provenance"),
        "Some provenance must appear in JSON: {}",
        json2
    );
}

#[test]
fn old_journal_line_still_deserializes() {
    // A LogAppended event serialized by an older version (no provenance field)
    // must still parse to Event with provenance=None.
    let old_line = r#"{"type":"log_appended","id":"abc","ts":"2026-01-01T00:00:00Z","content":"old entry","append_id":"deadbeef"}"#;
    let event: Event = serde_json::from_str(old_line).unwrap();
    match &event {
        Event::LogAppended {
            content,
            provenance,
            ..
        } => {
            assert_eq!(content, "old entry");
            assert!(
                provenance.is_none(),
                "old journal must deserialize with provenance=None"
            );
        }
        _ => panic!("expected LogAppended"),
    }
}

#[test]
fn find_strand_rejects_empty_id() {
    let _env = setup();
    let id = create_strand("real strand");
    let (events, _) = read_events_lossy(&ensure_journal().unwrap());
    // empty / whitespace id must NOT silently resolve to the first strand
    assert_eq!(
        find_strand(&events, ""),
        None,
        "empty id must not match any strand"
    );
    assert_eq!(
        find_strand(&events, "   "),
        None,
        "whitespace id must not match"
    );
    // a real prefix still resolves
    assert_eq!(find_strand(&events, &id[..8]), Some(id.clone()));
}

#[test]
fn v2_journal_anchor_written_after_write_and_doctor_verifies() {
    let _env = setup();
    create_strand("anchored strand");
    let path = ensure_journal().unwrap();
    let read = read_journal_lossy(&path);
    let events: Vec<Event> = read.events.iter().map(|(_, event)| event.clone()).collect();
    let anchor_count = events
        .iter()
        .filter(|event| matches!(event, Event::JournalAnchored { .. }))
        .count();
    assert!(anchor_count >= 1, "write transaction must append an anchor");

    let total_lines = fs::read_to_string(&path).unwrap().lines().count();
    let report = diagnostics::build_doctor_journal_report(
        &events,
        total_lines,
        read.skipped(),
        0,
        0,
        chrono::Utc::now(),
    );

    assert_eq!(report.integrity.anchor_count, anchor_count);
    assert!(
        !report.integrity.has_errors(),
        "fresh anchored journal must verify: {:?}",
        report.integrity
    );
    assert_eq!(report.integrity.unanchored_event_count, 0);
}

#[test]
fn doctor_integrity_detects_tampered_anchored_entry() {
    let _env = setup();
    create_strand("anchor tamper source");
    let path = ensure_journal().unwrap();
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.contains("anchor tamper source"));
    let tampered = raw.replacen("anchor tamper source", "anchor tamper edited", 1);
    fs::write(&path, tampered).unwrap();

    let read = read_journal_lossy(&path);
    let events: Vec<Event> = read.events.iter().map(|(_, event)| event.clone()).collect();
    let total_lines = fs::read_to_string(&path).unwrap().lines().count();
    let report = diagnostics::build_doctor_journal_report(
        &events,
        total_lines,
        read.skipped(),
        0,
        0,
        chrono::Utc::now(),
    );

    assert!(report.has_errors(), "tampering must be an integrity error");
    assert!(
        !report.integrity.chain_errors.is_empty() || !report.integrity.anchor_errors.is_empty(),
        "tampering must surface in integrity details: {:?}",
        report.integrity
    );
}

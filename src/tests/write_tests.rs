use super::*;

#[test]
fn append_explicit_empty_id_errors() {
    let _env = setup();
    create_strand("existing strand");
    let result = cmd_append(
        Some("must not fallback"),
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
fn positional_with_explicit_id() {
    let _env = setup();
    let id1 = create_strand("first strand");
    let result = cmd_append(
        Some("explicit id test"),
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
    let result = cmd_append(Some("   "), false, None, None, None, None);
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
    let result = cmd_append(None, false, None, None, None, None);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("content source"));
}

#[test]
fn content_source_conflict_positional_and_stdin() {
    let _env = setup();
    create_strand("first strand");
    let result = cmd_append(Some("content"), true, None, None, None, None);
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
        true,
        Some(file_path.to_str().unwrap()),
        None,
        None,
        None,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("only one content source"));
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
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    let result = cmd_append_with_seen_offset(
        Some("[progress] write with stale seen offset"),
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
fn add_provenance_stored_on_first_log_entry() {
    let _env = setup();
    cmd_add(AddRequest {
        content: Some("add prov test"),
        provenance_raw: Some(r#"{"producer":"tester"}"#),
        ..Default::default()
    })
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
    cmd_add(AddRequest {
        content: Some("add no prov"),
        ..Default::default()
    })
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
    cmd_add(AddRequest {
        content: Some("child line"),
        parent: Some(&parent),
        ..Default::default()
    })
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
    let result = cmd_add(AddRequest {
        content: Some("orphan child"),
        parent: Some("0000019dd34b"),
        ..Default::default()
    });
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
    let result = cmd_add(AddRequest {
        content: Some("child"),
        parent: Some("  "),
        ..Default::default()
    });
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("--parent cannot be empty"));
}
// ── ③ --edge-type: renamed flag still resolves correctly ─────────────

#[test]
fn add_positional_content_creates_strand() {
    let _env = setup();
    // Positional content: existing path
    let result = cmd_add(AddRequest {
        content: Some("add positional"),
        ..Default::default()
    });
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
    let result = cmd_add(AddRequest {
        file: Some(path_str),
        ..Default::default()
    });
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
    let result = cmd_add(AddRequest {
        content: Some("content"),
        stdin: true,
        ..Default::default()
    });
    assert!(result.is_err(), "add with two content sources must error");
}

#[test]
fn add_no_content_source_errors() {
    let _env = setup();
    let result = cmd_add(AddRequest::default());
    assert!(result.is_err(), "add with no content source must error");
}

#[test]
fn add_empty_file_content_errors() {
    let env = setup();
    let file_path = env.path().join("empty.md");
    fs::write(&file_path, "").unwrap();
    let path_str = file_path.to_str().unwrap();
    let result = cmd_add(AddRequest {
        file: Some(path_str),
        ..Default::default()
    });
    assert!(result.is_err(), "add --file with empty file must error");
}

#[test]
fn add_nonexistent_file_errors() {
    let _env = setup();
    let result = cmd_add(AddRequest {
        file: Some("/nonexistent/path/to/file.txt"),
        ..Default::default()
    });
    assert!(
        result.is_err(),
        "add --file with nonexistent file must error"
    );
}

// ── W073: typo marker suggestion ─────────────────────────────────────────

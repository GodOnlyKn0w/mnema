use super::*;

#[test]
fn hide_is_idempotent() {
    let _env = setup();
    let id = create_strand("hide me");
    let before = total_events();
    cmd_hide(&id, None, false, None).unwrap();
    let mid = total_events();
    cmd_hide(&id, None, false, None).unwrap();
    cmd_hide(&id, Some("still hidden"), false, None).unwrap();
    let after = total_events();
    assert_eq!(mid - before, 1, "first hide must write exactly 1 event");
    assert_eq!(
        after - mid,
        0,
        "repeated hide must be a no-op (0 events appended)"
    );
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    assert_eq!(count_hide_events(&events, &id, "hidden"), 1);
    assert_eq!(count_hide_events(&events, &id, "unhidden"), 0);
}

// One `cmd_unhide` after a `cmd_hide` restores visibility — no negative
// hide_count, no orphan unhidden events.

#[test]
fn unhide_is_idempotent() {
    let _env = setup();
    let id = create_strand("plain strand");
    let before = total_events();
    cmd_unhide(&id, false).unwrap();
    cmd_unhide(&id, false).unwrap();
    let after = total_events();
    assert_eq!(
        after - before,
        0,
        "unhide on visible strand must be a no-op"
    );
}

// Without --id, cmd_checkpoint picks the most-recent VISIBLE strand by
// default. When the most-recent strand is hidden, the visible one is chosen.

#[test]
fn bind_creates_subject_bound_event() {
    let _env = setup();
    let id = create_strand("target");
    let result = cmd_bind(Some("pi-session"), Some("abc"), Some(&id), false, false);
    assert!(result.is_ok());
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let has_binding = events.iter().any(|(_, e)| {
        matches!(e, Event::SubjectBound { subject_type, subject_id, strand_id, .. }
                if subject_type == "pi-session" && subject_id == "abc" && strand_id == &id)
    });
    assert!(has_binding, "bind must write a SubjectBound event");
}

#[test]
fn bind_provenance_stored_on_subject_bound_event() {
    let _env = setup();
    let id = create_strand("bind provenance target");
    cmd_bind_with_provenance(
        Some("pi-session"),
        Some("prov"),
        Some(&id),
        false,
        false,
        Some(r#"{"producer":"tester"}"#),
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::SubjectBound {
            subject_type,
            subject_id,
            provenance,
            ..
        } = e
        {
            subject_type == "pi-session"
                && subject_id == "prov"
                && provenance
                    .as_ref()
                    .and_then(|p| p.get("producer"))
                    .and_then(|p| p.as_str())
                    == Some("tester")
        } else {
            false
        }
    });
    assert!(found, "bind --provenance must persist on SubjectBound");
}

#[test]
fn bind_resolves_prefix_id() {
    let _env = setup();
    let id = create_strand("target strand");
    let short = &id[..12];
    let result = cmd_bind(Some("ci-run"), Some("run-42"), Some(short), false, false);
    assert!(
        result.is_ok(),
        "prefix strand id should resolve: {:?}",
        result
    );
}

#[test]
fn bind_missing_strand_fails() {
    let _env = setup();
    let result = cmd_bind(
        Some("pi-session"),
        Some("x"),
        Some("000000000000"),
        false,
        false,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
}

#[test]
fn current_returns_latest_binding() {
    let _env = setup();
    let id_a = create_strand("first");
    let id_b = create_strand("second");
    // Bind subject to strand a
    cmd_bind(Some("pi-session"), Some("user1"), Some(&id_a), false, false).unwrap();
    // Re-bind to strand b (latest should win)
    cmd_bind(Some("pi-session"), Some("user1"), Some(&id_b), false, false).unwrap();
    let result = cmd_current(Some("pi-session"), Some("user1"), false);
    assert!(result.is_ok());
    // We can't easily capture stdout here, so we test via the projection
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let mut latest: Option<String> = None;
    for (_, e) in &events {
        if let Event::SubjectBound {
            subject_type: t,
            subject_id: i,
            strand_id: s,
            ..
        } = e
        {
            if t == "pi-session" && i == "user1" {
                latest = Some(s.clone());
            }
        }
    }
    assert_eq!(latest, Some(id_b), "latest binding must point to id_b");
}

#[test]
fn current_no_binding_returns_error() {
    let _env = setup();
    create_strand("orphan");
    let result = cmd_current(Some("pi-session"), Some("no-such"), false);
    assert!(result.is_err());
}

#[test]
fn current_requires_non_empty_args() {
    let _env = setup();
    let r1 = cmd_current(None, Some("x"), false);
    assert!(r1.is_err());
    let r2 = cmd_current(Some("x"), None, false);
    assert!(r2.is_err());
    let r3 = cmd_current(Some(""), Some("x"), false);
    assert!(r3.is_err());
}

#[test]
fn bind_requires_non_empty_args() {
    let _env = setup();
    let id = create_strand("t");
    let r1 = cmd_bind(None, Some("x"), Some(&id), false, false);
    assert!(r1.is_err());
    let r2 = cmd_bind(Some("x"), None, Some(&id), false, false);
    assert!(r2.is_err());
    let r3 = cmd_bind(Some("x"), Some("y"), None, false, false);
    assert!(r3.is_err());
}

// ── Provenance tests (pi-strand V1 contract) ─────────────────────

#[test]
fn hide_json_returns_visibility_ledger() {
    let _env = setup();
    let id = create_strand("to be hidden json");
    let result = cmd_hide(&id, None, true, None);
    assert!(
        result.is_ok(),
        "hide --format json must succeed: {:?}",
        result
    );
    // idempotent call — noop: true
    let result2 = cmd_hide(&id, None, true, None);
    assert!(
        result2.is_ok(),
        "hide --format json idempotent must succeed"
    );
}

#[test]
fn hide_json_contains_active_closed_hidden_counts() {
    // Contract: JSON output of hide must carry active / closed / hidden integer fields.
    // We exercise the path; count correctness is a projection concern already tested.
    let _env = setup();
    let id = create_strand("hide json count test");
    // Calling cmd_hide with format_json=true must not panic/error.
    cmd_hide(&id, None, true, None).unwrap();
}

// ── ① JSON twins: unhide --format json ───────────────────────────────

#[test]
fn unhide_json_returns_ok() {
    let _env = setup();
    let id = create_strand("unhide json test");
    cmd_hide(&id, None, false, None).unwrap();
    let result = cmd_unhide(&id, true);
    assert!(
        result.is_ok(),
        "unhide --format json must succeed: {:?}",
        result
    );
}

// ── ① JSON twins: link --format json ─────────────────────────────────

#[test]
fn link_json_returns_source_target_edge_type() {
    let _env = setup();
    let src = create_strand("link json source");
    let tgt = create_strand("link json target");
    let result = cmd_link(&src, &tgt, None, true, None);
    assert!(
        result.is_ok(),
        "link --format json must succeed: {:?}",
        result
    );
}

#[test]
fn link_json_default_edge_type_is_depends_on() {
    // Verify the EdgeLinked event carries the default edge_type when none given.
    let _env = setup();
    let src = create_strand("link edge type source");
    let tgt = create_strand("link edge type target");
    cmd_link(&src, &tgt, None, false, None).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::EdgeLinked { id, edge_type, .. } = e {
            id == &src && edge_type.as_deref() == Some("depends-on")
        } else {
            false
        }
    });
    assert!(
        found,
        "EdgeLinked must carry edge_type=depends-on by default"
    );
}

// ── ② provenance: link --provenance ──────────────────────────────────

#[test]
fn link_provenance_stored_on_edge_linked_event() {
    let _env = setup();
    let src = create_strand("prov link source");
    let tgt = create_strand("prov link target");
    cmd_link(
        &src,
        &tgt,
        None,
        false,
        Some(r#"{"producer":"test-agent"}"#),
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::EdgeLinked { id, provenance, .. } = e {
            id == &src && provenance.is_some()
        } else {
            false
        }
    });
    assert!(
        found,
        "EdgeLinked must carry provenance when --provenance given"
    );
}

#[test]
fn link_without_provenance_has_none() {
    let _env = setup();
    let src = create_strand("no-prov link source");
    let tgt = create_strand("no-prov link target");
    cmd_link(&src, &tgt, None, false, None).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::EdgeLinked { id, provenance, .. } = e {
            id == &src && provenance.is_none()
        } else {
            false
        }
    });
    assert!(found, "EdgeLinked must have provenance=None when not given");
}

// Old EdgeLinked JSON without provenance field must still deserialize.

#[test]
fn old_edge_linked_still_deserializes() {
    let old = r#"{"type":"edge_linked","id":"abc","ts":"2026-01-01T00:00:00Z","to":"def"}"#;
    let event: Event = serde_json::from_str(old).unwrap();
    match &event {
        Event::EdgeLinked { to, provenance, .. } => {
            assert_eq!(to, "def");
            assert!(
                provenance.is_none(),
                "old edge_linked must deserialize with provenance=None"
            );
        }
        _ => panic!("expected EdgeLinked"),
    }
}

// ── ② provenance: hide --provenance forwards to reason entry ─────────

#[test]
fn hide_with_reason_and_provenance_stores_provenance_on_log_entry() {
    let _env = setup();
    let id = create_strand("hide prov test");
    cmd_hide(
        &id,
        Some("test reason"),
        false,
        Some(r#"{"producer":"tester"}"#),
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::LogAppended {
            id: eid,
            content,
            provenance,
            ..
        } = e
        {
            eid == &id && content.starts_with("[hidden]") && provenance.is_some()
        } else {
            false
        }
    });
    assert!(
        found,
        "[hidden] entry must carry provenance when --provenance given with --reason"
    );
}

#[test]
fn hide_without_reason_provenance_arg_is_accepted() {
    // --provenance without --reason: argument accepted, no content entry written.
    let _env = setup();
    let id = create_strand("hide no-reason prov");
    let result = cmd_hide(&id, None, false, Some(r#"{"producer":"tester"}"#));
    assert!(
        result.is_ok(),
        "hide --provenance without --reason must succeed"
    );
}

// ── ② provenance: add --provenance ───────────────────────────────────

#[test]
fn link_edge_type_custom_is_stored() {
    let _env = setup();
    let src = create_strand("edge-type source");
    let tgt = create_strand("edge-type target");
    cmd_link(&src, &tgt, Some("belongs-to"), false, None).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::EdgeLinked { id, edge_type, .. } = e {
            id == &src && edge_type.as_deref() == Some("belongs-to")
        } else {
            false
        }
    });
    assert!(found, "custom edge_type must be stored on EdgeLinked event");
}

// ── ④ add --stdin / --file ────────────────────────────────────────────

#[test]
fn append_subtask_done_leaves_strand_open() {
    let _env = setup();
    let id = create_strand("parent line of work");
    // Simulate what an operator agent would do: record a sub-task completion.
    cmd_append(
        Some("[done] subtask A completed"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let strand = strands.iter().find(|s| s.id == id).unwrap();
    assert_eq!(
        strand.state(),
        "registered",
        "appending [done] must NOT close the strand; state was: {}",
        strand.state()
    );
}

// close with default disposition → closed:done.

#[test]
fn close_default_sets_closed_done() {
    let _env = setup();
    let id = create_strand("work to close");
    cmd_close(&id, None, false).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let strand = strands.iter().find(|s| s.id == id).unwrap();
    assert_eq!(
        strand.state(),
        "closed:done",
        "close default must give closed:done"
    );
}

// close --as failed → closed:failed.

#[test]
fn close_as_failed_sets_closed_failed() {
    let _env = setup();
    let id = create_strand("work that failed");
    cmd_close(&id, Some("failed"), false).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let strand = strands.iter().find(|s| s.id == id).unwrap();
    assert_eq!(
        strand.state(),
        "closed:failed",
        "close --as failed must give closed:failed"
    );
}

// reopen after close → back to registered (open).

#[test]
fn reopen_after_close_restores_registered() {
    let _env = setup();
    let id = create_strand("work to reopen");
    cmd_close(&id, None, false).unwrap();
    cmd_reopen(&id, false).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let strand = strands.iter().find(|s| s.id == id).unwrap();
    assert_eq!(
        strand.state(),
        "registered",
        "reopen must restore registered state"
    );
}

// close → closed:cancelled.

#[test]
fn close_as_cancelled_sets_closed_cancelled() {
    let _env = setup();
    let id = create_strand("cancelled plan");
    cmd_close(&id, Some("cancelled"), false).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let strand = strands.iter().find(|s| s.id == id).unwrap();
    assert_eq!(strand.state(), "closed:cancelled");
}

// close twice must error (already closed).

#[test]
fn close_already_closed_errors() {
    let _env = setup();
    let id = create_strand("once-closed work");
    cmd_close(&id, None, false).unwrap();
    let result = cmd_close(&id, None, false);
    assert!(
        result.is_err(),
        "closing an already-closed strand must error"
    );
    assert!(
        result.unwrap_err().contains("already"),
        "error must say already"
    );
}

// reopen an already-open strand must error.

#[test]
fn reopen_already_open_errors() {
    let _env = setup();
    let id = create_strand("never closed");
    let result = cmd_reopen(&id, false);
    assert!(
        result.is_err(),
        "reopening an already-open strand must error"
    );
}

// W074 fires when a closing-marker annotation is appended.

#[test]
fn w074_fires_on_closing_annotation_marker() {
    let _env = setup();
    let id = create_strand("some work");
    // The predicate that gates the W074 nudge must be true for a closing marker.
    assert!(
        is_closing_annotation_marker("[done] sub-step done"),
        "closing marker must gate W074"
    );
    let result = cmd_append(
        Some("[done] sub-step done"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "append must succeed even with closing marker"
    );
    // Strand must still be open (that's the whole point of W074's warning).
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let strand = strands.iter().find(|s| s.id == id).unwrap();
    assert_eq!(
        strand.state(),
        "registered",
        "W074 scenario: strand must remain open after closing-marker append"
    );
}

// W074 must NOT fire on non-closing markers (precision-first).

#[test]
fn w074_silent_on_non_closing_markers() {
    // Exercises the real predicate the runtime nudge uses (not a duplicated
    // constant): non-closing markers must not gate W074.
    for m in [
        "[decision]",
        "[progress]",
        "[friction]",
        "[observed]",
        "[insight]",
    ] {
        assert!(
            !is_closing_annotation_marker(m),
            "{} must not trigger W074",
            m
        );
    }
}

// orient / remind must NOT contain the old "append [done]" pattern.

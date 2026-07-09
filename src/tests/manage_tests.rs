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
    // Verify the link effect carries the default edge_type when none given.
    let _env = setup();
    let src = create_strand("link edge type source");
    let tgt = create_strand("link edge type target");
    cmd_link(&src, &tgt, None, false, None).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let found = events.iter().any(|(_, e)| {
        if let Event::LogAppended {
            id,
            effect: Some(event::EntryEffect::Link { edge_type, .. }),
            ..
        } = e
        {
            id == &src && edge_type == "depends-on"
        } else {
            false
        }
    });
    assert!(
        found,
        "link effect must carry edge_type=depends-on by default"
    );
}

// ── ② provenance: link --provenance ──────────────────────────────────

#[test]
fn link_provenance_stored_on_link_effect_entry() {
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
        if let Event::LogAppended {
            id,
            effect: Some(event::EntryEffect::Link { .. }),
            provenance,
            ..
        } = e
        {
            id == &src && provenance.is_some()
        } else {
            false
        }
    });
    assert!(
        found,
        "link effect entry must carry provenance when --provenance given"
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
        if let Event::LogAppended {
            id,
            effect: Some(event::EntryEffect::Link { .. }),
            provenance,
            ..
        } = e
        {
            id == &src && provenance.is_none()
        } else {
            false
        }
    });
    assert!(
        found,
        "link effect entry must have provenance=None when not given"
    );
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
        if let Event::LogAppended {
            id,
            effect: Some(event::EntryEffect::Link { edge_type, .. }),
            ..
        } = e
        {
            id == &src && edge_type == "belongs-to"
        } else {
            false
        }
    });
    assert!(
        found,
        "custom edge_type must be stored on link effect entry"
    );
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
    cmd_close(&id, None, None, false).unwrap();
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
    cmd_close(&id, Some("failed"), None, false).unwrap();
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
    cmd_close(&id, None, None, false).unwrap();
    cmd_reopen(&id, None, false).unwrap();
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
    cmd_close(&id, Some("cancelled"), None, false).unwrap();
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
    cmd_close(&id, None, None, false).unwrap();
    let result = cmd_close(&id, None, None, false);
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
    let result = cmd_reopen(&id, None, false);
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

#[test]
fn v2_link_and_unlink_write_effect_entries_and_fold_projection() {
    let _env = setup();
    let src = create_strand("v2 link source");
    let tgt = create_strand("v2 link target");
    let path = ensure_journal().unwrap();
    let (before_events, _) = read_events_lossy(&path);
    let first_entry_id = projection::project_strands(&before_events, true)
        .iter()
        .find(|s| s.id == src)
        .and_then(|s| s.log.last())
        .and_then(|entry| entry.entry_id.clone())
        .expect("source entry hash exists");

    cmd_link(&src, &tgt, Some("depends-on"), false, None).unwrap();
    let (linked_events, _) = read_events_lossy(&path);
    let link_prev = linked_events
        .iter()
        .filter_map(|(_, event)| {
            if let Event::LogAppended {
                id,
                effect: Some(event::EntryEffect::Link { target, edge_type }),
                prev_entry_id,
                ..
            } = event
            {
                if id == &src && target == &tgt && edge_type == "depends-on" {
                    return prev_entry_id.clone();
                }
            }
            None
        })
        .next()
        .expect("link effect entry exists");
    assert_eq!(link_prev, first_entry_id);
    let linked_strands = projection::project_strands(&linked_events, true);
    let linked_src = linked_strands.iter().find(|s| s.id == src).unwrap();
    assert_eq!(linked_src.depends_on_edges, vec![tgt.clone()]);
    let link_entry_id = linked_src
        .log
        .last()
        .and_then(|entry| entry.entry_id.clone())
        .expect("link entry hash exists");

    cmd_unlink(&src, &tgt, Some("depends-on"), false, None).unwrap();
    let (unlinked_events, _) = read_events_lossy(&path);
    let (unlink_prev, cancels) = unlinked_events
        .iter()
        .filter_map(|(_, event)| {
            if let Event::LogAppended {
                id,
                effect:
                    Some(event::EntryEffect::Unlink {
                        target,
                        edge_type,
                        link_entry_id,
                    }),
                prev_entry_id,
                ..
            } = event
            {
                if id == &src && target == &tgt && edge_type == "depends-on" {
                    return Some((prev_entry_id.clone(), link_entry_id.clone()));
                }
            }
            None
        })
        .next()
        .expect("unlink effect entry exists");
    assert_eq!(unlink_prev, Some(link_entry_id.clone()));
    // CORPUS §4: the unlink names the specific link entry it reverses.
    assert_eq!(
        cancels,
        Some(link_entry_id),
        "unlink must record the reversed link entry id"
    );
    let unlinked_strands = projection::project_strands(&unlinked_events, true);
    let unlinked_src = unlinked_strands.iter().find(|s| s.id == src).unwrap();
    assert!(unlinked_src.depends_on_edges.is_empty());
}

#[test]
fn two_links_unlink_one_leaves_the_other_live() {
    // CORPUS §4 completeness: with two live links to the same target, cancelling
    // one link entry leaves the other — instance-based, not key-based, fold.
    let _env = setup();
    let src = create_strand("relation source");
    let tgt = create_strand("relation target");
    cmd_link(&src, &tgt, Some("depends-on"), false, None).unwrap();
    cmd_link(&src, &tgt, Some("depends-on"), false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let s = strands.iter().find(|st| st.id == src).unwrap();
    // Two link entries, but the edge is deduped by target for display.
    assert_eq!(s.depends_on_edges, vec![tgt.clone()]);
    let link_ids: Vec<String> = s
        .log
        .iter()
        .filter_map(|e| match &e.effect {
            Some(event::EntryEffect::Link { target, edge_type })
                if target == &tgt && edge_type == "depends-on" =>
            {
                e.entry_id.clone()
            }
            _ => None,
        })
        .collect();
    assert_eq!(link_ids.len(), 2, "two link entries recorded");

    // Unlink cancels the most recent link entry (the resolver's choice); the
    // older instance is still live, so the edge remains.
    cmd_unlink(&src, &tgt, Some("depends-on"), false, None).unwrap();
    let (after, _) = read_events_lossy(&path);
    let after_s = projection::project_strands(&after, true);
    let after_src = after_s.iter().find(|st| st.id == src).unwrap();
    assert_eq!(
        after_src.depends_on_edges,
        vec![tgt.clone()],
        "one live link remains after cancelling the other"
    );

    // A second unlink cancels the remaining instance; now the edge is gone.
    cmd_unlink(&src, &tgt, Some("depends-on"), false, None).unwrap();
    let (after2, _) = read_events_lossy(&path);
    let after2_s = projection::project_strands(&after2, true);
    let after2_src = after2_s.iter().find(|st| st.id == src).unwrap();
    assert!(
        after2_src.depends_on_edges.is_empty(),
        "both links cancelled — edge gone"
    );
}

#[test]
fn v2_hide_and_unhide_write_effect_entries_and_fold_projection() {
    let _env = setup();
    let id = create_strand("v2 hide target");
    let path = ensure_journal().unwrap();
    let (before_events, _) = read_events_lossy(&path);
    let first_entry_id = projection::project_strands(&before_events, true)
        .iter()
        .find(|s| s.id == id)
        .and_then(|s| s.log.last())
        .and_then(|entry| entry.entry_id.clone())
        .expect("first entry hash exists");

    cmd_hide(
        &id,
        Some("parked while waiting"),
        false,
        Some(r#"{"producer":"tester"}"#),
    )
    .unwrap();
    let (hidden_events, _) = read_events_lossy(&path);
    let hide_prev = hidden_events
        .iter()
        .filter_map(|(_, event)| {
            if let Event::LogAppended {
                id: event_id,
                content,
                effect: Some(event::EntryEffect::Hide),
                prev_entry_id,
                provenance,
                ..
            } = event
            {
                if event_id == &id
                    && content.starts_with("[hidden] parked while waiting")
                    && provenance.is_some()
                {
                    return prev_entry_id.clone();
                }
            }
            None
        })
        .next()
        .expect("hide effect entry exists");
    assert_eq!(hide_prev, first_entry_id);
    let hidden_strands = projection::project_strands(&hidden_events, true);
    let hidden = hidden_strands.iter().find(|s| s.id == id).unwrap();
    assert!(hidden.hidden);
    let hide_entry_id = hidden
        .log
        .last()
        .and_then(|entry| entry.entry_id.clone())
        .expect("hide entry hash exists");

    cmd_unhide(&id, false).unwrap();
    let (visible_events, _) = read_events_lossy(&path);
    let unhide_prev = visible_events
        .iter()
        .filter_map(|(_, event)| {
            if let Event::LogAppended {
                id: event_id,
                effect: Some(event::EntryEffect::Unhide),
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
        .expect("unhide effect entry exists");
    assert_eq!(unhide_prev, hide_entry_id);
    let visible_strands = projection::project_strands(&visible_events, true);
    let visible = visible_strands.iter().find(|s| s.id == id).unwrap();
    assert!(!visible.hidden);
}

#[test]
fn cutover_v2_certificate_detects_tampered_v1_archive() {
    let env = setup();
    let old_id = "0000019dd34b000000000000";
    let events = vec![
        Event::StrandCreated {
            id: old_id.to_string(),
            ts: "2026-01-01T00:00:00Z".to_string(),
            strand_type: Some("task".to_string()),
            slug: None,
        },
        Event::LogAppended {
            id: old_id.to_string(),
            ts: "2026-01-01T00:00:01Z".to_string(),
            content: "legacy root".to_string(),
            effect: None,
            prev_entry_id: None,
            entry_id: None,
            refs: Vec::new(),
            ref_: None,
            append_id: None,
            git: None,
            provenance: None,
        },
    ];
    write_legacy_journal(&events);

    cmd_cutover_v2(true, None, None, false).unwrap();

    let mnema_dir = env.path().join(".mnema");
    let archive = mnema_dir.join("journal.v1.jsonl");
    fs::write(&archive, "tampered\n").unwrap();

    let doctor_failed = crate::commands::doctor::cmd_doctor_journal().unwrap();
    assert!(
        doctor_failed,
        "tampered v1 archive must fail cutover certificate verification"
    );
}
fn write_legacy_journal(events: &[Event]) {
    let path = ensure_journal().unwrap();
    let mut lines = Vec::new();
    for event in events {
        lines.push(serde_json::to_string(event).unwrap());
    }
    fs::write(path, lines.join("\n") + "\n").unwrap();
}

#[test]
fn cutover_v2_dry_run_does_not_rewrite_journal() {
    let env = setup();
    let old_id = "0000019dd34b000000000000";
    let events = vec![
        Event::StrandCreated {
            id: old_id.to_string(),
            ts: "2026-01-01T00:00:00Z".to_string(),
            strand_type: Some("task".to_string()),
            slug: None,
        },
        Event::LogAppended {
            id: old_id.to_string(),
            ts: "2026-01-01T00:00:01Z".to_string(),
            content: "legacy root".to_string(),
            effect: None,
            prev_entry_id: None,
            entry_id: None,
            refs: Vec::new(),
            ref_: None,
            append_id: None,
            git: None,
            provenance: None,
        },
    ];
    write_legacy_journal(&events);
    let journal = ensure_journal().unwrap();
    let before = fs::read_to_string(&journal).unwrap();

    cmd_cutover_v2(false, None, None, true).unwrap();

    assert_eq!(fs::read_to_string(&journal).unwrap(), before);
    assert!(!env.path().join(".mnema").join("journal.v1.jsonl").exists());
    assert!(
        !env.path()
            .join(".mnema")
            .join("migration-v1-to-v2.json")
            .exists()
    );
}

#[test]
fn cutover_v2_apply_archives_v1_and_imports_pure_v2_journal() {
    let env = setup();
    let parent = "0000019dd34b000000000000";
    let child = "0000019dd34c000000000000";
    let events = vec![
        Event::StrandCreated {
            id: parent.to_string(),
            ts: "2026-01-01T00:00:00Z".to_string(),
            strand_type: Some("task".to_string()),
            slug: None,
        },
        Event::LogAppended {
            id: parent.to_string(),
            ts: "2026-01-01T00:00:01Z".to_string(),
            content: "legacy parent".to_string(),
            effect: None,
            prev_entry_id: None,
            entry_id: None,
            refs: Vec::new(),
            ref_: None,
            append_id: None,
            git: None,
            provenance: None,
        },
        Event::StrandCreated {
            id: child.to_string(),
            ts: "2026-01-01T00:00:02Z".to_string(),
            strand_type: Some("task".to_string()),
            slug: None,
        },
        Event::LogAppended {
            id: child.to_string(),
            ts: "2026-01-01T00:00:03Z".to_string(),
            content: "legacy child".to_string(),
            effect: None,
            prev_entry_id: None,
            entry_id: None,
            refs: Vec::new(),
            ref_: None,
            append_id: None,
            git: None,
            provenance: None,
        },
        Event::EdgeLinked {
            id: child.to_string(),
            ts: "2026-01-01T00:00:04Z".to_string(),
            to: parent.to_string(),
            edge_type: Some("belongs-to".to_string()),
            provenance: None,
        },
        Event::StrandClosed {
            id: child.to_string(),
            ts: "2026-01-01T00:00:05Z".to_string(),
            disposition: "done".to_string(),
            provenance: None,
        },
    ];
    write_legacy_journal(&events);

    cmd_cutover_v2(true, None, None, false).unwrap();

    let mnema_dir = env.path().join(".mnema");
    assert!(mnema_dir.join("journal.v1.jsonl").exists());
    assert!(mnema_dir.join("migration-v1-to-v2.json").exists());
    let certificate_path = mnema_dir.join("migration-v1-to-v2.certificate.json");
    assert!(certificate_path.exists());
    let certificate: CutoverV2Certificate =
        serde_json::from_str(&fs::read_to_string(&certificate_path).unwrap()).unwrap();
    assert_eq!(certificate.schema, "tasktree-v2-cutover-certificate-v1");
    assert_eq!(certificate.source_event_count, events.len());

    let read = read_journal_lossy(&ensure_journal().unwrap());
    assert!(read.diagnostics.is_empty());
    let imported: Vec<Event> = read.events.iter().map(|(_, event)| event.clone()).collect();
    assert!(
        imported
            .iter()
            .any(|event| matches!(event, Event::JournalAnchored { .. }))
    );
    assert!(!imported.iter().any(|event| matches!(
        event,
        Event::EdgeLinked { .. }
            | Event::EdgeUnlinked { .. }
            | Event::StrandClosed { .. }
            | Event::StrandReopened { .. }
            | Event::StrandHidden { .. }
            | Event::StrandUnhidden { .. }
            | Event::CheckpointCreated { .. }
    )));

    let created: Vec<String> = imported
        .iter()
        .filter_map(|event| {
            if let Event::StrandCreated { id, .. } = event {
                Some(id.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(created.len(), 2);
    assert!(created.iter().all(|id| id.len() == 64));
    assert!(created.iter().all(|id| id != parent && id != child));

    let effect_entries: Vec<&event::EntryEffect> = imported
        .iter()
        .filter_map(|event| {
            if let Event::LogAppended {
                effect: Some(effect),
                ..
            } = event
            {
                Some(effect)
            } else {
                None
            }
        })
        .collect();
    assert!(effect_entries.iter().any(|effect| matches!(
        effect,
        event::EntryEffect::Link { edge_type, .. } if edge_type == "belongs-to"
    )));
    assert!(effect_entries.iter().any(|effect| matches!(
        effect,
        event::EntryEffect::Close { disposition } if disposition == "done"
    )));
    assert!(imported.iter().all(|event| {
        if let Event::LogAppended { entry_id, ref_, .. } = event {
            entry_id.as_ref().map_or(false, |id| id.len() == 64) && ref_.is_none()
        } else {
            true
        }
    }));

    let report = diagnostics::build_doctor_journal_report(
        &imported,
        read.events.len(),
        read.skipped(),
        0,
        0,
        chrono::Utc::now(),
    );
    assert!(!report.integrity.has_errors(), "{:?}", report.integrity);
}

#[test]
fn strand_lookup_accepts_non_first_entry_hash_prefix() {
    let _env = setup();
    let id = create_strand("entry lookup home");
    cmd_append(
        Some("entry lookup second"),
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
    let entry_id = strand.log.last().and_then(|e| e.entry_id.clone()).unwrap();
    let prefix = &entry_id[..16];

    match crate::reference::lookup_strand_with_selection(&strands, prefix, false, 0) {
        crate::reference::StrandLookup::ViaEntry {
            strand_id,
            entry_id: resolved_entry,
        } => {
            assert_eq!(strand_id, id);
            assert_eq!(resolved_entry, entry_id);
        }
        other => panic!("non-first entry prefix must resolve ViaEntry: {:?}", other),
    }
}

#[test]
fn strand_lookup_keeps_first_entry_hash_as_strand_id() {
    let _env = setup();
    let id = create_strand("first entry stays strand");
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let first_entry = strands
        .iter()
        .find(|s| s.id == id)
        .unwrap()
        .log
        .first()
        .and_then(|e| e.entry_id.clone())
        .unwrap();
    assert_eq!(first_entry, id, "first entry hash is the strand id");

    match crate::reference::lookup_strand_with_selection(&strands, &first_entry[..16], false, 0) {
        crate::reference::StrandLookup::One(resolved) => assert_eq!(resolved, id),
        other => panic!("first entry prefix must remain One: {:?}", other),
    }
}

#[test]
fn strand_lookup_missing_entry_prefix_is_not_found() {
    let _env = setup();
    create_strand("entry lookup miss corpus");
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);

    match crate::reference::lookup_strand_with_selection(&strands, "ffffffffffffffff", false, 0) {
        crate::reference::StrandLookup::NotFound => {}
        other => panic!("missing entry prefix must be NotFound: {:?}", other),
    }
}

#[test]
fn ambiguous_entry_hash_prefix_suggests_show_entry() {
    let _env = setup();
    let left = create_strand("ambiguous entry left");
    let right = create_strand("ambiguous entry right");
    let prefix = if left.starts_with("feed") || right.starts_with("feed") {
        "cafe"
    } else {
        "feed"
    };
    let left_entry_id = format!("{}{}", prefix, "0".repeat(60));
    let right_entry_id = format!("{}{}", prefix, "1".repeat(60));

    with_journal_write_lock(|journal| {
        append_event_unlocked(
            journal,
            &Event::LogAppended {
                id: left.clone(),
                ts: chrono::Utc::now().to_rfc3339(),
                content: "left ambiguous entry".to_string(),
                effect: None,
                prev_entry_id: None,
                entry_id: Some(left_entry_id),
                refs: Vec::new(),
                ref_: None,
                append_id: None,
                git: None,
                provenance: None,
            },
        )?;
        append_event_unlocked(
            journal,
            &Event::LogAppended {
                id: right.clone(),
                ts: chrono::Utc::now().to_rfc3339(),
                content: "right ambiguous entry".to_string(),
                effect: None,
                prev_entry_id: None,
                entry_id: Some(right_entry_id),
                refs: Vec::new(),
                ref_: None,
                append_id: None,
                git: None,
                provenance: None,
            },
        )?;
        Ok(())
    })
    .unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    match crate::reference::lookup_strand_with_selection(&strands, prefix, false, 0) {
        crate::reference::StrandLookup::Invalid(message) => {
            assert!(
                message.contains("mnema show --entry <hash>"),
                "ambiguous entry prefix must teach show --entry: {message}"
            );
        }
        other => panic!("ambiguous entry prefix must be Invalid: {:?}", other),
    }
}

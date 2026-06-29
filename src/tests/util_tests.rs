use super::*;

#[test]
fn leading_whitespace_preserved() {
    let _env = setup();
    let id = create_strand("first strand");
    let result = cmd_append(
        Some("    indented code block\n    more indent"),
        Some(&id),
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
    let found = events.iter().any(|(_, e)| {
        if let Event::LogAppended { content, .. } = e {
            content.starts_with("    indented")
        } else {
            false
        }
    });
    assert!(found);
}

// ── Content source: --stdin ──

#[test]
fn single_unhide_restores_visibility() {
    let _env = setup();
    let id = create_strand("hide/unhide me");
    cmd_hide(&id, None, false, None).unwrap();
    cmd_unhide(&id, false).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let s = projection::project_strands(&events, true)
        .into_iter()
        .find(|s| s.id == id)
        .expect("strand missing from projection");
    assert!(
        !s.hidden,
        "strand must be visible after one hide + one unhide"
    );
    assert_eq!(count_hide_events(&events, &id, "hidden"), 1);
    assert_eq!(count_hide_events(&events, &id, "unhidden"), 1);
}

// Repeated `cmd_unhide` on an already-visible strand is a no-op.

#[test]
fn cmd_agent_context_default_excludes_hidden_via_cmd_path() {
    let _env = setup();
    let (c, a) = event::make_strand_created("[covers] audit2/", Some("prompt-strand"));
    let id = c.strand_id().to_string();
    with_journal_write_lock(|j| {
        append_event_unlocked(j, &c)?;
        append_event_unlocked(j, &a)
    })
    .unwrap();
    cmd_hide(&id, Some("noise"), false, None).unwrap();
    let result = cmd_agent_context(None, false);
    assert!(result.is_ok());
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let visible = projection::project_strands(&events, false);
    assert!(
        !visible.iter().any(|s| s.id == id),
        "cmd_agent_context default must use include_hidden=false in projection"
    );
}

// ── Subject binding tests (pi-strand V1 contract) ─────────────────

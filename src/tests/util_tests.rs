use super::*;

#[test]
fn leading_whitespace_preserved() {
    let _env = setup();
    let id = create_strand("first strand");
    let result = cmd_append(
        Some("    indented code block\n    more indent"),
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


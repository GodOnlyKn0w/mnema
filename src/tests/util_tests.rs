use super::*;

#[test]
fn leading_whitespace_preserved() {
    let _env = setup();
    let id = create_strand("first strand");
    let result = cmd_append(
        Some("    indented code block\n    more indent"),
        None,
        false,
        false,
        None,
        Some(&id),
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

#[test]
fn display_ts_pairs_relative_with_absolute() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-07-04T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert_eq!(
        display_ts("2026-07-01T09:30:00Z", now),
        "3d ago(07-01 09:30)"
    );
    assert_eq!(
        display_ts("2026-07-04T10:00:00Z", now),
        "2h ago(07-04 10:00)"
    );
    assert_eq!(
        display_ts("2026-07-04T11:59:30Z", now),
        "just now(07-04 11:59)"
    );
    // Future timestamp (clock skew) and unparseable input: absolute only,
    // the machine asserts nothing it cannot verify.
    assert_eq!(display_ts("2026-07-05T00:00:00Z", now), "07-05 00:00");
    assert_eq!(display_ts("garbage", now), "garbage");
}

#[test]
fn ts_gap_seconds_measures_in_line_gaps() {
    assert_eq!(
        ts_gap_seconds("2026-06-10T00:00:00Z", "2026-06-29T00:00:00Z"),
        Some(19 * 86_400)
    );
    assert_eq!(ts_gap_seconds("garbage", "2026-06-29T00:00:00Z"), None);
}

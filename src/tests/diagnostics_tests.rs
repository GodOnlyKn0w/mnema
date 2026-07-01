use super::*;

#[test]
fn w068_fires_on_overdue_deadline_and_respects_closing() {
    let _env = setup();
    let id = create_strand("ship the feature");
    cmd_append(
        Some("[deadline] finish rollout by=2000-01-01"),
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
    let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
    assert!(
        diags.iter().any(|(c, _)| *c == "W068"),
        "expected W068, got {:?}",
        diags
    );

    // Closing the strand silences the warning (precision over recall).
    cmd_close(&id, Some("cancelled"), false).unwrap();
    let (events, _) = read_events_lossy(&path);
    let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
    let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
    assert!(!diags.iter().any(|(c, _)| *c == "W068"));
}

#[test]
fn w068_future_deadline_is_silent() {
    let _env = setup();
    let id = create_strand("future work");
    cmd_append(
        Some("[deadline] finish by=2999-01-01"),
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
    let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
    assert!(
        diags.is_empty(),
        "future deadline must not fire: {:?}",
        diags
    );
}

#[test]
fn w069_fires_on_two_producers_same_marker() {
    let _env = setup();
    let id = create_strand("contested task");
    cmd_append(
        Some("[done] finished it"),
        false,
        None,
        Some(&id),
        None,
        Some(r#"{"producer":"alpha"}"#),
    )
    .unwrap();
    cmd_append(
        Some("[done] also finished it"),
        false,
        None,
        Some(&id),
        None,
        Some(r#"{"producer":"beta"}"#),
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
    let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
    let w069: Vec<_> = diags.iter().filter(|(c, _)| *c == "W069").collect();
    assert_eq!(w069.len(), 1, "expected one W069, got {:?}", diags);
    assert!(w069[0].1.contains("alpha") && w069[0].1.contains("beta"));
}

#[test]
fn w069_single_producer_is_silent() {
    let _env = setup();
    let id = create_strand("solo task");
    cmd_append(
        Some("[done] finished"),
        false,
        None,
        Some(&id),
        None,
        Some(r#"{"producer":"alpha"}"#),
    )
    .unwrap();
    cmd_append(
        Some("[verified] checked"),
        false,
        None,
        Some(&id),
        None,
        Some(r#"{"producer":"alpha"}"#),
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
    let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
    assert!(diags.iter().all(|(c, _)| *c != "W069"));
}

#[test]
fn w062_fires_on_cross_strand_keyword_within_window() {
    let _env = setup();
    let a = create_strand("storage work");
    let b = create_strand("policy work");
    cmd_append(
        Some("[decision] adopt sqlite for local persistence"),
        false,
        None,
        Some(&a),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[constraint] sqlite writes are forbidden in production"),
        false,
        None,
        Some(&b),
        None,
        None,
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
    let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
    let w062: Vec<_> = diags.iter().filter(|(c, _)| *c == "W062").collect();
    assert_eq!(w062.len(), 1, "expected one W062, got {:?}", diags);
    assert!(w062[0].1.contains("sqlite"));
}

#[test]
fn w062_same_strand_or_no_shared_keyword_is_silent() {
    let _env = setup();
    let a = create_strand("one line");
    cmd_append(
        Some("[decision] adopt sqlite here"),
        false,
        None,
        Some(&a),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[constraint] sqlite writes forbidden"),
        false,
        None,
        Some(&a),
        None,
        None,
    )
    .unwrap();
    let b = create_strand("other line");
    cmd_append(
        Some("[constraint] postgres only in staging"),
        false,
        None,
        Some(&b),
        None,
        None,
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
    let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
    assert!(
        diags.iter().all(|(c, _)| *c != "W062"),
        "same-strand pair must not fire: {:?}",
        diags
    );
}

// ── vocabulary consistency: catalog markers must be writable ──

// Extract bracket markers of the form `[a-z][a-z0-9_:-]*]` from a string.
// Hand-rolled to avoid a regex dependency.

#[test]
fn append_help_markers_are_writable() {
    // The Append after_help now points to `tasktree explain markers` instead
    // of listing markers inline (L2 slim-down). The contract is now on the
    // markers topic body: every bracket marker in the body must be accepted
    // by validate_lifecycle_marker.
    let topic = diagnostics::topic_lookup("markers").expect("markers topic must exist");
    let markers = extract_bracket_markers(topic.body);
    assert!(
        !markers.is_empty(),
        "markers topic body must list at least one marker"
    );
    let mut failures: Vec<String> = Vec::new();
    for marker in &markers {
        let test_content = format!("{} x", marker);
        if let Err(e) = validate_lifecycle_marker(&test_content) {
            failures.push(format!("{}: {}", marker, e));
        }
    }
    assert!(
        failures.is_empty(),
        "markers in topic body rejected by validate_lifecycle_marker:\n{}",
        failures.join("\n")
    );
}

#[test]
fn levenshtein_basic() {
    assert_eq!(levenshtein("decision", "decision"), 0);
    assert_eq!(levenshtein("freiction", "friction"), 1); // one extra char
    assert_eq!(levenshtein("decsion", "decision"), 1); // transposition/missing
    assert_eq!(levenshtein("", "abc"), 3);
    assert_eq!(levenshtein("abc", ""), 3);
    assert_eq!(levenshtein("kitten", "sitting"), 3);
}

#[test]
fn suggest_marker_typo_triggers() {
    // [freiction] → friction (distance 1)
    let r = suggest_marker("[freiction]");
    assert_eq!(r, Some("[friction]"), "freiction should suggest friction");

    // [decsion] → decision (distance 1)
    let r2 = suggest_marker("[decsion]");
    assert_eq!(r2, Some("[decision]"), "decsion should suggest decision");
}

#[test]
fn suggest_marker_exact_match_is_silent() {
    // Exact match must return None (not a typo)
    assert_eq!(suggest_marker("[decision]"), None);
    assert_eq!(suggest_marker("[friction]"), None);
    assert_eq!(suggest_marker("[done]"), None);
}

#[test]
fn suggest_marker_custom_tags_are_silent() {
    // Custom tags with hyphens, digits, or uppercase-looking codes must be silent
    assert_eq!(
        suggest_marker("[my-tag]"),
        None,
        "hyphen tag must be silent"
    );
    assert_eq!(suggest_marker("[W062]"), None, "W-code must be silent");
    assert_eq!(suggest_marker("[2026-06]"), None, "date tag must be silent");
    assert_eq!(
        suggest_marker("[myCustomTag]"),
        None,
        "long distant tag must be silent"
    );
}

#[test]
fn suggest_marker_non_bracket_is_silent() {
    // Content not starting with [ must never fire W073 (validate_lifecycle_marker returns Ok)
    assert!(validate_lifecycle_marker("plain text").is_ok());
    assert!(validate_lifecycle_marker("just a note").is_ok());
}

#[test]
fn known_markers_covers_all_topic_markers() {
    // Every bracket marker in the markers topic body must be in known_markers().
    let topic = diagnostics::topic_lookup("markers").expect("markers topic must exist");
    let in_topic = extract_bracket_markers(topic.body);
    let km: Vec<&str> = known_markers().to_vec();
    let mut missing: Vec<String> = Vec::new();
    for m in &in_topic {
        // Skip [hidden] — present in known_markers but not required to be
        // listed in topic body prose
        if !km.contains(&m.as_str()) {
            missing.push(m.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "markers in topic body not in known_markers(): {:?}",
        missing
    );
}

#[test]
fn w073_append_typo_succeeds_and_suggest_fires() {
    // Verify: cmd_append succeeds (W073 never blocks writes).
    // Verify: suggest_marker returns a suggestion for the typo.
    let _env = setup();
    let id = create_strand("w073 test strand");
    let result = cmd_append(
        Some("[freiction] this is a typo marker"),
        false,
        None,
        Some(&id),
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "append must succeed even with typo marker: {:?}",
        result
    );
    // Confirm suggest_marker would have fired
    let suggestion = suggest_marker("[freiction]");
    assert_eq!(suggestion, Some("[friction]"));
}

#[test]
fn w073_exact_marker_is_silent() {
    // Correctly spelled markers must not trigger W073.
    assert_eq!(suggest_marker("[decision]"), None);
    assert_eq!(suggest_marker("[constraint]"), None);
    assert_eq!(suggest_marker("[progress]"), None);
}

// ── Lifecycle: close / reopen / W074 regression tests ─────────────────

// Footgun nail: appending [done] to a strand must NOT close it.
// This is the principal regression test for the lifecycle refactor.

#[test]
fn leading_marker_extracts_token_or_none() {
    assert_eq!(leading_marker("[decision] foo"), Some("decision"));
    assert_eq!(leading_marker("  [friction] bar"), Some("friction"));
    assert_eq!(leading_marker("plain text"), None);
    assert_eq!(leading_marker("[] empty"), None);
    assert_eq!(leading_marker("no close bracket [x"), None);
}

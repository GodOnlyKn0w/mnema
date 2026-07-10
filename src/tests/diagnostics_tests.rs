use super::*;

#[test]
fn w068_fires_on_overdue_deadline_and_respects_closing() {
    let _env = setup();
    let id = create_strand("ship the feature");
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
    cmd_close(&id, Some("cancelled"), None, false).unwrap();
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
    let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
    let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
    assert!(
        diags.is_empty(),
        "future deadline must not fire: {:?}",
        diags
    );
}

// ── W071: checkpoint on closed strand ──────────────────────────────────

#[test]
fn w071_fires_on_closed_strand() {
    let _env = setup();
    let id = create_strand("closed work");
    cmd_close(&id, Some("done"), None, false).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let strand = strands.iter().find(|s| s.id == id).unwrap();
    let result = diagnostics::check_w071_closed_strand(strand);
    assert!(result.is_some(), "W071 must fire on closed strand");
    let (code, detail) = result.unwrap();
    assert_eq!(code, "W071");
    assert!(
        detail.contains("done"),
        "detail must mention state: {}",
        detail
    );
}

#[test]
fn w071_silent_on_open_strand() {
    let _env = setup();
    let id = create_strand("open work");
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let strand = strands.iter().find(|s| s.id == id).unwrap();
    let result = diagnostics::check_w071_closed_strand(strand);
    assert!(result.is_none(), "W071 must not fire on registered strand");
}

// ── W059: append on closed strand ─────────────────────────────────────

#[test]
fn w059_fires_on_closed_strand() {
    let _env = setup();
    let id = create_strand("closed append target");
    cmd_close(&id, Some("done"), None, false).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let strand = strands.iter().find(|s| s.id == id).unwrap();
    let result = diagnostics::check_w059_append_closed_strand(strand);
    assert!(result.is_some(), "W059 must fire on closed append target");
    let warning = result.unwrap();
    assert_eq!(warning.code, "W059");
    assert_eq!(warning.state, "closed:done");
    assert!(
        warning.detail.contains("closed:done"),
        "detail must mention state: {}",
        warning.detail
    );
    assert!(warning.add_from.contains(&shorten(&id)));
    assert!(warning.reopen.contains(&shorten(&id)));
}

#[test]
fn w059_silent_on_open_strand() {
    let _env = setup();
    let id = create_strand("open append target");
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let strand = strands.iter().find(|s| s.id == id).unwrap();
    let result = diagnostics::check_w059_append_closed_strand(strand);
    assert!(result.is_none(), "W059 must not fire on registered strand");
}

#[test]
fn append_explicit_closed_strand_succeeds_and_json_carries_w059() {
    let _env = setup();
    let id = create_strand("closed explicit append target");
    cmd_close(&id, Some("failed"), None, false).unwrap();

    let outcome = execute_append(AppendRequest {
        content: Some("[progress] late result"),
        legacy_id: None,
        new: false,
        stdin: false,
        file: None,
        explicit_id: Some(&id),
        provenance_raw: None,
        seen_offset: None,
        why: None,
        allow_selection: false,
    })
    .expect("append to explicitly closed strand must still exit 0");

    assert_eq!(outcome.kind, AppendOutcomeKind::AppendedExisting);
    let warning = outcome
        .closed_target_warning
        .as_ref()
        .expect("closed target warning must be carried on AppendOutcome");
    assert_eq!(warning.code, "W059");
    assert_eq!(warning.state, "closed:failed");
    assert!(warning.detail.contains("mnema add --from"));
    assert!(warning.detail.contains("mnema reopen --id"));

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let strand = strands.iter().find(|s| s.id == id).unwrap();
    assert!(
        strand.log.iter().any(|e| e.content.contains("late result")),
        "append must still write the requested entry"
    );

    let warnings: Vec<output::SeenOffsetWarningOutput<'_>> = outcome
        .seen_warning
        .iter()
        .map(output::SeenOffsetWarningOutput::from)
        .collect();
    let dto = output::AppendOutput {
        strand_id: &outcome.strand_id,
        entry_id: &outcome.entry_id,
        entry_id_prefix: outcome.entry_id.as_deref().map(crate::util::shorten),
        content_preview: outcome.stored_content.chars().take(120).collect::<String>(),
        provenance: &outcome.provenance,
        seen_offset: outcome.seen_offset,
        seen_gap: outcome.seen_warning.as_ref().map(|w| w.seen_gap),
        warnings,
        closed_target: outcome
            .closed_target_warning
            .as_ref()
            .map(output::ClosedTargetOutput::from),
        result: outcome.card_state.as_ref().map(|(card, _)| card.clone()),
        resolved_by: outcome.resolved_by,
        active_count: outcome.active_count,
    };
    let json = serde_json::to_value(&dto).expect("append DTO must serialize");
    assert_eq!(json["closed_target"]["code"], "W059");
    assert_eq!(json["closed_target"]["state"], "closed:failed");
    assert!(
        json["closed_target"]["add_from"]
            .as_str()
            .unwrap()
            .contains("mnema add --from")
    );
    assert!(
        json["closed_target"]["reopen"]
            .as_str()
            .unwrap()
            .contains("mnema reopen --id")
    );
    assert!(json["warnings"].as_array().unwrap().is_empty());
}

#[test]
fn append_open_strand_json_closed_target_is_null() {
    let _env = setup();
    let id = create_strand("open explicit append target");
    let outcome = execute_append(AppendRequest {
        content: Some("[progress] normal result"),
        legacy_id: None,
        new: false,
        stdin: false,
        file: None,
        explicit_id: Some(&id),
        provenance_raw: None,
        seen_offset: None,
        why: None,
        allow_selection: false,
    })
    .expect("append to open strand must succeed");

    let warnings: Vec<output::SeenOffsetWarningOutput<'_>> = outcome
        .seen_warning
        .iter()
        .map(output::SeenOffsetWarningOutput::from)
        .collect();
    let dto = output::AppendOutput {
        strand_id: &outcome.strand_id,
        entry_id: &outcome.entry_id,
        entry_id_prefix: outcome.entry_id.as_deref().map(crate::util::shorten),
        content_preview: outcome.stored_content.chars().take(120).collect::<String>(),
        provenance: &outcome.provenance,
        seen_offset: outcome.seen_offset,
        seen_gap: outcome.seen_warning.as_ref().map(|w| w.seen_gap),
        warnings,
        closed_target: outcome
            .closed_target_warning
            .as_ref()
            .map(output::ClosedTargetOutput::from),
        result: outcome.card_state.as_ref().map(|(card, _)| card.clone()),
        resolved_by: outcome.resolved_by,
        active_count: outcome.active_count,
    };
    let json = serde_json::to_value(&dto).expect("append DTO must serialize");
    assert!(json["closed_target"].is_null());
}

// ── checkpoint + W071 end-to-end: writes succeed (exit 0) ─────────────

#[test]
fn append_help_markers_are_writable() {
    // The Append after_help now points to `mnema explain markers` instead
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

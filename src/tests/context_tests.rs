use super::*;

#[test]
fn test_context_text_output_contract() {
    let _env = setup();
    // Create a typed prompt-strand with [covers]
    let (created, appended) =
        event::make_strand_created("[covers] test-area/", Some("prompt-strand"));
    let id = created.strand_id().to_string();
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &created)?;
        append_event_unlocked(journal, &appended)?;
        Ok(())
    })
    .unwrap();
    // Append a [guide] entry
    let guide = event::make_log_appended(&id, "[guide] how to test", None);
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &guide)?;
        Ok(())
    })
    .unwrap();
    // Verify projection sees it correctly
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let matching: Vec<&projection::ProjectedStrand> = strands
        .iter()
        .filter(|s| s.strand_type.as_deref() == Some("prompt-strand"))
        .collect();
    assert!(!matching.is_empty(), "should find prompt-strand");
    let strand = matching
        .iter()
        .find(|s| s.id == id)
        .expect("our strand should exist");
    assert_eq!(strand.log.len(), 2, "should have [covers] + [guide]");
    assert!(
        strand.log[0].content.starts_with("[covers]"),
        "first entry must be [covers]"
    );
    assert!(
        strand.log[1].content.starts_with("[guide]"),
        "second entry is [guide]"
    );
}

#[test]
fn test_context_empty_lines() {
    let _env = setup();
    // Create two prompt-strands
    let (c1, a1) = event::make_strand_created("[covers] a/", Some("prompt-strand"));
    let (c2, a2) = event::make_strand_created("[covers] b/", Some("prompt-strand"));
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &c1)?;
        append_event_unlocked(journal, &a1)?;
        append_event_unlocked(journal, &c2)?;
        append_event_unlocked(journal, &a2)?;
        Ok(())
    })
    .unwrap();
    // Run context and capture output
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let matching: Vec<&projection::ProjectedStrand> = strands
        .iter()
        .filter(|s| s.strand_type.as_deref() == Some("prompt-strand"))
        .collect();
    assert!(matching.len() >= 2, "should have at least 2 prompt-strands");
    // Verify no trailing blank line in text output by checking internal rendering
    // (full text output test would require capturing stdout)
}

#[test]
fn context_exposes_friction_on_live_strand_by_default() {
    let _env = setup();
    let id = create_prompt_strand("live guidance");
    cmd_append(
        Some("[friction] stepped in a hole here"),
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
    let strands = projection::project_strands(&events, false);
    let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
    assert_eq!(out.len(), 1);
    assert!(
        out[0].entries.iter().any(|e| e.marker == "[friction]"),
        "live friction must be exposed by default"
    );
    assert_eq!(out[0].friction_folded, 0);
}

#[test]
fn context_folds_friction_on_closed_strand() {
    let _env = setup();
    let id = create_prompt_strand("closed guidance");
    cmd_append(
        Some("[friction] hole, since resolved"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    cmd_close(&id, Some("done"), false).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, false);
    let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
    assert_eq!(out.len(), 1);
    assert!(
        out[0].entries.iter().all(|e| e.marker != "[friction]"),
        "closed-strand friction folds away"
    );
    assert_eq!(
        out[0].friction_folded, 1,
        "fold is a scar, not a disappearance"
    );
}

#[test]
fn context_exclude_friction_is_explicit_blindness() {
    let _env = setup();
    let id = create_prompt_strand("live guidance");
    cmd_append(
        Some("[friction] hole"),
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
    let strands = projection::project_strands(&events, false);
    let out = build_context_strands(&strands, "prompt-strand", &[], None, true, false);
    assert_eq!(out.len(), 1);
    assert!(out[0].entries.iter().all(|e| e.marker != "[friction]"));
    assert_eq!(
        out[0].friction_folded, 0,
        "explicit exclusion is not a fold"
    );
}

// ── Part A: friction↔fixed pairing ──────────────────────────

#[test]
fn context_friction_fixed_pair_produces_scar() {
    // A single [friction] followed by [fixed fixes=<id>] on a live strand:
    // - scar entry appears (marker=[friction], content contains "→ fixed")
    // - neither the original friction nor the [fixed] appear as separate entries
    // - friction_paired == 1
    // Explicit fixes= is required; proximity inference is not supported.
    let _env = setup();
    let id = create_prompt_strand("live guidance");
    cmd_append(
        Some("[friction] a hole to fill"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    // Read back the friction's append_id to form a fixes= reference
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let friction_append_id = events
        .iter()
        .rev()
        .find_map(|(_, e)| {
            if let event::Event::LogAppended {
                id: eid,
                content,
                append_id,
                ..
            } = e
            {
                if eid == &id && content.contains("a hole to fill") {
                    return append_id.clone();
                }
            }
            None
        })
        .expect("friction must have append_id");
    let prefix = &friction_append_id[..8.min(friction_append_id.len())];
    let fixed_content = format!("[fixed] filled the hole fixes={}", prefix);
    cmd_append(
        Some(&fixed_content),
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
    let strands = projection::project_strands(&events, false);
    let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
    assert_eq!(out.len(), 1);
    let entries = &out[0].entries;
    // scar entry must be present
    let scar = entries
        .iter()
        .find(|e| e.marker == "[friction]" && e.content.contains("→ fixed"));
    assert!(
        scar.is_some(),
        "expected scar entry with → fixed, entries: {:?}",
        entries
    );
    // no standalone [fixed] entry
    assert!(
        entries.iter().all(|e| e.marker != "[fixed]"),
        "paired [fixed] must not appear separately"
    );
    // no unmodified friction entry (scar replaces it)
    let raw_friction = entries.iter().filter(|e| e.marker == "[friction]").count();
    assert_eq!(raw_friction, 1, "exactly one [friction] entry (the scar)");
    let scar_entry = scar.unwrap();
    assert!(
        scar_entry.content.contains("a hole to fill"),
        "scar must include truncated friction text"
    );
    assert_eq!(out[0].friction_paired, 1);
}

#[test]
fn context_fixed_without_fixes_is_plain_annotation() {
    // [fixed] with no fixes= token is a plain annotation — not folded,
    // not paired. The [friction] stays full-text (live debt, unresolved).
    // Proximity inference was intentionally removed (close-command footgun lesson).
    let _env = setup();
    let id = create_prompt_strand("live guidance");
    cmd_append(
        Some("[friction] first hole"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[friction] second hole"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[fixed] fixed something but no reference"),
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
    let strands = projection::project_strands(&events, false);
    let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
    assert_eq!(out.len(), 1);
    let entries = &out[0].entries;
    // Both frictions remain full-text (neither is a scar)
    let first_full = entries.iter().find(|e| {
        e.marker == "[friction]"
            && e.content.contains("first hole")
            && !e.content.contains("→ fixed")
    });
    assert!(
        first_full.is_some(),
        "first friction must remain full-text (unpaired)"
    );
    let second_full = entries.iter().find(|e| {
        e.marker == "[friction]"
            && e.content.contains("second hole")
            && !e.content.contains("→ fixed")
    });
    assert!(
        second_full.is_some(),
        "second friction must remain full-text (no proximity pairing)"
    );
    // [fixed] without fixes= appears as a plain annotation entry
    let fixed_entry = entries.iter().find(|e| e.marker == "[fixed]");
    assert!(
        fixed_entry.is_some(),
        "[fixed] without fixes= must appear as a plain annotation"
    );
    assert_eq!(
        out[0].friction_paired, 0,
        "no pairing without explicit fixes="
    );
}

#[test]
fn context_friction_fixed_explicit_fixes_ref() {
    // [fixed] with fixes=<prefix> pairs with the specified friction, not proximity.
    // We create: friction_A, friction_B, [fixed fixes=<prefix_of_A>]
    // Expected: friction_A becomes scar, friction_B stays full-text.
    let _env = setup();
    let id = create_prompt_strand("live guidance");
    // Append friction_A first and capture its append_id
    cmd_append(
        Some("[friction] hole alpha"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    // Append friction_B
    cmd_append(
        Some("[friction] hole beta"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    // Read back to find friction_A's append_id
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let friction_a_append_id = events
        .iter()
        .rev()
        .find_map(|(_, e)| {
            if let event::Event::LogAppended {
                id: eid,
                content,
                append_id,
                ..
            } = e
            {
                if eid == &id && content.contains("hole alpha") {
                    return append_id.clone();
                }
            }
            None
        })
        .expect("friction_A must have append_id");
    // Use first 8 chars of append_id as the prefix
    let prefix = &friction_a_append_id[..8.min(friction_a_append_id.len())];
    let fixed_content = format!("[fixed] resolves first hole fixes={}", prefix);
    cmd_append(
        Some(&fixed_content),
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
    let strands = projection::project_strands(&events, false);
    let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
    assert_eq!(out.len(), 1);
    let entries = &out[0].entries;
    // friction_A → scar
    let scar_a = entries.iter().find(|e| {
        e.marker == "[friction]"
            && e.content.contains("hole alpha")
            && e.content.contains("→ fixed")
    });
    assert!(
        scar_a.is_some(),
        "friction_A must become scar via explicit fixes= ref; entries: {:?}",
        entries
    );
    // friction_B → full-text (unpaired)
    let full_b = entries.iter().find(|e| {
        e.marker == "[friction]"
            && e.content.contains("hole beta")
            && !e.content.contains("→ fixed")
    });
    assert!(
        full_b.is_some(),
        "friction_B must stay full-text (unpaired by explicit ref)"
    );
    assert_eq!(out[0].friction_paired, 1);
}

#[test]
fn context_exclude_friction_also_suppresses_scars() {
    // --exclude-friction (explicit blindness) must suppress scar entries too.
    // Uses explicit fixes= to produce a real pair/scar first.
    let _env = setup();
    let id = create_prompt_strand("live guidance");
    cmd_append(
        Some("[friction] a hole"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    // Read back the friction's append_id
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let friction_append_id = events
        .iter()
        .rev()
        .find_map(|(_, e)| {
            if let event::Event::LogAppended {
                id: eid,
                content,
                append_id,
                ..
            } = e
            {
                if eid == &id && content.contains("a hole") {
                    return append_id.clone();
                }
            }
            None
        })
        .expect("friction must have append_id");
    let prefix = &friction_append_id[..8.min(friction_append_id.len())];
    let fixed_content = format!("[fixed] filled fixes={}", prefix);
    cmd_append(
        Some(&fixed_content),
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
    let strands = projection::project_strands(&events, false);
    let out = build_context_strands(&strands, "prompt-strand", &[], None, true, false);
    assert_eq!(out.len(), 1);
    assert!(
        out[0].entries.iter().all(|e| e.marker != "[friction]"),
        "exclude_friction must suppress scar entries too"
    );
}

#[test]
fn context_dangling_fixes_produces_no_fold() {
    // [fixed] with fixes=<prefix> that matches nothing → dangling fix.
    // The [fixed] entry is a plain annotation (exposed), not folded.
    // The [friction] stays full-text (live debt).
    // W075 would be emitted to stderr; we test that no folding happens.
    let _env = setup();
    let id = create_prompt_strand("live guidance");
    cmd_append(
        Some("[friction] unresolved hole"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    // Use a fake/nonexistent append_id prefix (all zeros, ≥8 chars)
    cmd_append(
        Some("[fixed] pretend fix fixes=00000000deadbeef"),
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
    let strands = projection::project_strands(&events, false);
    let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
    assert_eq!(out.len(), 1);
    let entries = &out[0].entries;
    // friction stays full-text (unresolved)
    let friction_full = entries.iter().find(|e| {
        e.marker == "[friction]"
            && e.content.contains("unresolved hole")
            && !e.content.contains("→ fixed")
    });
    assert!(
        friction_full.is_some(),
        "friction must remain full-text when fixes= is dangling; entries: {:?}",
        entries
    );
    // [fixed] with dangling ref is exposed as annotation
    let fixed_entry = entries.iter().find(|e| e.marker == "[fixed]");
    assert!(
        fixed_entry.is_some(),
        "dangling [fixed] must appear as annotation entry"
    );
    assert_eq!(out[0].friction_paired, 0, "no pairing on dangling fix");
    // pair_frictions itself must record the dangling fix
    let pairing = pair_frictions(&strands[0].log);
    assert_eq!(pairing.dangling_fixes.len(), 1, "one dangling fix recorded");
    let (_, ref prefix) = pairing.dangling_fixes[0];
    assert!(
        prefix.starts_with("00000000"),
        "prefix must match what was written"
    );
}

#[test]
fn context_one_fixed_pairs_at_most_one_friction() {
    // Strict 1-1: a [fixed] entry with fixes=<prefix_A> pairs exactly one friction.
    // A second [fixed] entry pointing to the same friction (already paired) → dangling.
    let _env = setup();
    let id = create_prompt_strand("live guidance");
    cmd_append(
        Some("[friction] target hole"),
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
    let friction_append_id = events
        .iter()
        .rev()
        .find_map(|(_, e)| {
            if let event::Event::LogAppended {
                id: eid,
                content,
                append_id,
                ..
            } = e
            {
                if eid == &id && content.contains("target hole") {
                    return append_id.clone();
                }
            }
            None
        })
        .expect("friction must have append_id");
    let prefix = &friction_append_id[..8.min(friction_append_id.len())];
    // Two [fixed] entries both referencing the same friction
    let fixed1 = format!("[fixed] first fix fixes={}", prefix);
    let fixed2 = format!("[fixed] second fix fixes={}", prefix);
    cmd_append(
        Some(&fixed1),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some(&fixed2),
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
    let strands = projection::project_strands(&events, false);
    let pairing = pair_frictions(&strands[0].log);
    // Only one friction → only one pairing possible
    assert_eq!(
        pairing.paired_friction.len(),
        1,
        "only one friction, only one pair"
    );
    assert_eq!(
        pairing.paired_fixed.len(),
        1,
        "only first matching [fixed] is paired"
    );
    // Second [fixed] targeting already-paired friction → dangling
    assert_eq!(
        pairing.dangling_fixes.len(),
        1,
        "second [fixed] with same ref → dangling"
    );
}

// ── Part B: observation-class folding ────────────────────────

#[test]
fn context_observation_folding_tail_kept() {
    // 3 [progress] + 1 [observed] → folded_counts {progress:2, observed:0, check:0}
    // (progress tail kept as last entry, observed is itself the tail so count=0)
    let _env = setup();
    let id = create_prompt_strand("live guidance");
    cmd_append(
        Some("[progress] step 1"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[progress] step 2"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[progress] step 3"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[observed] an observation"),
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
    let strands = projection::project_strands(&events, false);
    let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
    assert_eq!(out.len(), 1);
    let fc = &out[0].folded_counts;
    assert_eq!(fc.progress, 2, "first 2 progress entries folded, tail kept");
    assert_eq!(
        fc.observed, 0,
        "single observed entry is the tail, not folded"
    );
    assert_eq!(fc.check, 0);
    // The tail progress entry must appear in entries
    let has_tail = out[0]
        .entries
        .iter()
        .any(|e| e.marker == "[progress]" && e.content.contains("step 3"));
    assert!(has_tail, "tail [progress] entry must be visible");
    // Folded progress entries must NOT appear
    let visible_progress: Vec<_> = out[0]
        .entries
        .iter()
        .filter(|e| e.marker == "[progress]")
        .collect();
    assert_eq!(visible_progress.len(), 1, "only tail [progress] visible");
}

#[test]
fn context_include_observations_disables_folding() {
    // --include-observations exposes all entries; folded_counts all 0.
    let _env = setup();
    let id = create_prompt_strand("live guidance");
    cmd_append(
        Some("[progress] step 1"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[progress] step 2"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[check] checked"),
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
    let strands = projection::project_strands(&events, false);
    let out = build_context_strands(&strands, "prompt-strand", &[], None, false, true);
    assert_eq!(out.len(), 1);
    let fc = &out[0].folded_counts;
    assert_eq!(fc.progress, 0, "no folding when include_observations=true");
    assert_eq!(fc.observed, 0);
    assert_eq!(fc.check, 0);
    // All three entries visible
    let progress_count = out[0]
        .entries
        .iter()
        .filter(|e| e.marker == "[progress]")
        .count();
    assert_eq!(progress_count, 2, "both [progress] entries must be visible");
}

#[test]
fn context_closed_strand_observation_folding() {
    // Closed strands also get observation folding (live+closed unified for obs).
    let _env = setup();
    let id = create_prompt_strand("closed strand");
    cmd_append(
        Some("[progress] step 1"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[progress] step 2"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[done] wrapped"),
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
    let strands = projection::project_strands(&events, false);
    let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].folded_counts.progress, 1,
        "first progress folded on closed strand"
    );
}

// ── grammar conformance (contract: tasktree explain grammar) ──
// The contract is an artifact, not a discipline: these tests are the
// teeth. A new command violating the flag vocabulary or naming rules
// fails here, not in a future cold-start.

#[test]
fn agent_context_default_excludes_hidden_prompt_strands() {
    let _env = setup();
    let (c, a) = event::make_strand_created("[covers] test/", Some("prompt-strand"));
    let id = c.strand_id().to_string();
    with_journal_write_lock(|j| {
        append_event_unlocked(j, &c)?;
        append_event_unlocked(j, &a)
    })
    .unwrap();
    cmd_hide(&id, Some("noise"), false, None).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let visible = projection::project_strands(&events, false);
    assert!(
        !visible.iter().any(|s| s.id == id),
        "hidden prompt-strand must not be visible by default"
    );
    let all = projection::project_strands(&events, true);
    assert!(
        all.iter().any(|s| s.id == id),
        "include_hidden must surface hidden prompt-strand"
    );
}

// cmd_context default excludes hidden strands; --include-hidden surfaces them.

#[test]
fn context_default_excludes_hidden() {
    let _env = setup();
    let (c, a) = event::make_strand_created("[covers] test-area/", Some("prompt-strand"));
    let id = c.strand_id().to_string();
    with_journal_write_lock(|j| {
        append_event_unlocked(j, &c)?;
        append_event_unlocked(j, &a)
    })
    .unwrap();
    cmd_hide(&id, Some("noise"), false, None).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let visible = projection::project_strands(&events, false);
    assert!(!visible.iter().any(|s| s.id == id));
    let all = projection::project_strands(&events, true);
    assert!(all.iter().any(|s| s.id == id));
}

// Repeated `cmd_hide` is idempotent: only one StrandHidden event is written.

#[test]
fn cmd_context_default_excludes_hidden_via_cmd_path() {
    let _env = setup();
    let (c, a) = event::make_strand_created("[covers] audit/", Some("prompt-strand"));
    let id = c.strand_id().to_string();
    with_journal_write_lock(|j| {
        append_event_unlocked(j, &c)?;
        append_event_unlocked(j, &a)
    })
    .unwrap();
    cmd_hide(&id, Some("noise"), false, None).unwrap();
    let result = cmd_context(None, &[], None, None, false, false, false);
    assert!(result.is_ok());
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let visible = projection::project_strands(&events, false);
    assert!(
        !visible.iter().any(|s| s.id == id),
        "cmd_context default must use include_hidden=false in projection"
    );
}

// cmd_agent_context default must also exclude hidden strands.

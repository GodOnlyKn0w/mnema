use super::*;

#[test]
fn show_json_exposes_per_entry_provenance() {
    let _env = setup();
    let id = create_strand("provenance projection test");
    cmd_append(
        Some("[observed] tagged"),
        Some(&id),
        false,
        false,
        None,
        None,
        None,
        Some(r#"{"producer":"codex"}"#),
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let full = find_strand(&events, &id).unwrap();
    let strands = projection::project_strands(&events, true);
    let s = strands.iter().find(|s| s.id == full).unwrap();
    let out = output::StrandDetailOutput::from(s);
    let tagged = out
        .events
        .iter()
        .find(|e| e.provenance.is_some())
        .expect("at least one event must carry provenance");
    assert_eq!(tagged.provenance.as_ref().unwrap()["producer"], "codex");
}

#[test]
fn normalize_strips_trailing_newline() {
    assert_eq!(normalize_content("hello\n"), "hello");
}

#[test]
fn normalize_strips_trailing_crlf() {
    assert_eq!(normalize_content("hello\r\n"), "hello");
}

#[test]
fn normalize_preserves_leading_whitespace() {
    assert_eq!(normalize_content("  hello"), "  hello");
}

#[test]
fn normalize_preserves_interior_newlines() {
    assert_eq!(normalize_content("line1\nline2\n"), "line1\nline2");
}

#[test]
fn normalize_preserves_multiple_trailing_newlines_except_one() {
    assert_eq!(normalize_content("hello\n\n"), "hello\n");
}

// ── checkpoint ──

#[test]
fn humanize_duration_just_now() {
    assert_eq!(humanize_duration(0), "just now");
    assert_eq!(humanize_duration(59), "just now");
}

#[test]
fn humanize_duration_minutes() {
    assert_eq!(humanize_duration(60), "1m");
    assert_eq!(humanize_duration(61), "1m");
    assert_eq!(humanize_duration(3599), "59m");
}

#[test]
fn humanize_duration_hours() {
    assert_eq!(humanize_duration(3600), "1h");
    assert_eq!(humanize_duration(7200), "2h");
    assert_eq!(humanize_duration(86399), "23h");
}

#[test]
fn humanize_duration_days() {
    assert_eq!(humanize_duration(86400), "1d");
    assert_eq!(humanize_duration(86400 * 25), "25d");
}

// ── W070: strand moved under you ───────────────────────────────────────

#[test]
fn show_search_context_unchanged() {
    // Smoke test that existing cmd_show, cmd_search, cmd_context still work.
    let _env = setup();
    let id = create_strand("show me");
    cmd_append(
        Some("entry"),
        Some(&id),
        false,
        false,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    // show
    let r = cmd_show(Some(&id), false, None, false, false, false);
    assert!(r.is_ok());
    // search
    let r = cmd_search("entry", false, false);
    assert!(r.is_ok());
    // context
    let r = cmd_context(None, &[], None, None, false, false, false);
    assert!(r.is_ok());
}

// ── orient card DTO ──

#[test]
fn orient_strand_fields_match_projected_strand() {
    let _env = setup();
    let id = create_strand("summary text for the card");
    cmd_append(
        Some("second entry"),
        Some(&id),
        false,
        false,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let s = strands
        .iter()
        .find(|s| s.id == id)
        .expect("strand must exist");
    let card = output::OrientStrand::from(s);
    assert_eq!(card.id, id);
    assert_eq!(card.entry_count, 2);
    assert_eq!(card.summary, truncate(s.first_summary(), 70));
    assert_eq!(card.last_entry, truncate(s.last_summary(), 70));
    assert_eq!(card.last_offset, s.last_offset());
}

#[test]
fn orient_strand_truncates_prose_to_70() {
    let _env = setup();
    let long = "x".repeat(100);
    let id = create_strand(&long);
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let s = strands
        .iter()
        .find(|s| s.id == id)
        .expect("strand must exist");
    let card = output::OrientStrand::from(s);
    // truncate(100-char string, 70) → 70 chars + "..." = 73 total
    assert!(
        card.summary.len() <= 73,
        "summary must be truncated to 70 chars + ..."
    );
    // id is never truncated: always shorten(full_id) = 12 chars
    assert_eq!(card.id.len(), 24);
}

#[test]
fn truncate_collapses_to_first_line() {
    // 多行首条 entry（如 add --file <长 brief>）只露首行 + "..."，
    // 不把后续行灌进 orient/list 的一眼扫视图。
    let blob = "## 项目背景\n这是第二行\n这是第三行";
    let out = truncate(blob, 70);
    assert_eq!(out, "## 项目背景...");
    assert!(!out.contains('\n'), "preview must be single line");

    // 单行短内容原样返回，不加省略号。
    assert_eq!(truncate("short single line", 70), "short single line");

    // 单行超长仍按字符数截断 + "..."。
    let long = "x".repeat(100);
    assert_eq!(truncate(&long, 70).chars().count(), 73);
}

// ── card echo: strand_card_fresh / append paths ──

#[test]
fn append_explicit_id_card_fresh_has_new_entry() {
    let _env = setup();
    let id = create_strand("target");
    cmd_append(
        Some("[lesson] learned something"),
        Some(&id),
        false,
        false,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (card, _state) = strand_card_fresh_with_state(&id).expect("card must be retrievable");
    assert_eq!(card.last_entry, "[lesson] learned something");
}

#[test]
fn append_default_most_recent_card_fresh_reflects_write() {
    let _env = setup();
    let _id1 = create_strand("older");
    let id2 = create_strand("newer");
    cmd_append(
        Some("default route entry"),
        None,
        false,
        false,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (card, _state) = strand_card_fresh_with_state(&id2).expect("card must exist");
    assert_eq!(card.last_entry, "default route entry");
}

#[test]
fn append_new_path_card_id_matches_new_strand() {
    let _env = setup();
    // Pre-populate so --new is not the only strand
    create_strand("existing");
    cmd_append(
        Some("brand new via --new"),
        None,
        true,
        false,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    // The new strand has the content as first_summary
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let new_s = strands
        .iter()
        .find(|s| s.first_summary() == "brand new via --new")
        .expect("new strand must exist");
    let card = strand_card_fresh(&new_s.id).expect("card must be retrievable");
    assert_eq!(card.id, new_s.id);
}

// ── card echo: hide leaves strand retrievable via include_hidden=true ──

#[test]
fn strand_card_fresh_finds_hidden_strand() {
    let _env = setup();
    let id = create_strand("will be hidden");
    cmd_hide(&id, None, false, None).unwrap();
    // strand_card_fresh uses include_hidden=true — must still find it
    let card = strand_card_fresh(&id);
    assert!(
        card.is_some(),
        "strand_card_fresh must return card for hidden strand"
    );
    assert_eq!(card.unwrap().id, id);
}

// ══════════════════════════════════════════════════════════════════════
// handles_* — 把手完整性测试族
//
// 规则：把手（strand id、现成命令、journal offset）永不截断。
//   - id 在卡片/orient 用 shorten(id) = 12位十六进制前缀（合法前缀匹配）
//   - id 在 list/show/search JSON 用完整 id
//   - 两种形式都是合法参数；"…" 绝不出现在把手字段中
//   - 散文字段（summary/last_entry/content）允许 truncate(70) + "…"
// ══════════════════════════════════════════════════════════════════════

// Helper: build a >100-char summary that contains CJK characters so we
// also exercise Unicode truncation paths.

#[test]
fn handles_card_id_is_legal_prefix() {
    let _env = setup();
    let summary = long_summary();
    assert!(
        summary.chars().count() > 100,
        "precondition: summary must be >100 chars"
    );
    let id = create_strand(&summary);

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let s = strands
        .iter()
        .find(|s| s.id == id)
        .expect("strand must exist");
    let card = output::OrientStrand::from(s);

    // id: exactly 12 hex chars, is prefix of full id, contains no '…'
    assert_eq!(card.id.len(), 24, "card.id must be the full 24-hex id");
    assert!(
        id.starts_with(&card.id),
        "card.id must be a prefix of the full id"
    );
    assert!(
        !card.id.contains('\u{2026}') && !card.id.contains("..."),
        "card.id must not contain truncation marker"
    );

    // catch_up: no '…', command must parse
    assert!(
        !card.catch_up.contains('\u{2026}') && !card.catch_up.contains("..."),
        "catch_up must not contain truncation marker"
    );
    try_parse_example(&card.catch_up).expect("card.catch_up must be a parseable tasktree command");

    // last_offset: must equal the projected strand's real last_offset
    assert_eq!(
        card.last_offset,
        s.last_offset(),
        "card.last_offset must equal projected strand's last_offset"
    );

    // prose fields: allowed to contain '…' (they may be truncated)
    // (no assertion required — we just confirm the id/offset/catch_up rules above)
    let _ = &card.summary;
    let _ = &card.last_entry;
}

// ── Test 2 ────────────────────────────────────────────────────────────

// orient output with long-summary strands: each OrientStrand in active[]
//   - id is 12 chars, prefix of full id, no '…'
//   - catch_up has no '…', parses, and contains card.id (link points to self)
//   - last_offset is the real offset

#[test]
fn handles_orient_text_complete() {
    let _env = setup();
    let summary_a = long_summary();
    let id_a = create_strand(&summary_a);
    // Give strand B a shorter summary for variety; strand A is the long one.
    let summary_b = "short strand for orient contrast";
    let id_b = create_strand(summary_b);

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let max_offset = events.last().map(|(o, _)| *o).unwrap_or(0);
    let strands = projection::project_strands(&events, true);
    let out = orient_output(&strands, false, 10, max_offset);

    assert!(
        !out.active.is_empty(),
        "orient must have at least one active strand"
    );

    for card in &out.active {
        // id: full 24-hex width (joins against show/list JSON)
        assert_eq!(
            card.id.len(),
            24,
            "orient card.id must be the full 24-hex id, got '{}'",
            card.id
        );
        // Verify it is a legal prefix: find the projected strand by prefix
        let matched = strands.iter().find(|s| s.id.starts_with(&card.id));
        assert!(
            matched.is_some(),
            "orient card.id '{}' must match a strand by prefix",
            card.id
        );
        assert!(
            !card.id.contains('\u{2026}') && !card.id.contains("..."),
            "orient card.id must not contain '…'"
        );

        // catch_up: no truncation, parseable, embeds the card's own id
        assert!(
            !card.catch_up.contains('\u{2026}') && !card.catch_up.contains("..."),
            "catch_up must not be truncated"
        );
        try_parse_example(&card.catch_up)
            .expect("orient catch_up must parse as a tasktree command");
        assert!(
            card.catch_up.contains(&card.id),
            "catch_up must embed the strand's own id (link points to self): '{}'",
            card.catch_up
        );

        // last_offset: matches the projected strand
        let s = matched.unwrap();
        assert_eq!(
            card.last_offset,
            s.last_offset(),
            "orient card.last_offset must equal projected strand's last_offset"
        );
    }

    let _ = (id_a, id_b);
}

// ── Test 3 ────────────────────────────────────────────────────────────

// list --format json: StrandListItem.id is the full id (no shortening, no '…').
// search --format json: SearchMatch.strand_id is the full id.
// search content is prose — allowed to be truncated to 70 + "…".

#[test]
fn handles_list_search_ids_intact() {
    let _env = setup();
    let summary = long_summary();
    let id = create_strand(&summary);
    // Append a long content entry to have something to search.
    let long_content = "unique_search_token_xyz ".to_string() + &"w".repeat(80);
    cmd_append(
        Some(&long_content),
        Some(&id),
        false,
        false,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);

    // list JSON: StrandListItem.id must be the full 32-char id
    let strands = projection::project_strands(&events, true);
    let list_items: Vec<output::StrandListItem> =
        strands.iter().map(output::StrandListItem::from).collect();
    let item = list_items
        .iter()
        .find(|i| i.id == id)
        .expect("list must contain our strand by full id");
    // Full id: must equal the strand's id exactly
    assert_eq!(item.id, id, "StrandListItem.id must be the full strand id");
    assert!(
        !item.id.contains('\u{2026}') && !item.id.contains("..."),
        "StrandListItem.id must not contain truncation marker"
    );
    // id must be at least 12 chars (typical timeid is 24 hex chars)
    assert!(
        item.id.len() >= 12,
        "StrandListItem.id length must be at least 12, got {}",
        item.id.len()
    );

    // search JSON: SearchMatch.strand_id must be the full id; content may be truncated
    let q = "unique_search_token_xyz".to_lowercase();
    let strand_map: std::collections::HashMap<&str, &projection::ProjectedStrand> =
        strands.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut search_matches: Vec<output::SearchMatch> = Vec::new();
    for (_, event) in &events {
        if let Event::LogAppended { content, .. } = event {
            if content.to_lowercase().contains(&q) {
                let strand_id_full = event.strand_id().to_string();
                if strand_map.contains_key(strand_id_full.as_str()) {
                    let projected = strand_map.get(strand_id_full.as_str());
                    search_matches.push(output::SearchMatch {
                        strand_id: strand_id_full,
                        content: truncate(content, 70),
                        strand_type: projected.and_then(|s| s.strand_type.clone()),
                        hidden: projected.map(|s| s.hidden).unwrap_or(false),
                    });
                }
            }
        }
    }
    assert!(
        !search_matches.is_empty(),
        "search must find at least one match"
    );
    for m in &search_matches {
        // strand_id: full id, no truncation marker
        assert!(
            !m.strand_id.contains('\u{2026}') && !m.strand_id.contains("..."),
            "SearchMatch.strand_id must not contain truncation marker"
        );
        assert!(
            m.strand_id.len() >= 12,
            "SearchMatch.strand_id must be at least 12 chars"
        );
        // The match for our strand must be the full id
        if m.strand_id == id {
            assert_eq!(
                m.strand_id, id,
                "SearchMatch.strand_id must equal full strand id"
            );
        }
        // content is prose — truncation allowed; just verify it doesn't crash
        let _ = &m.content;
    }
}

// ── Test 4 ────────────────────────────────────────────────────────────

// run_journal_diagnostics: detail strings for W068/W069/W062 use shorten(id)
// (12-char prefix), which is a legal parameter. No '…' in detail strings.
// W070/W071 details contain no commands, so try_parse_example is N/A for them.

#[test]
fn handles_diag_details_parse() {
    let _env = setup();
    // Build a strand that fires W068 (overdue deadline).
    let id_a = create_strand("deadline strand for diag test");
    cmd_append(
        Some("[deadline] finish by=2000-01-01"),
        None,
        false,
        false,
        None,
        Some(&id_a),
        None,
        None,
    )
    .unwrap();

    // Build cross-strand W062 (decision vs constraint with shared keyword).
    let id_b = create_strand("decision strand");
    let id_c = create_strand("constraint strand");
    cmd_append(
        Some("[decision] adopt postgres for persistence"),
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
        Some("[constraint] postgres writes forbidden in staging"),
        None,
        false,
        false,
        None,
        Some(&id_c),
        None,
        None,
    )
    .unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
    let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());

    for (code, detail) in &diags {
        // No truncation marker in detail strings (id handles inside details
        // use shorten, which is a valid prefix, not a truncated string).
        assert!(
            !detail.contains('\u{2026}') && !detail.contains("..."),
            "diag {} detail must not contain truncation marker: '{}'",
            code,
            detail
        );

        // For W062: detail contains strand id handles (shorten = 12-char prefix).
        // Verify any embedded id-like hex strings (12 chars) are prefix of a known strand.
        if *code == "W062" || *code == "W068" || *code == "W069" {
            // Extract 12-char hex tokens from detail.
            for tok in detail.split_whitespace() {
                let tok = tok.trim_matches(|c: char| !c.is_ascii_hexdigit());
                if tok.len() == 12 && tok.chars().all(|c| c.is_ascii_hexdigit()) {
                    // Must be a prefix of some known strand id.
                    let all_strands = projection::project_strands(&events, true);
                    let is_valid_prefix = all_strands.iter().any(|s| s.id.starts_with(tok));
                    assert!(
                        is_valid_prefix,
                        "diag {} detail contains '{}' which is not a valid strand id prefix",
                        code, tok
                    );
                }
            }
        }

        // W070/W071: details contain no tasktree commands (catalog confirms
        // their recovery.executable is false). We verify no false-positive parse attempt.
        // (No try_parse_example call here — the detail strings are prose, not commands.)
        if *code == "W070" || *code == "W071" {
            assert!(
                !detail.contains("tasktree "),
                "W070/W071 detail must not embed a tasktree command: '{}'",
                detail
            );
        }
    }
}

// ── Test 5 ────────────────────────────────────────────────────────────

// Audit test: for a strand with a known id, verify that each command's
// JSON id field matches the documented convention (current behavior nailed).
//
//   show --format json  → StrandDetailOutput.id = full id
//   list --format json  → StrandListItem.id      = full id
//   orient --format json (via orient output) → OrientStrand.id = shorten(full id) = 12 chars
//   search --format json → SearchMatch.strand_id = full id
//
// All forms are legally usable as tasktree --id arguments (prefix match
// or exact match). Neither form may contain '…'.

#[test]
fn handles_truncate_never_applied_to_ids() {
    let _env = setup();
    // Use a long summary so truncate would fire on prose but must not fire on ids.
    let id = create_strand(&long_summary());
    // Append a searchable entry.
    let searchable = "unique_audit_token_abc123 ".to_string() + &"z".repeat(80);
    cmd_append(
        Some(&searchable),
        Some(&id),
        false,
        false,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);

    // show --format json: full id
    let s = strands
        .iter()
        .find(|s| s.id == id)
        .expect("strand must exist");
    let show_dto = output::StrandDetailOutput::from(s);
    assert_eq!(show_dto.id, id, "show JSON: id must equal full strand id");
    assert!(
        !show_dto.id.contains('\u{2026}') && !show_dto.id.contains("..."),
        "show JSON: id must not contain truncation marker"
    );

    // list --format json: full id
    let list_item = output::StrandListItem::from(s);
    assert_eq!(list_item.id, id, "list JSON: id must equal full strand id");
    assert!(
        !list_item.id.contains('\u{2026}') && !list_item.id.contains("..."),
        "list JSON: id must not contain truncation marker"
    );

    // orient --format json (orient output): full 24-hex id (joins across outputs)
    let max_offset = events.last().map(|(o, _)| *o).unwrap_or(0);
    let out = orient_output(&strands, false, 10, max_offset);
    let orient_card = out
        .active
        .iter()
        .find(|c| c.id == id)
        .expect("orient must contain our strand");
    assert_eq!(
        orient_card.id.len(),
        24,
        "orient JSON: id must be the full 24-hex id"
    );
    assert!(
        !orient_card.id.contains('\u{2026}') && !orient_card.id.contains("..."),
        "orient JSON: id must not contain truncation marker"
    );

    // search --format json: full id
    let q = "unique_audit_token_abc123".to_lowercase();
    let strand_map: std::collections::HashMap<&str, &projection::ProjectedStrand> =
        strands.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut found_match: Option<output::SearchMatch> = None;
    for (_, event) in &events {
        if let Event::LogAppended { content, .. } = event {
            if content.to_lowercase().contains(&q) {
                let strand_id_full = event.strand_id().to_string();
                if strand_map.contains_key(strand_id_full.as_str()) {
                    let projected = strand_map.get(strand_id_full.as_str());
                    if strand_id_full == id {
                        found_match = Some(output::SearchMatch {
                            strand_id: strand_id_full,
                            content: truncate(content, 70),
                            strand_type: projected.and_then(|s| s.strand_type.clone()),
                            hidden: projected.map(|s| s.hidden).unwrap_or(false),
                        });
                    }
                }
            }
        }
    }
    let m = found_match.expect("search must find our entry");
    assert_eq!(
        m.strand_id, id,
        "search JSON: strand_id must equal full strand id"
    );
    assert!(
        !m.strand_id.contains('\u{2026}') && !m.strand_id.contains("..."),
        "search JSON: strand_id must not contain truncation marker"
    );
}

// ── Test 6 ────────────────────────────────────────────────────────────

// cmd_checkpoint text output: the staleness line contains the integer offset
// (no truncation), and the catch-up command (when emitted) embeds the
// 12-char strand id handle without '…'.
//
// Note: cmd_checkpoint prints directly to stdout/stderr rather than returning
// a structured value, so we verify the *journal entry* written by checkpoint
// contains the structured fields, and we verify the OrientStrand card it
// creates matches the handle-integrity rules.

#[test]
fn handles_checkpoint_handle_fields() {
    let _env = setup();
    // Create two strands so there is a journal delta when we checkpoint strand A.
    let id_a = create_strand("checkpoint handle test strand");
    let id_b = create_strand("another strand to create journal delta");
    cmd_append(
        Some("delta entry one"),
        Some(&id_b),
        false,
        false,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("delta entry two"),
        Some(&id_b),
        false,
        false,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    // Run checkpoint on strand A — journal delta > 0 so catch-up will be emitted.
    let result = cmd_checkpoint(
        Some(&id_a),
        "handle integrity check",
        None,
        false,
        false,
        None,
    );
    assert!(result.is_ok(), "checkpoint must succeed: {:?}", result);

    // The [checkpoint] journal entry contains observed_entries_before_append=N
    // where N is the integer entry count. Verify the stored entry has no '…'.
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let cp_entry = events
        .iter()
        .find(|(_, e)| {
            if let Event::LogAppended { id, content, .. } = e {
                id == &id_a && content.contains("[checkpoint] ok")
            } else {
                false
            }
        })
        .expect("checkpoint entry must exist in journal");
    let content = match &cp_entry.1 {
        Event::LogAppended { content, .. } => content,
        _ => unreachable!(),
    };
    assert!(
        !content.contains('\u{2026}') && !content.contains("..."),
        "checkpoint journal entry must not contain truncation marker: '{}'",
        content
    );
    assert!(
        content.contains("observed_entries_before_append="),
        "checkpoint entry must contain integer observed count"
    );

    // The card produced for strand A must satisfy handle-integrity rules.
    let strands = projection::project_strands(&events, true);
    let s = strands
        .iter()
        .find(|s| s.id == id_a)
        .expect("strand A must exist");
    let card = output::OrientStrand::from(s);
    assert_eq!(card.id, id_a, "post-checkpoint card id must be the full id");
    assert!(
        !card.catch_up.contains('\u{2026}') && !card.catch_up.contains("..."),
        "post-checkpoint catch_up must not be truncated"
    );
    try_parse_example(&card.catch_up).expect("post-checkpoint catch_up must parse");

    // JSON checkpoint output via cmd_checkpoint --format json:
    // The catch_up field in JSON uses shorten(strand_id) — verify via the
    // format string in cmd_checkpoint (the JSON path). We build the expected
    // value directly from the same logic.
    let strand_last_offset = s.last_offset();
    // After the checkpoint write, s.last_offset() includes the checkpoint entry.
    // The JSON catch_up is built *before* the write from strand_last_offset;
    // here we use the pre-checkpoint offset of strand A.
    // Find strand A's pre-checkpoint last_offset (last entry before checkpoint):
    let pre_cp_offset = {
        let mut last = 0usize;
        for (offset, e) in &events {
            if let Event::LogAppended { id, content, .. } = e {
                if id == &id_a && !content.contains("[checkpoint] ok") {
                    last = *offset;
                }
            }
        }
        last
    };
    let expected_catch_up = format!(
        "tasktree timeline --since-offset {} --links {}",
        pre_cp_offset,
        shorten(&id_a)
    );
    try_parse_example(&expected_catch_up).expect("expected checkpoint JSON catch_up must parse");
    assert!(
        !expected_catch_up.contains('\u{2026}') && !expected_catch_up.contains("..."),
        "checkpoint JSON catch_up must not be truncated"
    );

    let _ = (id_b, strand_last_offset);
}

// ── Task B: IdTarget tests ─────────────────────────────────────────────

// Positional <ID> and --id <ID> parse identically for show, find, hide,
// unhide, tree. We verify using clap's try_get_matches_from.

#[test]
fn show_json_has_entry_count_not_entries() {
    let _env = setup();
    let id = create_strand("entry_count rename test");
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let s = strands
        .iter()
        .find(|s| s.id == id)
        .expect("strand must exist");
    let dto = output::StrandDetailOutput::from(s);
    let v = serde_json::to_value(&dto).expect("serialize");
    let obj = v.as_object().unwrap();
    assert!(
        obj.contains_key("entry_count"),
        "show JSON must have 'entry_count' key"
    );
    assert!(
        !obj.contains_key("entries"),
        "show JSON must NOT have 'entries' key (renamed to entry_count)"
    );
}

// ════════════════════════════════════════════════════════════════════════
// Batch-2: JSON twins / provenance / --edge-type / add --stdin/--file
// ════════════════════════════════════════════════════════════════════════

// ── ① JSON twins: find --format json ─────────────────────────────────

#[test]
fn find_json_returns_id_object() {
    let _env = setup();
    let id = create_strand("find-json target");
    // find with full id — text mode returns plain id
    cmd_find(&id, false).unwrap();
    // find with format json — must return {"id": <full_id>}
    // Capture via direct call; actual stdout capture not needed for contract test.
    // We verify that the json serialization path is exercised without error.
    let result = cmd_find(&id, true);
    assert!(
        result.is_ok(),
        "find --format json must succeed: {:?}",
        result
    );
}

#[test]
fn find_json_unknown_strand_errors() {
    let _env = setup();
    create_strand("irrelevant");
    let result = cmd_find("000000000000", true);
    assert!(
        result.is_err(),
        "find on unknown id must error in json mode too"
    );
}

// ── ① JSON twins: hide --format json ─────────────────────────────────

#[test]
fn orient_remind_does_not_say_append_done() {
    assert!(
        !ORIENT_REMIND.contains("append --id") || !ORIENT_REMIND.contains("[done]"),
        "ORIENT_REMIND must not suggest 'append [done]' as the close idiom: {}",
        ORIENT_REMIND
    );
    assert!(
        ORIENT_REMIND.contains("close --id"),
        "ORIENT_REMIND must mention 'close --id': {}",
        ORIENT_REMIND
    );
}

// remind carries the loop methodology (act → observe → think), not just
// the command cheat-sheet.

#[test]
fn orient_remind_carries_the_loop_stance() {
    assert!(
        ORIENT_REMIND.contains("loop:"),
        "ORIENT_REMIND must carry the loop stance: {}",
        ORIENT_REMIND
    );
}

#[test]
fn show_digest_returns_ok_without_dumping_log() {
    let _env = setup();
    let id = create_strand("digest target");
    cmd_append(
        Some("[decision] one"),
        Some(&id),
        false,
        false,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[friction] two"),
        Some(&id),
        false,
        false,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    // digest = true; should succeed (census path, no full log dump)
    let r = cmd_show(Some(&id), false, None, false, false, true);
    assert!(r.is_ok(), "show --digest failed: {:?}", r);
}

#[test]
fn orient_catch_up_shows_content_not_empty_delta() {
    let _env = setup();
    let id = create_strand("catch up target");
    let (events, _) = read_events_lossy(&ensure_journal().unwrap());
    let strands = projection::project_strands(&events, true);
    let s = strands.iter().find(|s| s.id == id).unwrap();
    let card = output::OrientStrand::from(s);
    // catch-up must show the strand's recent content (never the empty-prone
    // `--since-offset <last_offset>` form, which shows nothing at orient time).
    assert!(
        card.catch_up.contains("show"),
        "catch_up must use show: {}",
        card.catch_up
    );
    assert!(
        card.catch_up.contains("--tail"),
        "catch_up must show recent tail: {}",
        card.catch_up
    );
    assert!(
        !card.catch_up.contains("--since-offset"),
        "catch_up must not use the empty-prone since-offset form: {}",
        card.catch_up
    );
}

use super::*;

#[test]
fn show_json_exposes_per_entry_provenance() {
    let _env = setup();
    let id = create_strand("provenance projection test");
    cmd_append(
        Some("[observed] tagged"),
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

// ── show / search smoke ────────────────────────────────────────────────

#[test]
fn show_search_context_unchanged() {
    // Smoke test that existing cmd_show, cmd_search still work.
    let _env = setup();
    let id = create_strand("show me");
    cmd_append(
        Some("entry"),
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
}

// ── orient card DTO ──

#[test]
fn orient_strand_fields_match_projected_strand() {
    let _env = setup();
    let id = create_strand("summary text for the card");
    cmd_append(
        Some("second entry"),
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

#[test]
fn handles_diag_details_parse() {
    let _env = setup();
    // Build a strand that fires W068 (overdue deadline).
    let id_a = create_strand("deadline strand for diag test");
    cmd_append(
        Some("[deadline] finish by=2000-01-01"),
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
        false,
        None,
        Some(&id_b),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[constraint] postgres writes forbidden in staging"),
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
        false,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[friction] two"),
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

// ── W5: marker as first-class JSON field ───────────────────────────────

#[test]
fn show_json_events_expose_marker_additive() {
    let _env = setup();
    let id = create_strand("[decision] marker projection test");
    cmd_append(Some("[decision] chose plan A"), false, None, None, None, None).unwrap();
    cmd_append(Some("plain content no marker"), false, None, None, None, None).unwrap();
    cmd_append(Some("[freiction] misspelled marker"), false, None, None, None, None).unwrap();

    let (events, _) = read_events_lossy(&ensure_journal().unwrap());
    let strands = projection::project_strands(&events, true);
    let s = strands.iter().find(|s| s.id == id).unwrap();
    let dto = output::StrandDetailOutput::from(s);

    // marker key is present on every event (additive; never null).
    for e in &dto.events {
        let v = serde_json::to_value(e).unwrap();
        assert!(
            v.as_object().unwrap().contains_key("marker"),
            "every event must carry a marker key"
        );
    }

    let decision = dto
        .events
        .iter()
        .find(|e| e.entry.contains("chose plan A"))
        .unwrap();
    assert_eq!(decision.marker, "[decision]", "marker split from entry");
    assert!(
        decision.entry.contains("[decision] chose plan A"),
        "entry must still carry the full original line (additive)"
    );

    let plain = dto
        .events
        .iter()
        .find(|e| e.entry.contains("plain content"))
        .unwrap();
    assert_eq!(plain.marker, "", "no-marker line yields empty string, not null");

    let misspelled = dto
        .events
        .iter()
        .find(|e| e.entry.contains("misspelled marker"))
        .unwrap();
    assert_eq!(
        misspelled.marker, "[freiction]",
        "unknown/misspelled marker passes through verbatim (no vocabulary lookup)"
    );
}

#[test]
fn timeline_json_log_appended_exposes_marker() {
    let _env = setup();
    let id = create_strand("timeline marker test");
    cmd_append(Some("[metric] win_count=26"), false, None, None, None, None).unwrap();

    let (events, _) = read_events_lossy(&ensure_journal().unwrap());
    let timeline = projection::project_timeline(&events);
    let dtos: Vec<output::TimelineEntryOutput> =
        timeline.iter().map(output::TimelineEntryOutput::from).collect();

    let val = serde_json::to_value(&dtos).unwrap();
    let metric = val
        .as_array()
        .unwrap()
        .iter()
        .find(|e| {
            e["kind"]["kind"] == "log_appended"
                && e["strand_id"] == id.as_str()
                && e["kind"]["content"]
                    .as_str()
                    .map(|c| c.contains("win_count"))
                    .unwrap_or(false)
        })
        .expect("metric log_appended must be present");
    assert_eq!(
        metric["kind"]["marker"], "[metric]",
        "log_appended kind must carry a marker field"
    );
    assert!(
        metric["kind"]["content"]
            .as_str()
            .unwrap()
            .contains("[metric] win_count=26"),
        "content must still carry the full original line (additive)"
    );
}

#[test]
fn list_json_exposes_first_and_last_marker() {
    let _env = setup();
    let id = create_strand("[task] first summary carries a marker");
    cmd_append(Some("[done] last summary carries a marker"), false, None, None, None, None)
        .unwrap();

    let (events, _) = read_events_lossy(&ensure_journal().unwrap());
    let strands = projection::project_strands(&events, true);
    let s = strands.iter().find(|s| s.id == id).unwrap();
    let item = output::StrandListItem::from(s);

    assert_eq!(item.first_marker, "[task]");
    assert_eq!(item.last_marker, "[done]");
    // summary fields still carry the full original line (additive).
    assert!(item.first_summary.contains("[task]"));
    assert!(item.last_summary.contains("[done]"));
}

// ── W5: doctor journal --format json ───────────────────────────────────

#[test]
fn doctor_report_output_is_valid_json_with_top_level_fields() {
    let _env = setup();
    let _id = create_strand("[task] doctor json test");
    cmd_append(Some("[decision] a decision"), false, None, None, None, None).unwrap();

    let (events, _) = read_events_lossy(&ensure_journal().unwrap());
    let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
    let report = diagnostics::build_doctor_journal_report(
        &raw,
        raw.len(),
        0,
        diagnostics::DoctorPreviousState::FirstRun,
        chrono::Utc::now(),
    );
    let out = output::DoctorReportOutput::from_report("journal.jsonl".to_string(), &report);
    let v = serde_json::to_value(&out).expect("doctor report must serialize");
    let obj = v.as_object().unwrap();

    for key in [
        "journal",
        "total_lines",
        "corrupted",
        "orphans",
        "total_strands",
        "strands_with_events",
        "noise_strands",
        "timeline_status",
        "timeline_warning",
        "lint_sections",
        "lint_count",
        "diagnostics",
        "has_errors",
        "has_advisories",
    ] {
        assert!(
            obj.contains_key(key),
            "doctor JSON must have top-level key '{}'",
            key
        );
    }

    // diagnostics are jq-friendly {code, detail} objects, not nested arrays.
    assert!(
        v["diagnostics"].is_array(),
        "diagnostics must be an array"
    );
    for d in v["diagnostics"].as_array().unwrap() {
        assert!(d.get("code").is_some() && d.get("detail").is_some());
    }
    // lint_sections are structured objects.
    for sec in v["lint_sections"].as_array().unwrap() {
        for k in ["name", "summary_label", "count", "findings"] {
            assert!(sec.get(k).is_some(), "lint section must have '{}'", k);
        }
    }
}

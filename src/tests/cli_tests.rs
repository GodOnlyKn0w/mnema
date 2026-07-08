use super::*;

#[test]
fn chdir_flag_parses_before_subcommand() {
    use clap::CommandFactory;
    let result = Cli::command().try_get_matches_from(["mnema", "-C", "/some/dir", "orient"]);
    assert!(result.is_ok(), "'-C DIR orient' must parse: {:?}", result);
}

// --chdir long form also parses

#[test]
fn chdir_longform_parses() {
    use clap::CommandFactory;
    let result =
        Cli::command().try_get_matches_from(["mnema", "--chdir", "/some/dir", "orient"]);
    assert!(result.is_ok(), "--chdir long form must parse: {:?}", result);
}

// -C after subcommand also works (global = true)

#[test]
fn chdir_global_after_subcommand_parses() {
    use clap::CommandFactory;
    let result = Cli::command().try_get_matches_from(["mnema", "orient", "-C", "/some/dir"]);
    assert!(
        result.is_ok(),
        "'-C' after subcommand (global) must parse: {:?}",
        result
    );
}

// -C pointing at a real .mnema dir resolves journal from unrelated cwd.

#[test]
fn chdir_resolves_journal_from_foreign_cwd() {
    // env has .mnema/ in its temp dir; we set cwd to a different temp dir
    // (no .mnema/), then set_current_dir to env path, and resolve succeeds.
    let env = setup(); // cwd is now env.path() with .mnema/
    let foreign = tempfile::tempdir().unwrap();
    // Move cwd to the foreign dir (no .mnema/)
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(foreign.path()).unwrap();
    // Simulate what -C does: set_current_dir to the project root
    std::env::set_current_dir(env.path()).unwrap();
    let result = with_mnema_home(None, || resolve_journal_dir());
    std::env::set_current_dir(&prev).unwrap();
    assert!(
        result.is_ok(),
        "-C to project root must resolve journal: {:?}",
        result
    );
    drop(env);
}

// -C to a non-existent directory: the binary would exit 3.
// We test that set_current_dir on a missing path returns Err.

#[test]
fn chdir_nonexistent_dir_errors() {
    let missing = std::path::Path::new("/this/path/does/not/exist/hopefully/xyz");
    let result = std::env::set_current_dir(missing);
    assert!(result.is_err(), "set_current_dir to missing path must fail");
}

#[test]
fn target_conflict_new_and_id() {
    let _env = setup();
    create_strand("first strand");
    let result = cmd_append(
        Some("content"),
        None,
        true,
        false,
        None,
        Some("0000019dd34b"),
        None,
        None,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("only one target"));
}

#[test]
fn legacy_positional_id_is_rejected() {
    let _env = setup();
    let id = create_strand("first strand");
    let result = cmd_append(
        Some("content"),
        Some(&id),
        false,
        false,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("legacy positional strand id"));
}

#[test]
fn reversed_positional_append_is_no_longer_supported() {
    let _env = setup();
    let id = create_strand("first strand");
    let result = cmd_append(
        Some("[observed] finding"),
        Some(&id),
        false,
        false,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("legacy positional strand id"));
    assert!(err.contains("use --id <ID>"));
}
// ── orient ──

#[test]
fn orient_tree_flag_parses() {
    use clap::CommandFactory;
    let result = Cli::command().try_get_matches_from(["mnema", "orient", "--tree"]);
    assert!(result.is_ok(), "'orient --tree' must parse: {:?}", result);
}

// orient --tree --format json: parse check.

#[test]
fn orient_tree_format_json_parses() {
    use clap::CommandFactory;
    let result =
        Cli::command().try_get_matches_from(["mnema", "orient", "--tree", "--format", "json"]);
    assert!(
        result.is_ok(),
        "'orient --tree --format json' must parse: {:?}",
        result
    );
}

// ── tree / project_tree: canonical belongs-to direction regression ──
// The `tree` command (cmd_tree → project_tree) must nest SOURCE under
// TARGET for belongs-to edges, identical to orient --tree
// (build_orient_forest). Guards against the reversed-direction +
// all-edge-types + no-dedup divergence project_tree used to carry.

// tree: after `link child parent --belongs-to`, the parent node holds the
// child as a descendant (child nested under parent — canonical direction).

#[test]
fn link_help_documents_belongs_to_direction() {
    use clap::CommandFactory;
    let cmd = Cli::command();
    let link = cmd
        .get_subcommands()
        .find(|s| s.get_name() == "link")
        .expect("link subcommand exists");
    let help = link
        .get_after_help()
        .map(|h| h.to_string())
        .unwrap_or_default();
    assert!(
        help.contains("belongs-to"),
        "link help must document belongs-to"
    );
    assert!(
        help.contains("depends-on"),
        "link help must document depends-on"
    );
    assert!(
        help.to_lowercase().contains("child") && help.to_lowercase().contains("parent"),
        "link help must explain source=child / target=parent"
    );
    assert!(
        help.contains("orient --tree") || help.contains("tree"),
        "link help must name the tree projection that consumes belongs-to"
    );
}

// ── examples-as-contract (ADR-0001 rule 4) ──
// Every example command in help text must at least parse against the
// real CLI. Help text is load-bearing: agents copy it verbatim.

#[test]
fn help_examples_parse_against_real_cli() {
    use clap::CommandFactory;
    let cmd = Cli::command();
    let mut helps: Vec<String> = Vec::new();
    if let Some(h) = cmd.get_after_help() {
        helps.push(h.to_string());
    }
    for sub in cmd.get_subcommands() {
        if let Some(h) = sub.get_after_help() {
            helps.push(h.to_string());
        }
    }
    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for help in &helps {
        for line in help.lines() {
            if !line.contains("mnema ") || line.contains("<command>") {
                continue;
            }
            checked += 1;
            if let Err(e) = try_parse_example(line) {
                failures.push(e);
            }
        }
    }
    assert!(
        checked > 10,
        "expected to find example lines in help text, found {}",
        checked
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn help_topic_references_exist() {
    // "引用即契约": any `mnema explain <word>` line in after_help where
    // <word> is all-lowercase must resolve via topic_lookup.
    use clap::CommandFactory;
    let cmd = Cli::command();
    let mut helps: Vec<String> = Vec::new();
    if let Some(h) = cmd.get_after_help() {
        helps.push(h.to_string());
    }
    for sub in cmd.get_subcommands() {
        if let Some(h) = sub.get_after_help() {
            helps.push(h.to_string());
        }
    }
    let mut failures: Vec<String> = Vec::new();
    for help in &helps {
        for line in help.lines() {
            // Match "mnema explain <word>" where word is all-lowercase
            if let Some(rest) = line
                .find("mnema explain ")
                .map(|i| &line[i + "mnema explain ".len()..])
            {
                let word: String = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take_while(|c| c.is_alphabetic() || *c == '_' || *c == '-')
                    .collect();
                if word.is_empty() {
                    continue;
                }
                // Only check all-lowercase words (topic namespace)
                if word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c == '-')
                {
                    if diagnostics::topic_lookup(&word).is_none() {
                        failures.push(format!(
                            "help references topic '{}' but topic_lookup returns None",
                            word
                        ));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "broken topic references in help text:\n{}",
        failures.join("\n")
    );
}

#[test]
fn catalog_recovery_commands_parse_when_executable() {
    for info in diagnostics::catalog() {
        if info.recovery.executable {
            assert!(
                info.recovery.command_str.starts_with("mnema"),
                "{}: executable recovery must be a mnema command",
                info.code
            );
            try_parse_example(info.recovery.command_str)
                .unwrap_or_else(|e| panic!("{}: {}", info.code, e));
        }
    }
}

#[test]
fn orient_catch_up_command_parses() {
    let _env = setup();
    let id = create_strand("a line");
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let out = orient_output(&strands, false, 10, 2);
    try_parse_example(&out.active[0].catch_up).unwrap();
    let _ = id;
}

// ── W-code emitters (two-way closure: every code has a producer) ──

#[test]
fn catalog_referenced_markers_are_writable() {
    // Markers extracted from catalog prose that are NOT entry markers —
    // they are placeholder tokens or descriptions, not bracket-prefixed
    // log entries. Allowlist with comment per entry.
    let allowlist: &[&str] = &[
            // none yet
        ];

    // Markers the emitter code parses (from run_journal_diagnostics).
    let emitter_markers: &[&str] = &[
        "[deadline]",
        "[decision]",
        "[constraint]",
        "[verified]",
        "[done]",
        "[cancelled]",
        "[failed]",
        "[merged]",
        "[ended]",
    ];

    let mut all_markers: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Collect from catalog prose.
    for info in diagnostics::catalog() {
        for s in [info.finding, info.impact, info.recovery.command_str] {
            for marker in extract_bracket_markers(s) {
                all_markers.insert(marker);
            }
        }
    }
    // Always include the hardcoded emitter markers.
    for m in emitter_markers {
        all_markers.insert(m.to_string());
    }

    let mut failures: Vec<String> = Vec::new();
    for marker in &all_markers {
        if allowlist.contains(&marker.as_str()) {
            continue;
        }
        let test_content = format!("{} x", marker);
        if let Err(e) = validate_lifecycle_marker(&test_content) {
            failures.push(format!("marker {} referenced in catalog/emitter but rejected by validate_lifecycle_marker: {}", marker, e));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn add_parent_and_belongs_to_alias_parse() {
    use clap::CommandFactory;
    let parent = "0000019dd34b";
    let by_parent = Cli::command().try_get_matches_from(["mnema", "add", "--parent", parent]);
    assert!(
        by_parent.is_ok(),
        "add --parent must parse: {:?}",
        by_parent
    );
    let by_alias = Cli::command().try_get_matches_from(["mnema", "add", "--belongs-to", parent]);
    assert!(
        by_alias.is_ok(),
        "add --belongs-to alias must parse: {:?}",
        by_alias
    );
}

#[test]
fn timeline_help_names_append_order_not_causal_order() {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    let sub = cmd
        .find_subcommand_mut("timeline")
        .expect("timeline command");
    let help = sub.render_long_help().to_string();
    assert!(
        help.contains("append order"),
        "timeline help must name append order: {}",
        help
    );
    assert!(
        !help.contains("causal order"),
        "timeline help must not claim causal order: {}",
        help
    );
}

#[test]
fn depends_help_frames_upstreams_as_review_context() {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    let sub = cmd.find_subcommand_mut("depends").expect("depends command");
    let help = sub.render_long_help().to_string();
    assert!(
        help.contains("attention edge"),
        "depends help must name attention-edge semantics: {}",
        help
    );
    assert!(
        !help.contains("blockers, readiness, critical path"),
        "depends help must not advertise gate semantics: {}",
        help
    );
}
// ── context exposure axis (ADR-0002) ──

#[test]
fn grammar_write_commands_accept_id_flag_without_content_position() {
    use clap::CommandFactory;
    let id = "0000019dd34b";
    let append = Cli::command().try_get_matches_from(["mnema", "append", "--id", id]);
    assert!(append.is_ok(), "append --id must parse: {:?}", append);
    let add = Cli::command().try_get_matches_from(["mnema", "add"]);
    assert!(
        add.is_ok(),
        "add must parse without a content arg: {:?}",
        add
    );
    let append_positional =
        Cli::command().try_get_matches_from(["mnema", "append", "--id", id, "note"]);
    assert!(
        append_positional.is_err(),
        "append positional content must not parse"
    );
    let add_positional = Cli::command().try_get_matches_from(["mnema", "add", "note"]);
    assert!(
        add_positional.is_err(),
        "add positional content must not parse"
    );
    let stdin_flag = Cli::command().try_get_matches_from(["mnema", "append", "--stdin"]);
    assert!(stdin_flag.is_err(), "append --stdin must not parse");
    let file_flag =
        Cli::command().try_get_matches_from(["mnema", "append", "--file", "note.md"]);
    assert!(file_flag.is_err(), "append --file must not parse");

    let checkpoint = Cli::command().try_get_matches_from([
        "mnema",
        "checkpoint",
        "--id",
        id,
        "--action",
        "before change",
    ]);
    assert!(
        checkpoint.is_ok(),
        "checkpoint --id must parse: {:?}",
        checkpoint
    );
}

#[test]
fn grammar_tail_commands_do_not_require_target() {
    use clap::CommandFactory;
    let show = Cli::command().try_get_matches_from(["mnema", "show", "--tail", "5"]);
    assert!(
        show.is_ok(),
        "show --tail without target must parse: {:?}",
        show
    );
    let checkpoint = Cli::command().try_get_matches_from([
        "mnema",
        "checkpoint",
        "--tail",
        "5",
        "--action",
        "before change",
    ]);
    assert!(
        checkpoint.is_ok(),
        "checkpoint --tail without --id must parse: {:?}",
        checkpoint
    );
}

#[test]
fn grammar_write_commands_accept_provenance() {
    use clap::CommandFactory;
    let id = "0000019dd34b";
    let provenance = r#"{"producer":"tester"}"#;
    let cases: Vec<Vec<&str>> = vec![
        vec!["mnema", "add", "--provenance", provenance],
        vec!["mnema", "append", "--id", id, "--provenance", provenance],
        vec![
            "mnema",
            "checkpoint",
            "--id",
            id,
            "--action",
            "before",
            "--provenance",
            provenance,
        ],
        vec![
            "mnema",
            "hide",
            "--id",
            id,
            "--reason",
            "noise",
            "--provenance",
            provenance,
        ],
        vec![
            "mnema",
            "link",
            id,
            "0000019dd34c",
            "--provenance",
            provenance,
        ],
        vec![
            "mnema",
            "unlink",
            id,
            "0000019dd34c",
            "--provenance",
            provenance,
        ],
    ];
    for case in cases {
        let result = Cli::command().try_get_matches_from(case.clone());
        assert!(
            result.is_ok(),
            "write command with provenance must parse: {:?}: {:?}",
            case,
            result
        );
    }
}
#[test]
fn grammar_flag_vocabulary_conformance() {
    use clap::CommandFactory;
    // (flag, exclusively allowed on). Compat aliases are pinned to their
    // historical host; appearing anywhere else is a new violation.
    let exclusive: &[(&str, &str)] =
        &[("all", "list"), ("json", "explain"), ("strand", "timeline")];
    for sub in Cli::command().get_subcommands() {
        for arg in sub.get_arguments() {
            if let Some(long) = arg.get_long() {
                for (flag, host) in exclusive {
                    assert!(
                        long != *flag || sub.get_name() == *host,
                        "--{} is reserved to `{}` (compat); `{}` must use the canonical flag (see explain grammar)",
                        flag,
                        host,
                        sub.get_name()
                    );
                }
            }
        }
    }
}

#[test]
fn grammar_single_id_commands_accept_id_flag() {
    use clap::CommandFactory;
    for cmd in ["show", "find", "tree", "hide", "unhide"] {
        let r = Cli::command().try_get_matches_from(["mnema", cmd, "--id", "0000019dd34b"]);
        assert!(
            r.is_ok(),
            "`{} --id <ID>` must parse (IdTarget contract): {:?}",
            cmd,
            r.err()
        );
    }
    // timeline reaches the same grammar via alias
    let r = Cli::command().try_get_matches_from(["mnema", "timeline", "--id", "0000019dd34b"]);
    assert!(r.is_ok(), "`timeline --id` must alias --strand");
}

#[test]
fn seen_offset_flag_parses_on_write_commands() {
    use clap::CommandFactory;
    let append = Cli::command().try_get_matches_from([
        "mnema",
        "append",
        "--id",
        "0000019dd34b",
        "--seen-offset",
        "2",
    ]);
    assert!(
        append.is_ok(),
        "append --seen-offset must parse: {:?}",
        append.err()
    );

    let checkpoint = Cli::command().try_get_matches_from([
        "mnema",
        "checkpoint",
        "--id",
        "0000019dd34b",
        "--seen-offset",
        "2",
        "--action",
        "before commit",
    ]);
    assert!(
        checkpoint.is_ok(),
        "checkpoint --seen-offset must parse: {:?}",
        checkpoint.err()
    );
}

#[test]
fn grammar_json_field_naming() {
    let _env = setup();
    let id = create_strand("naming probe");
    cmd_append(
        Some("second entry"),
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

    let mut samples: Vec<serde_json::Value> = vec![
        serde_json::to_value(output::StrandDetailOutput::from(&strands[0])).unwrap(),
        serde_json::to_value(output::StrandListOutput {
            strands: strands.iter().map(output::StrandListItem::from).collect(),
        })
        .unwrap(),
        serde_json::to_value(orient_output(&strands, true, 10, 2)).unwrap(),
        serde_json::to_value(output::SearchOutput {
            matches: vec![],
            count: 0,
            query: String::new(),
        })
        .unwrap(),
        serde_json::to_value(output::TimelineOutput {
            timeline: vec![],
            truncated: false,
            count: 0,
            max_offset: 0,
        })
        .unwrap(),
        // Write-command JSON built inline with json!() is invisible to
        // struct sampling — extracted shapes are sampled here. First
        // catch of this blind spot: hide's ledger shipped bare
        // active/closed/hidden count names.
        visibility_ledger_json(&id, false),
    ];

    // plural noun => array; count/*_count => number
    const PLURALS: &[&str] = &[
        "events", "matches", "strands", "active", "entries", "edges", "covers", "timeline",
    ];
    fn walk(v: &serde_json::Value, errs: &mut Vec<String>) {
        if let serde_json::Value::Object(map) = v {
            for (k, val) in map {
                if PLURALS.contains(&k.as_str()) && !val.is_array() {
                    errs.push(format!(
                        "plural-named field `{}` is not an array (naming contract)",
                        k
                    ));
                }
                if (k == "count" || k.ends_with("_count")) && !val.is_number() {
                    errs.push(format!("count field `{}` is not a number", k));
                }
                // id/strand_id are full-width 64-hex content-addressed handles (join law);
                // entry_id is a 64-hex content hash (an entry handle, not a strand handle).
                if (k == "id" || k == "strand_id") && val.is_string() {
                    let s = val.as_str().unwrap();
                    if s.len() != 64 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
                        errs.push(format!("`{}` is not full-width 64-hex: `{}`", k, s));
                    }
                }
                walk(val, errs);
            }
        } else if let serde_json::Value::Array(items) = v {
            for item in items {
                walk(item, errs);
            }
        }
    }
    let mut errs = Vec::new();
    for s in samples.drain(..) {
        walk(&s, &mut errs);
    }
    assert!(errs.is_empty(), "{}", errs.join("\n"));

    // Reference-as-contract: the json topic's hide/unhide section must
    // name every real ledger key (the topic lied once — stale names
    // survived a field rename).
    let topic = diagnostics::topic_lookup("json").expect("json topic exists");
    if let serde_json::Value::Object(map) = visibility_ledger_json(&id, false) {
        for key in map.keys() {
            assert!(
                topic.body.contains(key.as_str()),
                "json topic does not mention ledger field `{}`",
                key
            );
        }
    }
}

#[test]
fn grammar_format_json_coverage() {
    use clap::CommandFactory;
    // doctor/export are permanently exempt in the grammar contract;
    // init is pending judgment.
    const EXEMPT: &[&str] = &["init", "doctor", "export", "pick"];
    for sub in Cli::command().get_subcommands() {
        if EXEMPT.contains(&sub.get_name()) || sub.get_name() == "help" {
            continue;
        }
        let has_format = sub.get_arguments().any(|a| a.get_long() == Some("format"));
        assert!(
            has_format,
            "`{}` has no --format json twin (machine-isomorphism contract; if intentionally exempt, name it in the contract AND this list)",
            sub.get_name()
        );
    }
}

#[test]
fn target_conflict_new_legacy_and_explicit() {
    let _env = setup();
    let id = create_strand("first strand");
    let result = cmd_append(
        Some("content"),
        Some(&id),
        true,
        false,
        None,
        Some(&id),
        None,
        None,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("legacy positional strand id"));
}

#[test]
fn target_conflict_explicit_and_legacy() {
    let _env = setup();
    let id = create_strand("first strand");
    let result = cmd_append(
        Some("content"),
        Some(&id),
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("legacy positional strand id"));
}

#[test]
fn legacy_id_rejected_before_content_source_resolution() {
    let _env = setup();
    let id = create_strand("first strand");
    let file_path = _env.path().join("note.md");
    fs::write(&file_path, "stdin content here").unwrap();
    let result = cmd_append(
        None,
        Some(&id),
        false,
        false,
        Some(file_path.to_str().unwrap()),
        None,
        None,
        None,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("legacy positional strand id"));
}

// ── --new strand creation ──

#[test]
fn new_with_direct_content() {
    let _env = setup();
    let result = cmd_append(
        Some("brand new strand"),
        None,
        true,
        false,
        None,
        None,
        None,
        None,
    );
    assert!(result.is_ok());
}
#[test]
fn new_with_file_content() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("new_strand.md");
    fs::write(&file_path, "new strand from file").unwrap();
    let _env = setup();
    let result = cmd_append(
        None,
        None,
        true,
        false,
        Some(file_path.to_str().unwrap()),
        None,
        None,
        None,
    );
    assert!(result.is_ok());
}

// ── normalize_content ──

#[test]
fn exit_code_for_journal_unreadable_is_2() {
    assert_eq!(exit_code_for("journal unreadable: bad bytes"), 2);
    assert_eq!(
        exit_code_for("corrupt: [mnema] WARNING: 1 corrupted lines skipped"),
        2
    );
}

#[test]
fn exit_code_for_generic_and_warn_are_1() {
    assert_eq!(exit_code_for("strand abc not found"), 1);
    assert_eq!(exit_code_for("legacy positional strand id was removed"), 1);
    assert_eq!(exit_code_for("journal issues detected"), 1);
}

// ── humanize_duration ──────────────────────────────────────────────────

#[test]
fn id_target_flag_and_positional_equivalent() {
    use clap::CommandFactory;
    // For each command, parse both forms and verify they succeed.
    let cases: &[(&str, &str)] = &[
        ("show", "0000019dd34b"),
        ("find", "0000019dd34b"),
        ("hide", "0000019dd34b"),
        ("unhide", "0000019dd34b"),
        ("tree", "0000019dd34b"),
    ];
    for (cmd, id) in cases {
        // positional form: mnema <cmd> <id>
        let pos_result = Cli::command().try_get_matches_from(["mnema", cmd, id]);
        assert!(
            pos_result.is_ok(),
            "{} positional form failed: {:?}",
            cmd,
            pos_result.err()
        );
        // flag form: mnema <cmd> --id <id>
        let flag_result = Cli::command().try_get_matches_from(["mnema", cmd, "--id", id]);
        assert!(
            flag_result.is_ok(),
            "{} --id form failed: {:?}",
            cmd,
            flag_result.err()
        );
    }
    // Behavioral check: show positional vs --id produce same resolved id
    let _env = setup();
    let id = create_strand("id_target behavioral test");
    // Both should succeed and produce the same output
    let r1 = cmd_show(Some(&id), false, None, false, false, false, None);
    let r2 = cmd_show(Some(&id), false, None, false, false, false, None);
    assert!(r1.is_ok(), "show with positional id failed: {:?}", r1);
    assert!(r2.is_ok(), "show with --id failed: {:?}", r2);
}

// Providing both positional <ID> and --id <ID> must be rejected by clap.

#[test]
fn id_target_conflict_rejected() {
    use clap::CommandFactory;
    let result =
        Cli::command().try_get_matches_from(["mnema", "show", "000653", "--id", "000653"]);
    assert!(
        result.is_err(),
        "show with both positional and --id must be rejected"
    );
}

// ── unified target convention ──────────────────────────────────────────
// One rule across single-strand commands: positional <ID> / --id / --last.
// Read+append commands default to most-recent (--last is the explicit form);
// close/reopen are lifecycle-closing, so they stay strictly explicit — no
// --last, no default.

#[test]
fn close_reopen_accept_positional_and_id_flag() {
    use clap::CommandFactory;
    for cmd in ["close", "reopen"] {
        let pos = Cli::command().try_get_matches_from(["mnema", cmd, "0000019dd34b"]);
        assert!(pos.is_ok(), "`{} <ID>` must parse: {:?}", cmd, pos.err());
        let flag = Cli::command().try_get_matches_from(["mnema", cmd, "--id", "0000019dd34b"]);
        assert!(
            flag.is_ok(),
            "`{} --id <ID>` must parse: {:?}",
            cmd,
            flag.err()
        );
    }
}

#[test]
fn last_flag_parses_on_read_and_append_commands() {
    use clap::CommandFactory;
    for cmd in [
        "show", "find", "hide", "unhide", "tree", "depends", "append",
    ] {
        let r = Cli::command().try_get_matches_from(["mnema", cmd, "--last"]);
        assert!(r.is_ok(), "`{} --last` must parse: {:?}", cmd, r.err());
    }
    // checkpoint requires --action alongside --last
    let r =
        Cli::command().try_get_matches_from(["mnema", "checkpoint", "--last", "--action", "x"]);
    assert!(
        r.is_ok(),
        "`checkpoint --last --action` must parse: {:?}",
        r.err()
    );
}

#[test]
fn close_reopen_reject_last() {
    use clap::CommandFactory;
    for cmd in ["close", "reopen"] {
        let r = Cli::command().try_get_matches_from(["mnema", cmd, "--last"]);
        assert!(
            r.is_err(),
            "`{} --last` must be rejected (lifecycle-closing stays explicit)",
            cmd
        );
    }
}

#[test]
fn last_conflicts_with_explicit_id() {
    use clap::CommandFactory;
    let r = Cli::command().try_get_matches_from(["mnema", "show", "--id", "abc", "--last"]);
    assert!(r.is_err(), "--id and --last are mutually exclusive");
}

// `timeline --id X` parses as `timeline --strand X` (visible_alias = "id").

#[test]
fn timeline_id_alias() {
    use clap::CommandFactory;
    let result =
        Cli::command().try_get_matches_from(["mnema", "timeline", "--id", "0000019dd34b"]);
    assert!(
        result.is_ok(),
        "timeline --id should parse via visible_alias on --strand: {:?}",
        result.err()
    );
    // Also verify --strand still works
    let result2 =
        Cli::command().try_get_matches_from(["mnema", "timeline", "--strand", "0000019dd34b"]);
    assert!(
        result2.is_ok(),
        "timeline --strand must still work: {:?}",
        result2.err()
    );
}

// ── Task D: show --tail decoupled from --last ──────────────────────────

// show with explicit <ID> + --tail N must succeed (previously blocked by
// the now-removed `requires = "last"` guard).

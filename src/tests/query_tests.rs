use super::*;

#[test]
fn orient_menu_shows_active_folds_closed() {
    let _env = setup();
    let open_id = create_strand("open line of work");
    let done_id = create_strand("finished line");
    cmd_close(&done_id, None, None, false).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let max_offset = events.last().map(|(o, _)| *o).unwrap();
    // orient_output always receives the full strand list (include_hidden=true
    // in projection); the visible/hidden split is done inside the orient view.
    let strands = projection::project_strands(&events, true);
    let out = orient_output(&strands, false, 10, max_offset);

    assert_eq!(out.max_offset, max_offset);
    assert_eq!(out.active.len(), 1);
    assert_eq!(out.closed_count, 1);
    let entry = &out.active[0];
    assert_eq!(entry.id, open_id);
    assert_eq!(entry.summary, "open line of work");
    // Catch-up is copy-paste runnable and shows the strand's recent
    // content (show --tail), never the empty-prone since-offset delta.
    // It carries the short prefix handle: human-facing views never spend
    // a full 64-hex hash where a resolvable prefix works.
    assert_eq!(
        entry.catch_up,
        format!(
            "tasktree show --id {} --tail 8",
            crate::util::shorten(&open_id)
        )
    );
    assert!(out.remind.contains("checkpoint"));
    assert!(
        out.remind.contains("matter concluded"),
        "remind must carry the closing segment"
    );
}

#[test]
fn orient_hidden_count_reflects_scar_principle() {
    let _env = setup();
    let open_id = create_strand("open work");
    let hidden_id = create_strand("will be hidden");
    cmd_hide(&hidden_id, None, false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let max_offset = events.last().map(|(o, _)| *o).unwrap();
    let strands = projection::project_strands(&events, true);

    // Default view (include_hidden=false): hidden strand must be absent
    // from active/closed pools but counted in hidden_count.
    let out = orient_output(&strands, false, 10, max_offset);
    assert_eq!(
        out.hidden_count, 1,
        "hidden strand must appear in hidden_count"
    );
    assert_eq!(
        out.closed_count, 0,
        "hidden strand must not inflate closed_count"
    );
    let active_ids: Vec<&str> = out.active.iter().map(|s| s.id.as_str()).collect();
    assert!(
        active_ids.contains(&open_id.as_str()),
        "visible strand must be in menu"
    );
    assert!(
        !active_ids.contains(&hidden_id.as_str()),
        "hidden strand absent from menu"
    );

    // include_hidden=true: hidden strand joins the pool; hidden_count=0.
    let out_all = orient_output(&strands, true, 10, max_offset);
    assert_eq!(
        out_all.hidden_count, 0,
        "include_hidden=true must yield hidden_count=0"
    );
    let all_ids: Vec<&str> = out_all.active.iter().map(|s| s.id.as_str()).collect();
    assert!(
        all_ids.contains(&hidden_id.as_str()),
        "include_hidden=true puts hidden strand in menu"
    );
}

#[test]
fn orient_limit_keeps_most_recent() {
    let _env = setup();
    let older = create_strand("older line");
    let newer = create_strand("newer line");
    cmd_append(
        Some("touched again"),
        None,
        false,
        false,
        None,
        Some(&older),
        None,
        None,
    )
    .unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let out = orient_output(&strands, false, 1, events.last().map(|(o, _)| *o).unwrap());

    assert_eq!(out.active.len(), 1);
    // `older` was touched last, so it outranks `newer` in the menu.
    assert_eq!(out.active[0].id, older);
    let _ = newer;
}

// ── orient --tree: belongs-to forest regression tests ──

// Default orient (no --tree) is unchanged when belongs-to edges exist.
// Regression guard: --tree is strictly opt-in.

#[test]
fn orient_flat_unaffected_by_belongs_to_edges() {
    let _env = setup();
    let parent = create_strand("parent task");
    let child = create_strand("child task");
    cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let max_offset = events.last().map(|(o, _)| *o).unwrap();
    let strands = projection::project_strands(&events, true);
    let out = orient_output(&strands, false, 10, max_offset);

    // Flat orient must still return both strands in a flat list
    assert_eq!(out.active.len(), 2, "flat orient: both strands must appear");
    let ids: Vec<&str> = out.active.iter().map(|s| s.id.as_str()).collect();
    assert!(
        ids.contains(&parent.as_str()),
        "flat orient: parent must appear"
    );
    assert!(
        ids.contains(&child.as_str()),
        "flat orient: child must appear"
    );
}

// orient --tree: child declared with belongs-to appears nested under parent.

#[test]
fn orient_tree_nests_belongs_to_child_under_parent() {
    let _env = setup();
    let parent = create_strand("parent task");
    let child = create_strand("child task");
    cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let max_offset = events.last().map(|(o, _)| *o).unwrap();
    let strands = projection::project_strands(&events, true);
    let out = orient_output(&strands, false, 10, max_offset);

    // Build active projection strands for tree construction (mirror of cmd_orient logic)
    let active_strands: Vec<&projection::ProjectedStrand> = out
        .active
        .iter()
        .filter_map(|card| strands.iter().find(|s| s.id == card.id))
        .collect();
    let roots = tree::build_orient_forest(&active_strands);

    // Parent is a root; child is nested under it
    assert_eq!(roots.len(), 1, "orient --tree: only the parent is a root");
    assert_eq!(roots[0].id, parent, "root must be the parent strand");
    assert_eq!(roots[0].children.len(), 1, "parent must have one child");
    assert_eq!(
        roots[0].children[0].id, child,
        "child must be nested under parent"
    );
}

// orient --tree: parallel siblings under same parent are both visible.

#[test]
fn orient_tree_parallel_siblings_visible_under_parent() {
    let _env = setup();
    let parent = create_strand("parent task");
    let sibling_a = create_strand("sibling A");
    let sibling_b = create_strand("sibling B");
    cmd_link(&sibling_a, &parent, Some("belongs-to"), false, None).unwrap();
    cmd_link(&sibling_b, &parent, Some("belongs-to"), false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let max_offset = events.last().map(|(o, _)| *o).unwrap();
    let strands = projection::project_strands(&events, true);
    let out = orient_output(&strands, false, 10, max_offset);

    let active_strands: Vec<&projection::ProjectedStrand> = out
        .active
        .iter()
        .filter_map(|card| strands.iter().find(|s| s.id == card.id))
        .collect();
    let roots = tree::build_orient_forest(&active_strands);

    assert_eq!(roots.len(), 1, "only parent is a root");
    assert_eq!(roots[0].id, parent, "root is the parent");
    assert_eq!(
        roots[0].children.len(),
        2,
        "both siblings must appear under parent"
    );
    let child_ids: Vec<&str> = roots[0].children.iter().map(|n| n.id.as_str()).collect();
    assert!(
        child_ids.contains(&sibling_a.as_str()),
        "sibling A must be visible"
    );
    assert!(
        child_ids.contains(&sibling_b.as_str()),
        "sibling B must be visible"
    );
}

// orient --tree: orphan strands (no belongs-to edge or parent not in active set)
// appear as top-level roots.

#[test]
fn orient_tree_orphan_strands_are_roots() {
    let _env = setup();
    let orphan_a = create_strand("orphan A (no edges)");
    let orphan_b = create_strand("orphan B (no edges)");

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let max_offset = events.last().map(|(o, _)| *o).unwrap();
    let strands = projection::project_strands(&events, true);
    let out = orient_output(&strands, false, 10, max_offset);

    let active_strands: Vec<&projection::ProjectedStrand> = out
        .active
        .iter()
        .filter_map(|card| strands.iter().find(|s| s.id == card.id))
        .collect();
    let roots = tree::build_orient_forest(&active_strands);

    assert_eq!(roots.len(), 2, "both orphan strands must appear as roots");
    let root_ids: Vec<&str> = roots.iter().map(|n| n.id.as_str()).collect();
    assert!(
        root_ids.contains(&orphan_a.as_str()),
        "orphan A must be a root"
    );
    assert!(
        root_ids.contains(&orphan_b.as_str()),
        "orphan B must be a root"
    );
    for root in &roots {
        assert!(
            root.children.is_empty(),
            "orphan nodes must have no children"
        );
    }
}

// orient --tree: no contention/conflict markers are emitted (precision discipline).

#[test]
fn orient_tree_no_contention_markers() {
    let _env = setup();
    let parent = create_strand("parent");
    let child_a = create_strand("child A");
    let child_b = create_strand("child B");
    cmd_link(&child_a, &parent, Some("belongs-to"), false, None).unwrap();
    cmd_link(&child_b, &parent, Some("belongs-to"), false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let max_offset = events.last().map(|(o, _)| *o).unwrap();
    let strands = projection::project_strands(&events, true);
    let out = orient_output(&strands, false, 10, max_offset);

    let active_strands: Vec<&projection::ProjectedStrand> = out
        .active
        .iter()
        .filter_map(|card| strands.iter().find(|s| s.id == card.id))
        .collect();
    let roots = tree::build_orient_forest(&active_strands);
    let roots: Vec<output::OrientForestNode> =
        roots.iter().map(output::OrientForestNode::from).collect();

    // Serialize to JSON and assert no "contention" word appears
    let json_str = serde_json::to_string(&roots).unwrap();
    assert!(
        !json_str.contains("contention"),
        "orient --tree JSON must not emit contention markers"
    );
    assert!(
        !json_str.contains("conflict"),
        "orient --tree JSON must not emit conflict markers"
    );
}

// orient --tree --format json: JSON structure is nested (roots array with children).

#[test]
fn orient_tree_json_shape_is_nested() {
    let _env = setup();
    let parent = create_strand("parent task");
    let child = create_strand("child task");
    cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let max_offset = events.last().map(|(o, _)| *o).unwrap();
    let strands = projection::project_strands(&events, true);
    let out = orient_output(&strands, false, 10, max_offset);

    let active_strands: Vec<&projection::ProjectedStrand> = out
        .active
        .iter()
        .filter_map(|card| strands.iter().find(|s| s.id == card.id))
        .collect();
    let roots = tree::build_orient_forest(&active_strands);
    let roots: Vec<output::OrientForestNode> =
        roots.iter().map(output::OrientForestNode::from).collect();
    let tree_out = output::OrientTreeOutput {
        max_offset,
        roots,
        closed_count: out.closed_count,
        hidden_count: out.hidden_count,
        integrity: out.integrity.clone(),
        notices: out.notices.clone(),
        remind: out.remind.clone(),
        pause: out.pause.clone(),
    };

    let json_str = serde_json::to_string(&tree_out).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    // Top-level has "roots" array
    assert!(
        parsed["roots"].is_array(),
        "orient --tree JSON must have 'roots' array"
    );
    let roots_arr = parsed["roots"].as_array().unwrap();
    assert_eq!(roots_arr.len(), 1, "one root (the parent)");

    // Root has "id", "children" fields
    let root = &roots_arr[0];
    assert_eq!(
        root["id"].as_str().unwrap(),
        parent.as_str(),
        "root id matches parent"
    );
    assert!(
        root["children"].is_array(),
        "root must have 'children' array"
    );
    let children = root["children"].as_array().unwrap();
    assert_eq!(children.len(), 1, "root has one child");
    assert_eq!(
        children[0]["id"].as_str().unwrap(),
        child.as_str(),
        "child id matches"
    );

    // Verify no extra fields added/removed (additive-only contract)
    assert!(
        parsed["max_offset"].is_number(),
        "max_offset must be present"
    );
    assert!(
        parsed["closed_count"].is_number(),
        "closed_count must be present"
    );
    assert!(
        parsed["hidden_count"].is_number(),
        "hidden_count must be present"
    );
    assert!(parsed["remind"].is_string(), "remind must be present");
}

// orient --tree: parse check — `tasktree orient --tree` is a valid CLI invocation.

#[test]
fn tree_nests_belongs_to_child_under_parent() {
    let _env = setup();
    let parent = create_strand("parent task");
    let child = create_strand("child task");
    cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);

    // Root the tree at the parent: parent must own the child.
    let root = tree::project_tree(&parent, &strands).expect("parent resolves");
    assert_eq!(root.id, parent, "tree root rooted at parent is the parent");
    assert_eq!(root.children.len(), 1, "parent must have exactly one child");
    assert_eq!(root.children[0].id, child, "child must nest under parent");
    assert!(root.children[0].children.is_empty(), "child is a leaf");
}

// tree: rooting at the child must NOT pull the parent in as a descendant.
// Direct regression on the old reversed direction (which nested parent
// under child by walking source→target as parent→child).

#[test]
fn tree_rooted_at_child_does_not_contain_parent() {
    let _env = setup();
    let parent = create_strand("parent task");
    let child = create_strand("child task");
    cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);

    let root = tree::project_tree(&child, &strands).expect("child resolves");
    assert_eq!(root.id, child, "root rooted at child is the child");
    assert!(
        root.children.is_empty(),
        "child has no descendants; parent must not be nested under it (reversed-direction guard)"
    );
}

// tree and orient --tree must agree on parent→child nesting for the same
// journal: single source of truth across both builders.

#[test]
fn tree_and_orient_forest_agree_on_nesting() {
    let _env = setup();
    let parent = create_strand("parent task");
    let child_a = create_strand("child A");
    let child_b = create_strand("child B");
    cmd_link(&child_a, &parent, Some("belongs-to"), false, None).unwrap();
    cmd_link(&child_b, &parent, Some("belongs-to"), false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let max_offset = events.last().map(|(o, _)| *o).unwrap();
    let strands = projection::project_strands(&events, true);

    // project_tree (cmd_tree) view
    let root = tree::project_tree(&parent, &strands).expect("parent resolves");
    let mut tree_child_ids: Vec<String> = root.children.iter().map(|c| c.id.clone()).collect();
    tree_child_ids.sort();

    // build_orient_forest (orient --tree) view
    let out = orient_output(&strands, false, 10, max_offset);
    let active_strands: Vec<&projection::ProjectedStrand> = out
        .active
        .iter()
        .filter_map(|card| strands.iter().find(|s| s.id == card.id))
        .collect();
    let roots = tree::build_orient_forest(&active_strands);
    let parent_root = roots
        .iter()
        .find(|n| n.id == parent)
        .expect("parent is a root in the forest");
    let mut forest_child_ids: Vec<String> =
        parent_root.children.iter().map(|c| c.id.clone()).collect();
    forest_child_ids.sort();

    assert_eq!(
        tree_child_ids, forest_child_ids,
        "tree and orient --tree must list the same children under the parent"
    );
    let mut expected_child_ids = vec![child_a.clone(), child_b.clone()];
    expected_child_ids.sort();
    assert_eq!(tree_child_ids, expected_child_ids);
}

// tree: a duplicate belongs-to link must not double-project the child.
// Read-side dedup folds repeated link entries (journal keeps both).

#[test]
fn tree_duplicate_belongs_to_link_does_not_double_project() {
    let _env = setup();
    let parent = create_strand("parent task");
    let child = create_strand("child task");
    // Link twice — same source, same target, same edge type.
    cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();
    cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);

    // The journal is append-only: both link effect entries are present.
    let link_events = events
        .iter()
        .filter(|(_, e)| {
            matches!(
                e,
                Event::LogAppended {
                    effect: Some(event::EntryEffect::Link { .. }),
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        link_events, 2,
        "journal keeps both link entries (append-only)"
    );

    // The projection folds them: belongs_to_edges holds one entry.
    let strands = projection::project_strands(&events, true);
    let child_proj = strands.iter().find(|s| s.id == child).unwrap();
    assert_eq!(
        child_proj.belongs_to_edges.len(),
        1,
        "duplicate links fold to one belongs_to edge in the projection"
    );

    // And the tree shows the child exactly once.
    let root = tree::project_tree(&parent, &strands).expect("parent resolves");
    assert_eq!(
        root.children.len(),
        1,
        "child must appear exactly once under parent"
    );
    assert_eq!(root.children[0].id, child);
}

// tree: non-belongs-to edges (depends-on) do not form the strand tree.
// project_tree uses belongs_to_edges only — a depends-on link must not nest.

#[test]
fn tree_ignores_non_belongs_to_edges() {
    let _env = setup();
    let task = create_strand("task");
    let blocker = create_strand("blocker");
    cmd_link(&task, &blocker, Some("depends-on"), false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);

    // Rooting at either end yields a lone node — depends-on does not nest.
    let root_task = tree::project_tree(&task, &strands).expect("task resolves");
    assert!(
        root_task.children.is_empty(),
        "depends-on must not nest under source"
    );
    let root_blocker = tree::project_tree(&blocker, &strands).expect("blocker resolves");
    assert!(
        root_blocker.children.is_empty(),
        "depends-on must not nest under target"
    );
}

// subtree_ids: descends from root through belongs-to children (same
// canonical direction as project_tree).

#[test]
fn subtree_ids_descends_through_belongs_to_children() {
    let _env = setup();
    let parent = create_strand("parent");
    let child = create_strand("child");
    let grandchild = create_strand("grandchild");
    cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();
    cmd_link(&grandchild, &child, Some("belongs-to"), false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);

    // From parent: the whole chain is reachable.
    let from_parent = tree::subtree_ids(&parent, &strands).expect("parent resolves");
    assert!(from_parent.contains(&parent), "root included");
    assert!(from_parent.contains(&child), "child reachable from parent");
    assert!(
        from_parent.contains(&grandchild),
        "grandchild reachable from parent"
    );

    // From child: parent is an ancestor, must NOT be in the descendant set.
    let from_child = tree::subtree_ids(&child, &strands).expect("child resolves");
    assert!(from_child.contains(&child), "root included");
    assert!(
        from_child.contains(&grandchild),
        "grandchild reachable from child"
    );
    assert!(
        !from_child.contains(&parent),
        "parent (ancestor) must not be in subtree"
    );
}

// link --help carries the direction semantics required by the work order:
// belongs-to marks source as child of target, and names tree / orient --tree.

#[test]
fn orient_is_pure_read() {
    let _env = setup();
    create_strand("a line");
    let path = ensure_journal().unwrap();
    let before = std::fs::read(&path).unwrap();
    cmd_orient(None, false, None, false).unwrap();
    cmd_orient(Some("json"), true, Some(3), false).unwrap();
    let after = std::fs::read(&path).unwrap();
    assert_eq!(before, after, "orient must never write to the journal");
}

#[test]
fn export_creates_file_with_metadata_header() {
    let _env = setup();
    create_strand("test export");

    let out = _env.path().join("export.jsonl");
    let out_str = out.to_str().unwrap();
    let result = cmd_export(out_str);
    assert!(result.is_ok());

    let exported = std::fs::read_to_string(&out).unwrap();
    let lines: Vec<&str> = exported.lines().collect();
    assert!(lines.len() >= 2);

    let meta: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(meta["type"], "export_metadata");
    assert_eq!(meta["source"], "tasktree export");
    assert!(meta["journal_lines"].as_u64().unwrap() > 0);
}

// `cmd_export` against a missing journal must fail. The error
// contract is `Err(...)` with a stable prefix; the OS-level wording
// after the prefix is locale-dependent (e.g. EN: "cannot read journal:
// ..."  /  ZH: "cannot read journal: 系统找不到指定的文件。 ..."),
// so we assert on the stable prefix only, not the full message.
//
// Also: this test uses an isolated temp dir + `TASKTREE_HOME` (via
// `with_tasktree_home`) so it cannot pollute the shared test
// environment. We never `remove_file` on a journal another test
// might be using, and we never panic while holding `CWD_LOCK` (the
// assertion below is a single guarded check, not a multi-step
// sequence that can partial-fail).

#[test]
fn export_no_journal_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    // Create `.tasktree/` but DO NOT create `journal.jsonl` inside it.
    // `resolve_journal_dir` succeeds (it only needs the dir to exist);
    // `cmd_export` then fails at the actual `std::fs::read` step
    // because the journal file is missing. This mirrors the user's
    // experience: a project where `.tasktree/` exists but no journal
    // has been written yet (e.g. first run after `tasktree init`).
    let tasktree = dir.path().join(".tasktree");
    std::fs::create_dir_all(&tasktree).unwrap();
    let out = dir.path().join("nojournal_export.jsonl");
    with_tasktree_home(Some(dir.path().to_str().unwrap()), || {
        let result = cmd_export(out.to_str().unwrap());
        let err = result.expect_err("cmd_export must return Err when no journal exists");
        assert!(
            err.starts_with("cannot read journal"),
            "expected stable 'cannot read journal' prefix, got: {err}"
        );
        // Output file must not have been created.
        assert!(!out.exists(), "export must not create output on failure");
    });
}

#[test]
fn list_since_offset_boundary() {
    let _env = setup();
    // Create two strands at different offsets
    let id_a = create_strand("strand A");
    let id_b = create_strand("strand B");
    // Append to B to give it a later offset
    let log = event::make_log_appended(&id_b, "extra entry", None);
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &log)?;
        Ok(())
    })
    .unwrap();

    // Read back to find offsets
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let strand_a = strands.iter().find(|s| s.id == id_a).unwrap();

    // --since-offset at A's last_offset → should exclude A, include B
    let mut filtered: Vec<&projection::ProjectedStrand> = strands
        .iter()
        .filter(|s| s.id == id_a || s.id == id_b)
        .collect();
    filtered.retain(|s| s.last_offset() > strand_a.last_offset());
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, id_b);

    // --stale-offset at A's last_offset → should include A, exclude B
    let mut stale: Vec<&projection::ProjectedStrand> = strands
        .iter()
        .filter(|s| s.id == id_a || s.id == id_b)
        .collect();
    stale.retain(|s| s.last_offset() <= strand_a.last_offset());
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].id, id_a);
}

// ── hidden-strand default visibility ──

#[test]
fn list_default_excludes_hidden() {
    let _env = setup();
    let id_a = create_strand("visible strand");
    let id_b = create_strand("will be hidden");
    cmd_hide(&id_b, Some("noise"), false, None).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let visible = projection::project_strands(&events, false);
    let visible_ids: Vec<&str> = visible.iter().map(|s| s.id.as_str()).collect();
    assert!(
        visible_ids.contains(&id_a.as_str()),
        "visible strand must appear in default list"
    );
    assert!(
        !visible_ids.contains(&id_b.as_str()),
        "hidden strand must NOT appear in default list"
    );
}

// list --all (or the include_hidden flag in cmd_list) returns hidden strands too.

#[test]
fn list_with_include_hidden_returns_all() {
    let _env = setup();
    let id_a = create_strand("visible strand");
    let id_b = create_strand("will be hidden");
    cmd_hide(&id_b, Some("noise"), false, None).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let all = projection::project_strands(&events, true);
    let all_ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
    assert!(all_ids.contains(&id_a.as_str()));
    assert!(
        all_ids.contains(&id_b.as_str()),
        "hidden strand must appear when include_hidden=true"
    );
}

// cmd_search default does not match content inside a hidden strand.

#[test]
fn search_default_excludes_hidden() {
    let _env = setup();
    let id = create_strand("anchor");
    cmd_append(
        Some("needle-haystack"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    cmd_hide(&id, Some("noise"), false, None).unwrap();
    // Default: include_hidden=false → search skips the hidden strand.
    let result = cmd_search("needle", false, false);
    assert!(result.is_ok());
}

// cmd_search --include-hidden matches inside hidden strands, and the
// projection's `hidden` field is true.

#[test]
fn search_include_hidden_projection_reports_hidden() {
    let _env = setup();
    let id = create_strand("anchor");
    cmd_append(
        Some("needle-haystack"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    cmd_hide(&id, Some("noise"), false, None).unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let all = projection::project_strands(&events, true);
    let s = all.iter().find(|s| s.id == id).expect("strand missing");
    assert!(s.hidden, "hidden flag must be true after cmd_hide");
    let visible = projection::project_strands(&events, false);
    assert!(
        !visible.iter().any(|s| s.id == id),
        "hidden strand must not appear in default view"
    );
    assert!(cmd_search("needle", false, true).is_ok());
}

// cmd_agent_context default does not surface hidden prompt-strands.

#[test]
fn show_tail_works_with_explicit_id() {
    let _env = setup();
    let id = create_strand("tail decoupling test");
    cmd_append(
        Some("entry two"),
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
        Some("entry three"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();

    // tail with explicit id — must succeed and show only last 2 entries
    let result = cmd_show(Some(&id), false, Some(2), false, false, false, None);
    assert!(
        result.is_ok(),
        "show <ID> --tail 2 must succeed: {:?}",
        result
    );
    // --last + --tail must still work
    let result2 = cmd_show(None, true, Some(2), false, false, false, None);
    assert!(
        result2.is_ok(),
        "show --last --tail must still work: {:?}",
        result2
    );
}

// ── Task C: entry_count rename — no "entries" key in JSON output ────────

// StrandDetailOutput (show --format json) must serialize as "entry_count",
// not "entries".

// ── show --entry --deref: rationale-chain expansion ─────────────────────

#[test]
fn show_entry_deref_expands_chain_and_prices_frontier() {
    let _env = setup();
    // Chain: downstream cites decision, decision cites evidence.
    let evidence = create_strand("evidence line");
    cmd_append(
        Some("A2 evidence detail"),
        None,
        false,
        false,
        None,
        Some(&evidence),
        None,
        None,
    )
    .unwrap();
    let decision = create_strand("decision line");
    cmd_append_with_seen_offset(
        Some("[decision] built on evidence"),
        None,
        false,
        false,
        None,
        Some(&decision),
        None,
        None,
        None,
        Some(&evidence),
    )
    .unwrap();
    let downstream = create_strand("downstream line");
    cmd_append_with_seen_offset(
        Some("[decision] downstream conclusion"),
        None,
        false,
        false,
        None,
        Some(&downstream),
        None,
        None,
        None,
        Some(&decision),
    )
    .unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let root_hash = strands
        .iter()
        .find(|s| s.id == downstream)
        .and_then(|s| s.log.last())
        .and_then(|entry| entry.entry_id.clone())
        .expect("root entry hash");

    // depth 1: root + the decision entry; the evidence ref sits on the
    // frontier, priced with its content length.
    let view = projection::build_entry_view(&strands, &root_hash[..16], 1).unwrap();
    assert_eq!(view.nodes.len(), 2, "root plus one hop");
    assert_eq!(view.nodes[0].hop, 0);
    assert_eq!(view.nodes[1].hop, 1);
    assert_eq!(
        view.nodes[1].cited_by.as_deref(),
        Some(root_hash.as_str()),
        "hop-1 node records which entry pulled it in"
    );
    assert_eq!(view.stubs.len(), 0);
    assert_eq!(view.frontier.len(), 1, "evidence ref waits at the boundary");
    assert_eq!(
        view.frontier[0].content_len,
        Some("A2 evidence detail".len()),
        "frontier prices the next hop"
    );

    // depth 2: the chain is fully expanded, nothing left on the frontier.
    let view = projection::build_entry_view(&strands, &root_hash[..16], 2).unwrap();
    assert_eq!(view.nodes.len(), 3);
    assert_eq!(view.frontier.len(), 0);
    assert_eq!(
        view.nodes[2].entry.content, "A2 evidence detail",
        "hop-2 node is the evidence entry itself"
    );
}

// ── show --producer: one writer's entries in a multi-writer strand ──────

#[test]
fn producer_filter_narrows_to_one_writer() {
    let _env = setup();
    let id = create_strand("multi-writer strand");
    cmd_append(
        Some("[observed] from codex"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        Some(r#"{"producer":"codex"}"#),
    )
    .unwrap();
    cmd_append(
        Some("[observed] from claude"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        Some(r#"{"producer":"claude"}"#),
    )
    .unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let s = strands.iter().find(|s| s.id == id).unwrap();

    let filtered = s.with_producer_filter("codex");
    assert_eq!(filtered.log.len(), 1, "only the codex entry survives");
    assert_eq!(filtered.log[0].content, "[observed] from codex");
    assert_eq!(filtered.id, s.id, "strand identity untouched");

    let none = s.with_producer_filter("nobody");
    assert_eq!(none.log.len(), 0, "unknown producer matches nothing");

    // Smoke: the command path accepts the filter on both output forms.
    assert!(cmd_show(Some(&id), false, None, false, false, false, Some("codex")).is_ok());
    assert!(cmd_show(Some(&id), false, None, true, false, false, Some("codex")).is_ok());
}

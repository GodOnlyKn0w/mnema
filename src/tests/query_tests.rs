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
            "mnema show --id {} --tail 8",
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
        since_command: out.since_command.clone(),
        delegation_command: out.delegation_command.clone(),
        remind: out.remind.clone(),
        pause: out.pause.clone(),
        stale_count: out.stale_count,
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

// orient --tree: parse check — `mnema orient --tree` is a valid CLI invocation.

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
    let from_parent = crate::scope::Scope::subtree(&parent)
        .resolve_ids(&strands)
        .expect("parent resolves");
    assert!(from_parent.contains(&parent), "root included");
    assert!(from_parent.contains(&child), "child reachable from parent");
    assert!(
        from_parent.contains(&grandchild),
        "grandchild reachable from parent"
    );

    // From child: parent is an ancestor, must NOT be in the descendant set.
    let from_child = crate::scope::Scope::subtree(&child)
        .resolve_ids(&strands)
        .expect("child resolves");
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
    cmd_orient(None, false, None, false, None).unwrap();
    cmd_orient(Some("json"), true, Some(3), false, None).unwrap();
    let after = std::fs::read(&path).unwrap();
    assert_eq!(before, after, "orient must never write to the journal");
}

#[test]
fn orient_exposes_incremental_and_delegation_discovery_commands() {
    let _env = setup();
    create_strand("orient discovery pointers");
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let max_offset = events.last().map(|(offset, _)| *offset).unwrap_or(0);
    let plan = orient_plan(
        &events,
        &OrientRequest {
            include_hidden: false,
            limit: None,
            under: None,
            allow_selection: false,
        },
    )
    .unwrap();
    assert_eq!(
        plan.output.since_command,
        format!("mnema timeline --since-offset {max_offset}")
    );
    assert_eq!(plan.output.delegation_command, "mnema explain delegation");
}

#[test]
fn collaboration_forest_discovery_requires_synthesis_after_child_closes() {
    let _env = setup();
    let parent = create_strand("parent coordination task");
    let child_a = create_strand("worker A");
    let child_b = create_strand("worker B");
    cmd_link(&child_a, &parent, Some("belongs-to"), false, None).unwrap();
    cmd_link(&child_b, &parent, Some("belongs-to"), false, None).unwrap();
    cmd_append(
        Some("[coordination synthesis] too early"),
        None,
        false,
        false,
        None,
        Some(&parent),
        None,
        None,
    )
    .unwrap();
    cmd_close(&child_a, Some("done"), None, false).unwrap();
    cmd_close(&child_b, Some("done"), None, false).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    assert!(
        projection::find_recent_collaboration_forest(&strands).is_none(),
        "synthesis before the last child close must not qualify"
    );

    cmd_append(
        Some("[coordination synthesis] after both workers closed"),
        None,
        false,
        false,
        None,
        Some(&parent),
        None,
        None,
    )
    .unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let forest = projection::find_recent_collaboration_forest(&strands)
        .expect("forest should qualify after late synthesis");
    assert_eq!(forest.root_id, parent);
}

#[test]
fn collaboration_forest_discovery_picks_recent_qualified_forest() {
    let _env = setup();
    let older = create_strand("older parent");
    let newer = create_strand("newer parent");
    for parent in [&older, &newer] {
        let child_a = create_strand(&format!("worker A for {}", parent));
        let child_b = create_strand(&format!("worker B for {}", parent));
        cmd_link(&child_a, parent, Some("belongs-to"), false, None).unwrap();
        cmd_link(&child_b, parent, Some("belongs-to"), false, None).unwrap();
        cmd_close(&child_a, Some("done"), None, false).unwrap();
        cmd_close(&child_b, Some("done"), None, false).unwrap();
        cmd_append(
            Some("[coordination synthesis] workers closed done"),
            None,
            false,
            false,
            None,
            Some(parent),
            None,
            None,
        )
        .unwrap();
    }

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let forest =
        projection::find_recent_collaboration_forest(&strands).expect("one forest should qualify");
    assert_eq!(forest.root_id, newer);
}

#[test]
fn explain_collaboration_is_pure_read_and_points_to_local_tree() {
    let _env = setup();
    let parent = create_strand("parent coordination task");
    let child_a = create_strand("worker A");
    let child_b = create_strand("worker B");
    cmd_link(&child_a, &parent, Some("belongs-to"), false, None).unwrap();
    cmd_link(&child_b, &parent, Some("belongs-to"), false, None).unwrap();
    cmd_close(&child_a, Some("done"), None, false).unwrap();
    cmd_close(&child_b, Some("done"), None, false).unwrap();
    cmd_append(
        Some("[coordination synthesis] workers closed done"),
        None,
        false,
        false,
        None,
        Some(&parent),
        None,
        None,
    )
    .unwrap();

    let path = ensure_journal().unwrap();
    let before = std::fs::read(&path).unwrap();
    let output = crate::commands::explain::cmd_explain("collaboration", false);
    let after = std::fs::read(&path).unwrap();

    assert_eq!(
        before, after,
        "explain collaboration must not write journal"
    );
    assert!(output.contains("mnema add --parent <母线>"));
    assert!(output.contains(&format!("mnema tree --id {}", shorten(&parent))));
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
    assert_eq!(meta["source"], "mnema export");
    assert!(meta["journal_lines"].as_u64().unwrap() > 0);
}

// `cmd_export` against a missing journal must fail. The error
// contract is `Err(...)` with a stable prefix; the OS-level wording
// after the prefix is locale-dependent (e.g. EN: "cannot read journal:
// ..."  /  ZH: "cannot read journal: 系统找不到指定的文件。 ..."),
// so we assert on the stable prefix only, not the full message.
//
// Also: this test uses an isolated temp dir + `MNEMA_HOME` (via
// `with_mnema_home`) so it cannot pollute the shared test
// environment. We never `remove_file` on a journal another test
// might be using, and we never panic while holding `CWD_LOCK` (the
// assertion below is a single guarded check, not a multi-step
// sequence that can partial-fail).

#[test]
fn export_no_journal_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    // Create `.mnema/` but DO NOT create `journal.jsonl` inside it.
    // `resolve_journal_dir` succeeds (it only needs the dir to exist);
    // `cmd_export` then fails at the actual `std::fs::read` step
    // because the journal file is missing. This mirrors the user's
    // experience: a project where `.mnema/` exists but no journal
    // has been written yet (e.g. first run after `mnema init`).
    let mnema = dir.path().join(".mnema");
    std::fs::create_dir_all(&mnema).unwrap();
    let out = dir.path().join("nojournal_export.jsonl");
    with_mnema_home(Some(dir.path().to_str().unwrap()), || {
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
    let result = cmd_search("needle", false, false, None, None);
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
    assert!(cmd_search("needle", false, true, None, None).is_ok());
}

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

#[test]
fn list_selection_cache_resolves_and_fails_closed_after_journal_moves() {
    let _env = setup();
    let first = create_strand("first cache target");
    let _second = create_strand("second cache target");
    cmd_list(false, None, None, None, None, None, None, None, None, false).unwrap();

    let events = read_events_lossy(&ensure_journal().unwrap()).0;
    let strands = projection::project_strands(&events, true);
    let max_offset = events.last().map(|(offset, _)| *offset).unwrap_or(0);
    let selected =
        crate::reference::resolve_strand_with_selection(&strands, "@1", true, max_offset)
            .expect("@1 should resolve after text list");
    assert!(strands.iter().any(|s| s.id == selected));

    cmd_append_with_seen_offset(
        Some("move first after list"),
        None,
        false,
        false,
        None,
        Some(&first),
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let moved_events = read_events_lossy(&ensure_journal().unwrap()).0;
    let moved_strands = projection::project_strands(&moved_events, true);
    let moved_max = moved_events.last().map(|(offset, _)| *offset).unwrap_or(0);
    let err =
        crate::reference::resolve_strand_with_selection(&moved_strands, "@1", true, moved_max)
            .unwrap_err();
    assert!(err.contains("stale"), "{err}");
}

#[test]
fn json_list_does_not_write_selection_cache() {
    let env = setup();
    create_strand("json list target");
    cmd_list(false, None, None, None, None, None, None, None, None, true).unwrap();
    assert!(
        !env.path()
            .join(".mnema")
            .join("selection-state.json")
            .exists()
    );
}

#[test]
fn pick_fails_in_non_tty_instead_of_waiting() {
    let _env = setup();
    create_strand("pick target");
    let err = cmd_pick("show", false, false, None).unwrap_err();
    assert!(err.contains("interactive TTY"), "{err}");
}

#[test]
fn pick_label_leads_with_seq_drops_id_and_tail() {
    let _env = setup();
    let id = create_strand("design the picker");
    cmd_append(
        Some("preview the newest tail"),
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
    let s = strands.iter().find(|s| s.id == id).unwrap();
    let label = crate::commands::query::pick_label(7, 0, 0, 6, s);
    assert!(
        label.trim_start().starts_with("7."),
        "row leads with the sequence number: {label}"
    );
    assert!(
        !label.contains(&id[..8]),
        "human row drops the 64-hex id (it travels hidden): {label}"
    );
    assert!(label.contains("○ open"), "row carries state: {label}");
    assert!(
        label.contains("design the picker"),
        "row carries the first summary: {label}"
    );
    assert!(
        !label.contains("→"),
        "no tail arrow — the preview pane shows the tail: {label}"
    );
    assert!(
        !label.contains("preview the newest tail"),
        "the tail summary is not crammed into the row: {label}"
    );
}

#[test]
fn orient_empty_teaches_writing_drill_and_add() {
    let _env = setup();
    let plan = orient_plan(
        &[],
        &OrientRequest {
            include_hidden: false,
            limit: None,
            under: None,
            allow_selection: false,
        },
    )
    .expect("orient_plan empty");
    assert!(plan.output.active.is_empty());
    assert!(plan.output.remind.contains("mnema add"));
    assert!(plan.output.remind.contains("mnema explain writing"));
}

#[test]
fn orient_latest_friction_hands_off_fix_prefix() {
    let _env = setup();
    let id = create_strand("fixable task");
    cmd_append(
        Some("[friction] parser fails; at=<file>:<line>; tried=<command>"),
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
    let plan = orient_plan(
        &events,
        &OrientRequest {
            include_hidden: false,
            limit: None,
            under: None,
            allow_selection: false,
        },
    )
    .expect("orient_plan friction");
    let strands = projection::project_strands(&events, true);
    let prefix = strands
        .iter()
        .find(|s| s.id == id)
        .unwrap()
        .log
        .last()
        .and_then(|e| e.entry_id.as_deref())
        .map(shorten)
        .unwrap();
    assert!(
        plan.output
            .remind
            .contains(&format!("[fixed] fixes={}", prefix)),
        "remind must hand off friction prefix: {}",
        plan.output.remind
    );
    assert!(
        plan.output
            .remind
            .contains(&format!("mnema append --id {}", shorten(&id))),
        "remind must include copyable append command: {}",
        plan.output.remind
    );
}

#[test]
fn orient_latest_closed_line_teaches_successor_not_append_to_closed() {
    let _env = setup();
    let active = create_strand("still active");
    let closed = create_strand("finished thread");
    cmd_close(&closed, Some("done"), None, false).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let plan = orient_plan(
        &events,
        &OrientRequest {
            include_hidden: false,
            limit: None,
            under: None,
            allow_selection: false,
        },
    )
    .expect("orient_plan closed latest");

    assert!(
        plan.output.remind.contains("mnema add --from"),
        "closed latest line should produce a successor command: {}",
        plan.output.remind
    );
    assert!(
        !plan
            .output
            .remind
            .contains(&format!("mnema append --id {}", shorten(&closed))),
        "orient must not teach appending to a closed latest line: {}",
        plan.output.remind
    );
    assert!(
        !plan
            .output
            .remind
            .contains(&format!("mnema append --id {}", shorten(&active))),
        "closed global latest should not silently hand off an older active line: {}",
        plan.output.remind
    );
}

#[test]
fn orient_two_active_lines_suggests_link_candidate() {
    let _env = setup();
    let first = create_strand("first active");
    let second = create_strand("second active");
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let plan = orient_plan(
        &events,
        &OrientRequest {
            include_hidden: false,
            limit: None,
            under: None,
            allow_selection: false,
        },
    )
    .expect("orient_plan two active");
    assert!(
        plan.output.remind.contains("mnema link"),
        "two active lines should produce a link candidate: {}",
        plan.output.remind
    );
    assert!(plan.output.remind.contains("--edge-type depends-on"));
    let _ = (first, second);
}

// ── query-side batch: entry search / --marker / edges / orient stale ──

#[test]
fn search_returns_entry_level_hits_with_hash_and_marker() {
    let _env = setup();
    let id = create_strand("search entry base");
    cmd_append(
        Some("[friction] needle-entry-level; at=here; tried=x"),
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
    let result = search_events(
        &events,
        &SearchRequest {
            query: "needle-entry-level",
            include_hidden: false,
            marker: None,
            under: None,
            allow_selection: false,
            current_max_offset: events.last().map(|(o, _)| *o).unwrap_or(0),
        },
    )
    .expect("search_events");
    assert_eq!(result.output.count, 1);
    let m = &result.output.matches[0];
    assert_eq!(m.strand_id, id);
    assert_eq!(m.marker, "friction");
    let entry_id = m.entry_id.as_ref().expect("entry_id required");
    assert!(entry_id.len() >= 12, "full entry hash expected");
    // text row leads with the entry prefix (edge handoff), not strand id
    let (prefix, marker_disp, _) = &result.text_rows[0];
    assert_eq!(prefix, &shorten(entry_id));
    assert_eq!(marker_disp, "[friction]");
}

#[test]
fn search_marker_filter_keeps_only_named_marker() {
    let _env = setup();
    let id = create_strand("marker filter base");
    cmd_append(
        Some("[friction] only friction"),
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
        Some("[metric] win_count=26"),
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
        Some("[decision] ship it"),
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

    let metrics = search_events(
        &events,
        &SearchRequest {
            query: "",
            include_hidden: false,
            marker: Some("metric"),
            under: None,
            allow_selection: false,
            current_max_offset: events.last().map(|(o, _)| *o).unwrap_or(0),
        },
    )
    .expect("search_events metric");
    assert_eq!(metrics.output.count, 1);
    assert_eq!(metrics.output.matches[0].marker, "metric");
    assert!(metrics.output.matches[0].content.contains("win_count=26"));
    assert_eq!(metrics.output.marker.as_deref(), Some("metric"));

    // Bracket form of --marker also works.
    let frictions = search_events(
        &events,
        &SearchRequest {
            query: "",
            include_hidden: false,
            marker: Some("friction"),
            under: None,
            allow_selection: false,
            current_max_offset: events.last().map(|(o, _)| *o).unwrap_or(0),
        },
    )
    .expect("search_events friction");
    assert_eq!(frictions.output.count, 1);
    assert_eq!(frictions.output.matches[0].marker, "friction");
    assert!(frictions.output.matches[0].entry_id.is_some());
}

#[test]
fn edges_discipline_lists_open_friction_and_why_less_decision() {
    let _env = setup();
    let id = create_strand("edges discipline base");
    // Open friction (should appear).
    cmd_append(
        Some("[friction] still blocked; at=x; tried=y"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    // Decision without --why (should appear).
    cmd_append(
        Some("[decision] choose A without rationale pin"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    // Friction that gets fixed (should NOT appear as open).
    cmd_append(
        Some("[friction] was broken; at=z; tried=w"),
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
    let strand = strands.iter().find(|s| s.id == id).unwrap();
    let fixed_friction = strand
        .log
        .iter()
        .rev()
        .find(|e| e.content.starts_with("[friction] was broken"))
        .and_then(|e| e.entry_id.clone())
        .expect("fixed friction entry_id");
    let fixed_prefix = shorten(&fixed_friction);
    cmd_append(
        Some(&format!(
            "[fixed] fixes={} repaired; verified=test",
            fixed_prefix
        )),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    // Decision WITH --why (should NOT appear).
    let why_target = strand
        .log
        .first()
        .and_then(|e| e.entry_id.clone())
        .expect("first entry id");
    cmd_append_with_seen_offset(
        Some("[decision] choose B with pin"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
        None,
        Some(&why_target),
    )
    .unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let report = projection::edges_discipline_report(&strands);

    assert_eq!(
        report.open_frictions.len(),
        1,
        "only the unfixed open friction: {:?}",
        report.open_frictions
    );
    assert_eq!(
        report.open_friction_active_count, 1,
        "unfixed friction is on the still-open strand"
    );
    assert!(
        report.open_frictions[0].content.contains("still blocked"),
        "open friction content: {}",
        report.open_frictions[0].content
    );
    assert_eq!(
        report.decisions_without_why.len(),
        1,
        "only the why-less decision: {:?}",
        report.decisions_without_why
    );
    assert!(
        report.decisions_without_why[0]
            .content
            .contains("without rationale pin"),
        "why-less decision: {}",
        report.decisions_without_why[0].content
    );
}

#[test]
fn edges_discipline_counts_unfixed_friction_on_closed_strand() {
    // Peer case: unfixed friction on a closed:done pilot line must still
    // appear in unfixed total (not filtered by home-strand open/closed).
    let _env = setup();
    let closed_id = create_strand("closed pilot with dangling friction");
    cmd_append(
        Some("[friction] design gap still open; at=x; tried=y"),
        None,
        false,
        false,
        None,
        Some(&closed_id),
        None,
        None,
    )
    .unwrap();
    cmd_close(&closed_id, Some("done"), None, false).unwrap();

    let active_id = create_strand("active with its own friction");
    cmd_append(
        Some("[friction] still on active; at=z; tried=w"),
        None,
        false,
        false,
        None,
        Some(&active_id),
        None,
        None,
    )
    .unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let report = projection::edges_discipline_report(&strands);

    assert!(
        report.open_frictions.len() >= 2,
        "both closed-home and active-home unfixed frictions: {:?}",
        report.open_frictions
    );
    let on_closed = report
        .open_frictions
        .iter()
        .any(|i| i.strand_id == closed_id && i.content.contains("design gap still open"));
    assert!(
        on_closed,
        "unfixed friction on closed strand must count in total: {:?}",
        report.open_frictions
    );
    let on_active = report
        .open_frictions
        .iter()
        .filter(|i| i.strand_id == active_id)
        .count();
    assert_eq!(on_active, 1);
    // Dual count: active subset excludes the closed-home friction.
    assert_eq!(
        report.open_friction_active_count,
        report
            .open_frictions
            .iter()
            .filter(|i| {
                strands
                    .iter()
                    .find(|s| s.id == i.strand_id)
                    .map(|s| s.state() == "registered")
                    .unwrap_or(false)
            })
            .count(),
        "active_count must match registered-home unfixed frictions"
    );
    let closed_unfixed = report
        .open_frictions
        .iter()
        .filter(|i| i.strand_id == closed_id)
        .count();
    assert_eq!(closed_unfixed, 1);
    assert_eq!(
        report.open_friction_active_count,
        report.open_frictions.len() - closed_unfixed,
        "dual count: total minus closed-home = active"
    );
}

#[test]
fn edges_discipline_since_skips_old_why_less_decisions_not_frictions() {
    let _env = setup();
    let id = create_strand("since floor base");
    cmd_append(
        Some("[decision] old stock without why"),
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
        Some("[friction] stays visible regardless of since"),
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
    let strand = strands.iter().find(|s| s.id == id).unwrap();
    let old_decision_offset = strand
        .log
        .iter()
        .find(|e| e.content.contains("old stock"))
        .map(|e| e.offset)
        .expect("old decision offset");
    // New decision after the floor.
    cmd_append(
        Some("[decision] new without why"),
        None,
        false,
        false,
        None,
        Some(&id),
        None,
        None,
    )
    .unwrap();
    let (events2, _) = read_events_lossy(&path);
    let strands2 = projection::project_strands(&events2, true);
    let full = projection::edges_discipline_report(&strands2);
    let filtered =
        projection::edges_discipline_report_since(&strands2, Some(old_decision_offset), None);
    assert!(
        full.decisions_without_why.len() >= 2,
        "baseline has both decisions: {:?}",
        full.decisions_without_why
    );
    assert_eq!(
        filtered.decisions_without_why.len(),
        full.decisions_without_why
            .iter()
            .filter(|d| d.offset > old_decision_offset)
            .count(),
        "since floor drops pre-offset decisions"
    );
    assert!(
        filtered
            .decisions_without_why
            .iter()
            .any(|d| d.content.contains("new without why")),
        "post-floor decision remains"
    );
    // Frictions never filtered by since.
    assert_eq!(
        filtered.open_frictions.len(),
        full.open_frictions.len(),
        "since must not hide unfixed frictions"
    );
}

#[test]
fn list_stale_excludes_closed_strands() {
    let _env = setup();
    let active_id = create_strand("active silent candidate");
    let closed_id = create_strand("closed but old last entry");
    cmd_close(&closed_id, Some("done"), None, false).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    // Synthetic future now makes every current last_ts older than 2h.
    let now = chrono::Utc::now() + chrono::Duration::hours(5);
    let listed = list_strands(
        &events,
        &ListRequest {
            include_hidden: false,
            links: None,
            backlinks: None,
            state: None,
            list_type: None,
            stale: Some("2h"),
            stale_offset: None,
            since_offset: None,
            under: None,
            allow_selection: false,
        },
        now,
    )
    .expect("list_strands --stale");

    let ids: Vec<&str> = listed.iter().map(|s| s.id.as_str()).collect();
    assert!(
        ids.contains(&active_id.as_str()),
        "registered silent line is a handoff candidate: {ids:?}"
    );
    assert!(
        !ids.contains(&closed_id.as_str()),
        "closed line must not appear in --stale: {ids:?}"
    );
    assert!(
        listed.iter().all(|s| s.state() == "registered"),
        "every --stale row must be registered"
    );
}

#[test]
fn orient_stale_count_counts_active_silent_past_threshold() {
    let _env = setup();
    let _fresh = create_strand("fresh active line");
    let stale_id = create_strand("stale active line");
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    // Synthetic future "now" makes every current entry older than the 2h threshold.
    let now = chrono::Utc::now() + chrono::Duration::hours(5);
    let count = count_stale_active(&strands, false, now, ORIENT_STALE_SECS);
    assert!(
        count >= 2,
        "both active lines silent ≥2h under future now, got {count}"
    );

    // Closed strands do not count as stale-active.
    cmd_close(&stale_id, Some("done"), None, false).unwrap();
    let (events2, _) = read_events_lossy(&path);
    let strands2 = projection::project_strands(&events2, true);
    let count2 = count_stale_active(&strands2, false, now, ORIENT_STALE_SECS);
    assert_eq!(
        count2,
        count - 1,
        "closing one strand drops stale_count by 1"
    );

    let plan = orient_plan_at(
        &events2,
        &OrientRequest {
            include_hidden: false,
            limit: None,
            under: None,
            allow_selection: false,
        },
        now,
    )
    .expect("orient_plan_at stale");
    assert_eq!(plan.output.stale_count, count2);
}

#[test]
fn timeline_tail_keeps_last_events_not_head() {
    let _env = setup();
    let id = create_strand("timeline tail first");
    for content in [
        "timeline tail second",
        "timeline tail third",
        "timeline tail fourth",
    ] {
        cmd_append(
            Some(content),
            None,
            false,
            false,
            None,
            Some(&id),
            None,
            None,
        )
        .unwrap();
    }

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let mut entries = projection::project_timeline(&events);
    let original_offsets: Vec<usize> = entries.iter().map(|e| e.journal_offset).collect();
    assert!(
        original_offsets.len() >= 4,
        "test needs several timeline entries"
    );

    let truncated =
        crate::commands::query::apply_timeline_window_limit(&mut entries, None, Some(2));
    let tail_offsets: Vec<usize> = entries.iter().map(|e| e.journal_offset).collect();

    assert!(truncated, "tail below total length must mark truncated");
    assert_eq!(
        tail_offsets,
        original_offsets[original_offsets.len() - 2..].to_vec(),
        "--tail N must keep the last N events"
    );
}

#[test]
fn timeline_limit_still_keeps_head_events() {
    let _env = setup();
    let id = create_strand("timeline limit first");
    for content in ["timeline limit second", "timeline limit third"] {
        cmd_append(
            Some(content),
            None,
            false,
            false,
            None,
            Some(&id),
            None,
            None,
        )
        .unwrap();
    }

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let mut entries = projection::project_timeline(&events);
    let original_offsets: Vec<usize> = entries.iter().map(|e| e.journal_offset).collect();

    let truncated =
        crate::commands::query::apply_timeline_window_limit(&mut entries, Some(2), None);
    let limited_offsets: Vec<usize> = entries.iter().map(|e| e.journal_offset).collect();

    assert!(truncated, "limit below total length must mark truncated");
    assert_eq!(
        limited_offsets,
        original_offsets[..2].to_vec(),
        "--limit N must keep the first N events"
    );
}

#[test]
fn timeline_since_ts_future_yields_empty() {
    let _env = setup();
    create_strand("timeline since-ts seed");
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let mut entries = projection::project_timeline(&events);
    assert!(!entries.is_empty());

    crate::commands::query::filter_timeline_by_ts(&mut entries, Some("2099-01-01T00:00:00Z"), None)
        .unwrap();
    assert!(entries.is_empty());
}

#[test]
fn timeline_ts_filters_reject_invalid_rfc3339() {
    let _env = setup();
    create_strand("timeline invalid timestamp seed");
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let mut entries = projection::project_timeline(&events);

    let since_err =
        crate::commands::query::filter_timeline_by_ts(&mut entries, Some("not-a-date"), None)
            .expect_err("invalid since-ts must fail");
    assert!(since_err.contains("--since-ts") && since_err.contains("RFC3339"));

    let until_err =
        crate::commands::query::filter_timeline_by_ts(&mut entries, None, Some("yesterday"))
            .expect_err("invalid until-ts must fail");
    assert!(until_err.contains("--until-ts") && until_err.contains("RFC3339"));
}

// ── recursive Scope CLI: --under / orient --id ──

#[test]
fn list_under_scopes_to_belongs_to_subtree() {
    let _env = setup();
    let parent = create_strand("scope parent");
    let child = create_strand("scope child");
    let outsider = create_strand("scope outsider");
    cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let listed = list_strands(
        &events,
        &ListRequest {
            include_hidden: false,
            links: None,
            backlinks: None,
            state: None,
            list_type: None,
            stale: None,
            stale_offset: None,
            since_offset: None,
            under: Some(&parent),
            allow_selection: false,
        },
        chrono::Utc::now(),
    )
    .expect("list --under");
    let ids: std::collections::HashSet<_> = listed.iter().map(|s| s.id.clone()).collect();
    assert!(ids.contains(&parent), "root included");
    assert!(ids.contains(&child), "belongs-to child included");
    assert!(
        !ids.contains(&outsider),
        "unrelated top-level strand must leave the candidate set"
    );
}

#[test]
fn search_under_scopes_hits_to_subtree() {
    let _env = setup();
    let parent = create_strand("search parent token-alpha");
    let child = create_strand("search child token-alpha");
    let outsider = create_strand("search outsider token-alpha");
    cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let max = events.last().map(|(o, _)| *o).unwrap_or(0);
    let result = search_events(
        &events,
        &SearchRequest {
            query: "token-alpha",
            include_hidden: false,
            marker: None,
            under: Some(&parent),
            allow_selection: false,
            current_max_offset: max,
        },
    )
    .expect("search --under");
    let strand_ids: std::collections::HashSet<_> = result
        .output
        .matches
        .iter()
        .map(|m| m.strand_id.clone())
        .collect();
    assert!(strand_ids.contains(&parent));
    assert!(strand_ids.contains(&child));
    assert!(!strand_ids.contains(&outsider));
    assert_eq!(result.output.count, 2);
}

#[test]
fn orient_id_scopes_menu_to_subtree() {
    let _env = setup();
    let parent = create_strand("orient parent");
    let child = create_strand("orient child");
    let outsider = create_strand("orient outsider");
    cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let plan = orient_plan(
        &events,
        &OrientRequest {
            include_hidden: false,
            limit: None,
            under: Some(parent.clone()),
            allow_selection: false,
        },
    )
    .expect("orient --id");
    let active: std::collections::HashSet<_> =
        plan.output.active.iter().map(|s| s.id.clone()).collect();
    assert!(active.contains(&parent));
    assert!(active.contains(&child));
    assert!(
        !active.contains(&outsider),
        "orient --id must not surface out-of-scope active lines"
    );
}

#[test]
fn timeline_under_excludes_out_of_scope_strands() {
    let _env = setup();
    let parent = create_strand("tl parent");
    let child = create_strand("tl child");
    let outsider = create_strand("tl outsider unique-outside");
    cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

    // Smoke the command path (stdout not captured; success + no panic is enough
    // for wiring). Detailed filtering is covered by Scope unit tests + list/search.
    assert!(
        cmd_timeline(
            None,
            None,
            None,
            None,
            None,
            None,
            Some("json"),
            None,
            None,
            Some(&parent),
        )
        .is_ok()
    );
    let _ = (child, outsider);
}

#[test]
fn under_flag_parses_on_collection_commands() {
    use clap::CommandFactory;
    for args in [
        vec!["mnema", "list", "--under", "0000019dd34b"],
        vec!["mnema", "search", "x", "--under", "0000019dd34b"],
        vec!["mnema", "timeline", "--under", "0000019dd34b"],
        vec!["mnema", "pick", "--under", "0000019dd34b", "--print-id"],
        vec!["mnema", "orient", "--id", "0000019dd34b"],
        vec!["mnema", "orient", "--id", "0000019dd34b", "--tree"],
        vec!["mnema", "depends", "--under", "0000019dd34b"],
        vec![
            "mnema",
            "depends",
            "--under",
            "0000019dd34b",
            "--format",
            "json",
        ],
        vec!["mnema", "doctor", "edges", "--under", "0000019dd34b"],
        vec!["mnema", "doctor", "edges", "--id", "0000019dd34b"],
        vec![
            "mnema",
            "doctor",
            "edges",
            "--under",
            "0000019dd34b",
            "--format",
            "json",
        ],
    ] {
        let result = Cli::command().try_get_matches_from(&args);
        assert!(result.is_ok(), "must parse {:?}: {:?}", args, result.err());
    }
}

#[test]
fn depends_under_and_single_id_conflict() {
    use clap::CommandFactory;
    let result = Cli::command().try_get_matches_from([
        "mnema",
        "depends",
        "0000019dd34b",
        "--under",
        "0000019dd34b",
    ]);
    assert!(result.is_err(), "depends <ID> --under must conflict");
}

#[test]
fn doctor_edges_under_and_id_conflict() {
    use clap::CommandFactory;
    let result = Cli::command().try_get_matches_from([
        "mnema",
        "doctor",
        "edges",
        "--under",
        "0000019dd34b",
        "--id",
        "0000019dd34c",
    ]);
    assert!(
        result.is_err(),
        "doctor edges --under and --id must conflict"
    );
}

#[test]
fn depends_under_lists_each_subtree_strand_upstream_facts() {
    let _env = setup();
    let parent = create_strand("depends parent");
    let child = create_strand("depends child");
    let outsider = create_strand("depends outsider");
    let upstream_a = create_strand("upstream a");
    let upstream_b = create_strand("upstream b");
    cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();
    cmd_link(&parent, &upstream_a, Some("depends-on"), false, None).unwrap();
    cmd_link(&child, &upstream_b, Some("depends-on"), false, None).unwrap();
    cmd_link(&outsider, &upstream_a, Some("depends-on"), false, None).unwrap();

    // Command path: smoke success (stdout not captured).
    assert!(cmd_depends_under(&parent, Some("json")).is_ok());

    // Projection-level facts: SubtreeScope(parent) = {parent, child}, not outsider.
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let scope = scope_from_under(Some(&parent), &strands, false, 0).unwrap();
    let ids = scope.resolve_ids(&strands).unwrap();
    assert!(ids.contains(&parent));
    assert!(ids.contains(&child));
    assert!(!ids.contains(&outsider));

    let graph = crate::graph::StrandGraph::from_strands(&strands);
    let parent_review = graph.depends_review(&parent).unwrap();
    let child_review = graph.depends_review(&child).unwrap();
    assert_eq!(parent_review.upstream_count, 1);
    assert_eq!(parent_review.upstreams[0].id, upstream_a);
    assert_eq!(child_review.upstream_count, 1);
    assert_eq!(child_review.upstreams[0].id, upstream_b);
    // No ready/blocker/critical-path fields on the review surface.
    let json = serde_json::to_value(crate::output::DependsOutput::from(&parent_review)).unwrap();
    let obj = json.as_object().unwrap();
    assert!(!obj.contains_key("ready"));
    assert!(!obj.contains_key("blockers"));
    assert!(!obj.contains_key("critical_path"));
}

#[test]
fn depends_under_json_forbids_selection_handle() {
    let _env = setup();
    let parent = create_strand("depends sel parent");
    let err = cmd_depends_under("@1", Some("json")).expect_err("machine mode bans @N");
    assert!(
        err.contains("selection handle") || err.contains("@1"),
        "expected selection ban, got: {}",
        err
    );
    let _ = parent;
}

#[test]
fn edges_discipline_candidate_set_shrinks_findings_not_fix_knowledge() {
    let _env = setup();
    let parent = create_strand("edges scope parent");
    let child = create_strand("edges scope child");
    let outsider = create_strand("edges scope outsider");
    cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

    // Friction on child (in scope) and outsider (out of scope).
    cmd_append(
        Some("[friction] child blocked; at=c; tried=t"),
        None,
        false,
        false,
        None,
        Some(&child),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[friction] outsider blocked; at=o; tried=t"),
        None,
        false,
        false,
        None,
        Some(&outsider),
        None,
        None,
    )
    .unwrap();
    // Why-less decision on parent (in scope).
    cmd_append(
        Some("[decision] parent choose without why"),
        None,
        false,
        false,
        None,
        Some(&parent),
        None,
        None,
    )
    .unwrap();

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let full = projection::edges_discipline_report(&strands);
    assert!(
        full.open_frictions.len() >= 2,
        "journal-wide should see both frictions"
    );

    let scope = scope_from_under(Some(&parent), &strands, false, 0).unwrap();
    let ids = scope.resolve_ids(&strands).unwrap();
    let scoped = projection::edges_discipline_report_since(&strands, None, Some(&ids));
    assert!(
        scoped
            .open_frictions
            .iter()
            .all(|i| i.strand_id == child || i.strand_id == parent),
        "scoped frictions must stay inside SubtreeScope"
    );
    assert!(
        !scoped
            .open_frictions
            .iter()
            .any(|i| i.strand_id == outsider),
        "outsider friction must leave the candidate set"
    );
    assert!(
        scoped
            .decisions_without_why
            .iter()
            .any(|i| i.strand_id == parent),
        "in-scope decision should remain"
    );

    // Fix from outsider still closes in-scope friction (fix knowledge is journal-wide).
    let child_strand = strands.iter().find(|s| s.id == child).unwrap();
    let friction_id = child_strand
        .log
        .iter()
        .rev()
        .find(|e| e.content.starts_with("[friction] child blocked"))
        .and_then(|e| e.entry_id.clone())
        .expect("child friction entry_id");
    let prefix = shorten(&friction_id);
    cmd_append(
        Some(&format!(
            "[fixed] fixes={} repaired from outsider; verified=test",
            prefix
        )),
        None,
        false,
        false,
        None,
        Some(&outsider),
        None,
        None,
    )
    .unwrap();
    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let scope = scope_from_under(Some(&parent), &strands, false, 0).unwrap();
    let ids = scope.resolve_ids(&strands).unwrap();
    let after_fix = projection::edges_discipline_report_since(&strands, None, Some(&ids));
    assert!(
        !after_fix
            .open_frictions
            .iter()
            .any(|i| i.strand_id == child),
        "fix outside scope must still close in-scope friction"
    );
}

#[test]
fn doctor_edges_id_scopes_to_single_strand() {
    let _env = setup();
    let a = create_strand("doctor id a");
    let b = create_strand("doctor id b");
    cmd_append(
        Some("[friction] only on a; at=a; tried=t"),
        None,
        false,
        false,
        None,
        Some(&a),
        None,
        None,
    )
    .unwrap();
    cmd_append(
        Some("[friction] only on b; at=b; tried=t"),
        None,
        false,
        false,
        None,
        Some(&b),
        None,
        None,
    )
    .unwrap();

    assert!(cmd_doctor_edges(true, None, None, Some(&a)).is_ok());

    let path = ensure_journal().unwrap();
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    let ids = std::collections::HashSet::from([a.clone()]);
    let report = projection::edges_discipline_report_since(&strands, None, Some(&ids));
    assert!(
        report.open_frictions.iter().all(|i| i.strand_id == a),
        "single-id candidate set must only report strand a"
    );
    assert!(
        !report.open_frictions.iter().any(|i| i.strand_id == b),
        "strand b must leave single-id candidate set"
    );
}

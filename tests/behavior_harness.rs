use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

#[derive(Debug, Deserialize)]
struct Manifest {
    schema: String,
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
struct Scenario {
    id: String,
    purpose: String,
    #[serde(default)]
    setup: Vec<Vec<String>>,
    setup_fixture: Option<String>,
    command: Vec<String>,
    format: String,
    allowed_dynamic_values: Vec<String>,
}

fn run(cwd: &Path, args: &[String], stdin: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mnema"));
    command
        .current_dir(cwd)
        .args(args)
        .env("NO_COLOR", "1")
        .env("TZ", "UTC")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().expect("spawn mnema");
    if let Some(body) = stdin {
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(body.as_bytes())
            .expect("write stdin");
    }
    child.wait_with_output().expect("wait for mnema")
}

fn ok(cwd: &Path, args: &[&str], stdin: Option<&str>) -> String {
    let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    let output = run(cwd, &args, stdin);
    assert!(
        output.status.success(),
        "mnema {args:?} failed\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

fn add(cwd: &Path, body: &str, extra: &[&str]) -> Value {
    let mut args = vec!["add", "--format", "json"];
    args.extend_from_slice(extra);
    serde_json::from_str(ok(cwd, &args, Some(body)).trim()).expect("add json")
}

fn id(value: &Value) -> String {
    value["id"].as_str().expect("strand id").to_string()
}

fn manifest() -> Manifest {
    serde_json::from_str(include_str!("behavior/scenarios.json")).expect("scenario manifest")
}

fn fixture_command(id: &str, fixture: &RecursiveFixture) -> Vec<String> {
    manifest()
        .scenarios
        .into_iter()
        .find(|scenario| scenario.id == id)
        .expect("fixture scenario")
        .command
        .into_iter()
        .map(|arg| {
            arg.replace("${root_id}", &fixture.root).replace(
                "${checkpoint_offset}",
                &fixture.checkpoint_offset.to_string(),
            )
        })
        .collect()
}

/// Independent belongs-to oracle. It deliberately consumes public timeline
/// JSON rather than production projection types.
#[derive(Default)]
struct ScopeOracle {
    children: HashMap<String, HashSet<String>>,
}

impl ScopeOracle {
    fn link(&mut self, child: &str, parent: &str) {
        self.children
            .entry(parent.to_string())
            .or_default()
            .insert(child.to_string());
    }

    fn unlink(&mut self, child: &str, parent: &str) {
        if let Some(children) = self.children.get_mut(parent) {
            children.remove(child);
        }
    }

    fn subtree(&self, root: &str) -> HashSet<String> {
        let mut found = HashSet::new();
        let mut pending = vec![root.to_string()];
        while let Some(id) = pending.pop() {
            if found.insert(id.clone()) {
                pending.extend(self.children.get(&id).into_iter().flatten().cloned());
            }
        }
        found
    }

    fn fold_global_timeline(rows: &[Value]) -> Self {
        let mut oracle = Self::default();
        for row in rows {
            let Some(source) = row["strand_id"].as_str() else {
                continue;
            };
            let kind = &row["kind"];
            let effect = &kind["effect"];
            if effect["edge_type"].as_str() != Some("belongs-to") {
                continue;
            }
            let Some(target) = effect["target"].as_str() else {
                continue;
            };
            match effect["kind"].as_str() {
                Some("link") => oracle.link(source, target),
                Some("unlink") => oracle.unlink(source, target),
                _ => {}
            }
        }
        oracle
    }
}

struct RecursiveFixture {
    root: String,
    child: String,
    grandchild: String,
    outsider: String,
    evidence: String,
    late_joiner: String,
    checkpoint_offset: usize,
}

fn recursive_scope_v1(cwd: &Path) -> RecursiveFixture {
    ok(cwd, &["init"], None);
    let root = id(&add(cwd, "[task] fixture root\n", &["--slug", "root"]));
    let child = id(&add(cwd, "[task] fixture child\n", &["--parent", &root]));
    let grandchild = id(&add(
        cwd,
        "[task] fixture grandchild\n",
        &["--parent", &child],
    ));
    let outsider = id(&add(cwd, "[task] fixture outsider\n", &[]));
    let evidence = id(&add(cwd, "[evidence] fixture rationale\n", &[]));
    ok(
        cwd,
        &["append", "--id", &child, "--ref", &evidence],
        Some("[progress] cited in-scope fact\n"),
    );

    let global: Value =
        serde_json::from_str(ok(cwd, &["timeline", "--format", "json"], None).trim())
            .expect("global timeline json");
    let checkpoint_offset = global["max_offset"].as_u64().expect("max offset") as usize;

    let late_joiner = id(&add(cwd, "[task] fixture late joiner\n", &[]));
    ok(
        cwd,
        &["append", "--id", &late_joiner],
        Some("[progress] prejoin secret\n"),
    );
    ok(
        cwd,
        &["link", &late_joiner, &root, "--edge-type", "belongs-to"],
        None,
    );
    ok(
        cwd,
        &["append", "--id", &child],
        Some("[progress] before leave\n"),
    );
    ok(
        cwd,
        &["unlink", &child, &root, "--edge-type", "belongs-to"],
        None,
    );
    ok(
        cwd,
        &["append", "--id", &child],
        Some("[progress] postleave secret\n"),
    );
    RecursiveFixture {
        root,
        child,
        grandchild,
        outsider,
        evidence,
        late_joiner,
        checkpoint_offset,
    }
}

fn timeline_rows(value: &Value) -> &[Value] {
    value["timeline"].as_array().expect("timeline array")
}

fn row_text(row: &Value) -> String {
    row.to_string()
}

fn normalized_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[test]
fn reviewed_text_snapshots_match_exact_public_output() {
    let root = tempfile::tempdir().unwrap();
    let help = run(root.path(), &["--help".to_string()], None);
    assert!(help.status.success());
    assert_eq!(
        normalized_text(&help.stdout),
        include_str!("behavior/snapshots/root-help.stdout.txt")
    );
    assert!(help.stderr.is_empty());

    let project = tempfile::tempdir().unwrap();
    assert!(
        run(project.path(), &["init".to_string()], None)
            .status
            .success()
    );
    let invalid = run(
        project.path(),
        &[
            "show".to_string(),
            "--id".to_string(),
            "not-an-id".to_string(),
        ],
        None,
    );
    assert_eq!(invalid.status.code(), Some(1));
    assert!(invalid.stdout.is_empty());
    assert_eq!(
        normalized_text(&invalid.stderr),
        include_str!("behavior/snapshots/invalid-id.stderr.txt")
    );
}

#[test]
fn behavior_manifest_is_executable_and_declares_normalization() {
    let manifest = manifest();
    assert_eq!(manifest.schema, "mnema-behavior-scenarios/v1");
    let ids: HashSet<&str> = manifest.scenarios.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids.len(), manifest.scenarios.len(), "scenario ids unique");
    for scenario in &manifest.scenarios {
        assert!(
            !scenario.purpose.trim().is_empty(),
            "{} purpose",
            scenario.id
        );
        assert!(!scenario.command.is_empty(), "{} command", scenario.id);
        assert!(matches!(
            scenario.format.as_str(),
            "text" | "canonical-json"
        ));
        let allowed: HashSet<&str> = scenario
            .allowed_dynamic_values
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(allowed.len(), scenario.allowed_dynamic_values.len());
        assert!(scenario.setup.is_empty() || scenario.setup_fixture.is_none());
    }

    // Execute the manifest's self-contained scenarios. Fixture scenarios have
    // stronger semantic coverage below.
    for scenario in manifest
        .scenarios
        .iter()
        .filter(|scenario| scenario.setup_fixture.is_none())
    {
        let dir = tempfile::tempdir().unwrap();
        for setup in &scenario.setup {
            let output = run(dir.path(), setup, None);
            assert!(output.status.success(), "{} setup failed", scenario.id);
        }
        let output = run(dir.path(), &scenario.command, None);
        if scenario.id == "invalid-id-diagnostic" {
            assert!(!output.status.success());
        } else {
            assert!(output.status.success(), "{} failed", scenario.id);
        }
        if scenario.format == "canonical-json" && output.status.success() {
            let _: Value = serde_json::from_slice(&output.stdout).expect("valid json output");
        }
    }
}

#[test]
fn recursive_scope_fixture_agrees_with_independent_current_scope_oracle() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = recursive_scope_v1(dir.path());
    let global: Value =
        serde_json::from_str(ok(dir.path(), &["timeline", "--format", "json"], None).trim())
            .unwrap();
    let oracle = ScopeOracle::fold_global_timeline(timeline_rows(&global));
    let expected = oracle.subtree(&fixture.root);
    assert_eq!(
        expected,
        HashSet::from([fixture.root.clone(), fixture.late_joiner.clone()])
    );

    let command = fixture_command("recursive-scope-journey", &fixture);
    let output = run(dir.path(), &command, None);
    assert!(output.status.success());
    let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
    let actual_ids: HashSet<String> = timeline_rows(&actual)
        .iter()
        .map(|row| row["strand_id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_ids, expected);
    assert!(!actual_ids.contains(&fixture.outsider));
    assert!(
        !actual_ids.contains(&fixture.evidence),
        "refs never expand subtree membership"
    );
    assert!(!actual_ids.contains(&fixture.grandchild));
}

#[test]
fn event_time_incremental_scope_differs_from_current_scope_at_join_and_leave() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = recursive_scope_v1(dir.path());
    let n = fixture.checkpoint_offset.to_string();
    let current: Value = serde_json::from_str(
        ok(
            dir.path(),
            &[
                "timeline",
                "--under",
                &fixture.root,
                "--since-offset",
                &n,
                "--format",
                "json",
            ],
            None,
        )
        .trim(),
    )
    .unwrap();
    let event_command = fixture_command("incremental-scope-journey", &fixture);
    let event_output = run(dir.path(), &event_command, None);
    assert!(event_output.status.success());
    let event_time: Value = serde_json::from_slice(&event_output.stdout).unwrap();
    let current_text = timeline_rows(&current)
        .iter()
        .map(row_text)
        .collect::<Vec<_>>()
        .join("\n");
    let event_text = timeline_rows(&event_time)
        .iter()
        .map(row_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(current_text.contains(&fixture.late_joiner));
    assert!(
        current_text.contains("prejoin secret"),
        "current membership includes late joiner's history"
    );
    assert!(
        !current_text.contains("before leave"),
        "departed child is not a current member"
    );
    assert!(!current_text.contains("postleave secret"));

    assert!(!event_text.contains("prejoin secret"));
    assert!(event_text.contains("before leave"));
    assert!(!event_text.contains("postleave secret"));
    assert!(event_text.contains(&fixture.child));
    assert!(!event_text.contains(&fixture.outsider));
    assert!(timeline_rows(&event_time).iter().any(|row| {
        row["strand_id"].as_str() == Some(fixture.late_joiner.as_str())
            && row["kind"]["effect"]["kind"].as_str() == Some("link")
            && row["kind"]["effect"]["target"].as_str() == Some(fixture.root.as_str())
    }));
    assert!(timeline_rows(&event_time).iter().any(|row| {
        row["strand_id"].as_str() == Some(fixture.child.as_str())
            && row["kind"]["effect"]["kind"].as_str() == Some("unlink")
            && row["kind"]["effect"]["target"].as_str() == Some(fixture.root.as_str())
    }));
}

#[test]
fn concurrent_parent_plus_ref_adds_converge_to_complete_results() {
    let dir = tempfile::tempdir().unwrap();
    ok(dir.path(), &["init"], None);
    let parent = id(&add(dir.path(), "[task] concurrent parent\n", &[]));
    let evidence = id(&add(dir.path(), "[evidence] shared basis\n", &[]));

    let mut workers = Vec::new();
    for index in 0..8 {
        let cwd = dir.path().to_path_buf();
        let parent = parent.clone();
        let evidence = evidence.clone();
        workers.push(std::thread::spawn(move || {
            let slug = format!("concurrent-child-{index}");
            let body = format!("[task] concurrent child {index}\n");
            let args = vec![
                "add".to_string(),
                "--format".to_string(),
                "json".to_string(),
                "--slug".to_string(),
                slug,
                "--parent".to_string(),
                parent,
                "--ref".to_string(),
                evidence,
            ];
            run(&cwd, &args, Some(&body))
        }));
    }

    let mut child_ids = HashSet::new();
    for worker in workers {
        let output = worker.join().expect("worker join");
        assert!(
            output.status.success(),
            "concurrent add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).expect("add output json");
        child_ids.insert(value["id"].as_str().unwrap().to_string());
        assert_eq!(value["refs"].as_array().unwrap().len(), 1);
    }
    assert_eq!(child_ids.len(), 8);

    let tree = ok(
        dir.path(),
        &["tree", "--id", &parent, "--format", "json"],
        None,
    );
    for child in &child_ids {
        assert!(tree.contains(child), "tree missing atomic child {child}");
    }
    let timeline: Value =
        serde_json::from_str(ok(dir.path(), &["timeline", "--format", "json"], None).trim())
            .unwrap();
    for child in &child_ids {
        let rows: Vec<&Value> = timeline_rows(&timeline)
            .iter()
            .filter(|row| row["strand_id"].as_str() == Some(child.as_str()))
            .collect();
        assert!(
            rows.len() >= 2,
            "child {child} exposed without its structural batch"
        );
        let joined = rows.iter().map(|row| row.to_string()).collect::<String>();
        assert!(
            joined.contains("belongs-to"),
            "child {child} missing parent link"
        );
        let show = ok(
            dir.path(),
            &["show", "--id", child, "--format", "json"],
            None,
        );
        assert!(
            show.contains(&evidence),
            "child {child} missing rationale ref"
        );
    }
    ok(dir.path(), &["doctor", "journal"], None);
}

#[test]
fn timeline_json_self_describes_scope_and_advances_empty_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = recursive_scope_v1(dir.path());
    let prefix = &fixture.root[..12];
    let baseline: Value =
        serde_json::from_str(ok(dir.path(), &["timeline", "--format", "json"], None).trim())
            .unwrap();
    let journal_tip = baseline["window"]["observed_through"].as_u64().unwrap();
    assert!(journal_tip > 0);

    let beyond = (journal_tip + 100).to_string();
    let empty: Value = serde_json::from_str(
        ok(
            dir.path(),
            &[
                "timeline",
                "--under",
                prefix,
                "--since-offset",
                &beyond,
                "--scope-at-event",
                "--format",
                "json",
            ],
            None,
        )
        .trim(),
    )
    .unwrap();
    assert_eq!(empty["count"], 0);
    assert_eq!(empty["scope"]["kind"], "subtree");
    assert_eq!(empty["scope"]["root"], fixture.root);
    assert_eq!(empty["scope"]["membership"], "event-time");
    assert_eq!(empty["window"]["since_offset"], journal_tip + 100);
    assert_eq!(empty["window"]["observed_through"], journal_tip);
    assert_eq!(empty["window"]["next_since_offset"], journal_tip + 100);
}

#[test]
fn canonical_and_legacy_ref_flags_preserve_order_and_fail_without_writes() {
    let dir = tempfile::tempdir().unwrap();
    ok(dir.path(), &["init"], None);
    let parent = id(&add(dir.path(), "[task] ref parent\n", &[]));
    let a = id(&add(dir.path(), "[evidence] ref a\n", &[]));
    let b = id(&add(dir.path(), "[evidence] ref b\n", &[]));

    let mixed = add(
        dir.path(),
        "[task] mixed refs child\n",
        &["--parent", &parent, "--ref", &a, "--from", &b],
    );
    assert_eq!(mixed["refs"], serde_json::json!([a, b]));
    let child = mixed["id"].as_str().unwrap();
    let appended: Value = serde_json::from_str(
        ok(
            dir.path(),
            &[
                "append", "--id", child, "--ref", &b, "--why", &a, "--format", "json",
            ],
            Some("[decision] mixed append refs\n"),
        )
        .trim(),
    )
    .unwrap();
    assert_eq!(appended["refs"], serde_json::json!([b, a]));

    let before: Value =
        serde_json::from_str(ok(dir.path(), &["timeline", "--format", "json"], None).trim())
            .unwrap();
    let before_tip = before["window"]["observed_through"].as_u64().unwrap();
    let bad_add = run(
        dir.path(),
        &[
            "add".to_string(),
            "--parent".to_string(),
            parent,
            "--ref".to_string(),
            "deadbeef".to_string(),
        ],
        Some("[task] must not exist\n"),
    );
    assert!(!bad_add.status.success());
    let bad_append = run(
        dir.path(),
        &[
            "append".to_string(),
            "--id".to_string(),
            child.to_string(),
            "--ref".to_string(),
            "deadbeef".to_string(),
        ],
        Some("[progress] must not append\n"),
    );
    assert!(!bad_append.status.success());
    let after: Value =
        serde_json::from_str(ok(dir.path(), &["timeline", "--format", "json"], None).trim())
            .unwrap();
    assert_eq!(after["window"]["observed_through"], before_tip);
    assert!(!after.to_string().contains("must not"));
}

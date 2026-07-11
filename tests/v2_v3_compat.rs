use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn copy_tree(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let destination = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            std::fs::copy(entry.path(), destination).unwrap();
        }
    }
}

fn run(cwd: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mnema"));
    command.current_dir(cwd).args(args);
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(body) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

fn json_ok(cwd: &Path, args: &[&str]) -> Value {
    let output = run(cwd, args, None);
    assert!(
        output.status.success(),
        "{args:?}\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn sha256(path: &Path) -> String {
    hex::encode(Sha256::digest(std::fs::read(path).unwrap()))
}

fn independent_migration_id(journal_id: &str, source_sha256: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"mnema.migration.v2-to-v3\0");
    hasher.update(journal_id.as_bytes());
    hasher.update([0]);
    hasher.update(source_sha256.as_bytes());
    hex::encode(hasher.finalize())
}

#[test]
fn immutable_v2_fixture_cutover_matches_golden_identity_and_projection() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v2/compat-v1");
    let expected: Value =
        serde_json::from_slice(&std::fs::read(fixture.join("expected.json")).unwrap()).unwrap();
    assert_eq!(expected["schema"], "mnema-v2-v3-compat-fixture/v1");
    assert_eq!(
        sha256(&fixture.join(".mnema/journal.jsonl")),
        expected["source_sha256"].as_str().unwrap()
    );
    assert_eq!(
        independent_migration_id(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            expected["source_sha256"].as_str().unwrap(),
        ),
        expected["migration_id"]
    );

    let temp = tempfile::tempdir().unwrap();
    copy_tree(&fixture, temp.path());
    let old_root = expected["old_root_id"].as_str().unwrap();
    let old_child = expected["old_child_id"].as_str().unwrap();

    let before_root = json_ok(temp.path(), &["show", "--id", old_root, "--format", "json"]);
    let before_child = json_ok(
        temp.path(),
        &["show", "--id", old_child, "--format", "json"],
    );
    assert_eq!(before_root["summary"], "[task] historical root");
    assert_eq!(before_child["status"], "closed:done");
    assert_eq!(
        before_child["belongs_to_edges"],
        serde_json::json!([old_root])
    );
    assert_eq!(
        before_child["events"][1]["refs"],
        serde_json::json!(["1".repeat(64)])
    );

    let dry = json_ok(temp.path(), &["cutover-v3", "--format", "json"]);
    for key in [
        "migration_id",
        "source_event_count",
        "target_record_count",
        "strand_count",
        "entry_count",
        "unresolved_ref_count",
    ] {
        assert_eq!(dry[key], expected[key], "dry-run golden field {key}");
    }
    assert_eq!(dry["projection_ok"], true);

    let applied = json_ok(temp.path(), &["cutover-v3", "--apply", "--format", "json"]);
    assert_eq!(applied["outcome"], "applied");
    let target = temp.path().join(".mnema/journals/journal.v3.jsonl");
    assert_eq!(sha256(&target), expected["target_sha256"].as_str().unwrap());
    let raw_v3: Vec<Value> = std::fs::read_to_string(&target)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        raw_v3.len(),
        expected["target_record_count"].as_u64().unwrap() as usize
    );
    let raw_link = raw_v3
        .iter()
        .find(|record| record["entry"]["payload"]["type"] == "link")
        .unwrap();
    assert_eq!(raw_link["entry"]["payload"]["edge_type"], "belongs-to");
    assert_eq!(
        raw_link["entry"]["payload"]["target_strand_id"],
        expected["new_root_id"]
    );
    let raw_decision = raw_v3
        .iter()
        .find(|record| record["entry"]["body"] == "[decision] child cites root")
        .unwrap();
    assert_eq!(raw_decision["entry_id"], expected["new_child_decision_id"]);
    assert_eq!(
        raw_decision["entry"]["refs"][0]["entry_id"],
        expected["new_root_entry_id"]
    );
    let raw_close = raw_v3
        .iter()
        .find(|record| record["entry"]["payload"]["type"] == "close")
        .unwrap();
    assert_eq!(raw_close["entry"]["payload"]["disposition"], "done");

    let new_root = expected["new_root_id"].as_str().unwrap();
    let new_child = expected["new_child_id"].as_str().unwrap();
    let after_root = json_ok(temp.path(), &["show", "--id", new_root, "--format", "json"]);
    let after_child = json_ok(
        temp.path(),
        &["show", "--id", new_child, "--format", "json"],
    );
    let after_decision = after_child["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["entry"] == "[decision] child cites root")
        .unwrap();
    assert_eq!(after_root["summary"], before_root["summary"]);
    assert_eq!(after_child["status"], before_child["status"]);
    assert_eq!(
        after_child["belongs_to_edges"],
        serde_json::json!([new_root])
    );
    assert_eq!(after_decision["refs"][0], expected["new_root_entry_id"]);
    assert_eq!(
        after_decision["entry_id"],
        expected["new_child_decision_id"]
    );
    let doctor = run(temp.path(), &["doctor", "journal"], None);
    assert!(doctor.status.success());
}

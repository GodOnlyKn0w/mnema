use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run(cwd: &std::path::Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mnema"));
    command.current_dir(cwd).args(args);
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mnema");
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    child.wait_with_output().expect("wait for mnema")
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn records(path: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn fresh_v3_add_and_append_keep_genesis_identity_and_append_only_prefix() {
    let dir = tempfile::tempdir().unwrap();
    success(run(dir.path(), &["init"], None));

    let manifest_path = dir.path().join(".mnema/active-journal.json");
    let journal_path = dir.path().join(".mnema/journals/journal.v3.jsonl");
    let manifest_at_activation = std::fs::read(&manifest_path).unwrap();
    let initial_prefix = std::fs::read(&journal_path).unwrap();

    let add = success(run(
        dir.path(),
        &["add", "--format", "json", "--slug", "root"],
        Some("[task] root\n"),
    ));
    let add: serde_json::Value = serde_json::from_str(add.trim()).unwrap();
    let strand_id = add["id"].as_str().unwrap().to_string();
    assert_eq!(64, strand_id.len());

    let after_add = std::fs::read(&journal_path).unwrap();
    assert!(after_add.starts_with(&initial_prefix));
    let add_records = records(&journal_path);
    let genesis = add_records[1]["entry"].as_object().unwrap();
    assert_eq!(
        Some(strand_id.as_str()),
        add_records[1]["strand_id"].as_str()
    );
    assert_eq!(
        Some(strand_id.as_str()),
        add_records[1]["entry_id"].as_str()
    );
    assert_eq!(Some("root"), genesis["strand"]["slug"].as_str());
    assert_eq!(Some("task"), genesis["strand"]["strand_type"].as_str());

    success(run(
        dir.path(),
        &["append", "--id", &strand_id, "--format", "json"],
        Some("[progress] next\n"),
    ));
    let final_bytes = std::fs::read(&journal_path).unwrap();
    assert!(final_bytes.starts_with(&after_add));
    assert_eq!(
        manifest_at_activation,
        std::fs::read(&manifest_path).unwrap()
    );

    let final_records = records(&journal_path);
    assert_eq!(5, final_records.len());
    assert_eq!(Some("anchor"), final_records[0]["record"].as_str());
    assert_eq!(Some("entry"), final_records[1]["record"].as_str());
    assert_eq!(Some("anchor"), final_records[2]["record"].as_str());
    assert_eq!(Some("entry"), final_records[3]["record"].as_str());
    assert_eq!(Some("anchor"), final_records[4]["record"].as_str());
    assert_eq!(
        Some(strand_id.as_str()),
        final_records[3]["strand_id"].as_str()
    );
    assert_eq!(
        final_records[1]["entry_id"],
        final_records[3]["entry"]["prev"]
    );

    success(run(
        dir.path(),
        &["show", "--id", &strand_id, "--digest"],
        None,
    ));
}

#[test]
fn fresh_v3_child_link_uses_actual_genesis_ids() {
    let dir = tempfile::tempdir().unwrap();
    success(run(dir.path(), &["init"], None));
    let parent = success(run(
        dir.path(),
        &["add", "--format", "json"],
        Some("parent\n"),
    ));
    let parent: serde_json::Value = serde_json::from_str(parent.trim()).unwrap();
    let parent_id = parent["id"].as_str().unwrap();
    let child = success(run(
        dir.path(),
        &["add", "--format", "json", "--parent", parent_id],
        Some("child\n"),
    ));
    let child: serde_json::Value = serde_json::from_str(child.trim()).unwrap();
    let child_id = child["id"].as_str().unwrap();

    let journal_path = dir.path().join(".mnema/journals/journal.v3.jsonl");
    let values = records(&journal_path);
    let link = values
        .iter()
        .find(|value| value["entry"]["kind"] == "effect")
        .expect("belongs-to effect");
    assert_eq!(Some(child_id), link["strand_id"].as_str());
    assert_eq!(
        Some(parent_id),
        link["entry"]["payload"]["target_strand_id"].as_str()
    );
    success(run(dir.path(), &["tree", "--id", parent_id], None));
}

#[test]
fn v3_doctor_follows_manifest_and_detects_active_tampering() {
    let dir = tempfile::tempdir().unwrap();
    success(run(dir.path(), &["init"], None));
    success(run(dir.path(), &["add"], Some("root\n")));

    let healthy = success(run(dir.path(), &["doctor", "journal"], None));
    assert!(healthy.contains("schema: v3"), "{healthy}");
    assert!(healthy.contains("integrity errors: 0"), "{healthy}");

    let journal = dir.path().join(".mnema/journals/journal.v3.jsonl");
    let mut bytes = std::fs::read(&journal).unwrap();
    bytes.extend_from_slice(b"{\"record\":\"entry\"}\n");
    std::fs::write(&journal, bytes).unwrap();
    let broken = run(dir.path(), &["doctor", "journal"], None);
    assert!(!broken.status.success());
    let stdout = String::from_utf8_lossy(&broken.stdout);
    let stderr = String::from_utf8_lossy(&broken.stderr);
    assert!(stdout.contains("integrity errors: 1"), "{stdout}\n{stderr}");
    assert!(stderr.contains("[integrity]"), "{stdout}\n{stderr}");
}

#[test]
fn fresh_init_removes_an_empty_legacy_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let mnema = dir.path().join(".mnema");
    std::fs::create_dir_all(&mnema).unwrap();
    std::fs::write(mnema.join("journal.jsonl"), "").unwrap();
    success(run(dir.path(), &["init"], None));
    assert!(!mnema.join("journal.jsonl").exists());
    success(run(dir.path(), &["doctor", "journal"], None));
}

#[test]
fn v3_checkpoint_writes_a_typed_payload() {
    let dir = tempfile::tempdir().unwrap();
    success(run(dir.path(), &["init"], None));
    let add = success(run(
        dir.path(),
        &["add", "--format", "json"],
        Some("checkpoint target\n"),
    ));
    let add: serde_json::Value = serde_json::from_str(add.trim()).unwrap();
    let id = add["id"].as_str().unwrap();
    success(run(
        dir.path(),
        &[
            "checkpoint",
            "--id",
            id,
            "--action",
            "continue implementation",
            "--format",
            "json",
        ],
        None,
    ));
    let values = records(&dir.path().join(".mnema/journals/journal.v3.jsonl"));
    let checkpoint = values
        .iter()
        .rev()
        .find(|value| value["record"] == "entry")
        .unwrap();
    assert_eq!(Some("checkpoint"), checkpoint["entry"]["kind"].as_str());
    assert_eq!(
        Some("continue implementation"),
        checkpoint["entry"]["payload"]["action"].as_str()
    );
    assert!(
        checkpoint["entry"]["payload"]["observed"]
            .as_str()
            .unwrap()
            .contains("entries_before_append=")
    );
}

#[test]
fn v3_doctor_warns_when_an_old_binary_creates_a_fresh_origin_shadow() {
    let dir = tempfile::tempdir().unwrap();
    success(run(dir.path(), &["init"], None));
    std::fs::write(
        dir.path().join(".mnema/journal.jsonl"),
        b"{\"type\":\"legacy-write\"}\n",
    )
    .unwrap();
    let output = run(dir.path(), &["doctor", "journal"], None);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("legacy-shadow-diverged"), "{stderr}");
    assert!(stderr.contains("old binary"), "{stderr}");
}

#[test]
fn v3_orient_uses_strict_v3_integrity_instead_of_v2_rehashing() {
    let dir = tempfile::tempdir().unwrap();
    success(run(dir.path(), &["init"], None));
    success(run(dir.path(), &["add"], Some("orient target\n")));
    let output = success(run(dir.path(), &["orient", "--format", "json"], None));
    let value: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
    let integrity = value["integrity"].as_str().unwrap();
    assert!(integrity.starts_with("ok (v3 strict-read"), "{integrity}");
}

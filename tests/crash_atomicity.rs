#![cfg(feature = "test-failpoints")]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run(cwd: &Path, args: &[&str], stdin: Option<&str>, failpoint: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mnema"));
    command.current_dir(cwd).args(args);
    if let Some(value) = failpoint {
        command.env("MNEMA_TEST_FAILPOINT", value);
    }
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

fn success(cwd: &Path, args: &[&str], stdin: Option<&str>) -> String {
    let output = run(cwd, args, stdin, None);
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn setup() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    success(dir.path(), &["init"], None);
    let added = success(
        dir.path(),
        &["add", "--format", "json"],
        Some("[task] crash root\n"),
    );
    let value: serde_json::Value = serde_json::from_str(added.trim()).unwrap();
    (dir, value["id"].as_str().unwrap().to_string())
}

#[test]
fn abort_before_batch_write_preserves_old_valid_state() {
    let (dir, id) = setup();
    let journal = dir.path().join(".mnema/journals/journal.v3.jsonl");
    let before = std::fs::read(&journal).unwrap();
    let crashed = run(
        dir.path(),
        &["append", "--id", &id],
        Some("[progress] must stay absent\n"),
        Some("before-v3-batch-write"),
    );
    assert!(!crashed.status.success());
    assert_eq!(std::fs::read(&journal).unwrap(), before);
    success(dir.path(), &["doctor", "journal"], None);
    let show = success(dir.path(), &["show", "--id", &id], None);
    assert!(!show.contains("must stay absent"));
}

#[test]
fn abort_after_complete_write_exposes_complete_valid_new_state() {
    let (dir, id) = setup();
    let crashed = run(
        dir.path(),
        &["append", "--id", &id],
        Some("[progress] complete batch survives\n"),
        Some("after-v3-write-before-sync"),
    );
    assert!(!crashed.status.success());
    success(dir.path(), &["doctor", "journal"], None);
    let show = success(dir.path(), &["show", "--id", &id], None);
    assert!(show.contains("complete batch survives"));
}

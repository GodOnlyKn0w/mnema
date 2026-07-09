use std::process::Command;

#[test]
fn add_positional_body_error_teaches_pipe_form_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_mnema"))
        .args(["add", "x"])
        .output()
        .expect("run mnema add x");

    assert_eq!(Some(3), output.status.code());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("echo \"x\" | mnema add"),
        "stderr should teach the stdin form, got:\n{}",
        stderr
    );
}

/// Repro of the dirty-write teaching bug: id-shaped positionals must become
/// `--id`, never get folded into the echoed body.
#[test]
fn append_id_plus_body_positional_teaches_id_flag_not_dirty_echo() {
    let output = Command::new(env!("CARGO_BIN_EXE_mnema"))
        .args(["append", "812e60f3252f", "my note"])
        .output()
        .expect("run mnema append id body");

    assert_eq!(Some(3), output.status.code());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("echo \"my note\" | mnema append --id 812e60f3252f"),
        "must route id to --id with body-only echo, got:\n{}",
        stderr
    );
    assert!(
        !stderr.contains("echo \"812e60f3252f my note\""),
        "must not fold id into body, got:\n{}",
        stderr
    );
}

#[test]
fn append_pure_id_positional_teaches_id_flag_without_echoing_id() {
    let output = Command::new(env!("CARGO_BIN_EXE_mnema"))
        .args(["append", "0000019dd34b"])
        .output()
        .expect("run mnema append pure-id");

    assert_eq!(Some(3), output.status.code());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("mnema append --id 0000019dd34b"),
        "pure id must become --id form, got:\n{}",
        stderr
    );
    assert!(
        !stderr.contains("echo \"0000019dd34b\""),
        "must not echo pure id as body, got:\n{}",
        stderr
    );
}

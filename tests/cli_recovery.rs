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

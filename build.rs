// Supplies TASKTREE_COMMIT and TASKTREE_BUILD_PROFILE for `env!` in main.rs,
// so a bare `cargo build` works without manual env setup. Externally provided
// values (e.g. from a release pipeline) take precedence over the git probe.

use std::process::Command;

fn main() {
    let commit = std::env::var("TASKTREE_COMMIT")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "--short=12", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());

    let profile = std::env::var("TASKTREE_BUILD_PROFILE")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string()));

    println!("cargo:rustc-env=TASKTREE_COMMIT={commit}");
    println!("cargo:rustc-env=TASKTREE_BUILD_PROFILE={profile}");
    println!("cargo:rerun-if-env-changed=TASKTREE_COMMIT");
    println!("cargo:rerun-if-env-changed=TASKTREE_BUILD_PROFILE");
    println!("cargo:rerun-if-changed=.git/HEAD");
    // Same-branch commits leave .git/HEAD untouched ("ref: refs/heads/X"
    // is stable) — watch the ref file HEAD points at, or the stamp goes
    // stale and the binary lies about its commit.
    if let Ok(head) = std::fs::read_to_string(".git/HEAD") {
        if let Some(r) = head.trim().strip_prefix("ref: ") {
            println!("cargo:rerun-if-changed=.git/{r}");
        }
    }
}

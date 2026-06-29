use crate::cli::{Cli, exit_code_for};
use crate::commands::context::*;
use crate::commands::manage::*;
use crate::commands::query::*;
use crate::commands::write::*;
use crate::diagnostics;
use crate::event::{self, Event, find_strand};
use crate::journal::*;
use crate::output;
use crate::projection;
use crate::render::{self, *};
use crate::tree;
use crate::util::*;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

mod cli_tests;
mod context_tests;
mod diagnostics_tests;
mod journal_tests;
mod manage_tests;
mod output_tests;
mod query_tests;
mod util_tests;
mod write_tests;

// Global lock to serialize current-directory changes across parallel tests.
static CWD_LOCK: Mutex<()> = Mutex::new(());

// Test harness: sets cwd to a temp dir with .tasktree/, restored on drop.
struct TestEnv {
    _dir: tempfile::TempDir,
    prev_cwd: PathBuf,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl TestEnv {
    fn new() -> Self {
        // Tolerate a poisoned CWD_LOCK from a previous test panic: the
        // lock is a pure serialisation aid, the data it guards is
        // restored in `Drop`, so recovering the inner guard is safe and
        // prevents one failing test from cascading into 30+.
        let lock = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let tasktree_dir = dir.path().join(".tasktree");
        fs::create_dir_all(&tasktree_dir).unwrap();
        let journal = tasktree_dir.join("journal.jsonl");
        fs::write(&journal, "").unwrap();
        let prev_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        TestEnv {
            _dir: dir,
            prev_cwd,
            _lock: lock,
        }
    }

    fn path(&self) -> &std::path::Path {
        self._dir.path()
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.prev_cwd);
    }
}

fn setup() -> TestEnv {
    TestEnv::new()
}

// ─────────────────────────────────────────────────────────────────
// resolve_journal_dir() tests (architecture.md §15.7)
// ─────────────────────────────────────────────────────────────────

// Mutex for serializing env-var-touching tests (TASKTREE_HOME).
static ENV_LOCK: Mutex<()> = Mutex::new(());

// Save and restore TASKTREE_HOME around a closure, returning its result.

fn with_tasktree_home<F: FnOnce() -> R, R>(new_value: Option<&str>, f: F) -> R {
    let _env_lock = ENV_LOCK.lock().unwrap();
    let prev = std::env::var("TASKTREE_HOME").ok();
    match new_value {
        Some(v) => unsafe { std::env::set_var("TASKTREE_HOME", v) },
        None => unsafe { std::env::remove_var("TASKTREE_HOME") },
    }
    let result = f();
    match prev {
        Some(v) => unsafe { std::env::set_var("TASKTREE_HOME", v) },
        None => unsafe { std::env::remove_var("TASKTREE_HOME") },
    }
    result
}

fn create_strand(content: &str) -> String {
    let (created, appended) = event::make_strand_created(content, None);
    let id = created.strand_id().to_string();
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &created)?;
        append_event_unlocked(journal, &appended)?;
        Ok(())
    })
    .unwrap();
    id
}

// ── Content source: positional ──

fn splitish(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    cur.push(c);
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                c if c.is_whitespace() => {
                    if !cur.is_empty() {
                        tokens.push(std::mem::take(&mut cur));
                    }
                }
                _ => cur.push(c),
            },
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn substitute(tok: &str) -> String {
    if !tok.contains('<') {
        return tok.to_string();
    }
    let upper = tok.to_uppercase();
    if upper.contains("ID") {
        "0000019dd34b".to_string()
    } else if upper.contains("<N>") {
        "5".to_string()
    } else if upper.contains("FORMAT") {
        "json".to_string()
    } else if upper.contains("PATH") || upper.contains("FILE") {
        "x.md".to_string()
    } else if upper.contains("CODE") {
        "W062".to_string()
    } else if upper.contains("RFC3339") {
        "2026-01-01T00:00:00Z".to_string()
    } else {
        "x".to_string()
    }
}

fn try_parse_example(line: &str) -> Result<(), String> {
    let start = match line.find("tasktree ") {
        Some(i) => i,
        None => return Ok(()),
    };
    // Grammar-notation lines ([--id <ID> | --new]) are usage patterns,
    // not copy-paste examples.
    if line.contains("[--") {
        return Ok(());
    }
    // Prose sentences may end the command with punctuation.
    let cmdline = line[start..].trim_end_matches(['.', ',', ';', ':', ')']);
    let tokens: Vec<String> = splitish(cmdline).iter().map(|t| substitute(t)).collect();
    use clap::CommandFactory;
    match Cli::command().try_get_matches_from(&tokens) {
        Ok(_) => Ok(()),
        Err(e) => match e.kind() {
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => Ok(()),
            _ => Err(format!(
                "example does not parse: `{}` -> {}",
                cmdline.trim(),
                e
            )),
        },
    }
}

fn extract_bracket_markers(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'[' {
            // First char must be a-z
            if i + 1 < len && bytes[i + 1].is_ascii_lowercase() {
                let start = i;
                let mut j = i + 1;
                while j < len {
                    let b = bytes[j];
                    if b.is_ascii_alphanumeric() || b == b'_' || b == b':' || b == b'-' {
                        j += 1;
                    } else {
                        break;
                    }
                }
                if j < len && bytes[j] == b']' {
                    out.push(s[start..=j].to_string());
                    i = j + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

fn create_prompt_strand(content: &str) -> String {
    let (created, appended) = event::make_strand_created(content, Some("prompt-strand"));
    let id = created.strand_id().to_string();
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &created)?;
        append_event_unlocked(journal, &appended)?;
        Ok(())
    })
    .unwrap();
    id
}

fn count_hide_events(events: &[(usize, Event)], strand_id: &str, kind: &str) -> i32 {
    let mut n = 0;
    for (_, e) in events {
        match (e, kind) {
            (Event::StrandHidden { id, .. }, "hidden") if id == strand_id => n += 1,
            (Event::StrandUnhidden { id, .. }, "unhidden") if id == strand_id => n += 1,
            _ => {}
        }
    }
    n
}

fn total_events() -> usize {
    let path = ensure_journal().unwrap();
    read_events_lossy(&path).0.len()
}

// list/context/search default to excluding hidden strands.

fn long_summary() -> String {
    // 50 ASCII chars + 30 CJK chars (each 1 char_count unit) = >80 visible chars;
    // total > 100 to ensure truncate(70) kicks in for prose fields.
    format!(
        "{}{}",
        "a".repeat(50),
        "测试摘要内容验证把手完整性规则不截断标识符".repeat(3),
    )
}

// ── Test 1 ────────────────────────────────────────────────────────────

// make_card on a strand with a very long summary:
//   - card.id == shorten(full_id) and is a prefix of full_id, no '…'
//   - card.catch_up has no '…', parses with try_parse_example
//   - card.last_offset == projected strand's last_offset (integer, not a text truncation)
//   - prose fields (summary, last_entry) may contain '…'

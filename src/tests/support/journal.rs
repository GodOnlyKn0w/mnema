use crate::event::{self, Event};
use crate::journal::{
    append_event_unlocked, ensure_journal, read_events_lossy, with_journal_write_lock,
};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

pub(in crate::tests) static CWD_LOCK: Mutex<()> = Mutex::new(());
pub(in crate::tests) static ENV_LOCK: Mutex<()> = Mutex::new(());

pub(in crate::tests) struct TestEnv {
    _dir: tempfile::TempDir,
    prev_cwd: PathBuf,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl TestEnv {
    fn new() -> Self {
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

    pub(in crate::tests) fn path(&self) -> &std::path::Path {
        self._dir.path()
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.prev_cwd);
    }
}

pub(in crate::tests) fn setup() -> TestEnv {
    TestEnv::new()
}

pub(in crate::tests) fn with_tasktree_home<F: FnOnce() -> R, R>(
    new_value: Option<&str>,
    f: F,
) -> R {
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

pub(in crate::tests) fn create_strand(content: &str) -> String {
    let (created, appended) = event::make_strand_created(content, None);
    let id = created
        .strand_id()
        .expect("strand-scoped event")
        .to_string();
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &created)?;
        append_event_unlocked(journal, &appended)?;
        Ok(())
    })
    .unwrap();
    id
}

pub(in crate::tests) fn create_prompt_strand(content: &str) -> String {
    let (created, appended) = event::make_strand_created(content, Some("prompt-strand"));
    let id = created
        .strand_id()
        .expect("strand-scoped event")
        .to_string();
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &created)?;
        append_event_unlocked(journal, &appended)?;
        Ok(())
    })
    .unwrap();
    id
}

pub(in crate::tests) fn count_hide_events(
    events: &[(usize, Event)],
    strand_id: &str,
    kind: &str,
) -> i32 {
    let mut n = 0;
    for (_, e) in events {
        match (e, kind) {
            (Event::StrandHidden { id, .. }, "hidden") if id == strand_id => n += 1,
            (Event::StrandUnhidden { id, .. }, "unhidden") if id == strand_id => n += 1,
            (
                Event::LogAppended {
                    id,
                    effect: Some(event::EntryEffect::Hide),
                    ..
                },
                "hidden",
            ) if id == strand_id => n += 1,
            (
                Event::LogAppended {
                    id,
                    effect: Some(event::EntryEffect::Unhide),
                    ..
                },
                "unhidden",
            ) if id == strand_id => n += 1,
            _ => {}
        }
    }
    n
}

pub(in crate::tests) fn total_events() -> usize {
    let path = ensure_journal().unwrap();
    read_events_lossy(&path)
        .0
        .iter()
        .filter(|(_, event)| !matches!(event, Event::JournalAnchored { .. }))
        .count()
}

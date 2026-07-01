use std::io::{BufRead, Write};
use std::path::PathBuf;
use fs2::FileExt;
use crate::event::Event;

pub(crate) const JOURNAL_DIR: &str = ".tasktree";
pub(crate) const JOURNAL_FILE: &str = ".tasktree/journal.jsonl";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JournalParseDiagnostic {
    pub(crate) line: usize,
    pub(crate) error: String,
    pub(crate) raw: Option<String>,
    pub(crate) unreadable: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct JournalRead {
    pub(crate) events: Vec<(usize, Event)>,
    pub(crate) diagnostics: Vec<JournalParseDiagnostic>,
    pub(crate) read_error: Option<String>,
}

impl JournalRead {
    pub(crate) fn skipped(&self) -> usize {
        self.diagnostics.len()
    }

    fn from_read_error(error: String) -> Self {
        Self { events: Vec::new(), diagnostics: Vec::new(), read_error: Some(error) }
    }
}

/// Resolve the journal directory with priority:
///   1. TASKTREE_HOME env var (explicit override; must contain .tasktree/)
///   2. Walk-up from cwd: nearest ancestor containing .tasktree/
///   3. Error if neither found (no silent fallback)
///
/// Walk-up enables shared journal across git worktrees: any worktree cwd
/// walk-ups to the project root .tasktree/. See architecture.md s15.7.
pub(crate) fn resolve_journal_dir() -> Result<PathBuf, String> {
    // 1. Explicit override
    if let Ok(home) = std::env::var("TASKTREE_HOME") {
        let p = PathBuf::from(&home);
        let resolved = if p.is_absolute() {
            p
        } else {
            std::env::current_dir()
                .map_err(|e| format!("cannot get cwd: {}", e))?
                .join(p)
        };
        let journal = resolved.join(JOURNAL_DIR);
        if !journal.is_dir() {
            return Err(format!(
                "TASKTREE_HOME={} does not contain {}",
                resolved.display(),
                JOURNAL_DIR
            ));
        }
        return Ok(journal);
    }

    // 2. Walk-up from cwd
    let mut current = std::env::current_dir()
        .map_err(|e| format!("cannot get cwd: {}", e))?;
    loop {
        let candidate = current.join(JOURNAL_DIR);
        if candidate.is_dir() {
            return Ok(candidate);
        }
        if !current.pop() {
            return Err(format!(
                "{}/ not found in cwd or any parent directory. Run tasktree init in project root.",
                JOURNAL_DIR
            ));
        }
    }
}

pub(crate) fn ensure_journal() -> Result<PathBuf, String> {
    Ok(resolve_journal_dir()?.join("journal.jsonl"))
}

/// Return path to .tasktree/journal.lock (dedicated lock file, not the journal itself).
pub(crate) fn journal_lock_path() -> Result<PathBuf, String> {
    Ok(resolve_journal_dir()?.join("journal.lock"))
}

/// Acquire exclusive lock on journal.lock, open journal.jsonl, run closure, flush, unlock.
/// Lock file opened with .create(true).read(true).write(true) — no append.
pub(crate) fn with_journal_write_lock<T>(f: impl FnOnce(&mut std::fs::File) -> Result<T, String>) -> Result<T, String> {
    let lock_path = journal_lock_path()?;
    let journal_path = ensure_journal()?;

    // Open lock file: create if not exists, no append mode
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| format!("cannot open journal.lock: {}", e))?;

    // Acquire exclusive lock on the lock file (must succeed — P0 guarantee)
    lock_file.lock_exclusive()
        .map_err(|e| format!("cannot acquire journal lock: {}", e))?;

    // Open journal for appending
    let mut journal = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(&journal_path)
        .map_err(|e| format!("cannot open journal: {}", e))?;

    let result = f(&mut journal);

    // Flush journal, then release lock
    let _ = journal.flush();
    let _ = lock_file.unlock();
    result
}

/// Acquire shared lock on journal.lock, open journal.jsonl for reading, run closure.
/// Multiple readers allowed concurrently; blocks writers (exclusive lock).
pub(crate) fn with_journal_read_lock<T>(f: impl FnOnce(&mut std::fs::File) -> Result<T, String>) -> Result<T, String> {
    let lock_path = journal_lock_path()?;
    let journal_path = ensure_journal()?;

    // Open lock file: create if not exists
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| format!("cannot open journal.lock: {}", e))?;

    // Acquire shared lock — multiple readers, blocks writers
    lock_file.lock_shared()
        .map_err(|e| format!("cannot acquire shared journal lock: {}", e))?;

    // Open journal for reading
    let mut journal = std::fs::OpenOptions::new()
        .read(true)
        .open(&journal_path)
        .map_err(|e| format!("cannot open journal for reading: {}", e))?;

    let result = f(&mut journal);
    let _ = lock_file.unlock();
    result
}

/// Read all events from the journal under a shared lock (consistent read).
pub(crate) fn read_journal_lossy_locked() -> JournalRead {
    match with_journal_read_lock(|journal| {
        let reader = std::io::BufReader::new(journal);
        Ok(read_journal_lossy_reader(reader))
    }) {
        Ok(read) => read,
        Err(e) => JournalRead::from_read_error(e),
    }
}

/// Append a single event to an already-open journal. Never locks.
pub(crate) fn append_event_unlocked(journal: &mut std::fs::File, event: &Event) -> Result<(), String> {
    let line = serde_json::to_string(event).map_err(|e| format!("serialize error: {}", e))?;
    writeln!(journal, "{}", line).map_err(|e| format!("write error: {}", e))
}

/// Append multiple events to an already-open journal. Never locks.
pub(crate) fn append_events_unlocked(journal: &mut std::fs::File, events: &[Event]) -> Result<(), String> {
    for event in events {
        append_event_unlocked(journal, event)?;
    }
    Ok(())
}

pub(crate) fn read_journal_lossy(path: &PathBuf) -> JournalRead {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return JournalRead::from_read_error(format!("cannot read journal: {}", e)),
    };
    let reader = std::io::BufReader::new(file);
    read_journal_lossy_reader(reader)
}

/// Compatibility wrapper for older callers that only need events + skipped count.
pub(crate) fn read_events_lossy(path: &PathBuf) -> (Vec<(usize, Event)>, usize) {
    let read = read_journal_lossy(path);
    let skipped = read.skipped();
    (read.events, skipped)
}

fn read_journal_lossy_reader<R: BufRead>(reader: R) -> JournalRead {
    let mut read = JournalRead::default();
    for (line_no, line) in reader.lines().enumerate() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                read.diagnostics.push(JournalParseDiagnostic {
                    line: line_no + 1,
                    error: format!("I/O error: {}", e),
                    raw: None,
                    unreadable: true,
                });
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Event>(&line) {
            Ok(event) => read.events.push((line_no, event)),
            Err(e) => {
                read.diagnostics.push(JournalParseDiagnostic {
                    line: line_no + 1,
                    error: e.to_string(),
                    raw: Some(line.chars().take(80).collect()),
                    unreadable: false,
                });
            }
        }
    }
    read
}
/// Extract Event values from offset-paired events, discarding offsets.
pub(crate) fn events_only(offset_events: &[(usize, Event)]) -> Vec<&Event> {
    offset_events.iter().map(|(_, e)| e).collect()
}

pub(crate) fn read_events_strict(path: &PathBuf) -> Result<Vec<(usize, Event)>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("cannot read journal: {}", e))?;
    let reader = std::io::BufReader::new(file);
    let mut events = Vec::new();
    for (line_no, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("journal line {}: I/O error: {}", line_no + 1, e))?;
        if line.trim().is_empty() {
            continue;
        }
        let event: Event = serde_json::from_str(&line)
            .map_err(|e| format!("journal line {}: parse error: {}", line_no + 1, e))?;
        events.push((line_no, event));
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_journal_lossy_reports_structured_parse_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        let (created, appended) = crate::event::make_strand_created("journal read test", None, None);
        writeln!(file, "{}", serde_json::to_string(&created).unwrap()).unwrap();
        writeln!(file, "not json").unwrap();
        writeln!(file, "{}", serde_json::to_string(&appended).unwrap()).unwrap();

        let read = read_journal_lossy(&path);

        assert_eq!(read.events.len(), 2);
        assert_eq!(read.skipped(), 1);
        assert_eq!(read.diagnostics[0].line, 2);
        assert_eq!(read.diagnostics[0].raw.as_deref(), Some("not json"));
        assert!(!read.diagnostics[0].unreadable);
        assert!(read.read_error.is_none());
    }
}
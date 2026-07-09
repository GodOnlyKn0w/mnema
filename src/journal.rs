use crate::event::{self, EntryEffect, Event};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::io::{BufRead, Write};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(crate) struct JournalAppendOutcome {
    pub(crate) event: Event,
    pub(crate) entry_id: Option<String>,
}

pub(crate) struct JournalEntryAppendRequest {
    pub(crate) strand_id: String,
    pub(crate) content: String,
    pub(crate) refs: Vec<String>,
    pub(crate) legacy_ref: Option<String>,
    pub(crate) effect: Option<EntryEffect>,
    pub(crate) provenance: Option<serde_json::Value>,
}

impl JournalAppendOutcome {
    fn from_event(event: Event) -> Self {
        match &event {
            Event::LogAppended { entry_id, .. } => Self {
                event: event.clone(),
                entry_id: entry_id.clone(),
            },
            _ => Self {
                event: event.clone(),
                entry_id: None,
            },
        }
    }
}
pub(crate) const JOURNAL_DIR: &str = ".mnema";
pub(crate) const JOURNAL_FILE: &str = ".mnema/journal.jsonl";
/// Sidecar identity metadata (not part of the append-only hash chain).
pub(crate) const JOURNAL_ID_FILE: &str = "journal-id.json";

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
        Self {
            events: Vec::new(),
            diagnostics: Vec::new(),
            read_error: Some(error),
        }
    }
}

/// Resolve the journal directory with priority:
///   1. MNEMA_HOME env var (explicit override; must contain .mnema/)
///   2. Walk-up from cwd: nearest ancestor containing .mnema/
///   3. Error if neither found (no silent fallback)
///
/// Walk-up enables shared journal across git worktrees: any worktree cwd
/// walk-ups to the project root .mnema/. See architecture.md s15.7.
pub(crate) fn resolve_journal_dir() -> Result<PathBuf, String> {
    // 1. Explicit override
    if let Ok(home) = std::env::var("MNEMA_HOME") {
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
                "MNEMA_HOME={} does not contain {}",
                resolved.display(),
                JOURNAL_DIR
            ));
        }
        return Ok(journal);
    }

    // 2. Walk-up from cwd
    let mut current = std::env::current_dir().map_err(|e| format!("cannot get cwd: {}", e))?;
    loop {
        let candidate = current.join(JOURNAL_DIR);
        if candidate.is_dir() {
            return Ok(candidate);
        }
        if !current.pop() {
            return Err(format!(
                "{}/ not found in cwd or any parent directory. Run mnema init in project root.",
                JOURNAL_DIR
            ));
        }
    }
}

pub(crate) fn ensure_journal() -> Result<PathBuf, String> {
    Ok(resolve_journal_dir()?.join("journal.jsonl"))
}

/// Return path to .mnema/journal.lock (dedicated lock file, not the journal itself).
pub(crate) fn journal_lock_path() -> Result<PathBuf, String> {
    Ok(resolve_journal_dir()?.join("journal.lock"))
}

/// Path to the journal-id sidecar under a resolved `.mnema/` directory.
pub(crate) fn journal_id_path_in(journal_dir: &std::path::Path) -> PathBuf {
    journal_dir.join(JOURNAL_ID_FILE)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct JournalIdFile {
    /// Stable random journal identity (64 hex). Written once; never rewritten.
    journal_id: String,
}

fn generate_journal_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut hasher = Sha256::new();
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    hasher.update(ts.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    // Extra entropy: hash path + thread so two concurrent inits diverge.
    if let Ok(cwd) = std::env::current_dir() {
        hasher.update(cwd.display().to_string().as_bytes());
    }
    hasher.update(format!("{:?}", std::thread::current().id()).as_bytes());
    // Mix a few more nanos reads against clock quantization.
    let ts2 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    hasher.update(ts2.to_le_bytes());
    hex::encode(hasher.finalize())
}

fn is_valid_journal_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn read_journal_id_file(path: &std::path::Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    let parsed: JournalIdFile = serde_json::from_str(&text)
        .map_err(|e| format!("corrupt journal-id file {}: {}", path.display(), e))?;
    if !is_valid_journal_id(&parsed.journal_id) {
        return Err(format!(
            "corrupt journal-id in {}: expected 64 hex digits",
            path.display()
        ));
    }
    Ok(Some(parsed.journal_id))
}

/// Create the journal-id sidecar only if absent (create_new). Never overwrites.
fn write_journal_id_file_new(path: &std::path::Path, journal_id: &str) -> Result<(), String> {
    use std::io::Write;
    let payload = JournalIdFile {
        journal_id: journal_id.to_string(),
    };
    let text = serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("cannot serialize journal-id: {}", e))?;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut f) => {
            f.write_all(text.as_bytes())
                .map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(format!(
            "journal-id already exists at {}",
            path.display()
        )),
        Err(e) => Err(format!("cannot create {}: {}", path.display(), e)),
    }
}

/// Return the stable journal-id for a `.mnema/` directory, creating one
/// idempotently if missing. Identity lives in a sidecar file — never in the
/// append-only journal chain (metadata, not content).
pub(crate) fn ensure_journal_id_in(journal_dir: &std::path::Path) -> Result<String, String> {
    let path = journal_id_path_in(journal_dir);
    if let Some(existing) = read_journal_id_file(&path)? {
        return Ok(existing);
    }
    let id = generate_journal_id();
    // create_new: if a peer won the race, re-read their value.
    match write_journal_id_file_new(&path, &id) {
        Ok(()) => Ok(id),
        Err(_) => match read_journal_id_file(&path)? {
            Some(existing) => Ok(existing),
            None => {
                write_journal_id_file_new(&path, &id)?;
                Ok(id)
            }
        },
    }
}

/// Resolve journal dir then ensure its stable journal-id exists.
pub(crate) fn ensure_journal_id() -> Result<String, String> {
    ensure_journal_id_in(&resolve_journal_dir()?)
}

/// Read journal-id if present; do not create. Used by pure-read paths that
/// must not invent identity as a side effect of inspection.
#[allow(dead_code)]
pub(crate) fn read_journal_id() -> Result<Option<String>, String> {
    let path = journal_id_path_in(&resolve_journal_dir()?);
    read_journal_id_file(&path)
}

/// Write-path warnings leave Journal Core through this injected sink; the
/// core never writes to stderr itself (ARCHITECTURE.md: core modules do not
/// depend on stdout/stderr behaviour). The CLI installs its presenter in
/// main(); with no sink installed (unit tests) warnings are dropped.
static JOURNAL_WARNING_SINK: std::sync::OnceLock<fn(&str)> = std::sync::OnceLock::new();

pub(crate) fn set_journal_warning_sink(sink: fn(&str)) {
    let _ = JOURNAL_WARNING_SINK.set(sink);
}

fn emit_journal_warning(message: &str) {
    if let Some(sink) = JOURNAL_WARNING_SINK.get() {
        sink(message);
    }
}

/// Acquire exclusive lock on journal.lock, open journal.jsonl, run closure, flush, unlock.
/// Lock file opened with .create(true).read(true).write(true) — no append.
pub(crate) fn with_journal_write_lock<T>(
    f: impl FnOnce(&mut std::fs::File) -> Result<T, String>,
) -> Result<T, String> {
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
    lock_file
        .lock_exclusive()
        .map_err(|e| format!("cannot acquire journal lock: {}", e))?;

    let before_len = std::fs::metadata(&journal_path)
        .map(|m| m.len())
        .unwrap_or(0);

    // Open journal for appending
    let mut journal = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(&journal_path)
        .map_err(|e| format!("cannot open journal: {}", e))?;

    let result = f(&mut journal);
    let mut anchor_result = Ok(());
    if result.is_ok() {
        let _ = journal.flush();
        let after_len = std::fs::metadata(&journal_path)
            .map(|m| m.len())
            .unwrap_or(0);
        if after_len > before_len {
            anchor_result = append_journal_anchor_unlocked(&mut journal, &journal_path);
        }
    }

    // Flush journal, then release lock
    let _ = journal.flush();
    let _ = lock_file.unlock();
    match (result, anchor_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(value), Err(e)) => {
            emit_journal_warning(&format!(
                "domain append succeeded but journal anchor append failed: {}",
                e
            ));
            Ok(value)
        }
        (Err(e), _) => Err(e),
    }
}

/// Acquire shared lock on journal.lock, open journal.jsonl for reading, run closure.
/// Multiple readers allowed concurrently; blocks writers (exclusive lock).
pub(crate) fn with_journal_read_lock<T>(
    f: impl FnOnce(&mut std::fs::File) -> Result<T, String>,
) -> Result<T, String> {
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
    lock_file
        .lock_shared()
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
pub(crate) fn append_event_unlocked(
    journal: &mut std::fs::File,
    event: &Event,
) -> Result<(), String> {
    let line = serde_json::to_string(event).map_err(|e| format!("serialize error: {}", e))?;
    writeln!(journal, "{}", line).map_err(|e| format!("write error: {}", e))
}

pub(crate) fn append_events(events: &[Event]) -> Result<(), String> {
    with_journal_write_lock(|journal| {
        for event in events {
            append_event_unlocked(journal, event)?;
        }
        Ok(())
    })
}

pub(crate) fn current_entry_head(events: &[(usize, Event)], strand_id: &str) -> Option<String> {
    let mut fold = crate::event::EntryChainFold::new(crate::event::EntryChainMode::Effective);
    for (_, event) in events {
        if matches!(event, Event::LogAppended { id, .. } if id == strand_id) {
            fold.apply(event);
        }
    }
    fold.head(strand_id)
}

fn append_entry_to_strand_unlocked(
    journal: &mut std::fs::File,
    events: &[(usize, Event)],
    req: JournalEntryAppendRequest,
) -> Result<JournalAppendOutcome, String> {
    let prev_entry_id = current_entry_head(events, &req.strand_id);
    let event = crate::event::make_log_appended_entry_with_effect(
        &req.strand_id,
        prev_entry_id.as_deref(),
        &req.content,
        req.refs,
        req.legacy_ref.as_deref(),
        req.effect,
        req.provenance,
    );
    append_event_unlocked(journal, &event)?;
    Ok(JournalAppendOutcome::from_event(event))
}

pub(crate) fn append_entry_to_strand_checked(
    req: JournalEntryAppendRequest,
    validate: impl FnOnce(&[(usize, Event)]) -> Result<(), String>,
) -> Result<JournalAppendOutcome, String> {
    let outcome = append_entry_to_strand_gated(req, |events| {
        validate(events)?;
        Ok(AppendGate::Proceed)
    })?;
    Ok(outcome.expect("Proceed gate always appends"))
}

/// Gate decision for `append_entry_to_strand_gated`: write the entry, or skip
/// it and report success (for idempotent commands like hide/unhide where the
/// target is already in the requested state).
pub(crate) enum AppendGate {
    Proceed,
    Skip,
}

/// Like `append_entry_to_strand_checked`, but the gate may also decide —
/// from the events read under the write lock — that no entry is needed.
/// Returns `Ok(None)` when the gate skipped, `Ok(Some(outcome))` when it wrote.
pub(crate) fn append_entry_to_strand_gated(
    req: JournalEntryAppendRequest,
    gate: impl FnOnce(&[(usize, Event)]) -> Result<AppendGate, String>,
) -> Result<Option<JournalAppendOutcome>, String> {
    with_journal_write_lock(|journal| {
        let path = ensure_journal()?;
        let read = read_journal_lossy(&path);
        if let Some(error) = read.read_error {
            return Err(error);
        }
        if !read.diagnostics.is_empty() {
            return Err(format!(
                "cannot append: journal has {} parse error(s); run doctor first",
                read.diagnostics.len()
            ));
        }
        match gate(&read.events)? {
            AppendGate::Skip => Ok(None),
            AppendGate::Proceed => {
                append_entry_to_strand_unlocked(journal, &read.events, req).map(Some)
            }
        }
    })
}

pub(crate) fn append_entry_to_strand(
    req: JournalEntryAppendRequest,
) -> Result<JournalAppendOutcome, String> {
    append_entry_to_strand_checked(req, |_| Ok(()))
}
fn append_journal_anchor_unlocked(
    journal: &mut std::fs::File,
    journal_path: &PathBuf,
) -> Result<(), String> {
    let read = read_journal_lossy(journal_path);
    if read.read_error.is_some() || !read.diagnostics.is_empty() {
        return Ok(());
    }
    let events: Vec<Event> = read.events.into_iter().map(|(_, event)| event).collect();
    if matches!(events.last(), Some(Event::JournalAnchored { .. })) {
        return Ok(());
    }
    let anchor = crate::event::make_journal_anchor(&events);
    append_event_unlocked(journal, &anchor)
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

// -- v2 cutover/import planning -------------------------------------------------
#[derive(Debug, serde::Serialize)]
pub(crate) struct CutoverV2Map {
    pub(crate) schema: &'static str,
    pub(crate) source_event_count: usize,
    pub(crate) source_digest: String,
    pub(crate) imported_event_count: usize,
    pub(crate) strands: std::collections::BTreeMap<String, String>,
    pub(crate) entries: Vec<CutoverV2EntryMap>,
    pub(crate) unresolved_refs: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct CutoverV2EntryMap {
    pub(crate) old_offset: usize,
    pub(crate) old_strand_id: String,
    pub(crate) new_strand_id: String,
    pub(crate) old_entry_id: Option<String>,
    pub(crate) new_entry_id: String,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub(crate) struct CutoverV2Certificate {
    pub(crate) schema: String,
    pub(crate) created_at: String,
    pub(crate) tool_version: String,
    pub(crate) tool_commit: String,
    pub(crate) source_journal: String,
    pub(crate) archive_journal: String,
    pub(crate) target_journal: String,
    pub(crate) map_path: String,
    pub(crate) source_event_count: usize,
    pub(crate) source_event_digest: String,
    pub(crate) imported_event_count: usize,
    pub(crate) source_journal_sha256: String,
    pub(crate) target_journal_initial_sha256: String,
    pub(crate) map_sha256: String,
}

pub(crate) fn cutover_certificate_path_for_map(map_path: &std::path::Path) -> PathBuf {
    map_path.with_extension("certificate.json")
}

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

pub(crate) struct CutoverV2Plan {
    pub(crate) events: Vec<Event>,
    pub(crate) map: CutoverV2Map,
}

#[derive(Clone)]
struct LegacyLogView {
    pub(crate) id: String,
    pub(crate) ts: String,
    pub(crate) content: String,
    pub(crate) effect: Option<event::EntryEffect>,
    pub(crate) refs: Vec<String>,
    pub(crate) ref_: Option<String>,
    pub(crate) git: Option<event::GitContext>,
    pub(crate) provenance: Option<serde_json::Value>,
    pub(crate) old_entry_id: Option<String>,
}

fn source_events_digest(source: &[(usize, Event)]) -> Result<String, String> {
    let mut hasher = Sha256::new();
    for (offset, event) in source {
        hasher.update(offset.to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(
            serde_json::to_vec(event).map_err(|e| format!("serialize source event: {}", e))?,
        );
        hasher.update(b"\n");
    }
    Ok(hex::encode(hasher.finalize()))
}

// Migration translator: the effect entry content templates spelled below
// ("close disposition=...", "link <type> <target>", ...) intentionally mirror
// the canonical pairs in `event::*_entry_parts`. This is v1→v2 translation of
// historical events, kept literal on purpose; new writes must go through the
// event factory constructors, never copy these strings.
pub(crate) fn build_cutover_v2_plan(source: &[(usize, Event)]) -> Result<CutoverV2Plan, String> {
    let mut strand_meta: std::collections::BTreeMap<
        String,
        (String, Option<String>, Option<String>),
    > = std::collections::BTreeMap::new();
    let mut first_log: std::collections::BTreeMap<String, LegacyLogView> =
        std::collections::BTreeMap::new();

    for (_, event) in source {
        match event {
            Event::StrandCreated {
                id,
                ts,
                strand_type,
                slug,
            } => {
                strand_meta.insert(id.clone(), (ts.clone(), strand_type.clone(), slug.clone()));
            }
            Event::LogAppended {
                id,
                ts,
                content,
                effect,
                refs,
                ref_,
                git,
                provenance,
                entry_id,
                ..
            } => {
                first_log
                    .entry(id.clone())
                    .or_insert_with(|| LegacyLogView {
                        id: id.clone(),
                        ts: ts.clone(),
                        content: content.clone(),
                        effect: effect.clone(),
                        refs: refs.clone(),
                        ref_: ref_.clone(),
                        git: git.clone(),
                        provenance: provenance.clone(),
                        old_entry_id: entry_id.clone(),
                    });
            }
            _ => {}
        }
    }

    let mut strand_map: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for (old_id, first) in &first_log {
        let first_effect =
            translate_effect(first.effect.clone(), &std::collections::BTreeMap::new());
        let new_id = event::compute_entry_id(
            None,
            &first.ts,
            &first.content,
            &first.refs,
            first_effect.as_ref(),
            first.provenance.as_ref(),
            first.git.as_ref(),
        );
        strand_map.insert(old_id.clone(), new_id);
    }

    let mut out = Vec::new();
    let mut created_new: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut heads_by_old: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    let mut entry_hash_map: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    let mut legacy_pin_map: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    let mut entries = Vec::new();
    let mut unresolved_refs = Vec::new();

    for (offset, event) in source {
        match event {
            Event::StrandCreated { .. } | Event::JournalAnchored { .. } => {}
            Event::LogAppended {
                id,
                ts,
                content,
                effect,
                refs,
                ref_,
                git,
                provenance,
                entry_id,
                ..
            } => {
                let view = LegacyLogView {
                    id: id.clone(),
                    ts: ts.clone(),
                    content: content.clone(),
                    effect: effect.clone(),
                    refs: refs.clone(),
                    ref_: ref_.clone(),
                    git: git.clone(),
                    provenance: provenance.clone(),
                    old_entry_id: entry_id.clone(),
                };
                import_log_view(
                    *offset,
                    view,
                    &strand_meta,
                    &mut strand_map,
                    &mut created_new,
                    &mut heads_by_old,
                    &mut entry_hash_map,
                    &mut legacy_pin_map,
                    &mut entries,
                    &mut unresolved_refs,
                    &mut out,
                )?;
            }
            Event::StrandClosed {
                id,
                ts,
                disposition,
                provenance,
            } => {
                import_effect_entry(
                    *offset,
                    id,
                    ts,
                    format!("close disposition={}", disposition),
                    Some(event::EntryEffect::close(disposition)),
                    provenance.clone(),
                    &strand_meta,
                    &mut strand_map,
                    &mut created_new,
                    &mut heads_by_old,
                    &mut entry_hash_map,
                    &mut legacy_pin_map,
                    &mut entries,
                    &mut unresolved_refs,
                    &mut out,
                )?;
            }
            Event::StrandReopened { id, ts, provenance } => {
                import_effect_entry(
                    *offset,
                    id,
                    ts,
                    "reopen erroneous close".to_string(),
                    Some(event::EntryEffect::Reopen),
                    provenance.clone(),
                    &strand_meta,
                    &mut strand_map,
                    &mut created_new,
                    &mut heads_by_old,
                    &mut entry_hash_map,
                    &mut legacy_pin_map,
                    &mut entries,
                    &mut unresolved_refs,
                    &mut out,
                )?;
            }
            Event::EdgeLinked {
                id,
                ts,
                to,
                edge_type,
                provenance,
            } => {
                let etype = edge_type.as_deref().unwrap_or("depends-on");
                let target = strand_map.get(to).cloned().unwrap_or_else(|| to.clone());
                import_effect_entry(
                    *offset,
                    id,
                    ts,
                    format!("link {} {}", etype, target),
                    Some(event::EntryEffect::link(&target, etype)),
                    provenance.clone(),
                    &strand_meta,
                    &mut strand_map,
                    &mut created_new,
                    &mut heads_by_old,
                    &mut entry_hash_map,
                    &mut legacy_pin_map,
                    &mut entries,
                    &mut unresolved_refs,
                    &mut out,
                )?;
            }
            Event::EdgeUnlinked {
                id,
                ts,
                to,
                edge_type,
                provenance,
            } => {
                let etype = edge_type.as_deref().unwrap_or("depends-on");
                let target = strand_map.get(to).cloned().unwrap_or_else(|| to.clone());
                import_effect_entry(
                    *offset,
                    id,
                    ts,
                    format!("unlink {} {}", etype, target),
                    // Legacy EdgeUnlinked had no per-link id → key tombstone.
                    Some(event::EntryEffect::unlink(&target, etype, None)),
                    provenance.clone(),
                    &strand_meta,
                    &mut strand_map,
                    &mut created_new,
                    &mut heads_by_old,
                    &mut entry_hash_map,
                    &mut legacy_pin_map,
                    &mut entries,
                    &mut unresolved_refs,
                    &mut out,
                )?;
            }
            Event::StrandHidden { id, ts } => {
                import_effect_entry(
                    *offset,
                    id,
                    ts,
                    "hide".to_string(),
                    Some(event::EntryEffect::Hide),
                    None,
                    &strand_meta,
                    &mut strand_map,
                    &mut created_new,
                    &mut heads_by_old,
                    &mut entry_hash_map,
                    &mut legacy_pin_map,
                    &mut entries,
                    &mut unresolved_refs,
                    &mut out,
                )?;
            }
            Event::StrandUnhidden { id, ts } => {
                import_effect_entry(
                    *offset,
                    id,
                    ts,
                    "unhide".to_string(),
                    Some(event::EntryEffect::Unhide),
                    None,
                    &strand_meta,
                    &mut strand_map,
                    &mut created_new,
                    &mut heads_by_old,
                    &mut entry_hash_map,
                    &mut legacy_pin_map,
                    &mut entries,
                    &mut unresolved_refs,
                    &mut out,
                )?;
            }
            Event::CheckpointCreated {
                id,
                ts,
                observed,
                action,
                provenance,
                ..
            } => {
                import_effect_entry(
                    *offset,
                    id,
                    ts,
                    format!(
                        "[checkpoint] imported observed=\"{}\" action=\"{}\"",
                        observed, action
                    ),
                    None,
                    provenance.clone(),
                    &strand_meta,
                    &mut strand_map,
                    &mut created_new,
                    &mut heads_by_old,
                    &mut entry_hash_map,
                    &mut legacy_pin_map,
                    &mut entries,
                    &mut unresolved_refs,
                    &mut out,
                )?;
            }
            Event::SubjectBound {
                id,
                ts,
                subject_type,
                subject_id,
                strand_id,
                provenance,
            } => {
                let mapped = strand_map
                    .get(strand_id)
                    .cloned()
                    .unwrap_or_else(|| strand_id.clone());
                out.push(Event::SubjectBound {
                    id: id.clone(),
                    ts: ts.clone(),
                    subject_type: subject_type.clone(),
                    subject_id: subject_id.clone(),
                    strand_id: mapped,
                    provenance: provenance.clone(),
                });
            }
        }
    }

    let anchor = event::make_journal_anchor(&out);
    out.push(anchor);
    let imported_event_count = out.len();

    Ok(CutoverV2Plan {
        events: out,
        map: CutoverV2Map {
            schema: "tasktree-v2-cutover-map-v1",
            source_event_count: source.len(),
            source_digest: source_events_digest(source)?,
            imported_event_count,
            strands: strand_map,
            entries,
            unresolved_refs,
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn import_effect_entry(
    offset: usize,
    old_id: &str,
    ts: &str,
    content: String,
    effect: Option<event::EntryEffect>,
    provenance: Option<serde_json::Value>,
    strand_meta: &std::collections::BTreeMap<String, (String, Option<String>, Option<String>)>,
    strand_map: &mut std::collections::BTreeMap<String, String>,
    created_new: &mut std::collections::HashSet<String>,
    heads_by_old: &mut std::collections::BTreeMap<String, String>,
    entry_hash_map: &mut std::collections::BTreeMap<String, String>,
    legacy_pin_map: &mut std::collections::BTreeMap<String, String>,
    entries: &mut Vec<CutoverV2EntryMap>,
    unresolved_refs: &mut Vec<String>,
    out: &mut Vec<Event>,
) -> Result<(), String> {
    let view = LegacyLogView {
        id: old_id.to_string(),
        ts: ts.to_string(),
        content,
        effect,
        refs: Vec::new(),
        ref_: None,
        git: None,
        provenance,
        old_entry_id: None,
    };
    import_log_view(
        offset,
        view,
        strand_meta,
        strand_map,
        created_new,
        heads_by_old,
        entry_hash_map,
        legacy_pin_map,
        entries,
        unresolved_refs,
        out,
    )
}

#[allow(clippy::too_many_arguments)]
fn import_log_view(
    offset: usize,
    view: LegacyLogView,
    strand_meta: &std::collections::BTreeMap<String, (String, Option<String>, Option<String>)>,
    strand_map: &mut std::collections::BTreeMap<String, String>,
    created_new: &mut std::collections::HashSet<String>,
    heads_by_old: &mut std::collections::BTreeMap<String, String>,
    entry_hash_map: &mut std::collections::BTreeMap<String, String>,
    legacy_pin_map: &mut std::collections::BTreeMap<String, String>,
    entries: &mut Vec<CutoverV2EntryMap>,
    unresolved_refs: &mut Vec<String>,
    out: &mut Vec<Event>,
) -> Result<(), String> {
    let translated_refs = translate_refs(
        &view.refs,
        view.ref_.as_deref(),
        entry_hash_map,
        legacy_pin_map,
        unresolved_refs,
    );
    let translated_effect = translate_effect(view.effect.clone(), strand_map);
    let prev = heads_by_old.get(&view.id).cloned();
    let entry_id = event::compute_entry_id(
        prev.as_deref(),
        &view.ts,
        &view.content,
        &translated_refs,
        translated_effect.as_ref(),
        view.provenance.as_ref(),
        view.git.as_ref(),
    );
    let new_strand_id = if let Some(existing) = strand_map.get(&view.id) {
        existing.clone()
    } else {
        strand_map.insert(view.id.clone(), entry_id.clone());
        entry_id.clone()
    };

    if !created_new.contains(&new_strand_id) {
        let (created_ts, strand_type, slug) = strand_meta
            .get(&view.id)
            .cloned()
            .unwrap_or_else(|| (view.ts.clone(), None, None));
        out.push(Event::StrandCreated {
            id: new_strand_id.clone(),
            ts: created_ts,
            strand_type,
            slug,
        });
        created_new.insert(new_strand_id.clone());
    }

    let append_id = event::compute_append_id(&new_strand_id, &view.ts, &view.content);
    out.push(Event::LogAppended {
        id: new_strand_id.clone(),
        ts: view.ts.clone(),
        content: view.content.clone(),
        effect: translated_effect,
        prev_entry_id: prev,
        entry_id: Some(entry_id.clone()),
        refs: translated_refs,
        ref_: None,
        append_id: Some(append_id),
        git: view.git.clone(),
        provenance: view.provenance.clone(),
    });

    if let Some(old_entry_id) = &view.old_entry_id {
        entry_hash_map.insert(old_entry_id.clone(), entry_id.clone());
    }
    legacy_pin_map.insert(format!("{}@{}", view.id, offset), entry_id.clone());
    heads_by_old.insert(view.id.clone(), entry_id.clone());
    entries.push(CutoverV2EntryMap {
        old_offset: offset,
        old_strand_id: view.id,
        new_strand_id,
        old_entry_id: view.old_entry_id,
        new_entry_id: entry_id,
    });
    Ok(())
}

fn translate_effect(
    effect: Option<event::EntryEffect>,
    strand_map: &std::collections::BTreeMap<String, String>,
) -> Option<event::EntryEffect> {
    match effect {
        Some(event::EntryEffect::Link { target, edge_type }) => Some(event::EntryEffect::Link {
            target: strand_map.get(&target).cloned().unwrap_or(target),
            edge_type,
        }),
        Some(event::EntryEffect::Unlink {
            target,
            edge_type,
            link_entry_id,
        }) => Some(event::EntryEffect::Unlink {
            target: strand_map.get(&target).cloned().unwrap_or(target),
            edge_type,
            link_entry_id,
        }),
        other => other,
    }
}

fn translate_refs(
    refs: &[String],
    legacy_ref: Option<&str>,
    entry_hash_map: &std::collections::BTreeMap<String, String>,
    legacy_pin_map: &std::collections::BTreeMap<String, String>,
    unresolved_refs: &mut Vec<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    for r in refs {
        out.push(entry_hash_map.get(r).cloned().unwrap_or_else(|| r.clone()));
    }
    if let Some(pin) = legacy_ref {
        if let Some(mapped) = legacy_pin_map.get(pin) {
            if !out.contains(mapped) {
                out.push(mapped.clone());
            }
        } else {
            unresolved_refs.push(pin.to_string());
        }
    }
    out
}

pub(crate) fn apply_cutover_v2(
    journal_path: &std::path::Path,
    archive_path: &std::path::Path,
    map_path: &std::path::Path,
    plan: &CutoverV2Plan,
) -> Result<(), String> {
    let lock_path = journal_lock_path()?;
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| format!("cannot open journal.lock: {}", e))?;
    fs2::FileExt::lock_exclusive(&lock_file)
        .map_err(|e| format!("cannot acquire journal lock: {}", e))?;

    let tmp_path = journal_path.with_extension("jsonl.v2tmp");
    let tmp_map_path = map_path.with_extension("json.tmp");
    let certificate_path = cutover_certificate_path_for_map(map_path);
    let tmp_certificate_path = certificate_path.with_extension("json.tmp");
    let mut archived_v1 = false;
    let mut installed_v2 = false;
    let mut installed_certificate = false;
    let result = (|| {
        if archive_path.exists() {
            return Err(format!(
                "archive already exists: {}",
                archive_path.display()
            ));
        }
        if map_path.exists() {
            return Err(format!(
                "mapping file already exists: {}",
                map_path.display()
            ));
        }
        if certificate_path.exists() {
            return Err(format!(
                "cutover certificate already exists: {}",
                certificate_path.display()
            ));
        }

        let current = read_journal_lossy(&journal_path.to_path_buf());
        if let Some(error) = current.read_error {
            return Err(error);
        }
        if !current.diagnostics.is_empty() {
            return Err(format!(
                "cannot cut over: journal has {} parse error(s); run doctor first",
                current.diagnostics.len()
            ));
        }
        let source_journal_bytes =
            std::fs::read(journal_path).map_err(|e| format!("read source journal bytes: {}", e))?;
        let current_digest = source_events_digest(&current.events)?;
        if current.events.len() != plan.map.source_event_count
            || current_digest != plan.map.source_digest
        {
            return Err(format!(
                "journal changed during cutover planning: expected {} events digest {}, found {} events digest {}; rerun cutover-v2",
                plan.map.source_event_count,
                plan.map.source_digest,
                current.events.len(),
                current_digest
            ));
        }

        write_events_jsonl(&tmp_path, &plan.events)?;
        let map_json = serde_json::to_string_pretty(&plan.map)
            .map_err(|e| format!("serialize migration map: {}", e))?;
        std::fs::write(&tmp_map_path, map_json)
            .map_err(|e| format!("write migration map: {}", e))?;
        let target_journal_bytes =
            std::fs::read(&tmp_path).map_err(|e| format!("read v2 journal bytes: {}", e))?;
        let map_bytes =
            std::fs::read(&tmp_map_path).map_err(|e| format!("read migration map bytes: {}", e))?;
        let certificate = CutoverV2Certificate {
            schema: "tasktree-v2-cutover-certificate-v1".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            tool_commit: env!("MNEMA_COMMIT").to_string(),
            source_journal: journal_path.display().to_string(),
            archive_journal: archive_path.display().to_string(),
            target_journal: journal_path.display().to_string(),
            map_path: map_path.display().to_string(),
            source_event_count: plan.map.source_event_count,
            source_event_digest: plan.map.source_digest.clone(),
            imported_event_count: plan.map.imported_event_count,
            source_journal_sha256: sha256_bytes(&source_journal_bytes),
            target_journal_initial_sha256: sha256_bytes(&target_journal_bytes),
            map_sha256: sha256_bytes(&map_bytes),
        };
        let certificate_json = serde_json::to_string_pretty(&certificate)
            .map_err(|e| format!("serialize cutover certificate: {}", e))?;
        std::fs::write(&tmp_certificate_path, certificate_json)
            .map_err(|e| format!("write cutover certificate: {}", e))?;
        std::fs::rename(journal_path, archive_path)
            .map_err(|e| format!("archive v1 journal: {}", e))?;
        archived_v1 = true;
        std::fs::rename(&tmp_path, journal_path)
            .map_err(|e| format!("install v2 journal: {}", e))?;
        installed_v2 = true;
        std::fs::rename(&tmp_map_path, map_path)
            .map_err(|e| format!("install migration map: {}", e))?;
        std::fs::rename(&tmp_certificate_path, &certificate_path)
            .map_err(|e| format!("install cutover certificate: {}", e))?;
        installed_certificate = true;
        Ok(())
    })();

    let _ = fs2::FileExt::unlock(&lock_file);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
        let _ = std::fs::remove_file(&tmp_map_path);
        let _ = std::fs::remove_file(&tmp_certificate_path);
        if installed_certificate {
            let _ = std::fs::remove_file(&certificate_path);
        }
        if archived_v1 {
            if installed_v2 {
                let _ = std::fs::remove_file(journal_path);
            }
            if !journal_path.exists() && archive_path.exists() {
                let _ = std::fs::rename(archive_path, journal_path);
            }
        }
    }
    result
}
fn write_events_jsonl(path: &std::path::Path, events: &[Event]) -> Result<(), String> {
    let mut file =
        std::fs::File::create(path).map_err(|e| format!("create {}: {}", path.display(), e))?;
    for event in events {
        let line = serde_json::to_string(event).map_err(|e| format!("serialize event: {}", e))?;
        writeln!(file, "{}", line).map_err(|e| format!("write {}: {}", path.display(), e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_journal_lossy_reports_structured_parse_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        let (created, appended) = crate::event::make_strand_created("journal read test", None);
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

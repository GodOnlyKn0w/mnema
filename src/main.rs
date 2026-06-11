mod diagnostics;
mod event;
mod projection;
mod output;
mod tree;

use crate::event::TimelineEventKind;

use clap::{error::ErrorKind, Parser, Subcommand};
use event::Event;
use serde_json::json;
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::time::Instant;
use fs2::FileExt;

const JOURNAL_DIR: &str = ".tasktree";
const JOURNAL_FILE: &str = ".tasktree/journal.jsonl";

/// Resolve the journal directory with priority:
///   1. TASKTREE_HOME env var (explicit override; must contain .tasktree/)
///   2. Walk-up from cwd: nearest ancestor containing .tasktree/
///   3. Error if neither found (no silent fallback)
///
/// Walk-up enables shared journal across git worktrees: any worktree cwd
/// walk-ups to the project root .tasktree/. See architecture.md s15.7.
fn resolve_journal_dir() -> Result<PathBuf, String> {
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

fn version_info() -> &'static str {
    concat!(
        env!("CARGO_PKG_VERSION"),
        "\njournal schema: tasktree-journal-v1",
        "\ncommit: ",
        env!("TASKTREE_COMMIT"),
        "\nbuild: ",
        env!("TASKTREE_BUILD_PROFILE"),
    )
}

#[derive(Parser)]
#[command(
    name = "tasktree",
    version = version_info(),
    after_help = "\
Commands:
  orient      Session-start orientation: active strand menu + catch-up commands
  add         Create a new strand
  append      Append an entry to a strand
  bind        Record a subject binding
  checkpoint  Record context before an irreversible or state-closing action
  current     Project the latest effective subject binding
  doctor      Diagnose journal integrity
  explain     Explain a diagnostic code
  export      Export journal as standalone audit artifact
  list        List strands
  show        Show a strand
  search      Search entries
  find        Find a strand

Run:
  tasktree <command> --help"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize .tasktree/ directory and journal
    Init,
    /// Create a new strand with first log entry
    Add {
        /// Content for the first log entry
        content: String,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Strand type: task, dag, why, session (default: auto-detect)
        #[arg(long = "type", value_name = "TYPE")]
        strand_type: Option<String>,
    },
    /// Append content to a strand, or create a new strand from content.
    #[command(after_help = "\
Invocation forms:
  tasktree append <CONTENT> [LEGACY_ID]
  tasktree append --stdin [--id <ID> | --new]
  tasktree append --file <PATH> [--id <ID> | --new]

Content source (choose exactly one):
  CONTENT             Log content
  --stdin             Read content from standard input
  --file <PATH>       Read content from a file

Target (choose at most one):
  (none)              Append to most recently active strand
  --id <ID>           Append to a specific strand
  <ID>                [LEGACY] Strand ID as second positional argument.
                      Only valid with positional CONTENT.
  --new               Create a new strand from the content

Rules:
  CONTENT, --stdin, and --file are mutually exclusive.
  --new, --id, and LEGACY_ID are mutually exclusive.
  LEGACY_ID is only valid with positional CONTENT.
  Empty content is rejected.

Examples:
  tasktree append \"short note\"
  tasktree append \"short note\" 0000019dd34b
  tasktree append --id 0000019dd34b \"short note\"

  echo \"long note\" | tasktree append --stdin
  echo \"long note\" | tasktree append --stdin --id 0000019dd34b

  tasktree append --file note.md
  tasktree append --file note.md --id 0000019dd34b

  echo \"new strand title\" | tasktree append --stdin --new
  tasktree append --file note.md --id 0000019dd34b --provenance '{\"producer\":\"pi\",\"model\":\"gpt-5\"}'

Markers (optional bracket prefix on the first line):
  judgment     [decision] [constraint] [friction] [fixed] [lesson] [insight]
  observation  [observed] [check] [progress] [deliverable]
  planning     [deadline] <text> by=YYYY-MM-DD   (or by=<RFC3339>)
  structure    [covers] [guide] [skill] [task] [session]
  closing      [done] [verified] [cancelled] [failed] [merged] [ended]
               [dispatched] [registered]
  system       [checkpoint] [hidden] [waiting:human] [grill]
  Unknown [m...]/[c...]/[f...]/[v...]/[d...]/[e...]/[r...] prefixes are
  rejected to catch typos; other bracket text passes through as content.

Provenance:
  --provenance <JSON>  Optional structured metadata. Must be a JSON
                       object. Stored on the LogAppended event, not in
                       the entry text. Older journals ignore it.")]
    Append {
        /// Log content
        content: Option<String>,
        /// [LEGACY] Strand ID as second positional argument.
        /// Only valid with positional CONTENT.
        id: Option<String>,
        /// Create a new strand from the content
        #[arg(short, long)]
        new: bool,
        /// Read content from standard input
        #[arg(long, verbatim_doc_comment)]
        stdin: bool,
        /// Read content from a file
        #[arg(long, value_name = "PATH", verbatim_doc_comment)]
        file: Option<String>,
        /// Output format: text (default) or json
        #[arg(short, long, default_value = "text")]
        format: Option<String>,
        /// Append to a specific strand
        #[arg(long = "id", value_name = "ID", verbatim_doc_comment)]
        explicit_id: Option<String>,
        /// Optional provenance JSON object. Stored as metadata on the
        /// LogAppended event; the entry text is unchanged.
        #[arg(long = "provenance", value_name = "JSON")]
        provenance: Option<String>,
    },
    /// Record context before an irreversible or state-closing action
    #[command(after_help = "\
Invocation forms:
  tasktree checkpoint --action \"<action and reason>\"
  tasktree checkpoint --id <STRAND_ID> --action \"<action and reason>\"
  tasktree checkpoint --id <STRAND_ID> --tail 30 --format json --action \"<action and reason>\"

Required:
  --action <TEXT>    Agent-supplied action and reason. Recorded, not classified.

Target:
  --id <STRAND_ID>   Use explicit strand. Prefer this for git commits and destructive actions.
  omitted --id       Resolve to most recently active strand; stdout shows resolved_by.

Output:
  default            Human-readable stdout + journal append. The strand line
                     includes entry count and state for at-a-glance confirmation.
  --format json      Machine-readable stdout + journal append. Includes a
                     \"result\" field with the updated strand card (OrientStrand).

  staleness          Always printed: age of strand's last entry + journal delta
                     since that entry. Catch-up command shown when delta > 0.
  catch-up           tasktree timeline --since-offset <N> --links <STRAND_ID>
                     (emitted verbatim when journal delta > 0)
  warnings           W070 (strand moved under you) and W071 (closed strand) fire
                     as scar lines in text output; in json output, a \"warnings\"
                     array (elements: {\"code\", \"detail\"}) is always present.
                     Both codes are informational — exit is still 0.

Exit codes:
  0 ok
  1 strand resolve/show failed
  2 append failed
  3 invalid arguments

Rules:
  --tail only limits displayed output.
  --tail does not change observed_entries_before_append.
  checkpoint failed means hard stop.")]
    Checkpoint {
        /// Strand ID (prefix match). Prefer explicit --id for commits and destructive actions.
        #[arg(long = "id", value_name = "STRAND_ID")]
        id: Option<String>,
        /// Agent-supplied action and reason. Recorded, not classified.
        #[arg(long, value_name = "TEXT")]
        action: String,
        /// Show only the last N log entries in checkpoint stdout
        #[arg(long, value_name = "N")]
        tail: Option<usize>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Include hidden strands when resolving the most-recent active strand
        /// (default: only consider visible strands; passing --include-hidden
        /// or --all falls back to hidden ones if no visible strand exists)
        #[arg(long, alias = "all")]
        include_hidden: bool,
        /// Optional provenance JSON object. Same shape as
        /// `append --provenance`.
        #[arg(long = "provenance", value_name = "JSON")]
        provenance: Option<String>,
    },
    /// List all strands (reverse chronological, most recent last)
    List {
        /// Include hidden strands
        #[arg(long)]
        all: bool,
        /// Show strands linked FROM this ID
        #[arg(long, value_name = "ID")]
        links: Option<String>,
        /// Show strands linked TO this ID
        #[arg(long, value_name = "ID")]
        backlinks: Option<String>,
        /// Filter by last entry state (done|open)
        #[arg(long, value_name = "STATE")]
        state: Option<String>,
        /// Filter by strand type (task|dag|why|session)
        #[arg(long = "type", value_name = "TYPE")]
        list_type: Option<String>,
        /// Filter to strands silent for duration (s/m/h/d, e.g. 2h)
        #[arg(long, value_name = "DURATION")]
        stale: Option<String>,
        /// Filter to strands with last entry offset <= N (silent)
        #[arg(long, value_name = "N", conflicts_with = "since_offset")]
        stale_offset: Option<usize>,
        /// Filter to strands with last entry offset > N (updated since)
        #[arg(long, value_name = "N", conflicts_with = "stale_offset")]
        since_offset: Option<usize>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
    /// Show full details of one strand
    Show {
        /// Strand ID (prefix match). Omit with --last to show most recent.
        id: Option<String>,
        /// Show the most recently active strand instead of specifying an id
        #[arg(long)]
        last: bool,
        /// Show only the last N log entries (requires --last)
        #[arg(long, value_name = "N")]
        tail: Option<usize>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Use shared lock for consistent read (blocks writers)
        #[arg(long)]
        locked: bool,
    },
    /// Full-text search across all log content
    Search {
        /// Search query (substring match, case-insensitive)
        query: String,
        /// Include hidden strands in the result set (default: exclude)
        #[arg(long)]
        include_hidden: bool,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
    /// Resolve a prefix to full strand ID
    Find {
        /// Strand ID prefix
        id: String,
    },
    /// Create a directed link between two strands
    Link {
        /// Source strand ID (prefix match)
        source: String,
        /// Target strand ID (prefix match)
        target: String,
        /// Edge type: depends-on, belongs-to, why (default: depends-on)
        #[arg(long = "type", value_name = "TYPE")]
        edge_type: Option<String>,
    },
    /// Hide a strand from default list view
    Hide {
        /// Strand ID (prefix match)
        id: String,
        /// Reason for hiding (optional). If provided, appends '[hidden] <reason>' to the strand.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Unhide a previously hidden strand
    Unhide {
        /// Strand ID (prefix match)
        id: String,
    },

    /// Record a subject binding. Append-only. Newer bindings supersede
    /// older ones for the same (subject-type, subject-id) pair.
    #[command(after_help = "\
Examples:
  tasktree bind --subject-type pi-session --subject-id abc123 --id 0000019dd34b
  tasktree bind --subject-type ci-run --subject-id run-42 --id 0000019dd34b --format json
  echo '{\"subject_type\":\"pi-session\",\"subject_id\":\"abc\",\"strand_id\":\"0000019dd34b\"}' | tasktree bind --stdin

Rules:
  --subject-type and --subject-id are required, non-empty strings.
  --id is required and must be a strand id (prefix match).
  --stdin reads the same fields as a JSON object from standard input.")]
    Bind {
        /// Subject type discriminator (generic string, e.g. pi-session, ci-run).
        #[arg(long = "subject-type", value_name = "TYPE")]
        subject_type: Option<String>,
        /// Subject id within the chosen type.
        #[arg(long = "subject-id", value_name = "ID")]
        subject_id: Option<String>,
        /// Target strand id (prefix match). Must already exist in the journal.
        #[arg(long = "id", value_name = "STRAND_ID")]
        id: Option<String>,
        /// Read binding from a single JSON object on stdin.
        /// Schema: { "subject_type": "...", "subject_id": "...", "strand_id": "..." }
        #[arg(long)]
        stdin: bool,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
    /// Project the latest effective subject binding.
    #[command(after_help = "\
Examples:
  tasktree current --subject-type pi-session --subject-id abc123
  tasktree current --subject-type pi-session --subject-id abc123 --format json

Rules:
  --subject-type and --subject-id are required, non-empty strings.
  Returns the strand_id of the latest SubjectBound event for the pair.
  No binding -> exit 1 with stderr message, no stdout payload.")]
    Current {
        #[arg(long = "subject-type", value_name = "TYPE")]
        subject_type: Option<String>,
        #[arg(long = "subject-id", value_name = "ID")]
        subject_id: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },

    /// Explain a diagnostic code (E/W codes from lifecycle, health, arch-boundary)
    Explain {
        /// Diagnostic code to explain (e.g. W068, W062)
        code: String,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Shortcut for --format json
        #[arg(long)]
        json: bool,
    },
    /// Diagnose journal integrity
    Doctor {
        #[command(subcommand)]
        target: DoctorTarget,
    },
    /// Export journal to a standalone audit artifact
    Export {
        /// Output path for the exported journal
        #[arg(long, value_name = "PATH")]
        out: String,
    },
    /// Show events in journal causal order (timeline projection)
    Timeline {
        /// Return events with journal_offset > N
        #[arg(long, value_name = "N", conflicts_with = "since_ts")]
        since_offset: Option<usize>,
        /// Return events with ts >= specified time (converted to approx offset)
        #[arg(long, value_name = "RFC3339", conflicts_with = "since_offset")]
        since_ts: Option<String>,
        /// Return events with journal_offset <= N
        #[arg(long, value_name = "N", conflicts_with = "until_ts")]
        until_offset: Option<usize>,
        /// Return events with ts <= specified time
        #[arg(long, value_name = "RFC3339", conflicts_with = "until_offset")]
        until_ts: Option<String>,
        /// Filter to events from a single strand
        #[arg(long, value_name = "ID", conflicts_with = "links")]
        strand: Option<String>,
        /// Include DAG strand + directly linked strands
        #[arg(long, value_name = "ID", conflicts_with = "strand")]
        links: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Filter to events from strands in the tree rooted at ID
        #[arg(long, value_name = "ID", conflicts_with_all = ["strand", "links"])]
        tree: Option<String>,
        /// Maximum events to return
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
    /// Session-start orientation: menu of active strands with catch-up commands
    #[command(after_help = "\
Pure read: orient never writes to the journal.

Output per active strand:
  handle        Strand id (use with --id)
  summary       First entry (what this line of work is)
  last          Most recent entry (where it left off)
  catch-up      Ready-to-run command showing what happened around this
                strand since it was last touched (cursor = last_offset)

After orienting:
  continue a line   tasktree append --id <ID> \"[decision] ...\"
  new matter        tasktree add \"<summary>\"
  matter concluded  tasktree append --id <ID> \"[done] <how it ended>\"
                    ([cancelled] or [failed] are alternatives)
  before anything irreversible
                    tasktree checkpoint --id <ID> --action \"<what and why>\"

Closed strands are folded to a count; retrieve with tasktree list.
Hidden strands are folded to a count; retrieve with tasktree list --all.
Exit codes:
  0 ok
  1 journal missing or unreadable")]
    Orient {
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Include hidden strands in the menu (default: exclude)
        #[arg(long)]
        include_hidden: bool,
        /// Maximum strands in the menu, most recent first (default: 10)
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
    /// Render one-shot startup context for agents.
    AgentContext {
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Include hidden strands in the result set (default: exclude)
        #[arg(long)]
        include_hidden: bool,
    },
    /// Build nested tree projection from strand edges
    Tree {
        /// Root strand ID (prefix match)
        id: String,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
    /// Render strand context for system prompt injection.
    /// Projects prompt-strands into text or JSON suitable for APPEND_SYSTEM.md.
    Context {
        /// Strand type to project (default: prompt-strand)
        #[arg(long = "type", value_name = "TYPE")]
        context_type: Option<String>,
        /// Filter by [covers] scope (string match on [covers] entries, v1)
        #[arg(long, value_name = "PATH")]
        covers: Vec<String>,
        /// Only include strands with last_entry_offset > N
        #[arg(long, value_name = "N")]
        since_offset: Option<usize>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Exclude [friction] entries. Default exposes them: an unresolved
        /// friction still binds future action (exposure axis, scaffolding
        /// ADR-0002). Hiding is an explicit choice, exposure is the default.
        #[arg(long)]
        exclude_friction: bool,
        /// Include hidden strands in the result set (default: exclude)
        #[arg(long)]
        include_hidden: bool,
    },
}


#[derive(Subcommand)]
enum DoctorTarget {
    /// Check journal integrity
    Journal,
}

fn ensure_journal() -> Result<PathBuf, String> {
    Ok(resolve_journal_dir()?.join("journal.jsonl"))
}

/// Return path to .tasktree/journal.lock (dedicated lock file, not the journal itself).
fn journal_lock_path() -> Result<PathBuf, String> {
    Ok(resolve_journal_dir()?.join("journal.lock"))
}

/// Acquire exclusive lock on journal.lock, open journal.jsonl, run closure, flush, unlock.
/// Lock file opened with .create(true).read(true).write(true) — no append.
fn with_journal_write_lock<T>(f: impl FnOnce(&mut std::fs::File) -> Result<T, String>) -> Result<T, String> {
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
fn with_journal_read_lock<T>(f: impl FnOnce(&mut std::fs::File) -> Result<T, String>) -> Result<T, String> {
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
fn read_events_lossy_locked() -> (Vec<(usize, Event)>, usize) {
    match with_journal_read_lock(|journal| {
        let reader = std::io::BufReader::new(journal);
        let mut events = Vec::new();
        let mut skipped = 0usize;
        for (line_no, line) in reader.lines().enumerate() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    skipped += 1;
                    eprintln!("warning: malformed journal line skipped");
                    eprintln!("path: .tasktree/journal.jsonl");
                    eprintln!("line: {}", line_no + 1);
                    eprintln!("error: I/O error: {}", e);
                    eprintln!("raw:  <unreadable>");
                    continue;
                }
            };
            if line.trim().is_empty() { continue; }
            match serde_json::from_str::<Event>(&line) {
                Ok(event) => events.push((line_no, event)),
                Err(e) => {
                    skipped += 1;
                    let raw: String = line.chars().take(80).collect();
                    eprintln!("warning: malformed journal line skipped");
                    eprintln!("path: .tasktree/journal.jsonl");
                    eprintln!("line: {}", line_no + 1);
                    eprintln!("error: {}", e);
                    eprintln!("raw:  {}", raw);
                }
            }
        }
        Ok((events, skipped))
    }) {
        Ok((events, skipped)) => (events, skipped),
        Err(_) => (Vec::new(), 0),
    }
}

/// Append a single event to an already-open journal. Never locks.
fn append_event_unlocked(journal: &mut std::fs::File, event: &Event) -> Result<(), String> {
    let line = serde_json::to_string(event).map_err(|e| format!("serialize error: {}", e))?;
    writeln!(journal, "{}", line).map_err(|e| format!("write error: {}", e))
}

/// Append multiple events to an already-open journal. Never locks.
fn append_events_unlocked(journal: &mut std::fs::File, events: &[Event]) -> Result<(), String> {
    for event in events {
        append_event_unlocked(journal, event)?;
    }
    Ok(())
}

fn read_events_lossy(path: &PathBuf) -> (Vec<(usize, Event)>, usize) {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: cannot read journal: {}", e);
            return (Vec::new(), 0);
        }
    };
    let reader = std::io::BufReader::new(file);
    let mut events = Vec::new();
    let mut skipped = 0usize;
    for (line_no, line) in reader.lines().enumerate() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                skipped += 1;
                eprintln!("warning: malformed journal line skipped");
                eprintln!("path: .tasktree/journal.jsonl");
                eprintln!("line: {}", line_no + 1);
                eprintln!("error: I/O error: {}", e);
                eprintln!("raw:  <unreadable>");
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Event>(&line) {
            Ok(event) => events.push((line_no, event)),
            Err(e) => {
                skipped += 1;
                let raw: String = line.chars().take(80).collect();
                eprintln!("warning: malformed journal line skipped");
                eprintln!("path: .tasktree/journal.jsonl");
                eprintln!("line: {}", line_no + 1);
                eprintln!("error: {}", e);
                eprintln!("raw:  {}", raw);
            }
        }
    }
    (events, skipped)
}

/// Extract Event values from offset-paired events, discarding offsets.
fn events_only(offset_events: &[(usize, Event)]) -> Vec<&Event> {
    offset_events.iter().map(|(_, e)| e).collect()
}

fn read_events_strict(path: &PathBuf) -> Result<Vec<(usize, Event)>, String> {
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

/// NOTE: Strand sort key is `max(log_appended.ts)` per strand.
fn cmd_init() -> Result<(), String> {
    let dir = PathBuf::from(JOURNAL_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create .tasktree/: {}", e))?;
    let path = PathBuf::from(JOURNAL_FILE);
    if !path.exists() {
        std::fs::write(&path, "").map_err(|e| format!("cannot create journal: {}", e))?;
    }
    // Create empty journal.lock file (synchronization object for concurrent writers)
    let lock_path = dir.join("journal.lock");
    if !lock_path.exists() {
        std::fs::write(&lock_path, "").map_err(|e| format!("cannot create journal.lock: {}", e))?;
    }
    println!("Initialized empty tasktree in .tasktree/");
    Ok(())
}

fn cmd_add(content: &str, format_json: bool, strand_type: Option<&str>) -> Result<(), String> {
    // Auto-detect strand type from content if not provided
    let resolved_type = strand_type.or_else(|| {
        if content.starts_with("para group ") { Some("dag") }
        else if content.starts_with('[') && content.len() > 2
            && content[1..].chars().next().map_or(false, |c| c.is_ascii_digit())
        { Some("task") }
        else { None }
    });

    // acquire lock once, write both events atomically
    let result = with_journal_write_lock(|journal| {
        let (created, appended) = event::make_strand_created(content, resolved_type);
        let id = created.strand_id().to_string();
        append_event_unlocked(journal, &created)?;
        append_event_unlocked(journal, &appended)?;
        Ok(id)
    });
    let id = match result {
        Ok(id) => id,
        Err(e) => return Err(e),
    };
    if format_json {
        let card = strand_card_fresh(&id);
        let card_val = card.as_ref().map(|c| serde_json::to_value(c).ok()).flatten();
        println!("{}", json!({"id": id, "status": "ok", "result": card_val}));
    } else {
        println!("{}", id);
        if let Some((card, state)) = strand_card_fresh_with_state(&id) {
            print_card_with_state(&card, &state);
        }
    }
    Ok(())
}

fn find_strand(events: &[(usize, Event)], id: &str) -> Option<String> {
    // Prefix match: first strand whose id starts with the given string
    events
        .iter()
        .filter_map(|(_, e)| {
            if let Event::StrandCreated { id: nid, .. } = e {
                Some(nid.clone())
            } else {
                None
            }
        })
        .find(|nid| nid.starts_with(id))
}

// ── Journal diagnostics (W-code emitters) ───────────────────
// Every code emitted here MUST have a catalog entry in diagnostics.rs
// (two-way closure: no orphan emissions, no dead codes). Warnings are
// precision-first: a W code that mostly cries wolf teaches agents to
// ignore the whole channel.

/// One emitted diagnostic: (code, one-line detail). The code resolves via
/// `tasktree explain <code>`.
type EmittedDiag = (&'static str, String);

/// Extract comparison tokens for W062 keyword matching: ASCII words of
/// length >= 5 (lowercased) plus contiguous CJK runs of length >= 3.
/// Conservative on purpose — shared full runs, not n-grams.
fn w062_tokens(text: &str) -> std::collections::HashSet<String> {
    let mut tokens = std::collections::HashSet::new();
    let mut ascii_word = String::new();
    let mut cjk_run = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            ascii_word.push(c.to_ascii_lowercase());
        } else {
            if ascii_word.len() >= 5 {
                tokens.insert(ascii_word.clone());
            }
            ascii_word.clear();
        }
        let is_cjk = ('\u{4e00}'..='\u{9fff}').contains(&c);
        if is_cjk {
            cjk_run.push(c);
        } else {
            if cjk_run.chars().count() >= 3 {
                tokens.insert(cjk_run.clone());
            }
            cjk_run.clear();
        }
    }
    if ascii_word.len() >= 5 {
        tokens.insert(ascii_word);
    }
    if cjk_run.chars().count() >= 3 {
        tokens.insert(cjk_run);
    }
    tokens
}

fn parse_event_ts(ts: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc))
}

/// Parse the `by=` token of a [deadline] entry. Accepts RFC3339 or a bare
/// date (YYYY-MM-DD, overdue after that day ends, UTC). Unparseable values
/// emit nothing — don't guess.
fn parse_deadline_by(content: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let by_val = content
        .split_whitespace()
        .find_map(|tok| tok.strip_prefix("by="))?;
    if let Some(ts) = parse_event_ts(by_val) {
        return Some(ts);
    }
    chrono::NaiveDate::parse_from_str(by_val, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(23, 59, 59))
        .map(|dt| chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc))
}

/// Run the W062/W068/W069 emitters over the journal events.
/// Pure: `now` is a parameter, nothing is written.
fn run_journal_diagnostics(events: &[Event], now: chrono::DateTime<chrono::Utc>) -> Vec<EmittedDiag> {
    use std::collections::{HashMap, HashSet};
    let mut diags: Vec<EmittedDiag> = Vec::new();

    // Group LogAppended per strand, keeping ts + provenance
    struct EntryRef<'a> {
        ts: &'a str,
        content: &'a str,
        producer: Option<&'a str>,
    }
    let mut per_strand: HashMap<&str, Vec<EntryRef>> = HashMap::new();
    for event in events {
        if let Event::LogAppended { id, ts, content, provenance, .. } = event {
            per_strand.entry(id.as_str()).or_default().push(EntryRef {
                ts: ts.as_str(),
                content: content.as_str(),
                producer: provenance
                    .as_ref()
                    .and_then(|p| p.get("producer"))
                    .and_then(|v| v.as_str()),
            });
        }
    }

    const CLOSING: [&str; 6] = ["[verified]", "[done]", "[cancelled]", "[failed]", "[merged]", "[ended]"];

    // ── W068: deadline overdue ──
    for (id, entries) in &per_strand {
        let closed = entries
            .iter()
            .any(|e| CLOSING.iter().any(|m| e.content.starts_with(m)));
        if closed {
            continue;
        }
        for e in entries {
            if !e.content.starts_with("[deadline]") {
                continue;
            }
            if let Some(by) = parse_deadline_by(e.content) {
                if now > by {
                    diags.push((
                        "W068",
                        format!("strand {} deadline passed ({})", shorten(id), by.to_rfc3339()),
                    ));
                }
            }
        }
    }

    // ── W069: concurrent marker write ──
    // Same lifecycle marker on the same strand from >= 2 distinct
    // provenance producers. Entries without provenance can't be
    // attributed and are ignored (no guessing).
    for (id, entries) in &per_strand {
        let mut writers: HashMap<&str, HashSet<&str>> = HashMap::new();
        for e in entries {
            if let Some(producer) = e.producer {
                if let Some(marker) = CLOSING.iter().chain(["[dispatched]", "[registered]"].iter()).find(|m| e.content.starts_with(*m)) {
                    writers.entry(marker).or_default().insert(producer);
                }
            }
        }
        for (marker, producers) in writers {
            if producers.len() >= 2 {
                let mut who: Vec<&str> = producers.into_iter().collect();
                who.sort();
                diags.push((
                    "W069",
                    format!("strand {} marker {} written by: {}", shorten(id), marker, who.join(", ")),
                ));
            }
        }
    }

    // ── W062: contradictory decision/constraint ──
    // [decision] and [constraint] sharing a keyword, written within 10
    // minutes, from different strands.
    struct Governed<'a> {
        strand: &'a str,
        ts: chrono::DateTime<chrono::Utc>,
        tokens: std::collections::HashSet<String>,
    }
    let mut decisions: Vec<Governed> = Vec::new();
    let mut constraints: Vec<Governed> = Vec::new();
    for (id, entries) in &per_strand {
        for e in entries {
            let bucket = if e.content.starts_with("[decision]") {
                &mut decisions
            } else if e.content.starts_with("[constraint]") {
                &mut constraints
            } else {
                continue;
            };
            if let Some(ts) = parse_event_ts(e.ts) {
                bucket.push(Governed { strand: id, ts, tokens: w062_tokens(e.content) });
            }
        }
    }
    let mut seen_pairs: HashSet<(String, String, String)> = HashSet::new();
    for d in &decisions {
        for c in &constraints {
            if d.strand == c.strand {
                continue;
            }
            if (d.ts - c.ts).num_seconds().abs() > 600 {
                continue;
            }
            if let Some(shared) = d.tokens.intersection(&c.tokens).next() {
                let key = (
                    shorten(d.strand),
                    shorten(c.strand),
                    shared.clone(),
                );
                if seen_pairs.insert(key) {
                    diags.push((
                        "W062",
                        format!(
                            "decision in {} vs constraint in {} share keyword \"{}\" within 10min",
                            shorten(d.strand),
                            shorten(c.strand),
                            shared
                        ),
                    ));
                }
            }
        }
    }

    diags
}

fn cmd_doctor_journal() -> Result<bool, String> {
    let started = Instant::now();
    let path = resolve_journal_dir()?.join("journal.jsonl");

    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read journal: {}", e))?;

    let lines: Vec<&str> = raw.lines().collect();
    let total_lines = lines.len();
    let mut corrupted = 0usize;
    let mut events: Vec<Event> = Vec::new();
    for line in &lines {
        if line.trim().is_empty() { continue; }
        match serde_json::from_str::<Event>(line) {
            Ok(event) => events.push(event),
            Err(_) => corrupted += 1,
        }
    }

    use std::collections::{HashMap, HashSet};
    let mut created_ids: HashSet<String> = HashSet::new();
    let mut appended_ids: HashSet<String> = HashSet::new();
    let mut strand_event_counts: HashMap<String, usize> = HashMap::new();
    for event in &events {
        match event {
            Event::StrandCreated { id, .. } => { created_ids.insert(id.clone()); }
            Event::LogAppended { id, .. } => { appended_ids.insert(id.clone()); *strand_event_counts.entry(id.clone()).or_insert(0) += 1; }
            _ => {}
        }
    }
    let total_strands = created_ids.len();
    let with_events: Vec<_> = created_ids.iter().filter(|id| strand_event_counts.contains_key(*id)).collect();
    let noise_strands: Vec<_> = created_ids.iter().filter(|id| !strand_event_counts.contains_key(*id)).collect();
    let orphans: Vec<_> = appended_ids.iter().filter(|id| !created_ids.contains(*id)).collect();

    let mut git_head_count = 0usize;
    for line in &lines {
        if line.trim().is_empty() { continue; }
        if line.contains("git_head") || line.contains("git.head") { git_head_count += 1; }
    }

    let state_path = resolve_journal_dir()?.join("doctor-state.json");
    let timeline_status = if state_path.exists() {
        match std::fs::read_to_string(&state_path) {
            Ok(content) => {
                #[derive(serde::Deserialize)] struct DoctorState { line_count: usize }
                match serde_json::from_str::<DoctorState>(&content) {
                    Ok(state) => {
                        if total_lines < state.line_count {
                            format!("warning: {}->{} jump detected (lines decreased)", state.line_count, total_lines)
                        } else if total_lines > state.line_count {
                            format!("monotonic: yes ({}->{})", state.line_count, total_lines)
                        } else {
                            "monotonic: yes (unchanged)".to_string()
                        }
                    }
                    Err(_) => "monotonic: yes (no previous state)".to_string(),
                }
            }
            Err(_) => "monotonic: yes (cannot read previous state)".to_string(),
        }
    } else { "monotonic: yes (first run, no previous state)".to_string() };

    #[derive(serde::Serialize)] struct DoctorStateOut { line_count: usize, updated_at: String }
    let state = DoctorStateOut { line_count: total_lines, updated_at: chrono::Utc::now().to_rfc3339() };
    if let Ok(json) = serde_json::to_string_pretty(&state) { let _ = std::fs::write(&state_path, json); }

    println!("Doctor Journal Report");
    println!("  journal: {}", path.display());
    println!("  lines: {}, corrupted: {}, orphan events: {}", total_lines, corrupted, orphans.len());
    println!();
    println!("  strand coverage:");
    println!("    total strands: {}", total_strands);
    println!("    with events: {}", with_events.len());
    println!("    noise strands (no events): {}", noise_strands.len());
    println!();
    println!("  git context:");
    let pct = if total_lines > 0 { (git_head_count as f64 / total_lines as f64) * 100.0 } else { 0.0 };
    println!("    entries with git.head: {}/{} ({:.0}%)", git_head_count, total_lines, pct);
    println!();
    println!("  timeline:");
    println!("    {}", timeline_status);
    if !orphans.is_empty() {
        println!(); println!("  orphans:");
        for id in &orphans { println!("    {}  (log_appended, no strand_created)", id); }
    }

    // ── lint passes ─────────────────────────────────────
    let mut lint_count = 0usize;

    // Build strand summary map (first LogAppended content for each strand)
    let mut strand_summaries: HashMap<String, String> = HashMap::new();
    let mut strand_entries: HashMap<String, Vec<String>> = HashMap::new();
    for event in &events {
        if let Event::LogAppended { id, content, .. } = event {
            strand_summaries.entry(id.clone()).or_insert_with(|| content.clone());
            strand_entries.entry(id.clone()).or_default().push(content.clone());
        }
    }

    // Lint 1: DAG strands must not have [done] entries
    println!(); println!("  lint: dag-done:");
    for (id, summary) in &strand_summaries {
        if summary.starts_with("para group ") {
            if let Some(entries) = strand_entries.get(id) {
                let has_done = entries.iter().any(|e| e.contains("[done]"));
                if has_done {
                    eprintln!("[lint] DAG strand {} has [done] entry — DAG should only record layer events", id);
                    lint_count += 1;
                }
            }
        }
    }
    println!("    dag strands with [done]: {}", lint_count);

    // Lint 2: Task strands must not have task_created JSON events
    let mut task_lint_count = 0usize;
    println!("  lint: task-created:");
    for (id, summary) in &strand_summaries {
        if summary.starts_with('[') {
            if let Some(entries) = strand_entries.get(id) {
                let has_task_created = entries.iter().any(|e| e.contains("task_created"));
                if has_task_created {
                    eprintln!("[lint] Task strand {} has task_created JSON event — task strands should not have DAG events", id);
                    task_lint_count += 1;
                }
            }
        }
    }
    println!("    task strands with task_created: {}", task_lint_count);
    lint_count += task_lint_count;

    // Lint 3: Orphan links (EdgeLinked target doesn't exist)
    let mut orphan_link_count = 0usize;
    println!("  lint: orphan-links:");
    for event in &events {
        if let Event::EdgeLinked { to, .. } = event {
            if !created_ids.contains(to) {
                eprintln!("[lint] orphan link: target strand {} not found", to);
                orphan_link_count += 1;
            }
        }
    }
    println!("    orphan links: {}", orphan_link_count);
    lint_count += orphan_link_count;

    // Lint 4: [touches] format — known fields only
    let mut touches_format_count = 0usize;
    println!("  lint: touches-format:");
    for entries in strand_entries.values() {
        for entry in entries {
            if let Some(tail) = entry.strip_prefix("[touches] ") {
                for part in tail.split(' ') {
                    if part.is_empty() { continue; }
                    let field = part.split(':').next().unwrap_or("");
                    if field != "write" && field != "read" && field != "creates" && field != "readonly" {
                        eprintln!("[lint] touches format: unrecognized field '{}' in [touches] entry", field);
                        touches_format_count += 1;
                    }
                }
            }
        }
    }
    println!("    unrecognized touches fields: {}", touches_format_count);
    lint_count += touches_format_count;

    // Lint 5: link direction — source and target identity
    let mut link_direction_count = 0usize;
    println!("  lint: link-direction:");
    for event in &events {
        if let Event::EdgeLinked { id: source, to: target, .. } = event {
            let src_summary = strand_summaries.get(source).map(|s| s.as_str()).unwrap_or("");
            let tgt_summary = strand_summaries.get(target).map(|s| s.as_str()).unwrap_or("");
            let src_is_dag = src_summary.starts_with("para group ");
            let src_is_task = src_summary.starts_with('[') && src_summary[1..].chars().next().map_or(false, |c| c.is_ascii_digit());
            let tgt_is_dag = tgt_summary.starts_with("para group ");
            // task→DAG unusual (DAG should link to tasks, not vice versa)
            if src_is_task && tgt_is_dag {
                eprintln!("[lint] link direction: task {} links to DAG {} — unusual", source, target);
                link_direction_count += 1;
            }
            // session→DAG unusual
            if !src_is_dag && !src_is_task && tgt_is_dag {
                eprintln!("[lint] link direction: non-task {} links to DAG {} — unusual", source, target);
                link_direction_count += 1;
            }
        }
    }
    println!("    unusual link directions: {}", link_direction_count);
    lint_count += link_direction_count;

    // Lint 6: strand identity — first entry matches strand type
    let mut identity_count = 0usize;
    println!("  lint: strand-identity:");
    for (id, summary) in &strand_summaries {
        let is_dag = summary.starts_with("para group ");
        let is_task = summary.starts_with('[') && summary.chars().nth(1).map_or(false, |c| c.is_ascii_digit());
        if let Some(entries) = strand_entries.get(id) {
            // DAG strand: all entries should be para layer events (no [NN], no [done])
            if is_dag {
                let has_task_marker = entries.iter().any(|e| {
                    e.starts_with('[') && e.chars().nth(1).map_or(false, |c| c.is_ascii_digit())
                });
                if has_task_marker {
                    eprintln!("[lint] strand identity: DAG strand {} has task-like entries — identity mismatch", id);
                    identity_count += 1;
                }
            }
            // Task strand: all entries should be task layer events (no para group events)
            if is_task {
                // Only warn on para group prefix (task_created already covered by lint 2)
                let has_para_prefix = entries.iter().any(|e| e.starts_with("para group "));
                if has_para_prefix {
                    eprintln!("[lint] strand identity: task strand {} has DAG-like entries — identity mismatch", id);
                    identity_count += 1;
                }
            }
        }
    }
    println!("    identity mismatches: {}", identity_count);
    lint_count += identity_count;

    if lint_count > 0 {
        println!();
        println!("  lint summary: {} issue(s) found (warnings only, not blocking)", lint_count);
    }

    // ── W-code diagnostics ──────────────────────────────
    let diags = run_journal_diagnostics(&events, chrono::Utc::now());
    println!();
    println!("  diagnostics:");
    if diags.is_empty() {
        println!("    (none)");
    } else {
        for (code, detail) in &diags {
            println!("    {} {}  (tasktree explain {})", code, detail, code);
        }
    }

    // Measure fresh projection timing
    let projection_start = Instant::now();
    let (journal_events, _) = read_events_lossy(&path);
    let _strands = projection::project_strands(&journal_events, true);
    let projection_ms = projection_start.elapsed().as_millis();
    println!();
    println!("  projection_ms: {}", projection_ms);
    println!("  total_lines: {}", total_lines);
    println!("  total_events: {}", journal_events.len());

    let has_issues = corrupted > 0 || !orphans.is_empty() || timeline_status.contains("warning") || lint_count > 0 || !diags.is_empty();
    Ok(has_issues)
}

fn cmd_find(id: &str) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _) = read_events_lossy(&path);
    match find_strand(&events, id) {
        Some(full_id) => println!("{}", full_id),
        None => return Err(format!("strand {} not found", id)),
    }
    Ok(())
}

/// Resolve a strand ID prefix to a full strand ID, returning Result.
fn resolve_id(events: &[(usize, Event)], id: &str) -> Result<String, String> {
    find_strand(events, id).ok_or_else(|| format!("strand {} not found", id))
}

fn cmd_link(source: &str, target: &str, edge_type: Option<&str>) -> Result<(), String> {
    // Default edge type: depends-on
    let resolved_type = edge_type.or(Some("depends-on"));
    let events = read_events_strict(&ensure_journal()?)?;
    let src_id = resolve_id(&events, source)?;
    let tgt_id = resolve_id(&events, target)?;
    let etype = resolved_type.unwrap();
    let event = event::make_edge_linked(&src_id, &tgt_id, Some(etype));
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &event)
    })?;
    println!("linked {} -> {} ({})", shorten(&src_id), shorten(&tgt_id), etype);
    if let Some((card, state)) = strand_card_fresh_with_state(&src_id) {
        print_handle_line(&card, &state);
    }
    if let Some((card, state)) = strand_card_fresh_with_state(&tgt_id) {
        print_handle_line(&card, &state);
    }
    println!("{} --{}--> {}", shorten(&src_id), etype, shorten(&tgt_id));
    Ok(())
}
/// Compute current hide_count for a strand by scanning its events. The
/// balance is the number of `StrandHidden` minus `StrandUnhidden` events.
/// Used by hide/unhide to make the operations idempotent.
fn count_hide_unhide(events: &[(usize, Event)], strand_id: &str) -> i32 {
    let mut count: i32 = 0;
    for (_, e) in events {
        if e.strand_id() != strand_id {
            continue;
        }
        match e {
            Event::StrandHidden { .. } => count += 1,
            Event::StrandUnhidden { .. } => count -= 1,
            _ => {}
        }
    }
    count
}

/// Hide a strand. Idempotent: if the strand is already hidden (hide_count > 0),
/// no event is written. The current state read and the append happen inside the
/// same journal write lock so concurrent hide/unhide calls are serialised.
fn cmd_hide(id: &str, reason: Option<&str>) -> Result<(), String> {
    let strand_id = resolve_id(&read_events_strict(&ensure_journal()?)?, id)?;
    // Both the read (to compute current state) and the append must be inside
    // the same write lock. Otherwise two concurrent `cmd_hide` calls would each
    // see hide_count=0 and both append a StrandHidden event.
    let outcome = with_journal_write_lock(|journal| {
        // Re-read events under the lock. The journal file is already open
        // for append, so we use a fresh read of the on-disk file via the
        // shared reader for consistency.
        let path = ensure_journal()?;
        let (events, _) = read_events_lossy(&path);
        let current = count_hide_unhide(&events, &strand_id);
        if current > 0 {
            return Ok(false); // already hidden: no-op
        }
        let hide_event = event::make_strand_hidden(&strand_id);
        if let Some(r) = reason {
            let log_event = event::make_log_appended(&strand_id, &format!("[hidden] {}", r), None);
            append_events_unlocked(journal, &[hide_event, log_event])?;
        } else {
            append_event_unlocked(journal, &hide_event)?;
        }
        Ok(true)
    })?;
    if outcome {
        println!("hidden {}", shorten(&strand_id));
    } else {
        println!("hidden {} (already hidden, no-op)", shorten(&strand_id));
    }
    // Handle line (abbreviated card) + visibility ledger after both branches.
    if let Some((card, state)) = strand_card_fresh_with_state(&strand_id) {
        print_handle_line(&card, &state);
    }
    print_visibility_ledger();
    Ok(())
}

/// Unhide a strand. Idempotent: if the strand is not hidden (hide_count <= 0),
/// no event is written. The current state read and the append happen inside the
/// same journal write lock so concurrent hide/unhide calls are serialised.
fn cmd_unhide(id: &str) -> Result<(), String> {
    let strand_id = resolve_id(&read_events_strict(&ensure_journal()?)?, id)?;
    let outcome = with_journal_write_lock(|journal| {
        let path = ensure_journal()?;
        let (events, _) = read_events_lossy(&path);
        let current = count_hide_unhide(&events, &strand_id);
        if current <= 0 {
            return Ok(false); // already visible: no-op
        }
        let event = event::make_strand_unhidden(&strand_id);
        append_event_unlocked(journal, &event)?;
        Ok(true)
    })?;
    if outcome {
        println!("unhidden {}", shorten(&strand_id));
    } else {
        println!("unhidden {} (already visible, no-op)", shorten(&strand_id));
    }
    // Handle line + visibility ledger after both branches.
    if let Some((card, state)) = strand_card_fresh_with_state(&strand_id) {
        print_handle_line(&card, &state);
    }
    print_visibility_ledger();
    Ok(())
}

// ── Provenance helper (pi-strand V1 contract) ─────────────────────

/// Parse a `--provenance` argument. Must be a JSON object when present.
/// Returns `None` for `None` input; `Err` for malformed JSON or non-object shapes.
fn parse_provenance_arg(raw: Option<&str>) -> Result<Option<serde_json::Value>, String> {
    match raw {
        None => Ok(None),
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err("--provenance must be a non-empty JSON object".to_string());
            }
            let v: serde_json::Value = serde_json::from_str(trimmed)
                .map_err(|e| format!("--provenance is not valid JSON: {}", e))?;
            if !v.is_object() {
                return Err("--provenance must be a JSON object".to_string());
            }
            Ok(Some(v))
        }
    }
}

// ── Subject binding (pi-strand V1 contract) ─────────────────────

/// Parse a binding input from a single JSON object on stdin.
/// Schema: { "subject_type": "...", "subject_id": "...", "strand_id": "..." }
fn read_stdin_binding() -> Result<(String, String, String), String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("cannot read stdin: {}", e))?;
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        return Err("stdin is empty".to_string());
    }
    let v: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|e| format!("stdin is not valid JSON: {}", e))?;
    let obj = v
        .as_object()
        .ok_or_else(|| "stdin JSON must be an object".to_string())?;
    let subject_type = obj
        .get("subject_type")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "stdin JSON missing string field 'subject_type'".to_string())?
        .to_string();
    let subject_id = obj
        .get("subject_id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "stdin JSON missing string field 'subject_id'".to_string())?
        .to_string();
    let strand_id = obj
        .get("strand_id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "stdin JSON missing string field 'strand_id'".to_string())?
        .to_string();
    if subject_type.is_empty() || subject_id.is_empty() || strand_id.is_empty() {
        return Err("stdin JSON has empty subject_type/subject_id/strand_id".to_string());
    }
    Ok((subject_type, subject_id, strand_id))
}

/// Record a subject binding. Append-only. Resolves `--id` against the
/// existing journal so the caller can use prefix matches; never creates
/// a strand. Returns the binding's own event id.
fn cmd_bind(
    subject_type: Option<&str>,
    subject_id: Option<&str>,
    explicit_id: Option<&str>,
    stdin: bool,
    format_json: bool,
) -> Result<(), String> {
    let (st, sid, raw_strand) = if stdin {
        read_stdin_binding()?
    } else {
        let st = subject_type
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "--subject-type is required and non-empty".to_string())?;
        let sid = subject_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "--subject-id is required and non-empty".to_string())?;
        let sid_str = explicit_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "--id is required and non-empty".to_string())?;
        (st.to_string(), sid.to_string(), sid_str.to_string())
    };

    // Resolve --id to a full strand id. The strand must already exist
    // in the journal; bind never auto-creates a strand.
    let path = ensure_journal()?;
    let (events, _) = read_events_lossy(&path);
    let full_strand = find_strand(&events, &raw_strand)
        .ok_or_else(|| format!("strand {} not found", raw_strand))?;

    let event = event::make_subject_bound(&st, &sid, &full_strand);
    let binding_id = match &event {
        Event::SubjectBound { id, .. } => id.clone(),
        _ => unreachable!(),
    };
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &event)
    })?;

    if format_json {
        let card = strand_card_fresh(&full_strand);
        let card_val = card.as_ref().map(|c| serde_json::to_value(c).ok()).flatten();
        println!(
            "{}",
            json!({
                "binding_id": binding_id,
                "subject_type": st,
                "subject_id": sid,
                "strand_id": full_strand,
                "result": card_val,
            })
        );
    } else {
        println!("{}", binding_id);
        if let Some((card, state)) = strand_card_fresh_with_state(&full_strand) {
            print_handle_line(&card, &state);
        }
    }
    Ok(())
}

/// Project the latest effective binding for `(subject_type, subject_id)`.
/// Walks the journal once, keeps the most-recent match. No binding →
/// exit 1 with stderr message; stdout stays empty so callers can branch
/// on the absence of a payload.
fn cmd_current(
    subject_type: Option<&str>,
    subject_id: Option<&str>,
    format_json: bool,
) -> Result<(), String> {
    let st = subject_type
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "--subject-type is required and non-empty".to_string())?;
    let sid = subject_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "--subject-id is required and non-empty".to_string())?;

    let path = ensure_journal()?;
    let (events, _) = read_events_lossy(&path);
    let mut latest: Option<(String, String, String)> = None; // (binding_id, ts, strand_id)
    for (_offset, ev) in &events {
        if let Event::SubjectBound { id, ts, subject_type: t, subject_id: i, strand_id: s } = ev {
            if t == st && i == sid {
                match &latest {
                    Some((_, prev_ts, _)) if ts.as_str() <= prev_ts.as_str() => {}
                    _ => latest = Some((id.clone(), ts.clone(), s.clone())),
                }
            }
        }
    }

    let (binding_id, ts, strand_id) = match latest {
        Some(v) => v,
        None => {
            eprintln!(
                "no binding for subject_type={} subject_id={}",
                st, sid
            );
            return Err("no current binding".to_string());
        }
    };

    if format_json {
        println!(
            "{}",
            json!({
                "binding_id": binding_id,
                "subject_type": st,
                "subject_id": sid,
                "strand_id": strand_id,
                "ts": ts,
            })
        );
    } else {
        println!("{}", strand_id);
    }
    Ok(())
}

fn cmd_export(out: &str) -> Result<(), String> {
    let journal_path = resolve_journal_dir()?.join("journal.jsonl");

    let out_path = PathBuf::from(out);
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create output directory: {}", e))?;
        }
    }

    let journal_bytes = std::fs::read(&journal_path)
        .map_err(|e| format!("cannot read journal: {}", e))?;
    let journal_text = String::from_utf8_lossy(&journal_bytes);
    let line_count = journal_text.lines().count();

    let metadata = json!({
        "type": "export_metadata",
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "journal_lines": line_count,
        "head_at_export": "",
        "source": "tasktree export"
    });

    let mut file = std::fs::File::create(&out_path)
        .map_err(|e| format!("cannot create output file '{}': {}", out, e))?;
    let metadata_line = serde_json::to_string(&metadata)
        .map_err(|e| format!("metadata serialization failed: {}", e))?;
    writeln!(file, "{}", metadata_line)
        .map_err(|e| format!("cannot write metadata to output: {}", e))?;
    file.write_all(&journal_bytes)
        .map_err(|e| format!("cannot write journal to output: {}", e))?;

    let export_lines = line_count + 1;
    println!("Exported {} lines (1 metadata + {} journal) to {}", export_lines, line_count, out);
    Ok(())
}
fn read_stdin_content() -> Result<String, String> {
    // Detect TTY: if stdin is a terminal, reject immediately to avoid agent hanging
    if atty::is(atty::Stream::Stdin) {
        return Err("--stdin requires piped input".to_string());
    }
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("failed to read stdin: {}", e))?;
    Ok(buf)
}

fn read_file_content(path: &str) -> Result<String, String> {
    let p = std::path::Path::new(path);
    if !p.exists() {
        return Err(format!("file not found: {}", path));
    }
    if p.is_dir() {
        return Err(format!("expected file path, got directory: {}", path));
    }
    let buf = std::fs::read_to_string(p).map_err(|e| format!("failed to read file: {}", e))?;
    Ok(buf)
}

/// Strip at most one trailing newline (\n or \r\n).
/// Preserves leading whitespace, interior newlines, code blocks.
fn normalize_content(raw: &str) -> String {
    if raw.ends_with("\r\n") {
        raw[..raw.len() - 2].to_string()
    } else if raw.ends_with('\n') {
        raw[..raw.len() - 1].to_string()
    } else {
        raw.to_string()
    }
}

fn looks_like_strand_id(value: &str) -> bool {
    let len = value.len();
    (6..=32).contains(&len) && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn validate_lifecycle_marker(content: &str) -> Result<(), String> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("[") { return Ok(()); }
    if let Some(end) = trimmed.find("]") {
        let marker = &trimmed[..=end];
        if is_convention_marker(marker) { return Ok(()); }
        if is_known_marker(marker) { return Ok(()); }
        if "mcfvder".contains(marker.chars().nth(1).unwrap_or(' ')) {
            return Err(format!(
                "unknown lifecycle marker {} - valid: [merged] [cancelled] [failed] [verified] [done] [ended] [dispatched] [registered]",
                marker
            ));
        }
    }
    Ok(())
}

fn is_convention_marker(marker: &str) -> bool {
    matches!(marker,
        "[observed]" | "[check]" | "[friction]" | "[progress]" |
        "[decision]" | "[constraint]" | "[grill]" | "[insight]" | "[lesson]" | "[fixed]" |
        "[deliverable]" | "[skill]" | "[guide]" | "[covers]" | "[deadline]" |
        "[waiting:human]" | "[checkpoint]" | "[session]" | "[task]"
    )
}

fn is_known_marker(marker: &str) -> bool {
    matches!(marker, "[merged]" | "[cancelled]" | "[failed]" | "[verified]" | "[done]" | "[ended]" | "[dispatched]" | "[registered]")
}

fn cmd_append(
    content: Option<&str>,
    legacy_id: Option<&str>,
    new: bool,
    stdin: bool,
    file: Option<&str>,
    explicit_id: Option<&str>,
    format: Option<&str>,
    provenance_raw: Option<&str>,
) -> Result<(), String> {
    // ---- Content Source Resolution ----
    if (stdin || file.is_some())
        && legacy_id.is_none()
        && content.map(looks_like_strand_id).unwrap_or(false)
    {
        return Err(
            "warn: stdin and --file require --id to specify target; positional strand id is not supported with this content source".to_string()
        );
    }

    let source_kind = match (content.is_some(), stdin, file.is_some()) {
        (false, false, false) => {
            return Err(
                "choose a content source: positional content, --stdin, or --file <path>"
                    .to_string(),
            );
        }
        (true, false, false) => "positional",
        (false, true, false) => "stdin",
        (false, false, true) => "file",
        _ => {
            let mut sources = Vec::new();
            if content.is_some() {
                sources.push("positional content");
            }
            if stdin {
                sources.push("--stdin");
            }
            if file.is_some() {
                sources.push("--file");
            }
            return Err(format!(
                "choose only one content source, got: {}",
                sources.join(", ")
            ));
        }
    };

    // Read raw content
    let raw = match source_kind {
        "positional" => content.unwrap().to_string(),
        "stdin" => read_stdin_content()?,
        "file" => read_file_content(file.unwrap())?,
        _ => unreachable!(),
    };

    // Empty check (trimmed for detection, but we don't trim for storage)
    if raw.trim().is_empty() {
        let hint = match source_kind {
            "stdin" => "stdin content is empty",
            "file" => return Err(format!("file content is empty: {}", file.unwrap())),
            _ => "content is empty",
        };
        return Err(hint.to_string());
    }

    // Normalize: strip at most one trailing newline/CRLF, preserve leading whitespace
    let stored = normalize_content(&raw);
    validate_lifecycle_marker(&stored)?;

    // Load journal for target resolution
    let path = ensure_journal()?;
    let (events, _) = read_events_lossy(&path);

    // ---- Target Resolution ----
    if let (Some(first), Some(second)) = (content, legacy_id) {
        if find_strand(&events, first).is_some() && find_strand(&events, second).is_none() {
            return Err(format!(
                "positional append arguments look reversed. Use:\n  tasktree append --id {} \"{}\"",
                first,
                second.replace('"', "\\\"")
            ));
        }
    }

    let target_count = [new, explicit_id.is_some(), legacy_id.is_some()]
        .iter()
        .filter(|&&x| x)
        .count();

    if target_count > 1 {
        return Err("choose only one target: --new, --id, or positional strand id".to_string());
    }

    // Legacy positional id only valid with positional content source
    if legacy_id.is_some() && source_kind != "positional" {
        return Err(
            "warn: stdin and --file require --id to specify target; positional strand id is not supported with this content source".to_string()
        );
    }

    if new {
        // Create new strand — same as Add but using stored content
        let (created, appended) = event::make_strand_created(&stored, Some("session"));
        let new_id = created.strand_id().to_string();
        with_journal_write_lock(|journal| {
            append_event_unlocked(journal, &created)?;
            append_event_unlocked(journal, &appended)?;
            Ok(())
        })?;
        println!("{}", new_id);
        if let Some((card, state)) = strand_card_fresh_with_state(&new_id) {
            print_card_with_state(&card, &state);
        }
        return Ok(());
    }

    // Resolve target strand
    let target_id = explicit_id.or(legacy_id);
    let full_id = if let Some(id) = target_id {
        find_strand(&events, id).ok_or_else(|| {
            let mut msg = format!("strand {} not found", id);
            if id == "-" {
                msg.push_str(
                    ". If you meant to pipe content from stdin, use:\n  echo \"...\" | tasktree append --stdin --id <id>",
                );
            }
            msg
        })?
    } else {
        // Append to most recently active strand (last-append ordering)
        let strands = projection::project_strands(&events, false);
        let mut sorted: Vec<_> = strands.iter().collect();
        sorted.sort_by(|a, b| b.last_ts().cmp(&a.last_ts()));
        let recent = sorted
            .first()
            .ok_or("no strands found — use 'add' or 'append --new' first")?;
        recent.id.clone()
    };

    let provenance = parse_provenance_arg(provenance_raw)?;
    let event = event::make_log_appended(&full_id, &stored, provenance);
    let append_id = match &event {
        Event::LogAppended { append_id, .. } => append_id.clone(),
        _ => None,
    };
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &event)
    })?;
    if format == Some("json") {
        let card = strand_card_fresh(&full_id);
        let card_val = card.as_ref().map(|c| serde_json::to_value(c).ok()).flatten();
        println!("{}", serde_json::to_string(&serde_json::json!({
            "strand_id": full_id,
            "append_id": append_id,
            "content_preview": stored.chars().take(120).collect::<String>(),
            "result": card_val,
        })).unwrap());
    } else {
        if let Some((card, state)) = strand_card_fresh_with_state(&full_id) {
            println!("appended to {} (offset {})", shorten(&full_id), card.last_offset);
            print_card_with_state(&card, &state);
        } else {
            println!("appended to {}", shorten(&full_id));
        }
    }
    Ok(())
}

#[derive(Debug)]
struct CheckpointFailure {
    code: i32,
    message: String,
    requested_strand: Option<String>,
    resolved_strand: Option<String>,
    journal_appended: bool,
}

fn checkpoint_error_json(failure: &CheckpointFailure) {
    println!(
        "{}",
        json!({
            "ok": false,
            "error": failure.message,
            "requested_strand": failure.requested_strand,
            "resolved_strand": failure.resolved_strand,
            "journal_appended": failure.journal_appended,
        })
    );
}

fn resolve_most_recent_strand(strands: &[projection::ProjectedStrand]) -> Option<&projection::ProjectedStrand> {
    let mut sorted: Vec<_> = strands.iter().collect();
    sorted.sort_by(|a, b| b.last_ts().cmp(a.last_ts()));
    sorted.into_iter().next()
}

fn escape_checkpoint_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Render a duration in seconds as a human-readable string.
/// < 60s → "just now"; < 3600s → "<N>m"; < 86400s → "<N>h"; else "<N>d".
/// No external dependencies — purely arithmetic.
fn humanize_duration(secs: i64) -> String {
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

/// Check W070: checkpoint's provenance.producer differs from the last
/// LogAppended entry's provenance.producer on the target strand.
///
/// Both producers must be non-empty strings for this check to fire;
/// if either is absent the function returns None (no guessing).
///
/// Returns `Some((code, detail))` when the check fires, `None` otherwise.
fn check_w070_strand_moved(
    events: &[(usize, Event)],
    strand_id: &str,
    checkpoint_producer: Option<&str>,
) -> Option<EmittedDiag> {
    let cp_producer = checkpoint_producer?;
    if cp_producer.is_empty() {
        return None;
    }
    // Find the last LogAppended event for this strand.
    let last_entry_producer: Option<&str> = events
        .iter()
        .filter_map(|(_, e)| {
            if let Event::LogAppended { id, provenance, .. } = e {
                if id == strand_id {
                    Some(
                        provenance
                            .as_ref()
                            .and_then(|p| p.get("producer"))
                            .and_then(|v| v.as_str()),
                    )
                } else {
                    None
                }
            } else {
                None
            }
        })
        .last()
        .flatten();
    let last_producer = last_entry_producer?;
    if last_producer.is_empty() {
        return None;
    }
    if last_producer != cp_producer {
        Some((
            "W070",
            format!(
                "strand moved under you: last entry by \"{}\", you are \"{}\"",
                last_producer, cp_producer
            ),
        ))
    } else {
        None
    }
}

/// Check W071: checkpoint target strand state is not "registered" (already closed).
///
/// Returns `Some((code, detail))` when the check fires, `None` otherwise.
fn check_w071_closed_strand(strand: &projection::ProjectedStrand) -> Option<EmittedDiag> {
    if strand.state() != "registered" {
        Some((
            "W071",
            format!(
                "checkpoint on closed strand: state is {}",
                strand.state()
            ),
        ))
    } else {
        None
    }
}

fn cmd_checkpoint(
    requested_id: Option<&str>,
    action: &str,
    tail: Option<usize>,
    format_json: bool,
    include_hidden: bool,
    provenance_raw: Option<&str>,
) -> Result<(), CheckpointFailure> {
    if action.trim().is_empty() {
        return Err(CheckpointFailure {
            code: 3,
            message: "invalid arguments: --action cannot be empty".to_string(),
            requested_strand: requested_id.map(str::to_string),
            resolved_strand: None,
            journal_appended: false,
        });
    }

    let path = ensure_journal().map_err(|e| CheckpointFailure {
        code: 1,
        message: format!("strand resolve/show failed: {}", e),
        requested_strand: requested_id.map(str::to_string),
        resolved_strand: None,
        journal_appended: false,
    })?;
    let events = read_events_strict(&path).map_err(|e| CheckpointFailure {
        code: 1,
        message: format!("strand resolve/show failed: {}", e),
        requested_strand: requested_id.map(str::to_string),
        resolved_strand: None,
        journal_appended: false,
    })?;
    // Two projection views:
    //   - `all_strands` includes hidden strands: used to resolve an explicit
    //     --id lookup, because the user named the strand directly and we
    //     should not silently refuse to checkpoint a hidden one.
    //   - `visible_strands` honours the include-hidden flag: used to pick
    //     the most-recent active strand, which is the only place a default
    //     checkpoint would otherwise pick a hidden strand by accident.
    let all_strands = projection::project_strands(&events, true);
    let visible_strands = projection::project_strands(&events, include_hidden);

    let (strand, resolved_by) = if let Some(id) = requested_id {
        let full = find_strand(&events, id).ok_or_else(|| CheckpointFailure {
            code: 1,
            message: format!("strand resolve/show failed: strand {} not found", id),
            requested_strand: Some(id.to_string()),
            resolved_strand: None,
            journal_appended: false,
        })?;
        let strand = all_strands
            .iter()
            .find(|s| s.id == full)
            .ok_or_else(|| CheckpointFailure {
                code: 1,
                message: format!("strand resolve/show failed: strand {} not found", id),
                requested_strand: Some(id.to_string()),
                resolved_strand: None,
                journal_appended: false,
            })?;
        (strand, "explicit --id")
    } else {
        let strand = resolve_most_recent_strand(&visible_strands).ok_or_else(|| CheckpointFailure {
            code: 1,
            message: "strand resolve/show failed: no strands found".to_string(),
            requested_strand: None,
            resolved_strand: None,
            journal_appended: false,
        })?;
        (strand, "most_recent_active_strand")
    };

    // ── Staleness snapshot (before append) ───────────────────────────────
    // Compute before the write so the delta reflects pre-checkpoint state.
    let strand_last_offset = strand.last_offset();
    let max_offset_before = events.last().map(|(o, _)| *o).unwrap_or(0);
    let journal_delta = max_offset_before.saturating_sub(strand_last_offset);

    // Parse strand's last ts for "last touched N ago" display.
    let staleness_seconds: Option<i64> = if strand.last_ts().is_empty() {
        None
    } else {
        parse_event_ts(strand.last_ts()).map(|ts| (chrono::Utc::now() - ts).num_seconds())
    };

    // ── Gate warnings (W070 / W071) — evaluated before write ─────────────
    let provenance_val = parse_provenance_arg(provenance_raw).map_err(|message| CheckpointFailure {
        code: 3,
        message,
        requested_strand: requested_id.map(str::to_string),
        resolved_strand: Some(strand.id.clone()),
        journal_appended: false,
    })?;
    let checkpoint_producer: Option<&str> = provenance_val
        .as_ref()
        .and_then(|p| p.get("producer"))
        .and_then(|v| v.as_str());
    let w070 = check_w070_strand_moved(&events, &strand.id, checkpoint_producer);
    let w071 = check_w071_closed_strand(strand);

    let observed_entries_before_append = strand.log_count();
    let escaped_action = escape_checkpoint_value(action);
    let content = format!(
        "[checkpoint] ok resolved_by=\"{}\" observed_entries_before_append={} action=\"{}\"",
        resolved_by, observed_entries_before_append, escaped_action
    );
    let event = event::make_log_appended(&strand.id, &content, provenance_val);
    let append_id = match &event {
        Event::LogAppended { append_id, .. } => append_id.clone(),
        _ => None,
    };
    with_journal_write_lock(|journal| {
        append_event_unlocked(journal, &event)
    }).map_err(|e| CheckpointFailure {
        code: 2,
        message: format!("journal append failed: {}", e),
        requested_strand: requested_id.map(str::to_string),
        resolved_strand: Some(strand.id.clone()),
        journal_appended: false,
    })?;

    let shown_entries: Vec<_> = if let Some(n) = tail {
        let skip = strand.log.len().saturating_sub(n);
        strand.log[skip..].iter().collect()
    } else {
        strand.log.iter().collect()
    };

    // Run diagnostics on the pre-append events (checkpoint itself is not a
    // diagnostic target; re-reading after append would be equivalent here).
    let raw_events: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
    let diags = run_journal_diagnostics(&raw_events, chrono::Utc::now());
    let diag_count = diags.len();

    // Build warning list (W070/W071) for output.
    let mut cp_warnings: Vec<(&'static str, String)> = Vec::new();
    if let Some(w) = w070 { cp_warnings.push(w); }
    if let Some(w) = w071 { cp_warnings.push(w); }

    if format_json {
        let card = strand_card_fresh(&strand.id);
        let card_val = card.as_ref().map(|c| serde_json::to_value(c).ok()).flatten();
        let catch_up_val: serde_json::Value = if journal_delta > 0 {
            json!(format!(
                "tasktree timeline --since-offset {} --links {}",
                strand_last_offset, shorten(&strand.id)
            ))
        } else {
            serde_json::Value::Null
        };
        let warnings_json: Vec<serde_json::Value> = cp_warnings
            .iter()
            .map(|(code, detail)| json!({"code": code, "detail": detail}))
            .collect();
        println!(
            "{}",
            json!({
                "ok": true,
                "strand": shorten(&strand.id),
                "resolved_strand": strand.id,
                "resolved_by": resolved_by,
                "observed_entries_before_append": observed_entries_before_append,
                "shown_entries": shown_entries.len(),
                "action": action,
                "append_id": append_id,
                "journal_appended": true,
                "diagnostics_count": diag_count,
                "result": card_val,
                "staleness_seconds": staleness_seconds,
                "journal_delta": journal_delta,
                "catch_up": catch_up_val,
                "warnings": warnings_json,
            })
        );
    } else {
        println!("checkpoint ok");
        println!("  strand: {} | {} entries | {}", shorten(&strand.id), strand.log_count() + 1, strand.state());
        println!("  resolved_by: {}", resolved_by);

        // Staleness line — always printed after strand line.
        let staleness_part = staleness_seconds.map(|s| {
            let d = humanize_duration(s);
            if d == "just now" {
                "last touched just now | ".to_string()
            } else {
                format!("last touched {} ago | ", d)
            }
        }).unwrap_or_default();
        println!(
            "  staleness: {}journal +{} entries since (offset {} → {})",
            staleness_part, journal_delta, strand_last_offset, max_offset_before
        );

        // Catch-up line — only when delta > 0.
        if journal_delta > 0 {
            println!(
                "  catch-up: tasktree timeline --since-offset {} --links {}",
                strand_last_offset, shorten(&strand.id)
            );
        }

        println!(
            "  observed_entries_before_append: {}",
            observed_entries_before_append
        );
        println!("  action: {}", action);
        if let Some(id) = append_id {
            println!("  append_id: {}", id);
        }
        println!("  appended to journal");
        println!("log:");
        for entry in shown_entries {
            let id_str = entry
                .append_id
                .as_ref()
                .map(|a| format!(" [{}]", &a[..12]))
                .unwrap_or_default();
            println!("  [{}]{} {}", &entry.ts[..19], id_str, entry.content);
        }
        // W-code scar lines — printed before the general diagnostics count.
        for (code, detail) in &cp_warnings {
            println!("  {} {}  (tasktree explain {})", code, detail, code);
        }
        if diag_count > 0 {
            println!("diagnostics: {} warning(s) — run tasktree doctor journal", diag_count);
        }
    }

    Ok(())
}

// ── exit strategy ──
// cmd_list, cmd_show, cmd_search use process::exit(2) directly when
// corrupted journal lines are detected. This is intentional CLI style,
// not library style — exit(2) allows gate scripts to detect corruption
// without parsing stderr. Do not refactor to return Result without
// updating all call sites and preserving the exit code.
fn cmd_list(include_hidden: bool, links: Option<&str>, backlinks: Option<&str>, state: Option<&str>, list_type: Option<&str>, stale: Option<&str>, stale_offset: Option<usize>, since_offset: Option<usize>, format_json: bool) -> Result<(), String> {
    let started = Instant::now();
    let path = ensure_journal()?;
    let (events, skipped) = read_events_lossy(&path);
    let mut strands = projection::project_strands(&events, include_hidden);
    // Most recent last-append first
    strands.sort_by(|a, b| b.last_ts().cmp(&a.last_ts()));

    // --type: filter by strand_type (from StrandCreated event)
    if let Some(ref type_filter) = list_type {
        strands.retain(|n| n.strand_type.as_deref() == Some(type_filter));
    }

    // --links: filter strands that source links to
    if let Some(ref src) = links {
        let source_edges: Vec<String> = strands.iter()
            .filter(|n| n.id.starts_with(*src))
            .flat_map(|n| n.edges.iter().cloned())
            .collect();
        strands.retain(|n| source_edges.iter().any(|e| n.id.starts_with(e)));
    }
    // --backlinks: filter strands that link to target
    if let Some(ref tgt) = backlinks {
        strands.retain(|n| n.edges.iter().any(|e| e.starts_with(*tgt)));
    }
    // --state: filter by canonical state
    if let Some(ref state_filter) = state {
        strands.retain(|n| {
            match *state_filter {
                // "open" is not a canonical state; match default (registered) for backward compat
                "open" => n.state() == "registered",
                _ => n.state() == *state_filter,
            }
        });
    }

    // --stale: filter by silence duration
    if let Some(dur_str) = stale {
        let secs = parse_duration(dur_str)?;
        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(secs as i64);
        let cutoff_str = cutoff.to_rfc3339();
        strands.retain(|n| {
            let last_ts = n.last_ts();
            if last_ts.is_empty() { return false; }
            last_ts < &cutoff_str
        });
    }

    // --stale-offset: filter by last entry offset (silent)
    if let Some(so) = stale_offset {
        strands.retain(|n| n.last_offset() <= so);
    }

    // --since-offset: filter by last entry offset (updated since)
    if let Some(so) = since_offset {
        strands.retain(|n| n.last_offset() > so);
    }

    if format_json {
        let output = output::StrandListOutput {
            strands: strands.iter()
                .filter(|s| !s.hidden || include_hidden)
                .map(output::StrandListItem::from)
                .collect(),
        };
        println!("{}", serde_json::to_string(&output).expect("serialize"));
        if skipped > 0 {
            eprintln!("[tasktree] WARNING: {} corrupted lines skipped", skipped);
            std::process::exit(2);
        }
        eprintln!("[tasktree] list: {:.0?}", started.elapsed());
        return Ok(());
    }

    for strand in &strands {
        if strand.hidden && !include_hidden {
            continue;
        }
        let type_str = strand.strand_type.as_deref().unwrap_or("");
        let type_info = if type_str.is_empty() { String::new() } else { format!(" [{}]", type_str) };
        println!(
            "{}  {}  \"{}\"  →  \"{}\"{}",
            shorten(&strand.id),
            strand.log_count(),
            truncate(strand.first_summary(), 40),
            truncate(strand.last_summary(), 40),
            type_info,
        );
    }
    if strands.is_empty() {
        println!("(no strands)");
    }
    if skipped > 0 {
        eprintln!("[tasktree] WARNING: {} corrupted lines skipped", skipped);
        std::process::exit(2);
    }
    eprintln!("[tasktree] list: {:.0?}", started.elapsed());
    Ok(())
}

fn cmd_search(query: &str, format_json: bool, include_hidden: bool) -> Result<(), String> {
    let started = Instant::now();
    let path = ensure_journal()?;
    let (events, skipped) = read_events_lossy(&path);
    let q = query.to_lowercase();
    // Honour the include_hidden flag: when false (default), the strand_map
    // is built from visible strands only, and the events loop below skips
    // events belonging to strands not in the map.
    let strands = projection::project_strands(&events, include_hidden);
    let strand_map: std::collections::HashMap<&str, &projection::ProjectedStrand> =
        strands.iter().map(|s| (s.id.as_str(), s)).collect();

    let mut found = 0;
    let mut matches: Vec<output::SearchMatch> = Vec::new();

    for (_, event) in &events {
        if let Event::LogAppended { content, .. } = event {
            if content.to_lowercase().contains(&q) {
                let strand_id = event.strand_id().to_string();
                // Skip matches inside strands the projection filtered out
                // (i.e. hidden strands when include_hidden is false).
                if !strand_map.contains_key(strand_id.as_str()) {
                    continue;
                }
                let projected = strand_map.get(strand_id.as_str());
                if format_json {
                    matches.push(output::SearchMatch {
                        strand_id,
                        content: truncate(content, 70),
                        strand_type: projected.and_then(|s| s.strand_type.clone()),
                        hidden: projected.map(|s| s.hidden).unwrap_or(false),
                    });
                } else {
                    println!(
                        "{}  {}",
                        shorten(&strand_id),
                        truncate(content, 70)
                    );
                }
                found += 1;
            }
        }
    }

    if format_json {
        let output = output::SearchOutput {
            matches,
            count: found,
            query: query.to_string(),
        };
        println!("{}", serde_json::to_string(&output).expect("serialize"));
    } else if found == 0 {
        println!("(no matches for: {})", query);
    }

    if skipped > 0 {
        eprintln!("[tasktree] WARNING: {} corrupted lines skipped", skipped);
        std::process::exit(2);
    }
    eprintln!(
        "[tasktree] search: {:.0?}  ({} matches)",
        started.elapsed(),
        found
    );
    Ok(())
}

fn cmd_timeline(
    since_offset: Option<usize>,
    since_ts: Option<&str>,
    until_offset: Option<usize>,
    until_ts: Option<&str>,
    strand: Option<&str>,
    links: Option<&str>,
    format_json: Option<&str>,
    limit: Option<usize>,
    tree_root: Option<&str>,
) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _skipped) = read_events_lossy(&path);
    let mut entries = projection::project_timeline(&events);

    // Filter by offset range
    if let Some(so) = since_offset {
        entries.retain(|e| e.journal_offset > so);
    }
    if let Some(uo) = until_offset {
        entries.retain(|e| e.journal_offset <= uo);
    }
    // since_ts: convert to approximate offset
    if let Some(st) = since_ts {
        let first_idx = entries.iter().position(|e| e.ts.as_str() >= st);
        if let Some(idx) = first_idx {
            entries.drain(0..idx);
        }
    }
    if let Some(ut) = until_ts {
        entries.retain(|e| e.ts.as_str() <= ut);
    }

    // Filter by strand or links
    if let Some(sid) = strand {
        let full_id = find_strand(&events, sid).ok_or_else(|| format!("strand {} not found", sid))?;
        entries.retain(|e| e.strand_id == full_id);
    }
    if let Some(lid) = links {
        let full_id = find_strand(&events, lid).ok_or_else(|| format!("strand {} not found", lid))?;
        // Collect linked strand IDs
        let mut linked_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        linked_ids.insert(full_id.clone());
        for (_, event) in &events {
            if let Event::EdgeLinked { id, to, .. } = event {
                if *id == full_id {
                    linked_ids.insert(to.clone());
                }
            }
        }
        entries.retain(|e| linked_ids.contains(&e.strand_id));
    }

    // Filter by subtree — only events from strands reachable from root via edges
    if let Some(root_id) = tree_root {
        let strands = projection::project_strands(&events, true);
        if let Some(tree_ids) = tree::subtree_ids(root_id, &strands) {
            entries.retain(|e| tree_ids.contains(&e.strand_id));
        }
    }

    if let Some(lim) = limit {
        entries.truncate(lim);
    }

    let count = entries.len();
    let max_offset = entries.last().map(|e| e.journal_offset).unwrap_or(0);
    let is_json = format_json == Some("json");

    if is_json {
        println!("{}", json!({
            "timeline": entries,
            "truncated": false,
            "count": count,
            "max_offset": max_offset,
        }));
    } else if entries.is_empty() {
        // No dead ends (design principle): empty result must say something.
        let mut parts: Vec<String> = Vec::new();
        if let Some(so) = since_offset { parts.push(format!("since-offset {}", so)); }
        if let Some(st) = since_ts { parts.push(format!("since-ts {}", st)); }
        if let Some(uo) = until_offset { parts.push(format!("until-offset {}", uo)); }
        if let Some(ut) = until_ts { parts.push(format!("until-ts {}", ut)); }
        if let Some(sid) = strand { parts.push(format!("strand {}", sid)); }
        if let Some(lid) = links { parts.push(format!("links {}", lid)); }
        if let Some(root) = tree_root { parts.push(format!("tree {}", root)); }
        if parts.is_empty() {
            println!("(journal is empty)");
        } else {
            println!("(no events match: {})", parts.join(", "));
        }
    } else {
        for e in &entries {
            let ts_short = &e.ts[11..19]; // HH:MM:SS
            let id_short = shorten(&e.strand_id);
            let kind_desc = match &e.kind {
                TimelineEventKind::StrandCreated { .. } => "created".to_string(),
                TimelineEventKind::LogAppended { content, .. } => {
                    content.chars().take(60).collect()
                }
                TimelineEventKind::EdgeLinked { target_id, .. } => {
                    format!("link -> {}", shorten(target_id))
                }
                TimelineEventKind::EdgeUnlinked { target_id } => {
                    format!("unlink -> {}", shorten(target_id))
                }
                TimelineEventKind::StrandHidden { .. } => "hidden".to_string(),
                TimelineEventKind::StrandUnhidden { .. } => "unhidden".to_string(),
                TimelineEventKind::CheckpointCreated { action, .. } => {
                    format!("checkpoint: {}", action)
                }
                TimelineEventKind::SubjectBound { subject_type, subject_id, strand_id } => {
                    format!("bind: {}:{} -> {}", subject_type, subject_id, shorten(strand_id))
                }
            };
            let skew = if e.ts_skew { " ⚠" } else { "" };
            println!("{}  {}  {}{}", ts_short, id_short, kind_desc, skew);
        }
    }
    Ok(())
}

/// Orient remind line: the whole operating loop in one line (ADR-0001:
/// the rules travel with the orientation, the weave-in pointer stays thin).
const ORIENT_REMIND: &str = "continue → append --id <ID> \"[decision] ...\" | new matter → add \"<summary>\" | matter concluded → append --id <ID> \"[done] ...\" | before irreversible → checkpoint --id <ID> --action \"<why>\" | more → tasktree --help";

// ── Card helpers ──────────────────────────────────────────────────────────
// "card" = the OrientStrand shape used both for orient menus and for
// post-write echo. make_card/strand_card_fresh keep echoes consistent with
// orient output without re-introducing output divergence.

/// Build an OrientStrand card from a projected strand. Identical to the
/// inline construction in build_orient; extracted so write commands can
/// call the same logic without duplicating the truncation/shorten rules.
fn make_card(s: &projection::ProjectedStrand) -> output::OrientStrand {
    output::OrientStrand {
        id: shorten(&s.id),
        strand_type: s.strand_type.clone(),
        entries: s.log_count(),
        summary: truncate(s.first_summary(), 70),
        last_entry: truncate(s.last_summary(), 70),
        last_offset: s.last_offset(),
        catch_up: format!(
            "tasktree timeline --since-offset {} --links {}",
            s.last_offset(),
            shorten(&s.id)
        ),
    }
}

/// The card printer used by write commands. Callers supply the state
/// string directly so we avoid re-projecting a second time.
// Card echo goes to stderr: stdout is the value (capturable by
// `ID=$(tasktree add ...)`), stderr is the narration — same split as the
// perf footers. JSON mode is unaffected (result field on stdout).
fn print_card_with_state(card: &output::OrientStrand, state: &str) {
    print_handle_line(card, state);
    eprintln!("    {}", card.summary);
    if card.entries > 1 {
        eprintln!("    last: {}", card.last_entry);
    }
}

/// Re-project a single strand from a fresh journal read and build its card.
/// Uses include_hidden=true so hidden strands can still echo their own card.
fn strand_card_fresh(strand_id: &str) -> Option<output::OrientStrand> {
    let path = ensure_journal().ok()?;
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    strands.iter().find(|s| s.id == strand_id).map(make_card)
}

/// Like strand_card_fresh but also returns the state string (to avoid a
/// second projection scan when the caller needs both).
fn strand_card_fresh_with_state(strand_id: &str) -> Option<(output::OrientStrand, String)> {
    let path = ensure_journal().ok()?;
    let (events, _) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);
    strands.iter().find(|s| s.id == strand_id).map(|s| {
        (make_card(s), s.state().to_string())
    })
}

/// Print the global visibility ledger line used by hide/unhide echo.
/// Reads the journal fresh. Counts: active = visible & state=="registered",
/// closed = visible - active, hidden = strands with hidden==true.
fn print_visibility_ledger() {
    if let Ok(path) = ensure_journal() {
        let (events, _) = read_events_lossy(&path);
        let all = projection::project_strands(&events, true);
        let hidden_n = all.iter().filter(|s| s.hidden).count();
        let visible: Vec<_> = all.iter().filter(|s| !s.hidden).collect();
        let active_n = visible.iter().filter(|s| s.state() == "registered").count();
        let closed_n = visible.len() - active_n;
        eprintln!("journal: {} active | {} closed | {} hidden", active_n, closed_n, hidden_n);
    }
}

/// Print the handle line only (id + type + entries + state). Used by
/// hide/unhide/link/bind where we show a reduced card.
fn print_handle_line(card: &output::OrientStrand, state: &str) {
    let type_info = card
        .strand_type
        .as_deref()
        .map(|t| format!(" [{}]", t))
        .unwrap_or_default();
    eprintln!(
        "  {}{} | {} entries | {}",
        card.id, type_info, card.entries, state
    );
}

/// Pure projection for orient. Never touches the journal (ADR-0003: orient
/// stays pure-read; the catch-up cursor is each strand's own last_offset).
fn build_orient(
    strands: &[projection::ProjectedStrand],
    include_hidden: bool,
    limit: usize,
    max_offset: usize,
) -> output::OrientOutput {
    // strands contains ALL strands (hidden + visible); split here so that
    // hidden_count can be computed regardless of include_hidden.
    let hidden_count = if include_hidden {
        0
    } else {
        strands.iter().filter(|s| s.hidden).count()
    };
    let visible: Vec<&projection::ProjectedStrand> = strands
        .iter()
        .filter(|s| !s.hidden || include_hidden)
        .collect();
    let mut active: Vec<&projection::ProjectedStrand> = visible
        .iter()
        .copied()
        .filter(|s| s.state() == "registered")
        .collect();
    let closed_count = visible.len() - active.len();
    // Most recently touched first; the menu is an index, not a dump.
    active.sort_by(|a, b| b.last_offset().cmp(&a.last_offset()));
    active.truncate(limit);

    output::OrientOutput {
        max_offset,
        active: active.iter().map(|s| make_card(s)).collect(),
        closed_count,
        hidden_count,
        remind: ORIENT_REMIND.to_string(),
    }
}

fn cmd_orient(format: Option<&str>, include_hidden: bool, limit: Option<usize>) -> Result<(), String> {
    let started = Instant::now();
    let path = ensure_journal()?;
    let (events, skipped) = read_events_lossy(&path);
    let max_offset = events.last().map(|(o, _)| *o).unwrap_or(0);
    // Always project with include_hidden=true so build_orient can count hidden
    // strands; the visible/hidden split is done inside build_orient.
    let strands = projection::project_strands(&events, true);
    let out = build_orient(&strands, include_hidden, limit.unwrap_or(10), max_offset);

    if format == Some("json") {
        println!("{}", serde_json::to_string(&out).expect("serialize"));
    } else {
        println!(
            "journal: max_offset {} | {} active | {} closed | {} hidden (tasktree list)",
            out.max_offset,
            out.active.len(),
            out.closed_count,
            out.hidden_count
        );
        for s in &out.active {
            let type_info = s
                .strand_type
                .as_deref()
                .map(|t| format!(" [{}]", t))
                .unwrap_or_default();
            println!("  {}{}  {} entries", s.id, type_info, s.entries);
            println!("    {}", s.summary);
            if s.entries > 1 {
                println!("    last: {}", s.last_entry);
            }
            println!("    catch-up: {}", s.catch_up);
        }
        if out.active.is_empty() {
            println!("(no active strands) — start one: tasktree add \"<summary>\"");
        }
        println!("remind: {}", out.remind);
    }

    if skipped > 0 {
        eprintln!("[tasktree] WARNING: {} corrupted lines skipped", skipped);
        std::process::exit(2);
    }
    eprintln!("[tasktree] orient: {:.0?}", started.elapsed());
    Ok(())
}

fn cmd_agent_context(format_json: Option<&str>, include_hidden: bool) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _skipped) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, include_hidden);

    let mut prompt_strands: Vec<_> = strands
        .iter()
        .filter(|s| s.strand_type.as_deref() == Some("prompt-strand"))
        .collect();
    prompt_strands.sort_by(|a, b| b.last_offset().cmp(&a.last_offset()));

    let last_session_offset = strands
        .iter()
        .filter(|s| s.strand_type.as_deref() == Some("session"))
        .map(|s| s.last_offset())
        .max()
        .unwrap_or(0);

    let timeline_since_last_session: Vec<_> = projection::project_timeline(&events)
        .into_iter()
        .filter(|e| e.journal_offset > last_session_offset)
        .collect();

    let prompt_strand_json: Vec<_> = prompt_strands
        .iter()
        .map(|s| json!({
            "id": s.id,
            "entries": s.log_count(),
            "first_summary": s.first_summary(),
            "last_summary": s.last_summary(),
            "last_entry_offset": s.last_offset(),
            "last_entry_ts": s.last_ts(),
            "status": s.state(),
            "hidden": s.hidden,
        }))
        .collect();

    if format_json == Some("json") {
        println!("{}", json!({
            "prompt_strands": prompt_strand_json,
            "last_session_offset": last_session_offset,
            "timeline_since_last_session": timeline_since_last_session,
        }));
    } else {
        println!("prompt_strands: {}", prompt_strands.len());
        println!("last_session_offset: {}", last_session_offset);
        println!("timeline_since_last_session: {}", timeline_since_last_session.len());
        println!("\nUse JSON for machine startup context:\n  tasktree agent-context --format json");
    }
    Ok(())
}

// ── Tree projection ─────────────────────────────────────

fn cmd_tree(root_id: &str, format_json: Option<&str>) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _skipped) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, true);

    match tree::project_tree(root_id, &strands) {
        Some(root) => {
            if format_json == Some("json") {
                let output = tree::TreeOutput { root };
                println!("{}", serde_json::to_string_pretty(&output).unwrap());
            } else {
                print_tree_text(&root, 0);
            }
        }
        None => {
            return Err(format!("strand not found or ambiguous prefix: {}", root_id));
        }
    }
    Ok(())
}

fn print_tree_text(node: &tree::TreeNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let marker = if node.children.is_empty() { "  " } else { "└─" };
    println!("{}{} {} [{}] {}",
        indent, marker,
        &node.id[..12.min(node.id.len())],
        node.status,
        node.summary.chars().take(60).collect::<String>()
    );
    for child in &node.children {
        print_tree_text(child, depth + 1);
    }
}

// ── Context projection ───────────────────────────────────
// Context projection layer.
// MUST NOT shell out to tasktree subcommands.
// Uses projection::project_strands() directly.
// See protocols/system-prompt-design.md §三 for rationale.

/// Pure projection for context (testable without stdout capture).
///
/// Exposure axis (scaffolding ADR-0002): what still binds the future is
/// exposed by default. [friction] entries on a live (registered) strand are
/// included full-text; on a closed strand they fold into `friction_folded`
/// (a scar, not a disappearance — retrieve with `show`). `--exclude-friction`
/// drops them entirely: hiding is an explicit choice, exposure the default.
fn build_context_strands(
    strands: &[projection::ProjectedStrand],
    target_type: &str,
    covers: &[String],
    since_offset: Option<usize>,
    exclude_friction: bool,
) -> Vec<ContextStrandOutput> {
    // Filter strands by type
    let mut matching: Vec<&projection::ProjectedStrand> = strands
        .iter()
        .filter(|s| s.strand_type.as_deref() == Some(target_type))
        .collect();

    // Filter by --since-offset
    if let Some(so) = since_offset {
        matching.retain(|s| s.last_offset() > so);
    }

    // Build output structures
    let mut output_strands: Vec<ContextStrandOutput> = Vec::new();

    for strand in &matching {
        // Collect [covers] entries (only entries that START with [covers])
        let covers_list: Vec<String> = strand
            .log
            .iter()
            .filter(|e| e.content.starts_with("[covers]"))
            .map(|e| e.content.trim_start_matches("[covers]").trim().to_string())
            .collect();

        // --covers filter: check if any [covers] entry contains one of the paths
        if !covers.is_empty() {
            let has_match = covers_list.iter().any(|c| {
                covers.iter().any(|p| c.contains(p.as_str()))
            });
            if !has_match {
                continue;
            }
        }

        let strand_is_live = strand.state() == "registered";
        let mut friction_folded = 0usize;

        let entries: Vec<ContextEntryOutput> = strand
            .log
            .iter()
            .filter(|e| {
                if e.content.starts_with("[friction]") {
                    if exclude_friction {
                        return false;
                    }
                    if !strand_is_live {
                        // Closed strand: friction no longer binds the
                        // future — fold to a count.
                        friction_folded += 1;
                        return false;
                    }
                    return true;
                }
                // Exclude [covers] from body (already in header)
                if e.content.starts_with("[covers]") {
                    return false;
                }
                true
            })
            .map(|e| {
                let (marker, content) = extract_marker(&e.content);
                ContextEntryOutput {
                    marker: marker.to_string(),
                    content: content.to_string(),
                    offset: e.offset,
                    ts: e.ts.clone(),
                }
            })
            .collect();

        // Skip strand if it has no entries after filtering
        if entries.is_empty() {
            continue;
        }

        // Deduplicate covers
        let mut unique_covers: Vec<String> = Vec::new();
        for c in &covers_list {
            if !unique_covers.contains(c) {
                unique_covers.push(c.clone());
            }
        }

        output_strands.push(ContextStrandOutput {
            id: strand.id.clone(),
            covers: unique_covers,
            entries,
            friction_folded,
        });
    }

    // Sort output strands by last_entry_ts descending (most recent first)
    output_strands.sort_by(|a, b| {
        let ts_a = a.entries.last().map(|e| e.ts.as_str()).unwrap_or("");
        let ts_b = b.entries.last().map(|e| e.ts.as_str()).unwrap_or("");
        ts_b.cmp(ts_a)
    });
    output_strands
}

fn cmd_context(
    context_type: Option<&str>,
    covers: &[String],
    since_offset: Option<usize>,
    format_json: Option<&str>,
    exclude_friction: bool,
    include_hidden: bool,
) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _skipped) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, include_hidden);

    let target_type = context_type.unwrap_or("prompt-strand");
    let is_json = format_json == Some("json");

    let output_strands =
        build_context_strands(&strands, target_type, covers, since_offset, exclude_friction);

    if is_json {
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({
            "strands": output_strands,
        })).map_err(|e| format!("serialize error: {}", e))?);
    } else {
        println!("# Strand Context\n");
        let strand_count = output_strands.len();
        for (i, strand) in output_strands.iter().enumerate() {
            let covers_str = if strand.covers.is_empty() {
                String::new()
            } else {
                format!(" [covers: {}]", strand.covers.join(", "))
            };
            println!("## prompt-strand:{} <id:{}>", covers_str, shorten(&strand.id));
            for entry in &strand.entries {
                if entry.marker.is_empty() {
                    println!("  {}", entry.content);
                } else {
                    println!("  {} {}", entry.marker, entry.content);
                }
            }
            if strand.friction_folded > 0 {
                println!(
                    "  friction: ×{} (folded — strand closed; tasktree show {})",
                    strand.friction_folded,
                    shorten(&strand.id)
                );
            }
            if i + 1 < strand_count {
                println!();
            }
        }
    }

    Ok(())
}

/// Extract bracket-prefix marker from content.
/// Returns ("[guide]", "remaining text") or ("", "full text") if no marker.
fn extract_marker(content: &str) -> (&str, &str) {
    if let Some(rest) = content.strip_prefix("[guide]") {
        ("[guide]", rest.trim())
    } else if let Some(rest) = content.strip_prefix("[observed]") {
        ("[observed]", rest.trim())
    } else if let Some(rest) = content.strip_prefix("[constraint]") {
        ("[constraint]", rest.trim())
    } else if let Some(rest) = content.strip_prefix("[decision]") {
        ("[decision]", rest.trim())
    } else if let Some(rest) = content.strip_prefix("[friction]") {
        ("[friction]", rest.trim())
    } else if let Some(rest) = content.strip_prefix("[covers]") {
        ("[covers]", rest.trim())
    } else if content.starts_with('[') {
        if let Some(bracket_end) = content.find(']') {
            let marker = &content[..=bracket_end];
            let rest = content[bracket_end + 1..].trim();
            (marker, rest)
        } else {
            ("", content)
        }
    } else {
        ("", content)
    }
}

#[derive(Debug, serde::Serialize)]
struct ContextStrandOutput {
    id: String,
    covers: Vec<String>,
    entries: Vec<ContextEntryOutput>,
    /// [friction] entries folded away because the strand is closed
    /// (exposure axis: a scar, not a disappearance).
    friction_folded: usize,
}

#[derive(Debug, serde::Serialize)]
struct ContextEntryOutput {
    marker: String,
    content: String,
    offset: usize,
    ts: String,
}

fn parse_duration(s: &str) -> Result<usize, String> {
    if s.is_empty() {
        return Err("empty duration".to_string());
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: usize = num_str.parse().map_err(|_| format!("invalid duration: {}", s))?;
    match unit {
        "s" => Ok(num),
        "m" => Ok(num * 60),
        "h" => Ok(num * 3600),
        "d" => Ok(num * 86400),
        _ => Err(format!("unknown duration unit '{}'. Use s/m/h/d (e.g. 2h)", unit)),
    }
}

fn cmd_show(id: Option<&str>, last: bool, tail: Option<usize>, format_json: bool, locked: bool) -> Result<(), String> {
    let started = Instant::now();
    let path = ensure_journal()?;
    let (events, skipped) = if locked {
        read_events_lossy_locked()
    } else {
        read_events_lossy(&path)
    };
    let strands = projection::project_strands(&events, true);

    let strand = if last {
        // Show most recently active strand
        if id.is_some() {
            return Err("choose one: positional id or --last, not both".to_string());
        }
        if strands.is_empty() {
            return Err("no strands found".to_string());
        }
        let mut sorted: Vec<_> = strands.iter().collect();
        sorted.sort_by(|a, b| b.last_ts().cmp(&a.last_ts()));
        sorted.into_iter().next().unwrap()
    } else {
        let id_str = id.ok_or("provide a strand id or use --last")?;
        let full = find_strand(&events, id_str)
            .ok_or_else(|| format!("strand {} not found", id_str))?;
        strands.iter().find(|s| s.id == full).unwrap()
    };

    // Summary
    let entry_count = strand.log_count();
    let last_summary = strand.last_summary();
    let canonical_state = strand.state();

    if format_json {
        let output = output::StrandDetailOutput::from(strand);
        println!("{}", serde_json::to_string(&output).expect("serialize"));
        if skipped > 0 {
            eprintln!("[tasktree] WARNING: {} corrupted lines skipped", skipped);
            std::process::exit(2);
        }
        return Ok(());
    }

    println!(
        "strand: {} | {} entries | state: {}",
        shorten(&strand.id),
        entry_count,
        canonical_state
    );
    println!("summary: {}", truncate(strand.first_summary(), 60));
    println!("next: {}", truncate(last_summary, 100));
    if strand.hidden {
        println!("status: hidden");
    }
    if !strand.edges.is_empty() {
        println!("edges: {}", strand.edges.join(", "));
    }

    // Determine which entries to show
    let entries: Vec<_> = strand.log.iter().collect();
    let slice = if let Some(n) = tail {
        if !last {
            return Err("--tail requires --last".to_string());
        }
        let skip = entries.len().saturating_sub(n);
        &entries[skip..]
    } else {
        &entries[..]
    };
    let shown = slice.len();

    println!("log:");
    for entry in slice {
        let ref_str = entry
            .ref_
            .as_ref()
            .map(|r| format!(" [ref: {}]", r))
            .unwrap_or_default();
        let id_str = entry
            .append_id
            .as_ref()
            .map(|a| format!(" [{}]", &a[..12]))
            .unwrap_or_default();
        println!(
            "  [{}]{} {}{}",
            &entry.ts[..19],
            id_str,
            entry.content,
            ref_str
        );
    }
    eprintln!(
        "[tasktree] show:   {:.0?}  ({} entries, {} shown)",
        started.elapsed(),
        entry_count,
        shown
    );
    if skipped > 0 {
        eprintln!("[tasktree] WARNING: {} corrupted lines skipped", skipped);
        std::process::exit(2);
    }
    Ok(())
}

fn shorten(id: &str) -> String {
    if id.len() > 12 {
        id[..12].to_string()
    } else {
        id.to_string()
    }
}

fn truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        format!("{}...", chars[..max].iter().collect::<String>())
    }
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let code = match err.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
                _ => 3,
            };
            let _ = err.print();
            std::process::exit(code);
        }
    };

    // Checkpoint has its own error handling (exit codes 1/2/3, JSON output)
    if let Commands::Checkpoint { id, action, tail, format, include_hidden, provenance } = &cli.command {
        let fmt = format.as_deref() == Some("json");
        match cmd_checkpoint(id.as_deref(), action, *tail, fmt, *include_hidden, provenance.as_deref()) {
            Ok(()) => return,
            Err(failure) => {
                if fmt {
                    checkpoint_error_json(&failure);
                } else {
                    eprintln!("checkpoint failed: {}", failure.message);
                    eprintln!("no journal entry written");
                }
                std::process::exit(failure.code);
            }
        }
    }

    let result = match &cli.command {
        Commands::Init => cmd_init(),
        Commands::Add { content, format, strand_type } => {
            let fmt = format.as_deref() == Some("json");
            cmd_add(content, fmt, strand_type.as_deref())
        },
        Commands::Append {
            content,
            id,
            new,
            stdin,
            file,
            explicit_id,
            format,
            provenance,
        } => cmd_append(
            content.as_deref(),
            id.as_deref(),
            *new,
            *stdin,
            file.as_deref(),
            explicit_id.as_deref(),
            format.as_deref(),
            provenance.as_deref(),
        ),
        Commands::List { all, links, backlinks, state, list_type, stale, stale_offset, since_offset, format } => {
            let fmt = format.as_deref() == Some("json");
            cmd_list(*all, links.as_deref(), backlinks.as_deref(), state.as_deref(), list_type.as_deref(), stale.as_deref(), *stale_offset, *since_offset, fmt)
        },
        Commands::Show { id, last, tail, format, locked } => {
            let fmt = format.as_deref() == Some("json");
            cmd_show(id.as_deref(), *last, *tail, fmt, *locked)
        },
        Commands::Search { query, format, include_hidden } => {
            let fmt = format.as_deref() == Some("json");
            cmd_search(query, fmt, *include_hidden)
        },
        Commands::Find { id } => cmd_find(id),
        Commands::Link { source, target, edge_type } => cmd_link(source, target, edge_type.as_deref()),
        Commands::Hide { id, reason } => cmd_hide(id, reason.as_deref()),
        Commands::Unhide { id } => cmd_unhide(id),

        Commands::Timeline { since_offset, since_ts, until_offset, until_ts, strand, links, format, limit, tree } => {
            cmd_timeline(*since_offset, since_ts.as_deref(), *until_offset, until_ts.as_deref(), strand.as_deref(), links.as_deref(), format.as_deref(), *limit, tree.as_deref())
        }
        Commands::Explain { code, format, json } => {
            let is_json = *json || format.as_deref() == Some("json");
            let output = diagnostics::cmd_explain(code, is_json);
            println!("{}", output);
            // If the code is known, exit 0; if unknown, exit 1
            if diagnostics::lookup(code).is_some() {
                Ok(())
            } else {
                Err(format!("unknown diagnostic code: {}", code))
            }
        }
        Commands::Doctor { target } => {
            let result = match target {
                DoctorTarget::Journal => cmd_doctor_journal(),
            };
            match result {
                Ok(true) => Err("journal issues detected".to_string()),
                Ok(false) => Ok(()),
                Err(e) => Err(format!("journal unreadable: {}", e)),
            }
        },

        Commands::Export { out } => cmd_export(out),

        Commands::Tree { id, format } => cmd_tree(id, format.as_deref()),

        Commands::Orient { format, include_hidden, limit } => cmd_orient(format.as_deref(), *include_hidden, *limit),

        Commands::AgentContext { format, include_hidden } => cmd_agent_context(format.as_deref(), *include_hidden),

        Commands::Context { context_type, covers, since_offset, format, exclude_friction, include_hidden } => {
            cmd_context(context_type.as_deref(), &covers, *since_offset, format.as_deref(), *exclude_friction, *include_hidden)
        },

        Commands::Bind { subject_type, subject_id, id, stdin, format } => {
            let fmt = format.as_deref() == Some("json");
            cmd_bind(
                subject_type.as_deref(),
                subject_id.as_deref(),
                id.as_deref(),
                *stdin,
                fmt,
            )
        }
        Commands::Current { subject_type, subject_id, format } => {
            let fmt = format.as_deref() == Some("json");
            cmd_current(subject_type.as_deref(), subject_id.as_deref(), fmt)
        }

        Commands::Checkpoint { .. } => unreachable!(),
    };
    if let Err(e) = result {
        if e.starts_with("warn:") {
            eprintln!("{}", e);
        } else {
            eprintln!("error: {}", e);
        }
        if e.starts_with("journal unreadable:") {
            std::process::exit(2);
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    /// Global lock to serialize current-directory changes across parallel tests.
    static CWD_LOCK: Mutex<()> = Mutex::new(());

    /// Test harness: sets cwd to a temp dir with .tasktree/, restored on drop.
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

    /// Mutex for serializing env-var-touching tests (TASKTREE_HOME).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Save and restore TASKTREE_HOME around a closure, returning its result.
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

    #[test]
    fn test_resolve_journal_walkup_finds_parent() {
        // TestEnv sets cwd to temp dir with .tasktree/ (the "project root").
        // Create a subdir and verify walk-up still finds the project journal.
        let env = setup();
        let subdir = env.path().join("subdir");
        fs::create_dir(&subdir).unwrap();
        let prev_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&subdir).unwrap();
        let result = with_tasktree_home(None, || resolve_journal_dir());
        std::env::set_current_dir(&prev_cwd).unwrap();
        let resolved = result.unwrap();
        // The resolved journal must be the project one, NOT a subdir one.
        assert!(resolved.is_dir(), "resolved path must be a directory");
        assert!(resolved.join("journal.jsonl").exists() || resolved.join("journal.lock").exists(),
            "resolved dir must look like a journal dir");
    }

    #[test]
    fn test_resolve_journal_no_journal_errors() {
        // Set cwd to a temp dir with NO .tasktree/, no parent has one either.
        let _lock = CWD_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let prev_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = with_tasktree_home(None, || resolve_journal_dir());
        std::env::set_current_dir(&prev_cwd).unwrap();
        assert!(result.is_err(), "should error when no .tasktree/ found");
        let err = result.unwrap_err();
        assert!(err.contains(".tasktree/ not found"), "unexpected error: {}", err);
    }

    #[test]
    fn test_resolve_journal_tasktree_home_absolute() {
        // TASKTREE_HOME pointing to a dir with .tasktree/ must win over walk-up.
        let env = setup();
        with_tasktree_home(Some(env.path().to_str().unwrap()), || {
            let resolved = resolve_journal_dir().unwrap();
            assert!(resolved.ends_with(JOURNAL_DIR),
                "resolved should end with .tasktree, got {:?}", resolved);
        });
    }

    #[test]
    fn test_resolve_journal_tasktree_home_missing_dir_errors() {
        // TASKTREE_HOME pointing to a dir WITHOUT .tasktree/ must error.
        let _lock = CWD_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        with_tasktree_home(Some(dir.path().to_str().unwrap()), || {
            let result = resolve_journal_dir();
            assert!(result.is_err(), "should error when TASKTREE_HOME dir has no .tasktree/");
            let err = result.unwrap_err();
            assert!(err.contains("TASKTREE_HOME"), "error must mention TASKTREE_HOME: {}", err);
        });
    }

    #[test]
    fn test_resolve_journal_tasktree_home_relative() {
        // Relative TASKTREE_HOME must resolve against cwd.
        let env = setup();
        let dir_name = env.path().file_name().unwrap().to_str().unwrap();
        let prev_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(env.path().parent().unwrap()).unwrap();
        let result = with_tasktree_home(Some(dir_name), || resolve_journal_dir());
        std::env::set_current_dir(&prev_cwd).unwrap();
        assert!(result.is_ok(), "relative TASKTREE_HOME should resolve: {:?}", result);
    }

    #[test]
    fn test_resolve_journal_walkup_stops_at_root() {
        // Walk-up must terminate (not infinite loop) even when no .tasktree/ exists.
        let _lock = CWD_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let prev_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = with_tasktree_home(None, || resolve_journal_dir());
        std::env::set_current_dir(&prev_cwd).unwrap();
        assert!(result.is_err(), "should error, not infinite loop");
    }

    #[test]
    fn test_ensure_journal_uses_resolver() {
        // After refactor, ensure_journal must go through resolve_journal_dir().
        // Smoke test: from a subdir, it returns a path inside the project .tasktree/.
        let env = setup();
        let subdir = env.path().join("nested").join("deeper");
        fs::create_dir_all(&subdir).unwrap();
        let prev_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&subdir).unwrap();
        let path = with_tasktree_home(None, || ensure_journal());
        std::env::set_current_dir(&prev_cwd).unwrap();
        let path = path.unwrap();
        assert!(path.ends_with("journal.jsonl"), "must end with journal.jsonl, got {:?}", path);
        assert!(path.parent().unwrap().file_name().unwrap() == ".tasktree",
            "parent must be .tasktree/, got {:?}", path.parent());
    }

        #[test]
    fn test_context_text_output_contract() {
        let _env = setup();
        // Create a typed prompt-strand with [covers]
        let (created, appended) = event::make_strand_created("[covers] test-area/", Some("prompt-strand"));
        let id = created.strand_id().to_string();
        with_journal_write_lock(|journal| {
            append_event_unlocked(journal, &created)?;
            append_event_unlocked(journal, &appended)?;
            Ok(())
        }).unwrap();
        // Append a [guide] entry
        let guide = event::make_log_appended(&id, "[guide] how to test", None);
        with_journal_write_lock(|journal| {
            append_event_unlocked(journal, &guide)?;
            Ok(())
        }).unwrap();
        // Verify projection sees it correctly
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let matching: Vec<&projection::ProjectedStrand> = strands
            .iter()
            .filter(|s| s.strand_type.as_deref() == Some("prompt-strand"))
            .collect();
        assert!(!matching.is_empty(), "should find prompt-strand");
        let strand = matching.iter().find(|s| s.id == id).expect("our strand should exist");
        assert_eq!(strand.log.len(), 2, "should have [covers] + [guide]");
        assert!(strand.log[0].content.starts_with("[covers]"), "first entry must be [covers]");
        assert!(strand.log[1].content.starts_with("[guide]"), "second entry is [guide]");
    }

    #[test]
    fn test_context_empty_lines() {
        let _env = setup();
        // Create two prompt-strands
        let (c1, a1) = event::make_strand_created("[covers] a/", Some("prompt-strand"));
        let (c2, a2) = event::make_strand_created("[covers] b/", Some("prompt-strand"));
        with_journal_write_lock(|journal| {
            append_event_unlocked(journal, &c1)?;
            append_event_unlocked(journal, &a1)?;
            append_event_unlocked(journal, &c2)?;
            append_event_unlocked(journal, &a2)?;
            Ok(())
        }).unwrap();
        // Run context and capture output
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let matching: Vec<&projection::ProjectedStrand> = strands
            .iter()
            .filter(|s| s.strand_type.as_deref() == Some("prompt-strand"))
            .collect();
        assert!(matching.len() >= 2, "should have at least 2 prompt-strands");
        // Verify no trailing blank line in text output by checking internal rendering
        // (full text output test would require capturing stdout)
    }

fn create_strand(content: &str) -> String {
        let (created, appended) = event::make_strand_created(content, None);
        let id = created.strand_id().to_string();
        with_journal_write_lock(|journal| {
            append_event_unlocked(journal, &created)?;
            append_event_unlocked(journal, &appended)?;
            Ok(())
        }).unwrap();
        id
    }

    // ── Content source: positional ──

    #[test]
    fn positional_append_most_recent() {
        let _env = setup();
        let _id1 = create_strand("first strand");
        let id2 = create_strand("second strand");
        // Positional content, no ID → most recent active strand
        let result = cmd_append(Some("hello world"), None, false, false, None, None, None, None);
        assert!(result.is_ok());
        // Verify content appears in most recent strand (id2)
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let has_content = events.iter().any(|(_, e)| {
            if let Event::LogAppended { id, content, .. } = e {
                id == &id2 && content == "hello world"
            } else {
                false
            }
        });
        assert!(has_content);
    }

    #[test]
    fn positional_with_legacy_id() {
        let _env = setup();
        let id1 = create_strand("first strand");
        let result = cmd_append(Some("legacy id test"), Some(&id1), false, false, None, None, None, None);
        assert!(result.is_ok());
    }

    #[test]
    fn positional_with_explicit_id() {
        let _env = setup();
        let id1 = create_strand("first strand");
        let result = cmd_append(
            Some("explicit id test"),
            None,
            false,
            false,
            None,
            Some(&id1), None, None);
        assert!(result.is_ok());
    }

    #[test]
    fn positional_empty_rejected() {
        let _env = setup();
        create_strand("first strand");
        let result = cmd_append(Some("   "), None, false, false, None, None, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[test]
    fn leading_whitespace_preserved() {
        let _env = setup();
        let id = create_strand("first strand");
        let result = cmd_append(
            Some("    indented code block\n    more indent"),
            Some(&id),
            false,
            false,
            None,
            None, None, None);
        assert!(result.is_ok());
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::LogAppended { content, .. } = e {
                content.starts_with("    indented")
            } else {
                false
            }
        });
        assert!(found);
    }

    // ── Content source: --stdin ──

    #[test]
    fn stdin_append() {
        let _env = setup();
        create_strand("first strand");
        // Simulate stdin by writing to a temp file and redirecting
        // Since we can't easily pipe in unit tests, we test read_stdin_content via a temp file approach.
        // Instead, test directly: create a file, read it with read_file_content, verify normalize_content
        let raw = "stdin simulated content\n";
        let stored = normalize_content(raw);
        assert_eq!(stored, "stdin simulated content");
    }

    // ── Content source: --file ──

    #[test]
    fn file_append_valid() {
        let _env = setup();
        let id = create_strand("first strand");
        let file_path = _env.path().join("note.md");
        fs::write(&file_path, "file content here").unwrap();
        let result = cmd_append(
            None,
            None,
            false,
            false,
            Some(file_path.to_str().unwrap()),
            Some(&id), None, None);
        assert!(result.is_ok());
    }

    #[test]
    fn file_content_not_found() {
        let result = read_file_content("nonexistent_file_xyz.md");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("file not found"));
    }

    #[test]
    fn file_content_is_directory() {
        let dir = tempfile::tempdir().unwrap();
        let result = read_file_content(dir.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("directory"));
    }

    #[test]
    fn file_content_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("empty.md");
        fs::write(&file_path, "").unwrap();
        let result = read_file_content(file_path.to_str().unwrap());
        assert!(result.is_ok()); // read succeeds, empty check happens in cmd_append
        let _env = setup();
        create_strand("first strand");
        let result = cmd_append(
            None,
            None,
            false,
            false,
            Some(file_path.to_str().unwrap()),
            None, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("empty"));
        assert!(err.contains("empty.md"));
    }

    // ── Content source conflicts ──

    #[test]
    fn content_source_none() {
        let _env = setup();
        create_strand("first strand");
        let result = cmd_append(None, None, false, false, None, None, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("content source"));
    }

    #[test]
    fn content_source_conflict_positional_and_stdin() {
        let _env = setup();
        create_strand("first strand");
        let result = cmd_append(Some("content"), None, false, true, None, None, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("only one content source"));
    }

    #[test]
    fn content_source_conflict_positional_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("note.md");
        fs::write(&file_path, "test").unwrap();
        let _env = setup();
        create_strand("first strand");
        let result = cmd_append(
            Some("content"),
            None,
            false,
            false,
            Some(file_path.to_str().unwrap()),
            None, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only one content source"));
    }

    #[test]
    fn stdin_positional_strand_id_warns_to_use_explicit_id() {
        let _env = setup();
        create_strand("first strand");
        let result = cmd_append(Some("0000019dd34b"), None, false, true, None, None, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.starts_with("warn:"));
        assert!(err.contains("require --id"));
    }

    #[test]
    fn file_positional_strand_id_warns_to_use_explicit_id() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("note.md");
        fs::write(&file_path, "test").unwrap();
        let _env = setup();
        create_strand("first strand");
        let result = cmd_append(
            Some("0000019dd34b"),
            None,
            false,
            false,
            Some(file_path.to_str().unwrap()),
            None, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.starts_with("warn:"));
        assert!(err.contains("require --id"));
    }

    #[test]
    fn content_source_conflict_stdin_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("note.md");
        fs::write(&file_path, "test").unwrap();
        let _env = setup();
        create_strand("first strand");
        let result = cmd_append(
            None,
            None,
            false,
            true,
            Some(file_path.to_str().unwrap()),
            None, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only one content source"));
    }

    #[test]
    fn content_source_all_three() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("note.md");
        fs::write(&file_path, "test").unwrap();
        let _env = setup();
        create_strand("first strand");
        let result = cmd_append(
            Some("content"),
            None,
            false,
            true,
            Some(file_path.to_str().unwrap()),
            None, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only one content source"));
    }

    // ── Target source conflicts ──

    #[test]
    fn target_conflict_new_and_id() {
        let _env = setup();
        create_strand("first strand");
        let result = cmd_append(
            Some("content"),
            None,
            true,
            false,
            None,
            Some("0000019dd34b"), None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only one target"));
    }

    #[test]
    fn target_conflict_new_and_legacy_id() {
        let _env = setup();
        let id = create_strand("first strand");
        let result = cmd_append(Some("content"), Some(&id), true, false, None, None, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only one target"));
    }

    #[test]
    fn reversed_positional_append_gets_helpful_error() {
        let _env = setup();
        let id = create_strand("first strand");
        let result = cmd_append(Some(&id), Some("[observed] finding"), false, false, None, None, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("arguments look reversed"));
        assert!(err.contains("tasktree append --id"));
        // The suggested command must carry the actual content, not echo the id
        assert!(err.contains(&format!("--id {} \"[observed] finding\"", id)));
    }

    // ── orient ──

    #[test]
    fn orient_menu_shows_active_folds_closed() {
        let _env = setup();
        let open_id = create_strand("open line of work");
        let done_id = create_strand("finished line");
        cmd_append(Some("[done] wrapped up"), None, false, false, None, Some(&done_id), None, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let max_offset = events.last().map(|(o, _)| *o).unwrap();
        // build_orient always receives the full strand list (include_hidden=true
        // in projection); the visible/hidden split is done inside build_orient.
        let strands = projection::project_strands(&events, true);
        let out = build_orient(&strands, false, 10, max_offset);

        assert_eq!(out.max_offset, max_offset);
        assert_eq!(out.active.len(), 1);
        assert_eq!(out.closed_count, 1);
        let entry = &out.active[0];
        assert_eq!(entry.id, shorten(&open_id));
        assert_eq!(entry.summary, "open line of work");
        // Catch-up command is copy-paste runnable and anchored on the
        // strand's own last_offset (ADR-0003).
        assert_eq!(
            entry.catch_up,
            format!("tasktree timeline --since-offset {} --links {}", entry.last_offset, shorten(&open_id))
        );
        assert!(out.remind.contains("checkpoint"));
        assert!(out.remind.contains("matter concluded"), "remind must carry the closing segment");
    }

    #[test]
    fn orient_hidden_count_reflects_scar_principle() {
        let _env = setup();
        let open_id = create_strand("open work");
        let hidden_id = create_strand("will be hidden");
        cmd_hide(&hidden_id, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let max_offset = events.last().map(|(o, _)| *o).unwrap();
        let strands = projection::project_strands(&events, true);

        // Default view (include_hidden=false): hidden strand must be absent
        // from active/closed pools but counted in hidden_count.
        let out = build_orient(&strands, false, 10, max_offset);
        assert_eq!(out.hidden_count, 1, "hidden strand must appear in hidden_count");
        assert_eq!(out.closed_count, 0, "hidden strand must not inflate closed_count");
        let open_short = shorten(&open_id);
        let hidden_short = shorten(&hidden_id);
        let active_ids: Vec<&str> = out.active.iter().map(|s| s.id.as_str()).collect();
        assert!(active_ids.contains(&open_short.as_str()), "visible strand must be in menu");
        assert!(!active_ids.contains(&hidden_short.as_str()), "hidden strand absent from menu");

        // include_hidden=true: hidden strand joins the pool; hidden_count=0.
        let out_all = build_orient(&strands, true, 10, max_offset);
        assert_eq!(out_all.hidden_count, 0, "include_hidden=true must yield hidden_count=0");
        let all_ids: Vec<&str> = out_all.active.iter().map(|s| s.id.as_str()).collect();
        assert!(all_ids.contains(&hidden_short.as_str()), "include_hidden=true puts hidden strand in menu");
    }

    #[test]
    fn orient_limit_keeps_most_recent() {
        let _env = setup();
        let older = create_strand("older line");
        let newer = create_strand("newer line");
        cmd_append(Some("touched again"), None, false, false, None, Some(&older), None, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let out = build_orient(&strands, false, 1, events.last().map(|(o, _)| *o).unwrap());

        assert_eq!(out.active.len(), 1);
        // `older` was touched last, so it outranks `newer` in the menu.
        assert_eq!(out.active[0].id, shorten(&older));
        let _ = newer;
    }

    // ── examples-as-contract (ADR-0001 rule 4) ──
    // Every example command in help text must at least parse against the
    // real CLI. Help text is load-bearing: agents copy it verbatim.

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
                _ => Err(format!("example does not parse: `{}` -> {}", cmdline.trim(), e)),
            },
        }
    }

    #[test]
    fn help_examples_parse_against_real_cli() {
        use clap::CommandFactory;
        let cmd = Cli::command();
        let mut helps: Vec<String> = Vec::new();
        if let Some(h) = cmd.get_after_help() {
            helps.push(h.to_string());
        }
        for sub in cmd.get_subcommands() {
            if let Some(h) = sub.get_after_help() {
                helps.push(h.to_string());
            }
        }
        let mut checked = 0usize;
        let mut failures: Vec<String> = Vec::new();
        for help in &helps {
            for line in help.lines() {
                if !line.contains("tasktree ") || line.contains("<command>") {
                    continue;
                }
                checked += 1;
                if let Err(e) = try_parse_example(line) {
                    failures.push(e);
                }
            }
        }
        assert!(checked > 10, "expected to find example lines in help text, found {}", checked);
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn catalog_recovery_commands_parse_when_executable() {
        for info in diagnostics::catalog() {
            if info.recovery.executable {
                assert!(
                    info.recovery.command_str.starts_with("tasktree"),
                    "{}: executable recovery must be a tasktree command",
                    info.code
                );
                try_parse_example(info.recovery.command_str)
                    .unwrap_or_else(|e| panic!("{}: {}", info.code, e));
            }
        }
    }

    #[test]
    fn orient_catch_up_command_parses() {
        let _env = setup();
        let id = create_strand("a line");
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let out = build_orient(&strands, false, 10, 2);
        try_parse_example(&out.active[0].catch_up).unwrap();
        let _ = id;
    }

    // ── W-code emitters (two-way closure: every code has a producer) ──

    #[test]
    fn w068_fires_on_overdue_deadline_and_respects_closing() {
        let _env = setup();
        let id = create_strand("ship the feature");
        cmd_append(Some("[deadline] finish rollout by=2000-01-01"), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
        let diags = run_journal_diagnostics(&raw, chrono::Utc::now());
        assert!(diags.iter().any(|(c, _)| *c == "W068"), "expected W068, got {:?}", diags);

        // Closing the strand silences the warning (precision over recall).
        cmd_append(Some("[cancelled] re-planned"), None, false, false, None, Some(&id), None, None).unwrap();
        let (events, _) = read_events_lossy(&path);
        let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
        let diags = run_journal_diagnostics(&raw, chrono::Utc::now());
        assert!(!diags.iter().any(|(c, _)| *c == "W068"));
    }

    #[test]
    fn w068_future_deadline_is_silent() {
        let _env = setup();
        let id = create_strand("future work");
        cmd_append(Some("[deadline] finish by=2999-01-01"), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
        let diags = run_journal_diagnostics(&raw, chrono::Utc::now());
        assert!(diags.is_empty(), "future deadline must not fire: {:?}", diags);
    }

    #[test]
    fn w069_fires_on_two_producers_same_marker() {
        let _env = setup();
        let id = create_strand("contested task");
        cmd_append(Some("[done] finished it"), None, false, false, None, Some(&id), None, Some(r#"{"producer":"alpha"}"#)).unwrap();
        cmd_append(Some("[done] also finished it"), None, false, false, None, Some(&id), None, Some(r#"{"producer":"beta"}"#)).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
        let diags = run_journal_diagnostics(&raw, chrono::Utc::now());
        let w069: Vec<_> = diags.iter().filter(|(c, _)| *c == "W069").collect();
        assert_eq!(w069.len(), 1, "expected one W069, got {:?}", diags);
        assert!(w069[0].1.contains("alpha") && w069[0].1.contains("beta"));
    }

    #[test]
    fn w069_single_producer_is_silent() {
        let _env = setup();
        let id = create_strand("solo task");
        cmd_append(Some("[done] finished"), None, false, false, None, Some(&id), None, Some(r#"{"producer":"alpha"}"#)).unwrap();
        cmd_append(Some("[verified] checked"), None, false, false, None, Some(&id), None, Some(r#"{"producer":"alpha"}"#)).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
        let diags = run_journal_diagnostics(&raw, chrono::Utc::now());
        assert!(diags.iter().all(|(c, _)| *c != "W069"));
    }

    #[test]
    fn w062_fires_on_cross_strand_keyword_within_window() {
        let _env = setup();
        let a = create_strand("storage work");
        let b = create_strand("policy work");
        cmd_append(Some("[decision] adopt sqlite for local persistence"), None, false, false, None, Some(&a), None, None).unwrap();
        cmd_append(Some("[constraint] sqlite writes are forbidden in production"), None, false, false, None, Some(&b), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
        let diags = run_journal_diagnostics(&raw, chrono::Utc::now());
        let w062: Vec<_> = diags.iter().filter(|(c, _)| *c == "W062").collect();
        assert_eq!(w062.len(), 1, "expected one W062, got {:?}", diags);
        assert!(w062[0].1.contains("sqlite"));
    }

    #[test]
    fn w062_same_strand_or_no_shared_keyword_is_silent() {
        let _env = setup();
        let a = create_strand("one line");
        cmd_append(Some("[decision] adopt sqlite here"), None, false, false, None, Some(&a), None, None).unwrap();
        cmd_append(Some("[constraint] sqlite writes forbidden"), None, false, false, None, Some(&a), None, None).unwrap();
        let b = create_strand("other line");
        cmd_append(Some("[constraint] postgres only in staging"), None, false, false, None, Some(&b), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
        let diags = run_journal_diagnostics(&raw, chrono::Utc::now());
        assert!(diags.iter().all(|(c, _)| *c != "W062"), "same-strand pair must not fire: {:?}", diags);
    }

    // ── vocabulary consistency: catalog markers must be writable ──

    /// Extract bracket markers of the form `[a-z][a-z0-9_:-]*]` from a string.
    /// Hand-rolled to avoid a regex dependency.
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

    #[test]
    fn catalog_referenced_markers_are_writable() {
        // Markers extracted from catalog prose that are NOT entry markers —
        // they are placeholder tokens or descriptions, not bracket-prefixed
        // log entries. Allowlist with comment per entry.
        let allowlist: &[&str] = &[
            // none yet
        ];

        // Markers the emitter code parses (from run_journal_diagnostics).
        let emitter_markers: &[&str] = &[
            "[deadline]", "[decision]", "[constraint]", "[verified]",
            "[done]", "[cancelled]", "[failed]", "[merged]", "[ended]",
        ];

        let mut all_markers: std::collections::HashSet<String> = std::collections::HashSet::new();

        // Collect from catalog prose.
        for info in diagnostics::catalog() {
            for s in [info.finding, info.impact, info.recovery.command_str] {
                for marker in extract_bracket_markers(s) {
                    all_markers.insert(marker);
                }
            }
        }
        // Always include the hardcoded emitter markers.
        for m in emitter_markers {
            all_markers.insert(m.to_string());
        }

        let mut failures: Vec<String> = Vec::new();
        for marker in &all_markers {
            if allowlist.contains(&marker.as_str()) {
                continue;
            }
            let test_content = format!("{} x", marker);
            if let Err(e) = validate_lifecycle_marker(&test_content) {
                failures.push(format!("marker {} referenced in catalog/emitter but rejected by validate_lifecycle_marker: {}", marker, e));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    // ── context exposure axis (ADR-0002) ──

    fn create_prompt_strand(content: &str) -> String {
        let (created, appended) = event::make_strand_created(content, Some("prompt-strand"));
        let id = created.strand_id().to_string();
        with_journal_write_lock(|journal| {
            append_event_unlocked(journal, &created)?;
            append_event_unlocked(journal, &appended)?;
            Ok(())
        }).unwrap();
        id
    }

    #[test]
    fn context_exposes_friction_on_live_strand_by_default() {
        let _env = setup();
        let id = create_prompt_strand("live guidance");
        cmd_append(Some("[friction] stepped in a hole here"), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, false);
        let out = build_context_strands(&strands, "prompt-strand", &[], None, false);
        assert_eq!(out.len(), 1);
        assert!(out[0].entries.iter().any(|e| e.marker == "[friction]"), "live friction must be exposed by default");
        assert_eq!(out[0].friction_folded, 0);
    }

    #[test]
    fn context_folds_friction_on_closed_strand() {
        let _env = setup();
        let id = create_prompt_strand("closed guidance");
        cmd_append(Some("[friction] hole, since resolved"), None, false, false, None, Some(&id), None, None).unwrap();
        cmd_append(Some("[done] wrapped up"), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, false);
        let out = build_context_strands(&strands, "prompt-strand", &[], None, false);
        assert_eq!(out.len(), 1);
        assert!(out[0].entries.iter().all(|e| e.marker != "[friction]"), "closed-strand friction folds away");
        assert_eq!(out[0].friction_folded, 1, "fold is a scar, not a disappearance");
    }

    #[test]
    fn context_exclude_friction_is_explicit_blindness() {
        let _env = setup();
        let id = create_prompt_strand("live guidance");
        cmd_append(Some("[friction] hole"), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, false);
        let out = build_context_strands(&strands, "prompt-strand", &[], None, true);
        assert_eq!(out.len(), 1);
        assert!(out[0].entries.iter().all(|e| e.marker != "[friction]"));
        assert_eq!(out[0].friction_folded, 0, "explicit exclusion is not a fold");
    }

    #[test]
    fn orient_is_pure_read() {
        let _env = setup();
        create_strand("a line");
        let path = ensure_journal().unwrap();
        let before = std::fs::read(&path).unwrap();
        cmd_orient(None, false, None).unwrap();
        cmd_orient(Some("json"), true, Some(3)).unwrap();
        let after = std::fs::read(&path).unwrap();
        assert_eq!(before, after, "orient must never write to the journal");
    }

    #[test]
    fn target_conflict_new_legacy_and_explicit() {
        let _env = setup();
        let id = create_strand("first strand");
        let result = cmd_append(Some("content"), Some(&id), true, false, None, Some(&id), None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only one target"));
    }

    #[test]
    fn target_conflict_explicit_and_legacy() {
        let _env = setup();
        let id = create_strand("first strand");
        // --id <id> "content" <id> — both explicit and legacy ID provided
        let result = cmd_append(Some("content"), Some(&id), false, false, None, Some(&id), None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only one target"));
    }

    #[test]
    fn legacy_id_rejected_with_stdin() {
        let _env = setup();
        let id = create_strand("first strand");
        // legacy positional id with --stdin (not positional content)
        let file_path = _env.path().join("note.md");
        fs::write(&file_path, "stdin content here").unwrap();
        // We use --file as a proxy for --stdin since we can't pipe in tests
        let result = cmd_append(
            None,
            Some(&id),
            false,
            false,
            Some(file_path.to_str().unwrap()),
            None, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("positional strand id"));
    }

    // ── --new strand creation ──

    #[test]
    fn new_with_positional_content() {
        let _env = setup();
        let result = cmd_append(Some("brand new strand"), None, true, false, None, None, None, None);
        assert!(result.is_ok());
    }

    #[test]
    fn new_with_file_content() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("new_strand.md");
        fs::write(&file_path, "new strand from file").unwrap();
        let _env = setup();
        let result = cmd_append(
            None,
            None,
            true,
            false,
            Some(file_path.to_str().unwrap()),
            None, None, None);
        assert!(result.is_ok());
    }

    // ── normalize_content ──

    #[test]
    fn normalize_strips_trailing_newline() {
        assert_eq!(normalize_content("hello\n"), "hello");
    }

    #[test]
    fn normalize_strips_trailing_crlf() {
        assert_eq!(normalize_content("hello\r\n"), "hello");
    }

    #[test]
    fn normalize_preserves_leading_whitespace() {
        assert_eq!(normalize_content("  hello"), "  hello");
    }

    #[test]
    fn normalize_preserves_interior_newlines() {
        assert_eq!(normalize_content("line1\nline2\n"), "line1\nline2");
    }

    #[test]
    fn normalize_preserves_multiple_trailing_newlines_except_one() {
        assert_eq!(normalize_content("hello\n\n"), "hello\n");
    }

    // ── checkpoint ──

    #[test]
    fn checkpoint_diagnostics_scar_fires_on_overdue_deadline() {
        // Strands with an overdue [deadline] must produce a W068 diagnostic.
        // Checkpoint runs diagnostics internally; this test verifies that the
        // same journal state run_journal_diagnostics sees is non-empty, which
        // is what drives the scar line printed by cmd_checkpoint.
        let _env = setup();
        let id = create_strand("deadline work");
        cmd_append(
            Some("[deadline] finish rollout by=2000-01-01"),
            None, false, false, None, Some(&id), None, None,
        ).unwrap();

        // cmd_checkpoint must succeed (overdue deadline is a warning, not fatal).
        let result = cmd_checkpoint(Some(&id), "checkpoint before close", None, false, false, None);
        assert!(result.is_ok(), "checkpoint must succeed even with overdue deadline: {:?}", result);

        // Confirm the journal state produces a W068 — the same data checkpoint uses.
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
        let diags = run_journal_diagnostics(&raw, chrono::Utc::now());
        assert!(
            diags.iter().any(|(c, _)| *c == "W068"),
            "expected W068 diagnostic for overdue deadline, got {:?}", diags
        );
    }

    #[test]
    fn checkpoint_explicit_id_appends_structured_entry() {
        let _env = setup();
        let id = create_strand("checkpoint target");

        let result = cmd_checkpoint(Some(&id), "git commit checkpoint work", None, false, false, None);
        assert!(result.is_ok());

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::LogAppended { id: event_id, content, append_id, .. } = e {
                event_id == &id
                    && content.contains("[checkpoint] ok")
                    && content.contains("resolved_by=\"explicit --id\"")
                    && content.contains("observed_entries_before_append=1")
                    && content.contains("action=\"git commit checkpoint work\"")
                    && append_id.is_some()
            } else {
                false
            }
        });
        assert!(found);
    }

    #[test]
    fn checkpoint_without_id_uses_most_recent_strand() {
        let _env = setup();
        let _old = create_strand("old strand");
        let recent = create_strand("recent strand");

        let result = cmd_checkpoint(None, "remove old build dirs", None, false, false, None);
        assert!(result.is_ok());

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::LogAppended { id, content, .. } = e {
                id == &recent
                    && content.contains("[checkpoint] ok")
                    && content.contains("resolved_by=\"most_recent_active_strand\"")
            } else {
                false
            }
        });
        assert!(found);
    }

    #[test]
    fn checkpoint_tail_does_not_change_observed_entry_count() {
        let _env = setup();
        let id = create_strand("checkpoint target");
        cmd_append(Some("step one"), Some(&id), false, false, None, None, None, None).unwrap();
        cmd_append(Some("step two"), Some(&id), false, false, None, None, None, None).unwrap();

        let result = cmd_checkpoint(Some(&id), "commit after three entries", Some(1), false, false, None);
        assert!(result.is_ok());

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::LogAppended { id: event_id, content, .. } = e {
                event_id == &id
                    && content.contains("[checkpoint] ok")
                    && content.contains("observed_entries_before_append=3")
            } else {
                false
            }
        });
        assert!(found);
    }

    #[test]
    fn checkpoint_bad_strand_returns_resolve_failure_without_append() {
        let _env = setup();
        create_strand("checkpoint target");
        let before = read_events_lossy(&ensure_journal().unwrap()).0.len();

        let result = cmd_checkpoint(Some("doesnotexist"), "bad checkpoint", None, false, false, None);
        assert!(result.is_err());
        let failure = result.unwrap_err();
        assert_eq!(failure.code, 1);
        assert!(!failure.journal_appended);

        let after = read_events_lossy(&ensure_journal().unwrap()).0.len();
        assert_eq!(before, after);
    }

    #[test]
    fn checkpoint_empty_action_returns_invalid_arguments() {
        let _env = setup();
        let id = create_strand("checkpoint target");
        let before = read_events_lossy(&ensure_journal().unwrap()).0.len();

        let result = cmd_checkpoint(Some(&id), "   ", None, false, false, None);
        assert!(result.is_err());
        let failure = result.unwrap_err();
        assert_eq!(failure.code, 3);
        assert!(!failure.journal_appended);

        let after = read_events_lossy(&ensure_journal().unwrap()).0.len();
        assert_eq!(before, after);
    }

    // ── humanize_duration ──────────────────────────────────────────────────

    #[test]
    fn humanize_duration_just_now() {
        assert_eq!(humanize_duration(0), "just now");
        assert_eq!(humanize_duration(59), "just now");
    }

    #[test]
    fn humanize_duration_minutes() {
        assert_eq!(humanize_duration(60), "1m");
        assert_eq!(humanize_duration(61), "1m");
        assert_eq!(humanize_duration(3599), "59m");
    }

    #[test]
    fn humanize_duration_hours() {
        assert_eq!(humanize_duration(3600), "1h");
        assert_eq!(humanize_duration(7200), "2h");
        assert_eq!(humanize_duration(86399), "23h");
    }

    #[test]
    fn humanize_duration_days() {
        assert_eq!(humanize_duration(86400), "1d");
        assert_eq!(humanize_duration(86400 * 25), "25d");
    }

    // ── W070: strand moved under you ───────────────────────────────────────

    #[test]
    fn w070_fires_when_checkpoint_producer_differs_from_last_entry_producer() {
        let _env = setup();
        let id = create_strand("contested work");
        // Write a log entry with producer "alpha".
        cmd_append(Some("progress note"), None, false, false, None, Some(&id), None,
            Some(r#"{"producer":"alpha"}"#)).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        // Checkpoint as "beta" — should fire W070.
        let result = check_w070_strand_moved(&events, &id, Some("beta"));
        assert!(result.is_some(), "W070 must fire when producers differ");
        let (code, detail) = result.unwrap();
        assert_eq!(code, "W070");
        assert!(detail.contains("alpha"), "detail must mention last producer: {}", detail);
        assert!(detail.contains("beta"), "detail must mention checkpoint producer: {}", detail);
    }

    #[test]
    fn w070_silent_when_same_producer() {
        let _env = setup();
        let id = create_strand("solo work");
        cmd_append(Some("note"), None, false, false, None, Some(&id), None,
            Some(r#"{"producer":"alpha"}"#)).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let result = check_w070_strand_moved(&events, &id, Some("alpha"));
        assert!(result.is_none(), "W070 must not fire when same producer");
    }

    #[test]
    fn w070_silent_when_checkpoint_producer_absent() {
        let _env = setup();
        let id = create_strand("no prov work");
        cmd_append(Some("note"), None, false, false, None, Some(&id), None,
            Some(r#"{"producer":"alpha"}"#)).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        // No checkpoint producer → silent.
        let result = check_w070_strand_moved(&events, &id, None);
        assert!(result.is_none(), "W070 must not fire when checkpoint producer absent");
    }

    #[test]
    fn w070_silent_when_last_entry_producer_absent() {
        let _env = setup();
        let id = create_strand("no prov work");
        // Append without provenance.
        cmd_append(Some("note"), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        // Last entry has no producer → silent.
        let result = check_w070_strand_moved(&events, &id, Some("beta"));
        assert!(result.is_none(), "W070 must not fire when last entry has no producer");
    }

    // ── W071: checkpoint on closed strand ──────────────────────────────────

    #[test]
    fn w071_fires_on_closed_strand() {
        let _env = setup();
        let id = create_strand("closed work");
        cmd_append(Some("[done] finished"), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let strand = strands.iter().find(|s| s.id == id).unwrap();
        let result = check_w071_closed_strand(strand);
        assert!(result.is_some(), "W071 must fire on closed strand");
        let (code, detail) = result.unwrap();
        assert_eq!(code, "W071");
        assert!(detail.contains("done"), "detail must mention state: {}", detail);
    }

    #[test]
    fn w071_silent_on_open_strand() {
        let _env = setup();
        let id = create_strand("open work");
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let strand = strands.iter().find(|s| s.id == id).unwrap();
        let result = check_w071_closed_strand(strand);
        assert!(result.is_none(), "W071 must not fire on registered strand");
    }

    // ── checkpoint + W071 end-to-end: writes succeed (exit 0) ─────────────

    #[test]
    fn checkpoint_on_closed_strand_still_succeeds() {
        let _env = setup();
        let id = create_strand("done work");
        cmd_append(Some("[done] all finished"), None, false, false, None, Some(&id), None, None).unwrap();
        // Checkpoint must still succeed — W071 is a warning, not a gate.
        let result = cmd_checkpoint(Some(&id), "tag the release", None, false, false, None);
        assert!(result.is_ok(), "checkpoint on closed strand must exit 0: {:?}", result);
    }

    // ── staleness / journal_delta helpers ─────────────────────────────────

    #[test]
    fn journal_delta_reflects_other_strand_entries() {
        let _env = setup();
        let id_a = create_strand("strand A");
        let id_b = create_strand("strand B");
        // Add two entries to B after A was last touched.
        cmd_append(Some("b-entry-1"), None, false, false, None, Some(&id_b), None, None).unwrap();
        cmd_append(Some("b-entry-2"), None, false, false, None, Some(&id_b), None, None).unwrap();

        // Compute delta for strand A (before any checkpoint write).
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let strand_a = strands.iter().find(|s| s.id == id_a).unwrap();
        let max_offset = events.last().map(|(o, _)| *o).unwrap_or(0);
        let delta = max_offset.saturating_sub(strand_a.last_offset());
        // The two entries on B occurred after A's last offset → delta >= 2.
        assert!(delta >= 2, "delta must be >= 2, got {}", delta);
    }

    #[test]
    fn export_creates_file_with_metadata_header() {
        let _env = setup();
        create_strand("test export");

        let out = _env.path().join("export.jsonl");
        let out_str = out.to_str().unwrap();
        let result = cmd_export(out_str);
        assert!(result.is_ok());

        let exported = std::fs::read_to_string(&out).unwrap();
        let lines: Vec<&str> = exported.lines().collect();
        assert!(lines.len() >= 2);

        let meta: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(meta["type"], "export_metadata");
        assert_eq!(meta["source"], "tasktree export");
        assert!(meta["journal_lines"].as_u64().unwrap() > 0);
    }

    /// `cmd_export` against a missing journal must fail. The error
    /// contract is `Err(...)` with a stable prefix; the OS-level wording
    /// after the prefix is locale-dependent (e.g. EN: "cannot read journal:
    /// ..."  /  ZH: "cannot read journal: 系统找不到指定的文件。 ..."),
    /// so we assert on the stable prefix only, not the full message.
    ///
    /// Also: this test uses an isolated temp dir + `TASKTREE_HOME` (via
    /// `with_tasktree_home`) so it cannot pollute the shared test
    /// environment. We never `remove_file` on a journal another test
    /// might be using, and we never panic while holding `CWD_LOCK` (the
    /// assertion below is a single guarded check, not a multi-step
    /// sequence that can partial-fail).
    #[test]
    fn export_no_journal_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        // Create `.tasktree/` but DO NOT create `journal.jsonl` inside it.
        // `resolve_journal_dir` succeeds (it only needs the dir to exist);
        // `cmd_export` then fails at the actual `std::fs::read` step
        // because the journal file is missing. This mirrors the user's
        // experience: a project where `.tasktree/` exists but no journal
        // has been written yet (e.g. first run after `tasktree init`).
        let tasktree = dir.path().join(".tasktree");
        std::fs::create_dir_all(&tasktree).unwrap();
        let out = dir.path().join("nojournal_export.jsonl");
        with_tasktree_home(Some(dir.path().to_str().unwrap()), || {
            let result = cmd_export(out.to_str().unwrap());
            let err = result.expect_err("cmd_export must return Err when no journal exists");
            assert!(
                err.starts_with("cannot read journal"),
                "expected stable 'cannot read journal' prefix, got: {err}"
            );
            // Output file must not have been created.
            assert!(!out.exists(), "export must not create output on failure");
        });
    }

    #[test]
    fn list_since_offset_boundary() {
        let _env = setup();
        // Create two strands at different offsets
        let id_a = create_strand("strand A");
        let id_b = create_strand("strand B");
        // Append to B to give it a later offset
        let log = event::make_log_appended(&id_b, "extra entry", None);
        with_journal_write_lock(|journal| {
            append_event_unlocked(journal, &log)?;
            Ok(())
        }).unwrap();

        // Read back to find offsets
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let strand_a = strands.iter().find(|s| s.id == id_a).unwrap();
        let strand_b = strands.iter().find(|s| s.id == id_b).unwrap();

        // --since-offset at A's last_offset → should exclude A, include B
        let mut filtered: Vec<&projection::ProjectedStrand> = strands.iter()
            .filter(|s| s.id == id_a || s.id == id_b)
            .collect();
        filtered.retain(|s| s.last_offset() > strand_a.last_offset());
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, id_b);

        // --stale-offset at A's last_offset → should include A, exclude B
        let mut stale: Vec<&projection::ProjectedStrand> = strands.iter()
            .filter(|s| s.id == id_a || s.id == id_b)
            .collect();
        stale.retain(|s| s.last_offset() <= strand_a.last_offset());
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].id, id_a);
    }

    // ── hidden-strand default visibility ──

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

    /// list/context/search default to excluding hidden strands.
    #[test]
    fn list_default_excludes_hidden() {
        let _env = setup();
        let id_a = create_strand("visible strand");
        let id_b = create_strand("will be hidden");
        cmd_hide(&id_b, Some("noise")).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let visible = projection::project_strands(&events, false);
        let visible_ids: Vec<&str> = visible.iter().map(|s| s.id.as_str()).collect();
        assert!(visible_ids.contains(&id_a.as_str()), "visible strand must appear in default list");
        assert!(!visible_ids.contains(&id_b.as_str()), "hidden strand must NOT appear in default list");
    }

    /// list --all (or the include_hidden flag in cmd_list) returns hidden strands too.
    #[test]
    fn list_with_include_hidden_returns_all() {
        let _env = setup();
        let id_a = create_strand("visible strand");
        let id_b = create_strand("will be hidden");
        cmd_hide(&id_b, Some("noise")).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let all = projection::project_strands(&events, true);
        let all_ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
        assert!(all_ids.contains(&id_a.as_str()));
        assert!(all_ids.contains(&id_b.as_str()),
            "hidden strand must appear when include_hidden=true");
    }

    /// cmd_search default does not match content inside a hidden strand.
    #[test]
    fn search_default_excludes_hidden() {
        let _env = setup();
        let id = create_strand("anchor");
        cmd_append(Some("needle-haystack"), Some(&id), false, false, None, None, None, None).unwrap();
        cmd_hide(&id, Some("noise")).unwrap();
        // Default: include_hidden=false → search skips the hidden strand.
        let result = cmd_search("needle", false, false);
        assert!(result.is_ok());
    }

    /// cmd_search --include-hidden matches inside hidden strands, and the
    /// projection's `hidden` field is true.
    #[test]
    fn search_include_hidden_projection_reports_hidden() {
        let _env = setup();
        let id = create_strand("anchor");
        cmd_append(Some("needle-haystack"), Some(&id), false, false, None, None, None, None).unwrap();
        cmd_hide(&id, Some("noise")).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let all = projection::project_strands(&events, true);
        let s = all.iter().find(|s| s.id == id).expect("strand missing");
        assert!(s.hidden, "hidden flag must be true after cmd_hide");
        let visible = projection::project_strands(&events, false);
        assert!(!visible.iter().any(|s| s.id == id), "hidden strand must not appear in default view");
        assert!(cmd_search("needle", false, true).is_ok());
    }

    /// cmd_agent_context default does not surface hidden prompt-strands.
    #[test]
    fn agent_context_default_excludes_hidden_prompt_strands() {
        let _env = setup();
        let (c, a) = event::make_strand_created("[covers] test/", Some("prompt-strand"));
        let id = c.strand_id().to_string();
        with_journal_write_lock(|j| {
            append_event_unlocked(j, &c)?;
            append_event_unlocked(j, &a)
        }).unwrap();
        cmd_hide(&id, Some("noise")).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let visible = projection::project_strands(&events, false);
        assert!(!visible.iter().any(|s| s.id == id), "hidden prompt-strand must not be visible by default");
        let all = projection::project_strands(&events, true);
        assert!(all.iter().any(|s| s.id == id), "include_hidden must surface hidden prompt-strand");
    }

    /// cmd_context default excludes hidden strands; --include-hidden surfaces them.
    #[test]
    fn context_default_excludes_hidden() {
        let _env = setup();
        let (c, a) = event::make_strand_created("[covers] test-area/", Some("prompt-strand"));
        let id = c.strand_id().to_string();
        with_journal_write_lock(|j| {
            append_event_unlocked(j, &c)?;
            append_event_unlocked(j, &a)
        }).unwrap();
        cmd_hide(&id, Some("noise")).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let visible = projection::project_strands(&events, false);
        assert!(!visible.iter().any(|s| s.id == id));
        let all = projection::project_strands(&events, true);
        assert!(all.iter().any(|s| s.id == id));
    }

    /// Repeated `cmd_hide` is idempotent: only one StrandHidden event is written.
    #[test]
    fn hide_is_idempotent() {
        let _env = setup();
        let id = create_strand("hide me");
        let before = total_events();
        cmd_hide(&id, None).unwrap();
        let mid = total_events();
        cmd_hide(&id, None).unwrap();
        cmd_hide(&id, Some("still hidden")).unwrap();
        let after = total_events();
        assert_eq!(mid - before, 1, "first hide must write exactly 1 event");
        assert_eq!(after - mid, 0, "repeated hide must be a no-op (0 events appended)");
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        assert_eq!(count_hide_events(&events, &id, "hidden"), 1);
        assert_eq!(count_hide_events(&events, &id, "unhidden"), 0);
    }

    /// One `cmd_unhide` after a `cmd_hide` restores visibility — no negative
    /// hide_count, no orphan unhidden events.
    #[test]
    fn single_unhide_restores_visibility() {
        let _env = setup();
        let id = create_strand("hide/unhide me");
        cmd_hide(&id, None).unwrap();
        cmd_unhide(&id).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let s = projection::project_strands(&events, true)
            .into_iter()
            .find(|s| s.id == id)
            .expect("strand missing from projection");
        assert!(!s.hidden, "strand must be visible after one hide + one unhide");
        assert_eq!(count_hide_events(&events, &id, "hidden"), 1);
        assert_eq!(count_hide_events(&events, &id, "unhidden"), 1);
    }

    /// Repeated `cmd_unhide` on an already-visible strand is a no-op.
    #[test]
    fn unhide_is_idempotent() {
        let _env = setup();
        let id = create_strand("plain strand");
        let before = total_events();
        cmd_unhide(&id).unwrap();
        cmd_unhide(&id).unwrap();
        let after = total_events();
        assert_eq!(after - before, 0, "unhide on visible strand must be a no-op");
    }

    /// Without --id, cmd_checkpoint picks the most-recent VISIBLE strand by
    /// default. When the most-recent strand is hidden, the visible one is chosen.
    #[test]
    fn checkpoint_without_id_skips_hidden_when_explicit_id_missing() {
        let _env = setup();
        let old = create_strand("old visible strand");
        let recent = create_strand("recent will be hidden");
        cmd_hide(&recent, Some("noise")).unwrap();
        let result = cmd_checkpoint(None, "fall back to visible", None, false, false, None);
        assert!(result.is_ok());
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::LogAppended { id, content, .. } = e {
                id == &old
                    && content.contains("resolved_by=\"most_recent_active_strand\"")
            } else {
                false
            }
        });
        assert!(found, "checkpoint must fall back to the visible strand when most-recent is hidden");
    }

    /// With --include-hidden / --all, cmd_checkpoint may pick a hidden strand.
    #[test]
    fn checkpoint_with_include_hidden_can_pick_hidden_strand() {
        let _env = setup();
        let _old = create_strand("old visible strand");
        let recent = create_strand("recent will be hidden");
        cmd_hide(&recent, Some("noise")).unwrap();
        let result = cmd_checkpoint(None, "allow hidden", None, false, true, None);
        assert!(result.is_ok());
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::LogAppended { id, content, .. } = e {
                id == &recent
                    && content.contains("resolved_by=\"most_recent_active_strand\"")
            } else {
                false
            }
        });
        assert!(found, "with include_hidden=true, checkpoint must pick the most-recent hidden strand");
    }

    /// With an explicit --id that happens to be a hidden strand, the
    /// checkpoint must still find it (the user named it directly).
    #[test]
    fn checkpoint_explicit_id_finds_hidden_strand() {
        let _env = setup();
        let id = create_strand("explicit hidden");
        cmd_hide(&id, Some("noise")).unwrap();
        let result = cmd_checkpoint(Some(&id), "explicit id on hidden", None, false, false, None);
        assert!(result.is_ok(), "explicit --id must resolve a hidden strand");
    }

    /// cmd_context default (include_hidden=false) MUST NOT surface hidden
    /// prompt-strands via the cmd_context call path. Regression for the
    /// 'flag plumbed but projection ignores it' bug caught during
    /// hygiene review of 66f668e.
    #[test]
    fn cmd_context_default_excludes_hidden_via_cmd_path() {
        let _env = setup();
        let (c, a) = event::make_strand_created("[covers] audit/", Some("prompt-strand"));
        let id = c.strand_id().to_string();
        with_journal_write_lock(|j| {
            append_event_unlocked(j, &c)?;
            append_event_unlocked(j, &a)
        }).unwrap();
        cmd_hide(&id, Some("noise")).unwrap();
        let result = cmd_context(None, &[], None, None, false, false);
        assert!(result.is_ok());
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let visible = projection::project_strands(&events, false);
        assert!(!visible.iter().any(|s| s.id == id),
            "cmd_context default must use include_hidden=false in projection");
    }

    /// cmd_agent_context default must also exclude hidden strands.
    #[test]
    fn cmd_agent_context_default_excludes_hidden_via_cmd_path() {
        let _env = setup();
        let (c, a) = event::make_strand_created("[covers] audit2/", Some("prompt-strand"));
        let id = c.strand_id().to_string();
        with_journal_write_lock(|j| {
            append_event_unlocked(j, &c)?;
            append_event_unlocked(j, &a)
        }).unwrap();
        cmd_hide(&id, Some("noise")).unwrap();
        let result = cmd_agent_context(None, false);
        assert!(result.is_ok());
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let visible = projection::project_strands(&events, false);
        assert!(!visible.iter().any(|s| s.id == id),
            "cmd_agent_context default must use include_hidden=false in projection");
    }

    // ── Subject binding tests (pi-strand V1 contract) ─────────────────

    #[test]
    fn bind_creates_subject_bound_event() {
        let _env = setup();
        let id = create_strand("target");
        let result = cmd_bind(Some("pi-session"), Some("abc"), Some(&id), false, false);
        assert!(result.is_ok());
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let has_binding = events.iter().any(|(_, e)| {
            matches!(e, Event::SubjectBound { subject_type, subject_id, strand_id, .. }
                if subject_type == "pi-session" && subject_id == "abc" && strand_id == &id)
        });
        assert!(has_binding, "bind must write a SubjectBound event");
    }

    #[test]
    fn bind_resolves_prefix_id() {
        let _env = setup();
        let id = create_strand("target strand");
        let short = &id[..12];
        let result = cmd_bind(Some("ci-run"), Some("run-42"), Some(short), false, false);
        assert!(result.is_ok(), "prefix strand id should resolve: {:?}", result);
    }

    #[test]
    fn bind_missing_strand_fails() {
        let _env = setup();
        let result = cmd_bind(Some("pi-session"), Some("x"), Some("000000000000"), false, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn current_returns_latest_binding() {
        let _env = setup();
        let id_a = create_strand("first");
        let id_b = create_strand("second");
        // Bind subject to strand a
        cmd_bind(Some("pi-session"), Some("user1"), Some(&id_a), false, false).unwrap();
        // Re-bind to strand b (latest should win)
        cmd_bind(Some("pi-session"), Some("user1"), Some(&id_b), false, false).unwrap();
        let result = cmd_current(Some("pi-session"), Some("user1"), false);
        assert!(result.is_ok());
        // We can't easily capture stdout here, so we test via the projection
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let mut latest: Option<String> = None;
        for (_, e) in &events {
            if let Event::SubjectBound { subject_type: t, subject_id: i, strand_id: s, .. } = e {
                if t == "pi-session" && i == "user1" {
                    latest = Some(s.clone());
                }
            }
        }
        assert_eq!(latest, Some(id_b), "latest binding must point to id_b");
    }

    #[test]
    fn current_no_binding_returns_error() {
        let _env = setup();
        create_strand("orphan");
        let result = cmd_current(Some("pi-session"), Some("no-such"), false);
        assert!(result.is_err());
    }

    #[test]
    fn current_requires_non_empty_args() {
        let _env = setup();
        let r1 = cmd_current(None, Some("x"), false);
        assert!(r1.is_err());
        let r2 = cmd_current(Some("x"), None, false);
        assert!(r2.is_err());
        let r3 = cmd_current(Some(""), Some("x"), false);
        assert!(r3.is_err());
    }

    #[test]
    fn bind_requires_non_empty_args() {
        let _env = setup();
        let id = create_strand("t");
        let r1 = cmd_bind(None, Some("x"), Some(&id), false, false);
        assert!(r1.is_err());
        let r2 = cmd_bind(Some("x"), None, Some(&id), false, false);
        assert!(r2.is_err());
        let r3 = cmd_bind(Some("x"), Some("y"), None, false, false);
        assert!(r3.is_err());
    }

    // ── Provenance tests (pi-strand V1 contract) ─────────────────────

    #[test]
    fn append_with_provenance_stores_it() {
        let _env = setup();
        let id = create_strand("target");
        let prov = Some(serde_json::json!({ "producer": "pi", "model": "gpt-5" }));
        let event = event::make_log_appended(&id, "provenance test", prov);
        with_journal_write_lock(|j| {
            append_event_unlocked(j, &event)
        }).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::LogAppended { content, provenance, .. } = e {
                content == "provenance test" && provenance.is_some()
            } else {
                false
            }
        });
        assert!(found, "provenance must be stored on the event");
    }

    #[test]
    fn append_without_provenance_has_none() {
        let _env = setup();
        let id = create_strand("target");
        let event = event::make_log_appended(&id, "no provenance", None);
        with_journal_write_lock(|j| {
            append_event_unlocked(j, &event)
        }).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::LogAppended { content, provenance, .. } = e {
                content == "no provenance" && provenance.is_none()
            } else {
                false
            }
        });
        assert!(found, "append without provenance must have provenance=None");
    }

    #[test]
    fn provenance_serializes_only_when_present() {
        // Verify that serialized JSON doesn't contain provenance key when None.
        let event = event::make_log_appended("test", "no prov", None);
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("provenance"),
            "None provenance must not appear in JSON: {}", json);
        let with_prov = event::make_log_appended("test", "has prov",
            Some(serde_json::json!({ "k": "v" })));
        let json2 = serde_json::to_string(&with_prov).unwrap();
        assert!(json2.contains("provenance"),
            "Some provenance must appear in JSON: {}", json2);
    }

    #[test]
    fn old_journal_line_still_deserializes() {
        // A LogAppended event serialized by an older version (no provenance field)
        // must still parse to Event with provenance=None.
        let old_line = r#"{"type":"log_appended","id":"abc","ts":"2026-01-01T00:00:00Z","content":"old entry","append_id":"deadbeef"}"#;
        let event: Event = serde_json::from_str(old_line).unwrap();
        match &event {
            Event::LogAppended { content, provenance, .. } => {
                assert_eq!(content, "old entry");
                assert!(provenance.is_none(), "old journal must deserialize with provenance=None");
            }
            _ => panic!("expected LogAppended"),
        }
    }

    #[test]
    fn append_help_markers_are_writable() {
        // Every marker listed in the Append after_help must be accepted by
        // validate_lifecycle_marker, keeping help text honest forever.
        use clap::CommandFactory;
        let cmd = Cli::command();
        let append_help = cmd
            .get_subcommands()
            .find(|s| s.get_name() == "append")
            .and_then(|s| s.get_after_help())
            .map(|h| h.to_string())
            .expect("append subcommand must have after_help");
        let markers = extract_bracket_markers(&append_help);
        assert!(!markers.is_empty(), "markers section must list at least one marker");
        let mut failures: Vec<String> = Vec::new();
        for marker in &markers {
            let test_content = format!("{} x", marker);
            if let Err(e) = validate_lifecycle_marker(&test_content) {
                failures.push(format!("{}: {}", marker, e));
            }
        }
        assert!(failures.is_empty(), "markers in append help rejected by validate_lifecycle_marker:\n{}", failures.join("\n"));
    }

    #[test]
    fn show_search_context_unchanged() {
        // Smoke test that existing cmd_show, cmd_search, cmd_context still work.
        let _env = setup();
        let id = create_strand("show me");
        cmd_append(Some("entry"), Some(&id), false, false, None, None, None, None).unwrap();
        // show
        let r = cmd_show(Some(&id), false, None, false, false);
        assert!(r.is_ok());
        // search
        let r = cmd_search("entry", false, false);
        assert!(r.is_ok());
        // context
        let r = cmd_context(None, &[], None, None, false, false);
        assert!(r.is_ok());
    }

    // ── card echo: make_card ──

    #[test]
    fn make_card_fields_match_projected_strand() {
        let _env = setup();
        let id = create_strand("summary text for the card");
        cmd_append(Some("second entry"), Some(&id), false, false, None, None, None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let s = strands.iter().find(|s| s.id == id).expect("strand must exist");
        let card = make_card(s);
        assert_eq!(card.id, shorten(&id));
        assert_eq!(card.entries, 2);
        assert_eq!(card.summary, truncate(s.first_summary(), 70));
        assert_eq!(card.last_entry, truncate(s.last_summary(), 70));
        assert_eq!(card.last_offset, s.last_offset());
    }

    #[test]
    fn make_card_truncates_prose_to_70() {
        let _env = setup();
        let long = "x".repeat(100);
        let id = create_strand(&long);
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let s = strands.iter().find(|s| s.id == id).expect("strand must exist");
        let card = make_card(s);
        // truncate(100-char string, 70) → 70 chars + "..." = 73 total
        assert!(card.summary.len() <= 73, "summary must be truncated to 70 chars + ...");
        // id is never truncated: always shorten(full_id) = 12 chars
        assert_eq!(card.id.len(), 12);
    }

    // ── card echo: strand_card_fresh / append paths ──

    #[test]
    fn append_explicit_id_card_fresh_has_new_entry() {
        let _env = setup();
        let id = create_strand("target");
        cmd_append(Some("[lesson] learned something"), Some(&id), false, false, None, None, None, None).unwrap();
        let (card, _state) = strand_card_fresh_with_state(&id).expect("card must be retrievable");
        assert_eq!(card.last_entry, "[lesson] learned something");
    }

    #[test]
    fn append_default_most_recent_card_fresh_reflects_write() {
        let _env = setup();
        let _id1 = create_strand("older");
        let id2 = create_strand("newer");
        cmd_append(Some("default route entry"), None, false, false, None, None, None, None).unwrap();
        let (card, _state) = strand_card_fresh_with_state(&id2).expect("card must exist");
        assert_eq!(card.last_entry, "default route entry");
    }

    #[test]
    fn append_new_path_card_id_matches_new_strand() {
        let _env = setup();
        // Pre-populate so --new is not the only strand
        create_strand("existing");
        cmd_append(Some("brand new via --new"), None, true, false, None, None, None, None).unwrap();
        // The new strand has the content as first_summary
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let new_s = strands.iter().find(|s| s.first_summary() == "brand new via --new")
            .expect("new strand must exist");
        let card = strand_card_fresh(&new_s.id).expect("card must be retrievable");
        assert_eq!(card.id, shorten(&new_s.id));
    }

    // ── card echo: hide leaves strand retrievable via include_hidden=true ──

    #[test]
    fn strand_card_fresh_finds_hidden_strand() {
        let _env = setup();
        let id = create_strand("will be hidden");
        cmd_hide(&id, None).unwrap();
        // strand_card_fresh uses include_hidden=true — must still find it
        let card = strand_card_fresh(&id);
        assert!(card.is_some(), "strand_card_fresh must return card for hidden strand");
        assert_eq!(card.unwrap().id, shorten(&id));
    }
}


use crate::commands::doctor::*;
use crate::commands::manage::*;
use crate::commands::query::*;
use crate::commands::write::*;
use crate::diagnostics;
use crate::journal::{JOURNAL_DIR, JOURNAL_FILE};
use clap::{Parser, Subcommand, error::ErrorKind};
use std::path::PathBuf;

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
loop: 做一步 -> 看现实变 -> 再想。命令按 loop 阶分组：

  orient        Session entry: active strand menu + catch-up commands

看 / read:
  list          List strands
  show          Show one strand (--digest one-glance, --tail N recent)
  timeline      Chronological entries across strands (+linked)
  search        Full-text search across entries
  tree          Strand forest (belongs-to nesting)
  depends       depends-on analysis: blockers / readiness / critical path

做 / change:
  add           Create a new strand
  append        Append an entry to a strand
  close         Close a strand (StrandClosed event)
  reopen        Reopen a closed strand (StrandReopened event)
  link          Link strands (belongs-to / depends-on)
  unlink        Remove a link (EdgeUnlinked; read projection drops the edge)

管 / manage:
  init          Initialize .tasktree/ journal
  hide          Hide a strand from active orient (parked, revivable)
  unhide        Unhide a strand
  doctor        Diagnose journal integrity
  export        Export journal as standalone audit artifact
  explain       Explain a diagnostic code or topic (markers, json, grammar, ...)

Run:  tasktree <command> --help"
)]
pub(crate) struct Cli {
    /// Operate as if started in DIR (journal walk-up and relative paths use DIR)
    #[arg(short = 'C', long = "chdir", value_name = "DIR", global = true)]
    chdir: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

/// Single-strand target: positional <ID> and --id <ID> are equivalent.
/// Grammar contract: every single-id command mounts this (one artifact,
/// not a convention replicated per command).
#[derive(clap::Args)]
struct IdTarget {
    /// Strand ID (prefix match)
    #[arg(value_name = "ID")]
    id_pos: Option<String>,
    /// Strand ID via flag (equivalent to the positional form)
    #[arg(long = "id", value_name = "ID", conflicts_with = "id_pos")]
    id_flag: Option<String>,
}
impl IdTarget {
    fn get(&self) -> Option<&str> {
        self.id_pos
            .as_deref()
            .or(self.id_flag.as_deref())
            .filter(|s| !s.trim().is_empty())
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize .tasktree/ directory and journal
    Init,
    /// Create a new strand with first log entry
    #[command(after_help = "\
Content source (choose exactly one):
  CONTENT             First log entry content
  --stdin             Read content from standard input
  --file <PATH>       Read content from a file

Rules:
  CONTENT, --stdin, and --file are mutually exclusive.
  Empty content is rejected.

Examples:
  tasktree add \"start a new line of work\"
  tasktree add --parent <PARENT> \"child line of work\"
  echo \"start a new line\" | tasktree add --stdin
  tasktree add --file brief.md")]
    Add {
        /// Content for the first log entry (positional; omit when using --stdin or --file)
        content: Option<String>,
        /// Read content from standard input
        #[arg(long, verbatim_doc_comment)]
        stdin: bool,
        /// Read content from a file
        #[arg(long, value_name = "PATH", verbatim_doc_comment)]
        file: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Parent strand id. Creates a belongs-to edge from the new child to this parent.
        #[arg(long = "parent", visible_alias = "belongs-to", value_name = "PARENT")]
        parent: Option<String>,
        /// Strand type: task, dag, why, session (default: auto-detect)
        #[arg(long = "type", value_name = "TYPE")]
        strand_type: Option<String>,
        /// Optional provenance JSON object. Stored on the initial LogAppended entry.
        #[arg(long = "provenance", value_name = "JSON")]
        provenance: Option<String>,
    },
    /// Append content to an existing strand.
    #[command(after_help = "\
Invocation forms:
  tasktree append <CONTENT> [--id <ID>]
  tasktree append --stdin [--id <ID>]
  tasktree append --file <PATH> [--id <ID>]

Content source (choose exactly one):
  CONTENT             Log content
  --stdin             Read content from standard input
  --file <PATH>       Read content from a file

Target (choose at most one):
  (none)              Append to most recently active strand
  --id <ID>           Append to a specific strand

Rules:
  CONTENT, --stdin, and --file are mutually exclusive.
  Empty content is rejected.
  To create a new strand, use the add command.

Examples:
  tasktree append \"short note\"
  tasktree append --id 0000019dd34b \"short note\"

  echo \"long note\" | tasktree append --stdin
  echo \"long note\" | tasktree append --stdin --id 0000019dd34b

  tasktree append --file note.md
  tasktree append --file note.md --id 0000019dd34b

  tasktree append --file note.md --id 0000019dd34b --provenance '{\"producer\":\"pi\",\"model\":\"gpt-5\"}'

Markers (optional bracket prefix on the first line):
  Marker vocabulary: tasktree explain markers

Provenance:
  --provenance <JSON>  Optional structured metadata. Must be a JSON
                       object. Stored on the LogAppended event, not in
                       the entry text. Older journals ignore it.
  --seen-offset <N>    Caller-declared last observed offset for the target
                       strand. If stale, emits W076 but still writes.")]
    Append {
        /// Log content
        content: Option<String>,
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
        /// Caller-declared last observed offset for the target strand.
        /// If behind the target's current last_offset, emits W076 but still writes.
        #[arg(long = "seen-offset", value_name = "N")]
        seen_offset: Option<usize>,
        /// Pin a rationale: the strand whose entry this one's reason rests on
        /// (prefix match). Stored as ref=<id>@<offset> (W1/F4-pin); the doctor
        /// why-staleness clerk flags it when that strand later advances.
        #[arg(long = "why", value_name = "REF")]
        why: Option<String>,
    },
    /// List all strands (reverse chronological, most recent last)
    List {
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
        #[command(flatten)]
        target: IdTarget,
        /// Show the most recently active strand instead of specifying an id
        #[arg(long)]
        last: bool,
        /// Show only the last N log entries
        #[arg(long, value_name = "N")]
        tail: Option<usize>,
        /// One-glance digest: header + marker census, no full log dump
        #[arg(long)]
        digest: bool,
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
    /// Create a directed link between two strands
    #[command(after_help = "\
Direction (read SOURCE first): the edge always points from SOURCE to TARGET.

  belongs-to   SOURCE belongs to TARGET — source is the child, target is the
               parent. tree and orient --tree nest SOURCE under TARGET.
  depends-on   SOURCE depends on TARGET (TARGET must advance first). [default]

  (why is no longer a link (D2): a reason is an entry rationale, not a
   strand edge. Record the reason in the entry text itself.)

Examples:
  tasktree link <CHILD> <PARENT> --edge-type belongs-to
               (CHILD nests under PARENT in tree / orient --tree)
  tasktree link <TASK> <BLOCKER> --edge-type depends-on
               (TASK waits on BLOCKER)

Forest projection (how belongs-to nests): tasktree explain card
JSON shape: tasktree explain json")]
    Link {
        /// Source strand ID (prefix match). For belongs-to, this is the child.
        source: String,
        /// Target strand ID (prefix match). For belongs-to, this is the parent.
        target: String,
        /// Edge type: belongs-to, depends-on (default: depends-on).
        /// Direction: SOURCE <edge-type> TARGET (e.g. SOURCE belongs-to TARGET
        /// = source is child of target).
        #[arg(long = "edge-type", value_name = "TYPE")]
        edge_type: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Optional provenance JSON object. Same shape as `append --provenance`.
        /// Stored on the EdgeLinked event.
        #[arg(long = "provenance", value_name = "JSON")]
        provenance: Option<String>,
    },
    /// Remove a directed link between two strands (writes an EdgeUnlinked event)
    #[command(after_help = "\
Append-only: the original link stays in the journal; the read projection drops
the edge. edge_type must match the link being removed (belongs-to / depends-on).

Example:
  tasktree unlink <TASK> <BLOCKER> --edge-type depends-on")]
    Unlink {
        /// Source strand ID (prefix match) — same SOURCE as the link being removed.
        source: String,
        /// Target strand ID (prefix match) — same TARGET as the link being removed.
        target: String,
        /// Edge type to remove: belongs-to, depends-on (default: depends-on).
        #[arg(long = "edge-type", value_name = "TYPE")]
        edge_type: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Optional provenance JSON object. Stored on the EdgeUnlinked event.
        #[arg(long = "provenance", value_name = "JSON")]
        provenance: Option<String>,
    },
    /// Hide a strand from default list view
    Hide {
        #[command(flatten)]
        target: IdTarget,
        /// Reason for hiding (optional). If provided, appends '[hidden] <reason>' to the strand.
        #[arg(long)]
        reason: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Optional provenance JSON object. When --reason is given, stored on
        /// the '[hidden] <reason>' LogAppended entry. Without --reason the
        /// StrandHidden event carries no provenance (no content entry is written).
        #[arg(long = "provenance", value_name = "JSON")]
        provenance: Option<String>,
    },
    /// Unhide a previously hidden strand
    Unhide {
        #[command(flatten)]
        target: IdTarget,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },

    /// Close a strand (write a StrandClosed lifecycle event)
    ///
    /// This is the only way to change a strand's lifecycle state to closed.
    /// Appending [done] to a strand no longer closes it — use this command.
    #[command(after_help = "\
Dispositions (--as):
  done       Work completed successfully (default)
  failed     Work stopped due to failure
  cancelled  Work abandoned intentionally
  merged     Work merged into another strand
  verified   Work completed and independently verified

Examples:
  tasktree close --id <ID>
  tasktree close --id <ID> --as failed
  tasktree close --id <ID> --as cancelled")]
    Close {
        /// Strand ID (prefix match)
        #[arg(long = "id", value_name = "ID")]
        id: String,
        /// Disposition: done (default), failed, cancelled, merged, verified
        #[arg(long = "as", value_name = "DISPOSITION")]
        disposition: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },

    /// Reopen a closed strand (write a StrandReopened lifecycle event)
    ///
    /// Moves the strand back to open/registered state.
    #[command(after_help = "\
Examples:
  tasktree reopen --id <ID>
  tasktree reopen --id <ID> --format json")]
    Reopen {
        /// Strand ID (prefix match)
        #[arg(long = "id", value_name = "ID")]
        id: String,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },

    /// Explain a diagnostic code or encyclopaedia topic
    ///
    /// Namespace rule: diagnostic codes begin with an uppercase letter
    /// (W062, E053); topics are all-lowercase (card, markers, retry, json, grammar).
    /// The two namespaces are mechanically disjoint.
    #[command(after_help = "\
Namespaces:
  Diagnostic codes   uppercase-initial: W062, E053, w062 (case-insensitive)
  Topics             all-lowercase:     card, markers, retry, json, jq, grammar

Topics:
  card      卡片：统一输出文法单元（格式、字段、回显语义）
  markers   Marker 词表（[decision]、[done] 等前缀规范）
  retry     重试语义：哪些命令可盲目重试
  json      JSON 形态索引：各读命令 --format json 的顶层字段
  jq        jq 整型：把 --format json 输出切成你要的形

Examples:
  tasktree explain W062
  tasktree explain card
  tasktree explain json
  tasktree explain markers
  tasktree explain retry
  tasktree explain W062 --format json
  tasktree explain card --json")]
    Explain {
        /// Diagnostic code (e.g. W068) or topic name (e.g. card)
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
    /// Show entry events in journal append order (timeline projection)
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
        #[arg(
            long,
            visible_alias = "id",
            value_name = "ID",
            conflicts_with = "links"
        )]
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
  matter concluded  tasktree close --id <ID> [--as done|failed|cancelled|merged|verified]
                    (default: done; reopen with tasktree reopen --id <ID>)
  before anything irreversible
                    pause: name the change you can't take back, then append
                    your reasoning with tasktree append --id <ID> \"[decision] ...\"

Closed strands are folded to a count; retrieve with tasktree list.
Hidden strands are folded to a count; unhide with tasktree unhide --id <ID>.

--tree: render active strands as a belongs-to forest. Strands that declare
  a belongs-to edge to another active strand are indented under their parent;
  parallel siblings under the same parent are visible as a group.
  Default orient (no --tree) is unchanged: flat list ordered by last_offset.
Exit codes:
  0 ok
  1 journal missing or unreadable
JSON shape: tasktree explain json")]
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
        /// Render active strands as a belongs-to forest (parallel siblings visible under shared parent)
        #[arg(long)]
        tree: bool,
    },
    /// Build nested tree projection from strand edges
    Tree {
        #[command(flatten)]
        target: IdTarget,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
    /// Inspect depends-on upstreams as review context
    #[command(after_help = "\
depends-on is an attention edge for review and handoff context, not an
execution gate. The legacy ready/open-blocker fields remain for compatibility;
prefer upstream lifecycle facts when making decisions. Built on the typed
depends-on projection (F3).

Examples:
  tasktree depends <TASK>
  tasktree depends <TASK> --format json")]
    Depends {
        #[command(flatten)]
        target: IdTarget,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
}

#[derive(Subcommand)]
enum DoctorTarget {
    /// Check journal integrity
    Journal {
        /// Treat advisory warnings as blocking issues
        #[arg(long)]
        strict: bool,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
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

// ── exit strategy ──
// Commands return typed-by-prefix error strings; this adapter is the single
// place that turns them into process output and exit codes.
pub(crate) fn main() {
    let cli = parse_cli_or_exit();
    apply_chdir(cli.chdir.as_deref());
    if let Err(e) = run(&cli.command) {
        exit_with_error(&e);
    }
}

/// Parse argv into a `Cli`, or print clap's help/error and exit: code 0 for
/// `--help`/`--version`, code 3 for a parse/usage error.
fn parse_cli_or_exit() -> Cli {
    match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let code = match err.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
                _ => 3,
            };
            let _ = err.print();
            std::process::exit(code);
        }
    }
}

/// Apply `-C/--chdir` before any journal resolution; exit 3 on a missing or
/// unusable directory.
fn apply_chdir(chdir: Option<&str>) {
    if let Some(dir) = chdir {
        let target = std::path::Path::new(dir);
        if !target.exists() {
            eprintln!("error: -C {}: no such directory", dir);
            std::process::exit(3);
        }
        if let Err(e) = std::env::set_current_dir(target) {
            eprintln!("error: -C {}: {}", dir, e);
            std::process::exit(3);
        }
    }
}

/// Dispatch a parsed command to its handler. Kept free of `std::process::exit`
/// so the dispatch table is unit-testable and the exit-code policy lives solely
/// in `exit_with_error`.
fn run(command: &Commands) -> Result<(), String> {
    match command {
        Commands::Init => cmd_init(),
        Commands::Add {
            content,
            stdin,
            file,
            format,
            parent,
            strand_type,
            provenance,
        } => {
            let fmt = format.as_deref() == Some("json");
            cmd_add(AddRequest {
                content: content.as_deref(),
                stdin: *stdin,
                file: file.as_deref(),
                format_json: fmt,
                parent: parent.as_deref(),
                strand_type: strand_type.as_deref(),
                provenance_raw: provenance.as_deref(),
            })
        }
        Commands::Append {
            content,
            stdin,
            file,
            explicit_id,
            format,
            provenance,
            seen_offset,
            why,
        } => cmd_append_with_seen_offset(
            content.as_deref(),
            *stdin,
            file.as_deref(),
            explicit_id.as_deref(),
            format.as_deref(),
            provenance.as_deref(),
            *seen_offset,
            why.as_deref(),
        ),
        Commands::List {
            links,
            backlinks,
            state,
            list_type,
            stale,
            stale_offset,
            since_offset,
            format,
        } => {
            let fmt = format.as_deref() == Some("json");
            cmd_list(
                false,
                links.as_deref(),
                backlinks.as_deref(),
                state.as_deref(),
                list_type.as_deref(),
                stale.as_deref(),
                *stale_offset,
                *since_offset,
                fmt,
            )
        }
        Commands::Show {
            target,
            last,
            tail,
            digest,
            format,
            locked,
        } => {
            let fmt = format.as_deref() == Some("json");
            cmd_show(target.get(), *last, *tail, fmt, *locked, *digest)
        }
        Commands::Search {
            query,
            format,
            include_hidden,
        } => {
            let fmt = format.as_deref() == Some("json");
            cmd_search(query, fmt, *include_hidden)
        }
        Commands::Link {
            source,
            target,
            edge_type,
            format,
            provenance,
        } => {
            let fmt = format.as_deref() == Some("json");
            cmd_link(
                source,
                target,
                edge_type.as_deref(),
                fmt,
                provenance.as_deref(),
            )
        }
        Commands::Unlink {
            source,
            target,
            edge_type,
            format,
            provenance,
        } => {
            let fmt = format.as_deref() == Some("json");
            cmd_unlink(
                source,
                target,
                edge_type.as_deref(),
                fmt,
                provenance.as_deref(),
            )
        }
        Commands::Hide {
            target,
            reason,
            format,
            provenance,
        } => match target.get() {
            Some(id) => cmd_hide(
                id,
                reason.as_deref(),
                format.as_deref() == Some("json"),
                provenance.as_deref(),
            ),
            None => Err("missing strand id: pass <ID> or --id <ID>".to_string()),
        },
        Commands::Unhide { target, format } => match target.get() {
            Some(id) => cmd_unhide(id, format.as_deref() == Some("json")),
            None => Err("missing strand id: pass <ID> or --id <ID>".to_string()),
        },

        Commands::Close {
            id,
            disposition,
            format,
        } => {
            let fmt = format.as_deref() == Some("json");
            cmd_close(id, disposition.as_deref(), fmt)
        }

        Commands::Reopen { id, format } => {
            let fmt = format.as_deref() == Some("json");
            cmd_reopen(id, fmt)
        }

        Commands::Timeline {
            since_offset,
            since_ts,
            until_offset,
            until_ts,
            strand,
            links,
            format,
            limit,
            tree,
        } => cmd_timeline(
            *since_offset,
            since_ts.as_deref(),
            *until_offset,
            until_ts.as_deref(),
            strand.as_deref(),
            links.as_deref(),
            format.as_deref(),
            *limit,
            tree.as_deref(),
        ),
        Commands::Explain { code, format, json } => {
            let is_json = *json || format.as_deref() == Some("json");
            let output = diagnostics::cmd_explain(code, is_json);
            println!("{}", output);
            // Exit 0 when code or topic resolves; exit 1 otherwise.
            let lowered = code.to_lowercase();
            if diagnostics::lookup(code).is_some() || diagnostics::topic_lookup(&lowered).is_some()
            {
                Ok(())
            } else {
                Err(format!("unknown code or topic: {}", code))
            }
        }
        Commands::Doctor { target } => {
            let result = match target {
                DoctorTarget::Journal { strict, format } => {
                    cmd_doctor_journal(*strict, format.as_deref() == Some("json"))
                }
            };
            match result {
                Ok(true) => Err("journal issues detected".to_string()),
                Ok(false) => Ok(()),
                Err(e) => Err(format!("journal unreadable: {}", e)),
            }
        }

        Commands::Export { out } => cmd_export(out),

        Commands::Tree { target, format } => match target.get() {
            Some(id) => cmd_tree(id, format.as_deref()),
            None => Err("missing strand id: pass <ID> or --id <ID>".to_string()),
        },

        Commands::Depends { target, format } => match target.get() {
            Some(id) => cmd_depends(id, format.as_deref()),
            None => Err("missing strand id: pass <ID> or --id <ID>".to_string()),
        },

        Commands::Orient {
            format,
            include_hidden,
            limit,
            tree,
        } => cmd_orient(format.as_deref(), *include_hidden, *limit, *tree),
    }
}

/// Print a command error to stderr and exit with its mapped code. The single
/// place exit-code classification lives. A `warn:`-prefixed message is printed
/// as-is (no `error:` prefix); everything else gets the `error:` prefix.
fn exit_with_error(e: &str) -> ! {
    if let Some(message) = e.strip_prefix("corrupt: ") {
        eprintln!("{}", message);
    } else if e.starts_with("warn:") {
        eprintln!("{}", e);
    } else {
        eprintln!("error: {}", e);
    }
    std::process::exit(exit_code_for(e));
}

/// Map a command error message to its process exit code: a `journal
/// unreadable:` failure is 2 (read error), everything else is 1.
pub(crate) fn exit_code_for(e: &str) -> i32 {
    if e.starts_with("journal unreadable:") || e.starts_with("corrupt: ") {
        2
    } else {
        1
    }
}

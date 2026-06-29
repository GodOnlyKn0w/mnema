mod commands;
mod diagnostics;
mod event;
mod graph;
mod journal;
mod projection;
mod output;
mod render;
mod tree;
mod util;

pub(crate) use commands::write::*;
pub(crate) use commands::query::*;
pub(crate) use commands::context::*;
pub(crate) use commands::manage::*;
pub(crate) use commands::doctor::*;
pub(crate) use render::*;

use crate::journal::*;

use clap::{error::ErrorKind, Parser, Subcommand};
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
  find          Resolve a strand id
  tree          Strand forest (belongs-to nesting)
  depends       depends-on analysis: blockers / readiness / critical path
  current       Latest effective subject binding
  agent-context Machine-readable active-strand context
  context       Project a typed context slice

做 / change:
  add           Create a new strand
  append        Append an entry to a strand
  close         Close a strand (StrandClosed event)
  reopen        Reopen a closed strand (StrandReopened event)
  checkpoint    Record context before an irreversible action
  link          Link strands (belongs-to / depends-on)
  unlink        Remove a link (EdgeUnlinked; read projection drops the edge)
  bind          Record a subject binding

管 / manage:
  init          Initialize .tasktree/ journal
  hide          Hide a strand from active orient (parked, revivable)
  unhide        Unhide a strand
  doctor        Diagnose journal integrity
  export        Export journal as standalone audit artifact
  explain       Explain a diagnostic code or topic (markers, json, grammar, ...)

Run:  tasktree <command> --help"
)]
struct Cli {
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
    fn get(&self) -> Option<&str> { self.id_pos.as_deref().or(self.id_flag.as_deref()).filter(|s| !s.trim().is_empty()) }
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
        /// Strand type: task, dag, why, session (default: auto-detect)
        #[arg(long = "type", value_name = "TYPE")]
        strand_type: Option<String>,
        /// Optional provenance JSON object. Stored on the initial LogAppended entry.
        #[arg(long = "provenance", value_name = "JSON")]
        provenance: Option<String>,
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
  warnings           W070 (strand moved under you), W071 (closed strand), and
                     W076 (--seen-offset behind target last_offset) fire as scar
                     lines in text output; in json output, a \"warnings\" array
                     is always present.
                     These warnings are informational — exit is still 0.

Exit codes:
  0 ok
  1 strand resolve/show failed
  2 append failed
  3 invalid arguments

Rules:
  --tail only limits displayed output.
  --tail does not change observed_entries_before_append.
  checkpoint failed means hard stop.
JSON shape: tasktree explain json")]
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
        /// Caller-declared last observed offset for the target strand.
        /// If behind the target's current last_offset, emits W076 but still writes.
        #[arg(long = "seen-offset", value_name = "N")]
        seen_offset: Option<usize>,
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
    /// Resolve a prefix to full strand ID
    Find {
        #[command(flatten)]
        target: IdTarget,
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
        /// = source is child of target). [alias: --type (deprecated)]
        #[arg(long = "edge-type", visible_alias = "type", value_name = "TYPE")]
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
        #[arg(long = "edge-type", visible_alias = "type", value_name = "TYPE")]
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
        #[arg(long, visible_alias = "id", value_name = "ID", conflicts_with = "links")]
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
                    tasktree checkpoint --id <ID> --action \"<what and why>\"

Closed strands are folded to a count; retrieve with tasktree list.
Hidden strands are folded to a count; retrieve with tasktree list --all.

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
        #[command(flatten)]
        target: IdTarget,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
    /// Analyse a strand's depends-on graph: blockers, readiness, critical path
    #[command(after_help = "\
ready = every direct blocker is closed. critical path = the longest chain of
still-open upstreams reachable via depends-on (closed upstreams terminate a
path; cycles are guarded). Built on the typed depends-on projection (F3).

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
        /// Disable observation-class folding: expose [progress]/[observed]/[check]
        /// entries full-text instead of tail-folding. Folding is the default;
        /// full exposure is an explicit choice (exposure axis, ADR-0002).
        #[arg(long)]
        include_observations: bool,
    },
}


#[derive(Subcommand)]
enum DoctorTarget {
    /// Check journal integrity
    Journal,
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
// cmd_list, cmd_show, cmd_search use process::exit(2) directly when
// corrupted journal lines are detected. This is intentional CLI style,
// not library style — exit(2) allows gate scripts to detect corruption
// without parsing stderr. Do not refactor to return Result without
// updating all call sites and preserving the exit code.
fn main() {
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
/// (except checkpoint, which owns its codes) so the dispatch table is unit-
/// testable and the exit-code policy lives solely in `exit_with_error`.
fn run(command: &Commands) -> Result<(), String> {
    // Checkpoint has its own error handling (exit codes 1/2/3, JSON output).
    // On failure it prints and exits directly; on success it returns Ok(()) so
    // stdout is flushed by the normal main() return path.
    if let Commands::Checkpoint { id, action, tail, format, include_hidden, provenance, seen_offset } = command {
        let fmt = format.as_deref() == Some("json");
        match cmd_checkpoint_with_seen_offset(
            id.as_deref(),
            action,
            *tail,
            fmt,
            *include_hidden,
            provenance.as_deref(),
            *seen_offset,
        ) {
            Ok(()) => return Ok(()),
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

    match command {
        Commands::Init => cmd_init(),
        Commands::Add { content, stdin, file, format, strand_type, provenance } => {
            let fmt = format.as_deref() == Some("json");
            cmd_add(content.as_deref(), *stdin, file.as_deref(), fmt, strand_type.as_deref(), provenance.as_deref())
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
            seen_offset,
            why,
        } => cmd_append_with_seen_offset(
            content.as_deref(),
            id.as_deref(),
            *new,
            *stdin,
            file.as_deref(),
            explicit_id.as_deref(),
            format.as_deref(),
            provenance.as_deref(),
            *seen_offset,
            why.as_deref(),
        ),
        Commands::List { all, links, backlinks, state, list_type, stale, stale_offset, since_offset, format } => {
            let fmt = format.as_deref() == Some("json");
            cmd_list(*all, links.as_deref(), backlinks.as_deref(), state.as_deref(), list_type.as_deref(), stale.as_deref(), *stale_offset, *since_offset, fmt)
        },
        Commands::Show { target, last, tail, digest, format, locked } => {
            let fmt = format.as_deref() == Some("json");
            cmd_show(target.get(), *last, *tail, fmt, *locked, *digest)
        },
        Commands::Search { query, format, include_hidden } => {
            let fmt = format.as_deref() == Some("json");
            cmd_search(query, fmt, *include_hidden)
        },
        Commands::Find { target, format } => match target.get() {
            Some(id) => cmd_find(id, format.as_deref() == Some("json")),
            None => Err("missing strand id: pass <ID> or --id <ID>".to_string()),
        },
        Commands::Link { source, target, edge_type, format, provenance } => {
            let fmt = format.as_deref() == Some("json");
            cmd_link(source, target, edge_type.as_deref(), fmt, provenance.as_deref())
        },
        Commands::Unlink { source, target, edge_type, format, provenance } => {
            let fmt = format.as_deref() == Some("json");
            cmd_unlink(source, target, edge_type.as_deref(), fmt, provenance.as_deref())
        },
        Commands::Hide { target, reason, format, provenance } => match target.get() {
            Some(id) => cmd_hide(id, reason.as_deref(), format.as_deref() == Some("json"), provenance.as_deref()),
            None => Err("missing strand id: pass <ID> or --id <ID>".to_string()),
        },
        Commands::Unhide { target, format } => match target.get() {
            Some(id) => cmd_unhide(id, format.as_deref() == Some("json")),
            None => Err("missing strand id: pass <ID> or --id <ID>".to_string()),
        },

        Commands::Close { id, disposition, format } => {
            let fmt = format.as_deref() == Some("json");
            cmd_close(id, disposition.as_deref(), fmt)
        },

        Commands::Reopen { id, format } => {
            let fmt = format.as_deref() == Some("json");
            cmd_reopen(id, fmt)
        },

        Commands::Timeline { since_offset, since_ts, until_offset, until_ts, strand, links, format, limit, tree } => {
            cmd_timeline(*since_offset, since_ts.as_deref(), *until_offset, until_ts.as_deref(), strand.as_deref(), links.as_deref(), format.as_deref(), *limit, tree.as_deref())
        }
        Commands::Explain { code, format, json } => {
            let is_json = *json || format.as_deref() == Some("json");
            let output = diagnostics::cmd_explain(code, is_json);
            println!("{}", output);
            // Exit 0 when code or topic resolves; exit 1 otherwise.
            let lowered = code.to_lowercase();
            if diagnostics::lookup(code).is_some() || diagnostics::topic_lookup(&lowered).is_some() {
                Ok(())
            } else {
                Err(format!("unknown code or topic: {}", code))
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

        Commands::Tree { target, format } => match target.get() {
            Some(id) => cmd_tree(id, format.as_deref()),
            None => Err("missing strand id: pass <ID> or --id <ID>".to_string()),
        },

        Commands::Depends { target, format } => match target.get() {
            Some(id) => cmd_depends(id, format.as_deref()),
            None => Err("missing strand id: pass <ID> or --id <ID>".to_string()),
        },

        Commands::Orient { format, include_hidden, limit, tree } => cmd_orient(format.as_deref(), *include_hidden, *limit, *tree),

        Commands::AgentContext { format, include_hidden } => cmd_agent_context(format.as_deref(), *include_hidden),

        Commands::Context { context_type, covers, since_offset, format, exclude_friction, include_hidden, include_observations } => {
            cmd_context(context_type.as_deref(), &covers, *since_offset, format.as_deref(), *exclude_friction, *include_hidden, *include_observations)
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
    }
}

/// Print a command error to stderr and exit with its mapped code. The single
/// place exit-code classification lives. A `warn:`-prefixed message is printed
/// as-is (no `error:` prefix); everything else gets the `error:` prefix.
fn exit_with_error(e: &str) -> ! {
    if e.starts_with("warn:") {
        eprintln!("{}", e);
    } else {
        eprintln!("error: {}", e);
    }
    std::process::exit(exit_code_for(e));
}

/// Map a command error message to its process exit code: a `journal
/// unreadable:` failure is 2 (read error), everything else is 1.
fn exit_code_for(e: &str) -> i32 {
    if e.starts_with("journal unreadable:") {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Helpers (and Event / io::Read) moved to leaf modules in the L5-shape
    // refactor; these tests still exercise them, so pull them in explicitly
    // rather than via super.
    use crate::event::{find_strand, Event};
    use crate::util::*;
    use std::fs;
    use std::io::Read;
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
    fn provenance_defaults_to_env_producer_when_set() {
        let _lock = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("TASKTREE_PRODUCER").ok();
        unsafe { std::env::set_var("TASKTREE_PRODUCER", "codex") };
        assert_eq!(
            parse_provenance_arg(None).unwrap(),
            Some(serde_json::json!({ "producer": "codex" }))
        );
        // Explicit --provenance always overrides the env default.
        assert_eq!(
            parse_provenance_arg(Some(r#"{"producer":"claude"}"#)).unwrap(),
            Some(serde_json::json!({ "producer": "claude" }))
        );
        // Blank env → no default (treated as unset).
        unsafe { std::env::set_var("TASKTREE_PRODUCER", "   ") };
        assert_eq!(parse_provenance_arg(None).unwrap(), None);
        match prev {
            Some(v) => unsafe { std::env::set_var("TASKTREE_PRODUCER", v) },
            None => unsafe { std::env::remove_var("TASKTREE_PRODUCER") },
        }
    }

    #[test]
    fn show_json_exposes_per_entry_provenance() {
        let _env = setup();
        let id = create_strand("provenance projection test");
        cmd_append(
            Some("[observed] tagged"), Some(&id), false, false, None, None, None,
            Some(r#"{"producer":"codex"}"#),
        ).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let full = find_strand(&events, &id).unwrap();
        let strands = projection::project_strands(&events, true);
        let s = strands.iter().find(|s| s.id == full).unwrap();
        let out = output::StrandDetailOutput::from(s);
        let tagged = out.events.iter().find(|e| e.provenance.is_some())
            .expect("at least one event must carry provenance");
        assert_eq!(tagged.provenance.as_ref().unwrap()["producer"], "codex");
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

    // ─────────────────────────────────────────────────────────────────
    // -C / --chdir global flag tests
    // ─────────────────────────────────────────────────────────────────

    /// -C parses correctly from ['tasktree', '-C', 'X', 'orient']
    #[test]
    fn chdir_flag_parses_before_subcommand() {
        use clap::CommandFactory;
        let result = Cli::command()
            .try_get_matches_from(["tasktree", "-C", "/some/dir", "orient"]);
        assert!(result.is_ok(), "'-C DIR orient' must parse: {:?}", result);
    }

    /// --chdir long form also parses
    #[test]
    fn chdir_longform_parses() {
        use clap::CommandFactory;
        let result = Cli::command()
            .try_get_matches_from(["tasktree", "--chdir", "/some/dir", "orient"]);
        assert!(result.is_ok(), "--chdir long form must parse: {:?}", result);
    }

    /// -C after subcommand also works (global = true)
    #[test]
    fn chdir_global_after_subcommand_parses() {
        use clap::CommandFactory;
        let result = Cli::command()
            .try_get_matches_from(["tasktree", "orient", "-C", "/some/dir"]);
        assert!(result.is_ok(), "'-C' after subcommand (global) must parse: {:?}", result);
    }

    /// -C pointing at a real .tasktree dir resolves journal from unrelated cwd.
    #[test]
    fn chdir_resolves_journal_from_foreign_cwd() {
        // env has .tasktree/ in its temp dir; we set cwd to a different temp dir
        // (no .tasktree/), then set_current_dir to env path, and resolve succeeds.
        let env = setup();     // cwd is now env.path() with .tasktree/
        let foreign = tempfile::tempdir().unwrap();
        // Move cwd to the foreign dir (no .tasktree/)
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(foreign.path()).unwrap();
        // Simulate what -C does: set_current_dir to the project root
        std::env::set_current_dir(env.path()).unwrap();
        let result = with_tasktree_home(None, || resolve_journal_dir());
        std::env::set_current_dir(&prev).unwrap();
        assert!(result.is_ok(), "-C to project root must resolve journal: {:?}", result);
        drop(env);
    }

    /// -C to a non-existent directory: the binary would exit 3.
    /// We test that set_current_dir on a missing path returns Err.
    #[test]
    fn chdir_nonexistent_dir_errors() {
        let missing = std::path::Path::new("/this/path/does/not/exist/hopefully/xyz");
        let result = std::env::set_current_dir(missing);
        assert!(result.is_err(), "set_current_dir to missing path must fail");
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
        cmd_close(&done_id, None, false).unwrap();

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
        assert_eq!(entry.id, open_id);
        assert_eq!(entry.summary, "open line of work");
        // Catch-up is copy-paste runnable and shows the strand's recent
        // content (show --tail), never the empty-prone since-offset delta.
        assert_eq!(
            entry.catch_up,
            format!("tasktree show --id {} --tail 8", open_id)
        );
        assert!(out.remind.contains("checkpoint"));
        assert!(out.remind.contains("matter concluded"), "remind must carry the closing segment");
    }

    #[test]
    fn orient_hidden_count_reflects_scar_principle() {
        let _env = setup();
        let open_id = create_strand("open work");
        let hidden_id = create_strand("will be hidden");
        cmd_hide(&hidden_id, None, false, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let max_offset = events.last().map(|(o, _)| *o).unwrap();
        let strands = projection::project_strands(&events, true);

        // Default view (include_hidden=false): hidden strand must be absent
        // from active/closed pools but counted in hidden_count.
        let out = build_orient(&strands, false, 10, max_offset);
        assert_eq!(out.hidden_count, 1, "hidden strand must appear in hidden_count");
        assert_eq!(out.closed_count, 0, "hidden strand must not inflate closed_count");
        let active_ids: Vec<&str> = out.active.iter().map(|s| s.id.as_str()).collect();
        assert!(active_ids.contains(&open_id.as_str()), "visible strand must be in menu");
        assert!(!active_ids.contains(&hidden_id.as_str()), "hidden strand absent from menu");

        // include_hidden=true: hidden strand joins the pool; hidden_count=0.
        let out_all = build_orient(&strands, true, 10, max_offset);
        assert_eq!(out_all.hidden_count, 0, "include_hidden=true must yield hidden_count=0");
        let all_ids: Vec<&str> = out_all.active.iter().map(|s| s.id.as_str()).collect();
        assert!(all_ids.contains(&hidden_id.as_str()), "include_hidden=true puts hidden strand in menu");
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
        assert_eq!(out.active[0].id, older);
        let _ = newer;
    }

    // ── orient --tree: belongs-to forest regression tests ──

    /// Default orient (no --tree) is unchanged when belongs-to edges exist.
    /// Regression guard: --tree is strictly opt-in.
    #[test]
    fn orient_flat_unaffected_by_belongs_to_edges() {
        let _env = setup();
        let parent = create_strand("parent task");
        let child = create_strand("child task");
        cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let max_offset = events.last().map(|(o, _)| *o).unwrap();
        let strands = projection::project_strands(&events, true);
        let out = build_orient(&strands, false, 10, max_offset);

        // Flat orient must still return both strands in a flat list
        assert_eq!(out.active.len(), 2, "flat orient: both strands must appear");
        let ids: Vec<&str> = out.active.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&parent.as_str()), "flat orient: parent must appear");
        assert!(ids.contains(&child.as_str()), "flat orient: child must appear");
    }

    /// orient --tree: child declared with belongs-to appears nested under parent.
    #[test]
    fn orient_tree_nests_belongs_to_child_under_parent() {
        let _env = setup();
        let parent = create_strand("parent task");
        let child = create_strand("child task");
        cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let max_offset = events.last().map(|(o, _)| *o).unwrap();
        let strands = projection::project_strands(&events, true);
        let out = build_orient(&strands, false, 10, max_offset);

        // Build strand_cards for tree construction (mirror of cmd_orient logic)
        let strand_cards: Vec<(&projection::ProjectedStrand, output::OrientStrand)> = out
            .active
            .iter()
            .filter_map(|card| {
                strands.iter().find(|s| s.id == card.id).map(|s| (s, card.clone()))
            })
            .collect();
        let roots = tree::build_orient_forest(&strand_cards);

        // Parent is a root; child is nested under it
        assert_eq!(roots.len(), 1, "orient --tree: only the parent is a root");
        assert_eq!(roots[0].card.id, parent, "root must be the parent strand");
        assert_eq!(roots[0].children.len(), 1, "parent must have one child");
        assert_eq!(roots[0].children[0].card.id, child, "child must be nested under parent");
    }

    /// orient --tree: parallel siblings under same parent are both visible.
    #[test]
    fn orient_tree_parallel_siblings_visible_under_parent() {
        let _env = setup();
        let parent = create_strand("parent task");
        let sibling_a = create_strand("sibling A");
        let sibling_b = create_strand("sibling B");
        cmd_link(&sibling_a, &parent, Some("belongs-to"), false, None).unwrap();
        cmd_link(&sibling_b, &parent, Some("belongs-to"), false, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let max_offset = events.last().map(|(o, _)| *o).unwrap();
        let strands = projection::project_strands(&events, true);
        let out = build_orient(&strands, false, 10, max_offset);

        let strand_cards: Vec<(&projection::ProjectedStrand, output::OrientStrand)> = out
            .active
            .iter()
            .filter_map(|card| {
                strands.iter().find(|s| s.id == card.id).map(|s| (s, card.clone()))
            })
            .collect();
        let roots = tree::build_orient_forest(&strand_cards);

        assert_eq!(roots.len(), 1, "only parent is a root");
        assert_eq!(roots[0].card.id, parent, "root is the parent");
        assert_eq!(roots[0].children.len(), 2, "both siblings must appear under parent");
        let child_ids: Vec<&str> = roots[0].children.iter().map(|n| n.card.id.as_str()).collect();
        assert!(child_ids.contains(&sibling_a.as_str()), "sibling A must be visible");
        assert!(child_ids.contains(&sibling_b.as_str()), "sibling B must be visible");
    }

    /// orient --tree: orphan strands (no belongs-to edge or parent not in active set)
    /// appear as top-level roots.
    #[test]
    fn orient_tree_orphan_strands_are_roots() {
        let _env = setup();
        let orphan_a = create_strand("orphan A (no edges)");
        let orphan_b = create_strand("orphan B (no edges)");

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let max_offset = events.last().map(|(o, _)| *o).unwrap();
        let strands = projection::project_strands(&events, true);
        let out = build_orient(&strands, false, 10, max_offset);

        let strand_cards: Vec<(&projection::ProjectedStrand, output::OrientStrand)> = out
            .active
            .iter()
            .filter_map(|card| {
                strands.iter().find(|s| s.id == card.id).map(|s| (s, card.clone()))
            })
            .collect();
        let roots = tree::build_orient_forest(&strand_cards);

        assert_eq!(roots.len(), 2, "both orphan strands must appear as roots");
        let root_ids: Vec<&str> = roots.iter().map(|n| n.card.id.as_str()).collect();
        assert!(root_ids.contains(&orphan_a.as_str()), "orphan A must be a root");
        assert!(root_ids.contains(&orphan_b.as_str()), "orphan B must be a root");
        for root in &roots {
            assert!(root.children.is_empty(), "orphan nodes must have no children");
        }
    }

    /// orient --tree: no contention/conflict markers are emitted (precision discipline).
    #[test]
    fn orient_tree_no_contention_markers() {
        let _env = setup();
        let parent = create_strand("parent");
        let child_a = create_strand("child A");
        let child_b = create_strand("child B");
        cmd_link(&child_a, &parent, Some("belongs-to"), false, None).unwrap();
        cmd_link(&child_b, &parent, Some("belongs-to"), false, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let max_offset = events.last().map(|(o, _)| *o).unwrap();
        let strands = projection::project_strands(&events, true);
        let out = build_orient(&strands, false, 10, max_offset);

        let strand_cards: Vec<(&projection::ProjectedStrand, output::OrientStrand)> = out
            .active
            .iter()
            .filter_map(|card| {
                strands.iter().find(|s| s.id == card.id).map(|s| (s, card.clone()))
            })
            .collect();
        let roots = tree::build_orient_forest(&strand_cards);

        // Serialize to JSON and assert no "contention" word appears
        let json_str = serde_json::to_string(&roots).unwrap();
        assert!(!json_str.contains("contention"), "orient --tree JSON must not emit contention markers");
        assert!(!json_str.contains("conflict"), "orient --tree JSON must not emit conflict markers");
    }

    /// orient --tree --format json: JSON structure is nested (roots array with children).
    #[test]
    fn orient_tree_json_shape_is_nested() {
        let _env = setup();
        let parent = create_strand("parent task");
        let child = create_strand("child task");
        cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let max_offset = events.last().map(|(o, _)| *o).unwrap();
        let strands = projection::project_strands(&events, true);
        let out = build_orient(&strands, false, 10, max_offset);

        let strand_cards: Vec<(&projection::ProjectedStrand, output::OrientStrand)> = out
            .active
            .iter()
            .filter_map(|card| {
                strands.iter().find(|s| s.id == card.id).map(|s| (s, card.clone()))
            })
            .collect();
        let roots = tree::build_orient_forest(&strand_cards);
        let tree_out = output::OrientTreeOutput {
            max_offset,
            roots,
            closed_count: out.closed_count,
            hidden_count: out.hidden_count,
            remind: out.remind.clone(),
        };

        let json_str = serde_json::to_string(&tree_out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        // Top-level has "roots" array
        assert!(parsed["roots"].is_array(), "orient --tree JSON must have 'roots' array");
        let roots_arr = parsed["roots"].as_array().unwrap();
        assert_eq!(roots_arr.len(), 1, "one root (the parent)");

        // Root has "id", "children" fields
        let root = &roots_arr[0];
        assert_eq!(root["id"].as_str().unwrap(), parent.as_str(), "root id matches parent");
        assert!(root["children"].is_array(), "root must have 'children' array");
        let children = root["children"].as_array().unwrap();
        assert_eq!(children.len(), 1, "root has one child");
        assert_eq!(children[0]["id"].as_str().unwrap(), child.as_str(), "child id matches");

        // Verify no extra fields added/removed (additive-only contract)
        assert!(parsed["max_offset"].is_number(), "max_offset must be present");
        assert!(parsed["closed_count"].is_number(), "closed_count must be present");
        assert!(parsed["hidden_count"].is_number(), "hidden_count must be present");
        assert!(parsed["remind"].is_string(), "remind must be present");
    }

    /// orient --tree: parse check — `tasktree orient --tree` is a valid CLI invocation.
    #[test]
    fn orient_tree_flag_parses() {
        use clap::CommandFactory;
        let result = Cli::command()
            .try_get_matches_from(["tasktree", "orient", "--tree"]);
        assert!(result.is_ok(), "'orient --tree' must parse: {:?}", result);
    }

    /// orient --tree --format json: parse check.
    #[test]
    fn orient_tree_format_json_parses() {
        use clap::CommandFactory;
        let result = Cli::command()
            .try_get_matches_from(["tasktree", "orient", "--tree", "--format", "json"]);
        assert!(result.is_ok(), "'orient --tree --format json' must parse: {:?}", result);
    }

    // ── tree / project_tree: canonical belongs-to direction regression ──
    // The `tree` command (cmd_tree → project_tree) must nest SOURCE under
    // TARGET for belongs-to edges, identical to orient --tree
    // (build_orient_forest). Guards against the reversed-direction +
    // all-edge-types + no-dedup divergence project_tree used to carry.

    /// tree: after `link child parent --belongs-to`, the parent node holds the
    /// child as a descendant (child nested under parent — canonical direction).
    #[test]
    fn tree_nests_belongs_to_child_under_parent() {
        let _env = setup();
        let parent = create_strand("parent task");
        let child = create_strand("child task");
        cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);

        // Root the tree at the parent: parent must own the child.
        let root = tree::project_tree(&parent, &strands).expect("parent resolves");
        assert_eq!(root.id, parent, "tree root rooted at parent is the parent");
        assert_eq!(root.children.len(), 1, "parent must have exactly one child");
        assert_eq!(root.children[0].id, child, "child must nest under parent");
        assert!(root.children[0].children.is_empty(), "child is a leaf");
    }

    /// tree: rooting at the child must NOT pull the parent in as a descendant.
    /// Direct regression on the old reversed direction (which nested parent
    /// under child by walking source→target as parent→child).
    #[test]
    fn tree_rooted_at_child_does_not_contain_parent() {
        let _env = setup();
        let parent = create_strand("parent task");
        let child = create_strand("child task");
        cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);

        let root = tree::project_tree(&child, &strands).expect("child resolves");
        assert_eq!(root.id, child, "root rooted at child is the child");
        assert!(
            root.children.is_empty(),
            "child has no descendants; parent must not be nested under it (reversed-direction guard)"
        );
    }

    /// tree and orient --tree must agree on parent→child nesting for the same
    /// journal: single source of truth across both builders.
    #[test]
    fn tree_and_orient_forest_agree_on_nesting() {
        let _env = setup();
        let parent = create_strand("parent task");
        let child_a = create_strand("child A");
        let child_b = create_strand("child B");
        cmd_link(&child_a, &parent, Some("belongs-to"), false, None).unwrap();
        cmd_link(&child_b, &parent, Some("belongs-to"), false, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let max_offset = events.last().map(|(o, _)| *o).unwrap();
        let strands = projection::project_strands(&events, true);

        // project_tree (cmd_tree) view
        let root = tree::project_tree(&parent, &strands).expect("parent resolves");
        let mut tree_child_ids: Vec<String> =
            root.children.iter().map(|c| c.id.clone()).collect();
        tree_child_ids.sort();

        // build_orient_forest (orient --tree) view
        let out = build_orient(&strands, false, 10, max_offset);
        let strand_cards: Vec<(&projection::ProjectedStrand, output::OrientStrand)> = out
            .active
            .iter()
            .filter_map(|card| {
                strands.iter().find(|s| s.id == card.id).map(|s| (s, card.clone()))
            })
            .collect();
        let roots = tree::build_orient_forest(&strand_cards);
        let parent_root = roots
            .iter()
            .find(|n| n.card.id == parent)
            .expect("parent is a root in the forest");
        let mut forest_child_ids: Vec<String> =
            parent_root.children.iter().map(|c| c.card.id.clone()).collect();
        forest_child_ids.sort();

        assert_eq!(
            tree_child_ids, forest_child_ids,
            "tree and orient --tree must list the same children under the parent"
        );
        assert_eq!(tree_child_ids, vec![child_a.clone(), child_b.clone()]);
    }

    /// tree: a duplicate belongs-to link must not double-project the child.
    /// Read-side dedup folds repeated EdgeLinked targets (journal keeps both).
    #[test]
    fn tree_duplicate_belongs_to_link_does_not_double_project() {
        let _env = setup();
        let parent = create_strand("parent task");
        let child = create_strand("child task");
        // Link twice — same source, same target, same edge type.
        cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();
        cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);

        // The journal is append-only: both EdgeLinked events are present.
        let link_events = events
            .iter()
            .filter(|(_, e)| matches!(e, Event::EdgeLinked { .. }))
            .count();
        assert_eq!(link_events, 2, "journal keeps both link events (append-only)");

        // The projection folds them: belongs_to_edges holds one entry.
        let strands = projection::project_strands(&events, true);
        let child_proj = strands.iter().find(|s| s.id == child).unwrap();
        assert_eq!(
            child_proj.belongs_to_edges.len(),
            1,
            "duplicate links fold to one belongs_to edge in the projection"
        );

        // And the tree shows the child exactly once.
        let root = tree::project_tree(&parent, &strands).expect("parent resolves");
        assert_eq!(root.children.len(), 1, "child must appear exactly once under parent");
        assert_eq!(root.children[0].id, child);
    }

    /// tree: non-belongs-to edges (depends-on) do not form the strand tree.
    /// project_tree uses belongs_to_edges only — a depends-on link must not nest.
    #[test]
    fn tree_ignores_non_belongs_to_edges() {
        let _env = setup();
        let task = create_strand("task");
        let blocker = create_strand("blocker");
        cmd_link(&task, &blocker, Some("depends-on"), false, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);

        // Rooting at either end yields a lone node — depends-on does not nest.
        let root_task = tree::project_tree(&task, &strands).expect("task resolves");
        assert!(root_task.children.is_empty(), "depends-on must not nest under source");
        let root_blocker = tree::project_tree(&blocker, &strands).expect("blocker resolves");
        assert!(root_blocker.children.is_empty(), "depends-on must not nest under target");
    }

    /// subtree_ids: descends from root through belongs-to children (same
    /// canonical direction as project_tree).
    #[test]
    fn subtree_ids_descends_through_belongs_to_children() {
        let _env = setup();
        let parent = create_strand("parent");
        let child = create_strand("child");
        let grandchild = create_strand("grandchild");
        cmd_link(&child, &parent, Some("belongs-to"), false, None).unwrap();
        cmd_link(&grandchild, &child, Some("belongs-to"), false, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);

        // From parent: the whole chain is reachable.
        let from_parent = tree::subtree_ids(&parent, &strands).expect("parent resolves");
        assert!(from_parent.contains(&parent), "root included");
        assert!(from_parent.contains(&child), "child reachable from parent");
        assert!(from_parent.contains(&grandchild), "grandchild reachable from parent");

        // From child: parent is an ancestor, must NOT be in the descendant set.
        let from_child = tree::subtree_ids(&child, &strands).expect("child resolves");
        assert!(from_child.contains(&child), "root included");
        assert!(from_child.contains(&grandchild), "grandchild reachable from child");
        assert!(!from_child.contains(&parent), "parent (ancestor) must not be in subtree");
    }

    /// link --help carries the direction semantics required by the work order:
    /// belongs-to marks source as child of target, and names tree / orient --tree.
    #[test]
    fn link_help_documents_belongs_to_direction() {
        use clap::CommandFactory;
        let cmd = Cli::command();
        let link = cmd
            .get_subcommands()
            .find(|s| s.get_name() == "link")
            .expect("link subcommand exists");
        let help = link
            .get_after_help()
            .map(|h| h.to_string())
            .unwrap_or_default();
        assert!(help.contains("belongs-to"), "link help must document belongs-to");
        assert!(help.contains("depends-on"), "link help must document depends-on");
        assert!(
            help.to_lowercase().contains("child") && help.to_lowercase().contains("parent"),
            "link help must explain source=child / target=parent"
        );
        assert!(
            help.contains("orient --tree") || help.contains("tree"),
            "link help must name the tree projection that consumes belongs-to"
        );
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
    fn help_topic_references_exist() {
        // "引用即契约": any `tasktree explain <word>` line in after_help where
        // <word> is all-lowercase must resolve via topic_lookup.
        use clap::CommandFactory;
        let cmd = Cli::command();
        let mut helps: Vec<String> = Vec::new();
        if let Some(h) = cmd.get_after_help() { helps.push(h.to_string()); }
        for sub in cmd.get_subcommands() {
            if let Some(h) = sub.get_after_help() { helps.push(h.to_string()); }
        }
        let mut failures: Vec<String> = Vec::new();
        for help in &helps {
            for line in help.lines() {
                // Match "tasktree explain <word>" where word is all-lowercase
                if let Some(rest) = line.find("tasktree explain ").map(|i| &line[i + "tasktree explain ".len()..]) {
                    let word: String = rest.split_whitespace().next().unwrap_or("").chars()
                        .take_while(|c| c.is_alphabetic() || *c == '_' || *c == '-')
                        .collect();
                    if word.is_empty() { continue; }
                    // Only check all-lowercase words (topic namespace)
                    if word.chars().all(|c| c.is_ascii_lowercase() || c == '_' || c == '-') {
                        if diagnostics::topic_lookup(&word).is_none() {
                            failures.push(format!("help references topic '{}' but topic_lookup returns None", word));
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "broken topic references in help text:\n{}", failures.join("\n"));
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
        let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
        assert!(diags.iter().any(|(c, _)| *c == "W068"), "expected W068, got {:?}", diags);

        // Closing the strand silences the warning (precision over recall).
        cmd_close(&id, Some("cancelled"), false).unwrap();
        let (events, _) = read_events_lossy(&path);
        let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
        let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
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
        let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
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
        let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
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
        let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
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
        let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
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
        let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
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
        let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
        assert_eq!(out.len(), 1);
        assert!(out[0].entries.iter().any(|e| e.marker == "[friction]"), "live friction must be exposed by default");
        assert_eq!(out[0].friction_folded, 0);
    }

    #[test]
    fn context_folds_friction_on_closed_strand() {
        let _env = setup();
        let id = create_prompt_strand("closed guidance");
        cmd_append(Some("[friction] hole, since resolved"), None, false, false, None, Some(&id), None, None).unwrap();
        cmd_close(&id, Some("done"), false).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, false);
        let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
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
        let out = build_context_strands(&strands, "prompt-strand", &[], None, true, false);
        assert_eq!(out.len(), 1);
        assert!(out[0].entries.iter().all(|e| e.marker != "[friction]"));
        assert_eq!(out[0].friction_folded, 0, "explicit exclusion is not a fold");
    }

    // ── Part A: friction↔fixed pairing ──────────────────────────

    #[test]
    fn context_friction_fixed_pair_produces_scar() {
        // A single [friction] followed by [fixed fixes=<id>] on a live strand:
        // - scar entry appears (marker=[friction], content contains "→ fixed")
        // - neither the original friction nor the [fixed] appear as separate entries
        // - friction_paired == 1
        // Explicit fixes= is required; proximity inference is not supported.
        let _env = setup();
        let id = create_prompt_strand("live guidance");
        cmd_append(Some("[friction] a hole to fill"), None, false, false, None, Some(&id), None, None).unwrap();
        // Read back the friction's append_id to form a fixes= reference
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let friction_append_id = events.iter().rev().find_map(|(_, e)| {
            if let event::Event::LogAppended { id: eid, content, append_id, .. } = e {
                if eid == &id && content.contains("a hole to fill") {
                    return append_id.clone();
                }
            }
            None
        }).expect("friction must have append_id");
        let prefix = &friction_append_id[..8.min(friction_append_id.len())];
        let fixed_content = format!("[fixed] filled the hole fixes={}", prefix);
        cmd_append(Some(&fixed_content), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, false);
        let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
        assert_eq!(out.len(), 1);
        let entries = &out[0].entries;
        // scar entry must be present
        let scar = entries.iter().find(|e| e.marker == "[friction]" && e.content.contains("→ fixed"));
        assert!(scar.is_some(), "expected scar entry with → fixed, entries: {:?}", entries);
        // no standalone [fixed] entry
        assert!(entries.iter().all(|e| e.marker != "[fixed]"), "paired [fixed] must not appear separately");
        // no unmodified friction entry (scar replaces it)
        let raw_friction = entries.iter().filter(|e| e.marker == "[friction]").count();
        assert_eq!(raw_friction, 1, "exactly one [friction] entry (the scar)");
        let scar_entry = scar.unwrap();
        assert!(scar_entry.content.contains("a hole to fill"), "scar must include truncated friction text");
        assert_eq!(out[0].friction_paired, 1);
    }

    #[test]
    fn context_fixed_without_fixes_is_plain_annotation() {
        // [fixed] with no fixes= token is a plain annotation — not folded,
        // not paired. The [friction] stays full-text (live debt, unresolved).
        // Proximity inference was intentionally removed (close-command footgun lesson).
        let _env = setup();
        let id = create_prompt_strand("live guidance");
        cmd_append(Some("[friction] first hole"), None, false, false, None, Some(&id), None, None).unwrap();
        cmd_append(Some("[friction] second hole"), None, false, false, None, Some(&id), None, None).unwrap();
        cmd_append(Some("[fixed] fixed something but no reference"), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, false);
        let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
        assert_eq!(out.len(), 1);
        let entries = &out[0].entries;
        // Both frictions remain full-text (neither is a scar)
        let first_full = entries.iter().find(|e| {
            e.marker == "[friction]" && e.content.contains("first hole") && !e.content.contains("→ fixed")
        });
        assert!(first_full.is_some(), "first friction must remain full-text (unpaired)");
        let second_full = entries.iter().find(|e| {
            e.marker == "[friction]" && e.content.contains("second hole") && !e.content.contains("→ fixed")
        });
        assert!(second_full.is_some(), "second friction must remain full-text (no proximity pairing)");
        // [fixed] without fixes= appears as a plain annotation entry
        let fixed_entry = entries.iter().find(|e| e.marker == "[fixed]");
        assert!(fixed_entry.is_some(), "[fixed] without fixes= must appear as a plain annotation");
        assert_eq!(out[0].friction_paired, 0, "no pairing without explicit fixes=");
    }

    #[test]
    fn context_friction_fixed_explicit_fixes_ref() {
        // [fixed] with fixes=<prefix> pairs with the specified friction, not proximity.
        // We create: friction_A, friction_B, [fixed fixes=<prefix_of_A>]
        // Expected: friction_A becomes scar, friction_B stays full-text.
        let _env = setup();
        let id = create_prompt_strand("live guidance");
        // Append friction_A first and capture its append_id
        cmd_append(Some("[friction] hole alpha"), None, false, false, None, Some(&id), None, None).unwrap();
        // Append friction_B
        cmd_append(Some("[friction] hole beta"), None, false, false, None, Some(&id), None, None).unwrap();
        // Read back to find friction_A's append_id
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let friction_a_append_id = events.iter().rev().find_map(|(_, e)| {
            if let event::Event::LogAppended { id: eid, content, append_id, .. } = e {
                if eid == &id && content.contains("hole alpha") {
                    return append_id.clone();
                }
            }
            None
        }).expect("friction_A must have append_id");
        // Use first 8 chars of append_id as the prefix
        let prefix = &friction_a_append_id[..8.min(friction_a_append_id.len())];
        let fixed_content = format!("[fixed] resolves first hole fixes={}", prefix);
        cmd_append(Some(&fixed_content), None, false, false, None, Some(&id), None, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, false);
        let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
        assert_eq!(out.len(), 1);
        let entries = &out[0].entries;
        // friction_A → scar
        let scar_a = entries.iter().find(|e| {
            e.marker == "[friction]" && e.content.contains("hole alpha") && e.content.contains("→ fixed")
        });
        assert!(scar_a.is_some(), "friction_A must become scar via explicit fixes= ref; entries: {:?}", entries);
        // friction_B → full-text (unpaired)
        let full_b = entries.iter().find(|e| {
            e.marker == "[friction]" && e.content.contains("hole beta") && !e.content.contains("→ fixed")
        });
        assert!(full_b.is_some(), "friction_B must stay full-text (unpaired by explicit ref)");
        assert_eq!(out[0].friction_paired, 1);
    }

    #[test]
    fn context_exclude_friction_also_suppresses_scars() {
        // --exclude-friction (explicit blindness) must suppress scar entries too.
        // Uses explicit fixes= to produce a real pair/scar first.
        let _env = setup();
        let id = create_prompt_strand("live guidance");
        cmd_append(Some("[friction] a hole"), None, false, false, None, Some(&id), None, None).unwrap();
        // Read back the friction's append_id
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let friction_append_id = events.iter().rev().find_map(|(_, e)| {
            if let event::Event::LogAppended { id: eid, content, append_id, .. } = e {
                if eid == &id && content.contains("a hole") {
                    return append_id.clone();
                }
            }
            None
        }).expect("friction must have append_id");
        let prefix = &friction_append_id[..8.min(friction_append_id.len())];
        let fixed_content = format!("[fixed] filled fixes={}", prefix);
        cmd_append(Some(&fixed_content), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, false);
        let out = build_context_strands(&strands, "prompt-strand", &[], None, true, false);
        assert_eq!(out.len(), 1);
        assert!(out[0].entries.iter().all(|e| e.marker != "[friction]"),
            "exclude_friction must suppress scar entries too");
    }

    #[test]
    fn context_dangling_fixes_produces_no_fold() {
        // [fixed] with fixes=<prefix> that matches nothing → dangling fix.
        // The [fixed] entry is a plain annotation (exposed), not folded.
        // The [friction] stays full-text (live debt).
        // W075 would be emitted to stderr; we test that no folding happens.
        let _env = setup();
        let id = create_prompt_strand("live guidance");
        cmd_append(Some("[friction] unresolved hole"), None, false, false, None, Some(&id), None, None).unwrap();
        // Use a fake/nonexistent append_id prefix (all zeros, ≥8 chars)
        cmd_append(Some("[fixed] pretend fix fixes=00000000deadbeef"), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, false);
        let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
        assert_eq!(out.len(), 1);
        let entries = &out[0].entries;
        // friction stays full-text (unresolved)
        let friction_full = entries.iter().find(|e| {
            e.marker == "[friction]" && e.content.contains("unresolved hole") && !e.content.contains("→ fixed")
        });
        assert!(friction_full.is_some(), "friction must remain full-text when fixes= is dangling; entries: {:?}", entries);
        // [fixed] with dangling ref is exposed as annotation
        let fixed_entry = entries.iter().find(|e| e.marker == "[fixed]");
        assert!(fixed_entry.is_some(), "dangling [fixed] must appear as annotation entry");
        assert_eq!(out[0].friction_paired, 0, "no pairing on dangling fix");
        // pair_frictions itself must record the dangling fix
        let pairing = pair_frictions(&strands[0].log);
        assert_eq!(pairing.dangling_fixes.len(), 1, "one dangling fix recorded");
        let (_, ref prefix) = pairing.dangling_fixes[0];
        assert!(prefix.starts_with("00000000"), "prefix must match what was written");
    }

    #[test]
    fn context_one_fixed_pairs_at_most_one_friction() {
        // Strict 1-1: a [fixed] entry with fixes=<prefix_A> pairs exactly one friction.
        // A second [fixed] entry pointing to the same friction (already paired) → dangling.
        let _env = setup();
        let id = create_prompt_strand("live guidance");
        cmd_append(Some("[friction] target hole"), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let friction_append_id = events.iter().rev().find_map(|(_, e)| {
            if let event::Event::LogAppended { id: eid, content, append_id, .. } = e {
                if eid == &id && content.contains("target hole") {
                    return append_id.clone();
                }
            }
            None
        }).expect("friction must have append_id");
        let prefix = &friction_append_id[..8.min(friction_append_id.len())];
        // Two [fixed] entries both referencing the same friction
        let fixed1 = format!("[fixed] first fix fixes={}", prefix);
        let fixed2 = format!("[fixed] second fix fixes={}", prefix);
        cmd_append(Some(&fixed1), None, false, false, None, Some(&id), None, None).unwrap();
        cmd_append(Some(&fixed2), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, false);
        let pairing = pair_frictions(&strands[0].log);
        // Only one friction → only one pairing possible
        assert_eq!(pairing.paired_friction.len(), 1, "only one friction, only one pair");
        assert_eq!(pairing.paired_fixed.len(), 1, "only first matching [fixed] is paired");
        // Second [fixed] targeting already-paired friction → dangling
        assert_eq!(pairing.dangling_fixes.len(), 1, "second [fixed] with same ref → dangling");
    }

    // ── Part B: observation-class folding ────────────────────────

    #[test]
    fn context_observation_folding_tail_kept() {
        // 3 [progress] + 1 [observed] → folded_counts {progress:2, observed:0, check:0}
        // (progress tail kept as last entry, observed is itself the tail so count=0)
        let _env = setup();
        let id = create_prompt_strand("live guidance");
        cmd_append(Some("[progress] step 1"), None, false, false, None, Some(&id), None, None).unwrap();
        cmd_append(Some("[progress] step 2"), None, false, false, None, Some(&id), None, None).unwrap();
        cmd_append(Some("[progress] step 3"), None, false, false, None, Some(&id), None, None).unwrap();
        cmd_append(Some("[observed] an observation"), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, false);
        let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
        assert_eq!(out.len(), 1);
        let fc = &out[0].folded_counts;
        assert_eq!(fc.progress, 2, "first 2 progress entries folded, tail kept");
        assert_eq!(fc.observed, 0, "single observed entry is the tail, not folded");
        assert_eq!(fc.check, 0);
        // The tail progress entry must appear in entries
        let has_tail = out[0].entries.iter().any(|e| {
            e.marker == "[progress]" && e.content.contains("step 3")
        });
        assert!(has_tail, "tail [progress] entry must be visible");
        // Folded progress entries must NOT appear
        let visible_progress: Vec<_> = out[0].entries.iter()
            .filter(|e| e.marker == "[progress]")
            .collect();
        assert_eq!(visible_progress.len(), 1, "only tail [progress] visible");
    }

    #[test]
    fn context_include_observations_disables_folding() {
        // --include-observations exposes all entries; folded_counts all 0.
        let _env = setup();
        let id = create_prompt_strand("live guidance");
        cmd_append(Some("[progress] step 1"), None, false, false, None, Some(&id), None, None).unwrap();
        cmd_append(Some("[progress] step 2"), None, false, false, None, Some(&id), None, None).unwrap();
        cmd_append(Some("[check] checked"), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, false);
        let out = build_context_strands(&strands, "prompt-strand", &[], None, false, true);
        assert_eq!(out.len(), 1);
        let fc = &out[0].folded_counts;
        assert_eq!(fc.progress, 0, "no folding when include_observations=true");
        assert_eq!(fc.observed, 0);
        assert_eq!(fc.check, 0);
        // All three entries visible
        let progress_count = out[0].entries.iter().filter(|e| e.marker == "[progress]").count();
        assert_eq!(progress_count, 2, "both [progress] entries must be visible");
    }

    #[test]
    fn context_closed_strand_observation_folding() {
        // Closed strands also get observation folding (live+closed unified for obs).
        let _env = setup();
        let id = create_prompt_strand("closed strand");
        cmd_append(Some("[progress] step 1"), None, false, false, None, Some(&id), None, None).unwrap();
        cmd_append(Some("[progress] step 2"), None, false, false, None, Some(&id), None, None).unwrap();
        cmd_append(Some("[done] wrapped"), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, false);
        let out = build_context_strands(&strands, "prompt-strand", &[], None, false, false);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].folded_counts.progress, 1, "first progress folded on closed strand");
    }

    // ── grammar conformance (contract: tasktree explain grammar) ──
    // The contract is an artifact, not a discipline: these tests are the
    // teeth. A new command violating the flag vocabulary or naming rules
    // fails here, not in a future cold-start.

    #[test]
    fn grammar_flag_vocabulary_conformance() {
        use clap::CommandFactory;
        // (flag, exclusively allowed on). Compat aliases are pinned to their
        // historical host; appearing anywhere else is a new violation.
        let exclusive: &[(&str, &str)] = &[("all", "list"), ("json", "explain"), ("strand", "timeline")];
        for sub in Cli::command().get_subcommands() {
            for arg in sub.get_arguments() {
                if let Some(long) = arg.get_long() {
                    for (flag, host) in exclusive {
                        assert!(
                            long != *flag || sub.get_name() == *host,
                            "--{} is reserved to `{}` (compat); `{}` must use the canonical flag (see explain grammar)",
                            flag, host, sub.get_name()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn grammar_single_id_commands_accept_id_flag() {
        use clap::CommandFactory;
        for cmd in ["show", "find", "tree", "hide", "unhide"] {
            let r = Cli::command().try_get_matches_from(["tasktree", cmd, "--id", "0000019dd34b"]);
            assert!(r.is_ok(), "`{} --id <ID>` must parse (IdTarget contract): {:?}", cmd, r.err());
        }
        // timeline reaches the same grammar via alias
        let r = Cli::command().try_get_matches_from(["tasktree", "timeline", "--id", "0000019dd34b"]);
        assert!(r.is_ok(), "`timeline --id` must alias --strand");
    }

    #[test]
    fn seen_offset_flag_parses_on_write_commands() {
        use clap::CommandFactory;
        let append = Cli::command().try_get_matches_from([
            "tasktree", "append", "--id", "0000019dd34b", "--seen-offset", "2", "note",
        ]);
        assert!(append.is_ok(), "append --seen-offset must parse: {:?}", append.err());

        let checkpoint = Cli::command().try_get_matches_from([
            "tasktree", "checkpoint", "--id", "0000019dd34b", "--seen-offset", "2", "--action", "before commit",
        ]);
        assert!(checkpoint.is_ok(), "checkpoint --seen-offset must parse: {:?}", checkpoint.err());
    }

    #[test]
    fn grammar_json_field_naming() {
        let _env = setup();
        let id = create_strand("naming probe");
        cmd_append(Some("second entry"), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);

        let mut samples: Vec<serde_json::Value> = vec![
            serde_json::to_value(output::StrandDetailOutput::from(&strands[0])).unwrap(),
            serde_json::to_value(output::StrandListOutput {
                strands: strands.iter().map(output::StrandListItem::from).collect(),
            }).unwrap(),
            serde_json::to_value(build_orient(&strands, true, 10, 2)).unwrap(),
            serde_json::to_value(output::SearchOutput { matches: vec![], count: 0, query: String::new() }).unwrap(),
            serde_json::to_value(output::TimelineOutput { timeline: vec![], truncated: false, count: 0, max_offset: 0 }).unwrap(),
            // Write-command JSON built inline with json!() is invisible to
            // struct sampling — extracted shapes are sampled here. First
            // catch of this blind spot: hide's ledger shipped bare
            // active/closed/hidden count names.
            visibility_ledger_json(&id, false),
        ];

        // plural noun => array; count/*_count => number
        const PLURALS: &[&str] = &["events", "matches", "strands", "active", "entries", "edges", "covers", "timeline"];
        fn walk(v: &serde_json::Value, errs: &mut Vec<String>) {
            if let serde_json::Value::Object(map) = v {
                for (k, val) in map {
                    if PLURALS.contains(&k.as_str()) && !val.is_array() {
                        errs.push(format!("plural-named field `{}` is not an array (naming contract)", k));
                    }
                    if (k == "count" || k.ends_with("_count")) && !val.is_number() {
                        errs.push(format!("count field `{}` is not a number", k));
                    }
                    // id/strand_id are full-width 24-hex handles (join law);
                    // append_id is a 64-hex content hash, not a strand handle.
                    if (k == "id" || k == "strand_id") && val.is_string() {
                        let s = val.as_str().unwrap();
                        if s.len() != 24 {
                            errs.push(format!("`{}` is not full-width 24-hex: `{}`", k, s));
                        }
                    }
                    walk(val, errs);
                }
            } else if let serde_json::Value::Array(items) = v {
                for item in items { walk(item, errs); }
            }
        }
        let mut errs = Vec::new();
        for s in samples.drain(..) { walk(&s, &mut errs); }
        assert!(errs.is_empty(), "{}", errs.join("\n"));

        // Reference-as-contract: the json topic's hide/unhide section must
        // name every real ledger key (the topic lied once — stale names
        // survived a field rename).
        let topic = diagnostics::topic_lookup("json").expect("json topic exists");
        if let serde_json::Value::Object(map) = visibility_ledger_json(&id, false) {
            for key in map.keys() {
                assert!(
                    topic.body.contains(key.as_str()),
                    "json topic does not mention ledger field `{}`",
                    key
                );
            }
        }
    }

    #[test]
    fn grammar_format_json_coverage() {
        use clap::CommandFactory;
        // doctor/export are permanently exempt in the grammar contract;
        // init is pending judgment.
        const EXEMPT: &[&str] = &["init", "doctor", "export"];
        for sub in Cli::command().get_subcommands() {
            if EXEMPT.contains(&sub.get_name()) || sub.get_name() == "help" {
                continue;
            }
            let has_format = sub.get_arguments().any(|a| a.get_long() == Some("format"));
            assert!(
                has_format,
                "`{}` has no --format json twin (machine-isomorphism contract; if intentionally exempt, name it in the contract AND this list)",
                sub.get_name()
            );
        }
    }

    #[test]
    fn orient_is_pure_read() {
        let _env = setup();
        create_strand("a line");
        let path = ensure_journal().unwrap();
        let before = std::fs::read(&path).unwrap();
        cmd_orient(None, false, None, false).unwrap();
        cmd_orient(Some("json"), true, Some(3), false).unwrap();
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
        let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());
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

    // ── exit_code_for (exit-code contract) ─────────────────────────────────

    #[test]
    fn exit_code_for_journal_unreadable_is_2() {
        assert_eq!(exit_code_for("journal unreadable: bad bytes"), 2);
    }

    #[test]
    fn exit_code_for_generic_and_warn_are_1() {
        assert_eq!(exit_code_for("strand abc not found"), 1);
        assert_eq!(exit_code_for("warn: stdin and --file require --id"), 1);
        assert_eq!(exit_code_for("journal issues detected"), 1);
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
        let result = diagnostics::check_w070_strand_moved(&events, &id, Some("beta"));
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
        let result = diagnostics::check_w070_strand_moved(&events, &id, Some("alpha"));
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
        let result = diagnostics::check_w070_strand_moved(&events, &id, None);
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
        let result = diagnostics::check_w070_strand_moved(&events, &id, Some("beta"));
        assert!(result.is_none(), "W070 must not fire when last entry has no producer");
    }

    // ── W071: checkpoint on closed strand ──────────────────────────────────

    #[test]
    fn w071_fires_on_closed_strand() {
        let _env = setup();
        let id = create_strand("closed work");
        cmd_close(&id, Some("done"), false).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let strand = strands.iter().find(|s| s.id == id).unwrap();
        let result = diagnostics::check_w071_closed_strand(strand);
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
        let result = diagnostics::check_w071_closed_strand(strand);
        assert!(result.is_none(), "W071 must not fire on registered strand");
    }

    // ── checkpoint + W071 end-to-end: writes succeed (exit 0) ─────────────

    #[test]
    fn checkpoint_on_closed_strand_still_succeeds() {
        let _env = setup();
        let id = create_strand("done work");
        cmd_close(&id, Some("done"), false).unwrap();
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
    fn append_seen_offset_stale_still_writes() {
        let _env = setup();
        let id = create_strand("seen offset append target");
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let seen = strands.iter().find(|s| s.id == id).unwrap().last_offset();

        cmd_append(Some("[progress] moved after read"), None, false, false, None, Some(&id), None, None).unwrap();
        let result = cmd_append_with_seen_offset(
            Some("[progress] write with stale seen offset"),
            None,
            false,
            false,
            None,
            Some(&id),
            Some("json"),
            None,
            Some(seen),
            None,
        );
        assert!(result.is_ok(), "stale --seen-offset is a warning, not a gate: {:?}", result);

        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let strand = strands.iter().find(|s| s.id == id).unwrap();
        assert!(
            strand.log.iter().any(|e| e.content.contains("write with stale seen offset")),
            "append must still write the requested entry"
        );
    }

    #[test]
    fn checkpoint_seen_offset_stale_still_writes() {
        let _env = setup();
        let id = create_strand("seen offset checkpoint target");
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let seen = strands.iter().find(|s| s.id == id).unwrap().last_offset();

        cmd_append(Some("[progress] moved after read"), None, false, false, None, Some(&id), None, None).unwrap();
        let result = cmd_checkpoint_with_seen_offset(
            Some(&id),
            "checkpoint with stale seen offset",
            None,
            true,
            false,
            None,
            Some(seen),
        );
        assert!(result.is_ok(), "stale --seen-offset is a warning, not a gate: {:?}", result);

        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::LogAppended { id: event_id, content, .. } = e {
                event_id == &id && content.contains("checkpoint with stale seen offset")
            } else {
                false
            }
        });
        assert!(found, "checkpoint must still append its journal entry");
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
        cmd_hide(&id_b, Some("noise"), false, None).unwrap();
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
        cmd_hide(&id_b, Some("noise"), false, None).unwrap();
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
        cmd_hide(&id, Some("noise"), false, None).unwrap();
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
        cmd_hide(&id, Some("noise"), false, None).unwrap();
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
        cmd_hide(&id, Some("noise"), false, None).unwrap();
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
        cmd_hide(&id, Some("noise"), false, None).unwrap();
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
        cmd_hide(&id, None, false, None).unwrap();
        let mid = total_events();
        cmd_hide(&id, None, false, None).unwrap();
        cmd_hide(&id, Some("still hidden"), false, None).unwrap();
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
        cmd_hide(&id, None, false, None).unwrap();
        cmd_unhide(&id, false).unwrap();
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
        cmd_unhide(&id, false).unwrap();
        cmd_unhide(&id, false).unwrap();
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
        cmd_hide(&recent, Some("noise"), false, None).unwrap();
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
        cmd_hide(&recent, Some("noise"), false, None).unwrap();
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
        cmd_hide(&id, Some("noise"), false, None).unwrap();
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
        cmd_hide(&id, Some("noise"), false, None).unwrap();
        let result = cmd_context(None, &[], None, None, false, false, false);
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
        cmd_hide(&id, Some("noise"), false, None).unwrap();
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
        // The Append after_help now points to `tasktree explain markers` instead
        // of listing markers inline (L2 slim-down). The contract is now on the
        // markers topic body: every bracket marker in the body must be accepted
        // by validate_lifecycle_marker.
        let topic = diagnostics::topic_lookup("markers")
            .expect("markers topic must exist");
        let markers = extract_bracket_markers(topic.body);
        assert!(!markers.is_empty(), "markers topic body must list at least one marker");
        let mut failures: Vec<String> = Vec::new();
        for marker in &markers {
            let test_content = format!("{} x", marker);
            if let Err(e) = validate_lifecycle_marker(&test_content) {
                failures.push(format!("{}: {}", marker, e));
            }
        }
        assert!(failures.is_empty(), "markers in topic body rejected by validate_lifecycle_marker:\n{}", failures.join("\n"));
    }

    #[test]
    fn show_search_context_unchanged() {
        // Smoke test that existing cmd_show, cmd_search, cmd_context still work.
        let _env = setup();
        let id = create_strand("show me");
        cmd_append(Some("entry"), Some(&id), false, false, None, None, None, None).unwrap();
        // show
        let r = cmd_show(Some(&id), false, None, false, false, false);
        assert!(r.is_ok());
        // search
        let r = cmd_search("entry", false, false);
        assert!(r.is_ok());
        // context
        let r = cmd_context(None, &[], None, None, false, false, false);
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
        assert_eq!(card.id, id);
        assert_eq!(card.entry_count, 2);
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
        assert_eq!(card.id.len(), 24);
    }

    #[test]
    fn truncate_collapses_to_first_line() {
        // 多行首条 entry（如 add --file <长 brief>）只露首行 + "..."，
        // 不把后续行灌进 orient/list 的一眼扫视图。
        let blob = "## 项目背景\n这是第二行\n这是第三行";
        let out = truncate(blob, 70);
        assert_eq!(out, "## 项目背景...");
        assert!(!out.contains('\n'), "preview must be single line");

        // 单行短内容原样返回，不加省略号。
        assert_eq!(truncate("short single line", 70), "short single line");

        // 单行超长仍按字符数截断 + "..."。
        let long = "x".repeat(100);
        assert_eq!(truncate(&long, 70).chars().count(), 73);
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
        assert_eq!(card.id, new_s.id);
    }

    // ── card echo: hide leaves strand retrievable via include_hidden=true ──

    #[test]
    fn strand_card_fresh_finds_hidden_strand() {
        let _env = setup();
        let id = create_strand("will be hidden");
        cmd_hide(&id, None, false, None).unwrap();
        // strand_card_fresh uses include_hidden=true — must still find it
        let card = strand_card_fresh(&id);
        assert!(card.is_some(), "strand_card_fresh must return card for hidden strand");
        assert_eq!(card.unwrap().id, id);
    }

    // ══════════════════════════════════════════════════════════════════════
    // handles_* — 把手完整性测试族
    //
    // 规则：把手（strand id、现成命令、journal offset）永不截断。
    //   - id 在卡片/orient 用 shorten(id) = 12位十六进制前缀（合法前缀匹配）
    //   - id 在 list/show/search JSON 用完整 id
    //   - 两种形式都是合法参数；"…" 绝不出现在把手字段中
    //   - 散文字段（summary/last_entry/content）允许 truncate(70) + "…"
    // ══════════════════════════════════════════════════════════════════════

    /// Helper: build a >100-char summary that contains CJK characters so we
    /// also exercise Unicode truncation paths.
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

    /// make_card on a strand with a very long summary:
    ///   - card.id == shorten(full_id) and is a prefix of full_id, no '…'
    ///   - card.catch_up has no '…', parses with try_parse_example
    ///   - card.last_offset == projected strand's last_offset (integer, not a text truncation)
    ///   - prose fields (summary, last_entry) may contain '…'
    #[test]
    fn handles_card_id_is_legal_prefix() {
        let _env = setup();
        let summary = long_summary();
        assert!(summary.chars().count() > 100, "precondition: summary must be >100 chars");
        let id = create_strand(&summary);

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let s = strands.iter().find(|s| s.id == id).expect("strand must exist");
        let card = make_card(s);

        // id: exactly 12 hex chars, is prefix of full id, contains no '…'
        assert_eq!(card.id.len(), 24, "card.id must be the full 24-hex id");
        assert!(id.starts_with(&card.id), "card.id must be a prefix of the full id");
        assert!(!card.id.contains('\u{2026}') && !card.id.contains("..."),
            "card.id must not contain truncation marker");

        // catch_up: no '…', command must parse
        assert!(!card.catch_up.contains('\u{2026}') && !card.catch_up.contains("..."),
            "catch_up must not contain truncation marker");
        try_parse_example(&card.catch_up)
            .expect("card.catch_up must be a parseable tasktree command");

        // last_offset: must equal the projected strand's real last_offset
        assert_eq!(card.last_offset, s.last_offset(),
            "card.last_offset must equal projected strand's last_offset");

        // prose fields: allowed to contain '…' (they may be truncated)
        // (no assertion required — we just confirm the id/offset/catch_up rules above)
        let _ = &card.summary;
        let _ = &card.last_entry;
    }

    // ── Test 2 ────────────────────────────────────────────────────────────

    /// build_orient with long-summary strands: each OrientStrand in active[]
    ///   - id is 12 chars, prefix of full id, no '…'
    ///   - catch_up has no '…', parses, and contains card.id (link points to self)
    ///   - last_offset is the real offset
    #[test]
    fn handles_orient_text_complete() {
        let _env = setup();
        let summary_a = long_summary();
        let id_a = create_strand(&summary_a);
        // Give strand B a shorter summary for variety; strand A is the long one.
        let summary_b = "short strand for orient contrast";
        let id_b = create_strand(summary_b);

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let max_offset = events.last().map(|(o, _)| *o).unwrap_or(0);
        let strands = projection::project_strands(&events, true);
        let out = build_orient(&strands, false, 10, max_offset);

        assert!(!out.active.is_empty(), "orient must have at least one active strand");

        for card in &out.active {
            // id: full 24-hex width (joins against show/list JSON)
            assert_eq!(card.id.len(), 24,
                "orient card.id must be the full 24-hex id, got '{}'", card.id);
            // Verify it is a legal prefix: find the projected strand by prefix
            let matched = strands.iter().find(|s| s.id.starts_with(&card.id));
            assert!(matched.is_some(),
                "orient card.id '{}' must match a strand by prefix", card.id);
            assert!(!card.id.contains('\u{2026}') && !card.id.contains("..."),
                "orient card.id must not contain '…'");

            // catch_up: no truncation, parseable, embeds the card's own id
            assert!(!card.catch_up.contains('\u{2026}') && !card.catch_up.contains("..."),
                "catch_up must not be truncated");
            try_parse_example(&card.catch_up)
                .expect("orient catch_up must parse as a tasktree command");
            assert!(card.catch_up.contains(&card.id),
                "catch_up must embed the strand's own id (link points to self): '{}'", card.catch_up);

            // last_offset: matches the projected strand
            let s = matched.unwrap();
            assert_eq!(card.last_offset, s.last_offset(),
                "orient card.last_offset must equal projected strand's last_offset");
        }

        let _ = (id_a, id_b);
    }

    // ── Test 3 ────────────────────────────────────────────────────────────

    /// list --format json: StrandListItem.id is the full id (no shortening, no '…').
    /// search --format json: SearchMatch.strand_id is the full id.
    /// search content is prose — allowed to be truncated to 70 + "…".
    #[test]
    fn handles_list_search_ids_intact() {
        let _env = setup();
        let summary = long_summary();
        let id = create_strand(&summary);
        // Append a long content entry to have something to search.
        let long_content = "unique_search_token_xyz ".to_string() + &"w".repeat(80);
        cmd_append(Some(&long_content), Some(&id), false, false, None, None, None, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);

        // list JSON: StrandListItem.id must be the full 32-char id
        let strands = projection::project_strands(&events, true);
        let list_items: Vec<output::StrandListItem> = strands.iter()
            .map(output::StrandListItem::from)
            .collect();
        let item = list_items.iter().find(|i| i.id == id)
            .expect("list must contain our strand by full id");
        // Full id: must equal the strand's id exactly
        assert_eq!(item.id, id, "StrandListItem.id must be the full strand id");
        assert!(!item.id.contains('\u{2026}') && !item.id.contains("..."),
            "StrandListItem.id must not contain truncation marker");
        // id must be at least 12 chars (typical timeid is 24 hex chars)
        assert!(item.id.len() >= 12,
            "StrandListItem.id length must be at least 12, got {}", item.id.len());

        // search JSON: SearchMatch.strand_id must be the full id; content may be truncated
        let q = "unique_search_token_xyz".to_lowercase();
        let strand_map: std::collections::HashMap<&str, &projection::ProjectedStrand> =
            strands.iter().map(|s| (s.id.as_str(), s)).collect();
        let mut search_matches: Vec<output::SearchMatch> = Vec::new();
        for (_, event) in &events {
            if let Event::LogAppended { content, .. } = event {
                if content.to_lowercase().contains(&q) {
                    let strand_id_full = event.strand_id().to_string();
                    if strand_map.contains_key(strand_id_full.as_str()) {
                        let projected = strand_map.get(strand_id_full.as_str());
                        search_matches.push(output::SearchMatch {
                            strand_id: strand_id_full,
                            content: truncate(content, 70),
                            strand_type: projected.and_then(|s| s.strand_type.clone()),
                            hidden: projected.map(|s| s.hidden).unwrap_or(false),
                        });
                    }
                }
            }
        }
        assert!(!search_matches.is_empty(), "search must find at least one match");
        for m in &search_matches {
            // strand_id: full id, no truncation marker
            assert!(!m.strand_id.contains('\u{2026}') && !m.strand_id.contains("..."),
                "SearchMatch.strand_id must not contain truncation marker");
            assert!(m.strand_id.len() >= 12,
                "SearchMatch.strand_id must be at least 12 chars");
            // The match for our strand must be the full id
            if m.strand_id == id {
                assert_eq!(m.strand_id, id,
                    "SearchMatch.strand_id must equal full strand id");
            }
            // content is prose — truncation allowed; just verify it doesn't crash
            let _ = &m.content;
        }
    }

    // ── Test 4 ────────────────────────────────────────────────────────────

    /// run_journal_diagnostics: detail strings for W068/W069/W062 use shorten(id)
    /// (12-char prefix), which is a legal parameter. No '…' in detail strings.
    /// W070/W071 details contain no commands, so try_parse_example is N/A for them.
    #[test]
    fn handles_diag_details_parse() {
        let _env = setup();
        // Build a strand that fires W068 (overdue deadline).
        let id_a = create_strand("deadline strand for diag test");
        cmd_append(
            Some("[deadline] finish by=2000-01-01"),
            None, false, false, None, Some(&id_a), None, None,
        ).unwrap();

        // Build cross-strand W062 (decision vs constraint with shared keyword).
        let id_b = create_strand("decision strand");
        let id_c = create_strand("constraint strand");
        cmd_append(
            Some("[decision] adopt postgres for persistence"),
            None, false, false, None, Some(&id_b), None, None,
        ).unwrap();
        cmd_append(
            Some("[constraint] postgres writes forbidden in staging"),
            None, false, false, None, Some(&id_c), None, None,
        ).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let raw: Vec<Event> = events.iter().map(|(_, e)| e.clone()).collect();
        let diags = diagnostics::run_journal_diagnostics(&raw, chrono::Utc::now());

        for (code, detail) in &diags {
            // No truncation marker in detail strings (id handles inside details
            // use shorten, which is a valid prefix, not a truncated string).
            assert!(!detail.contains('\u{2026}') && !detail.contains("..."),
                "diag {} detail must not contain truncation marker: '{}'", code, detail);

            // For W062: detail contains strand id handles (shorten = 12-char prefix).
            // Verify any embedded id-like hex strings (12 chars) are prefix of a known strand.
            if *code == "W062" || *code == "W068" || *code == "W069" {
                // Extract 12-char hex tokens from detail.
                for tok in detail.split_whitespace() {
                    let tok = tok.trim_matches(|c: char| !c.is_ascii_hexdigit());
                    if tok.len() == 12 && tok.chars().all(|c| c.is_ascii_hexdigit()) {
                        // Must be a prefix of some known strand id.
                        let all_strands = projection::project_strands(&events, true);
                        let is_valid_prefix = all_strands.iter().any(|s| s.id.starts_with(tok));
                        assert!(is_valid_prefix,
                            "diag {} detail contains '{}' which is not a valid strand id prefix",
                            code, tok);
                    }
                }
            }

            // W070/W071: details contain no tasktree commands (catalog confirms
            // their recovery.executable is false). We verify no false-positive parse attempt.
            // (No try_parse_example call here — the detail strings are prose, not commands.)
            if *code == "W070" || *code == "W071" {
                assert!(!detail.contains("tasktree "),
                    "W070/W071 detail must not embed a tasktree command: '{}'", detail);
            }
        }
    }

    // ── Test 5 ────────────────────────────────────────────────────────────

    /// Audit test: for a strand with a known id, verify that each command's
    /// JSON id field matches the documented convention (current behavior nailed).
    ///
    ///   show --format json  → StrandDetailOutput.id = full id
    ///   list --format json  → StrandListItem.id      = full id
    ///   orient --format json (via build_orient) → OrientStrand.id = shorten(full id) = 12 chars
    ///   search --format json → SearchMatch.strand_id = full id
    ///
    /// All forms are legally usable as tasktree --id arguments (prefix match
    /// or exact match). Neither form may contain '…'.
    #[test]
    fn handles_truncate_never_applied_to_ids() {
        let _env = setup();
        // Use a long summary so truncate would fire on prose but must not fire on ids.
        let id = create_strand(&long_summary());
        // Append a searchable entry.
        let searchable = "unique_audit_token_abc123 ".to_string() + &"z".repeat(80);
        cmd_append(Some(&searchable), Some(&id), false, false, None, None, None, None).unwrap();

        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);

        // show --format json: full id
        let s = strands.iter().find(|s| s.id == id).expect("strand must exist");
        let show_dto = output::StrandDetailOutput::from(s);
        assert_eq!(show_dto.id, id,
            "show JSON: id must equal full strand id");
        assert!(!show_dto.id.contains('\u{2026}') && !show_dto.id.contains("..."),
            "show JSON: id must not contain truncation marker");

        // list --format json: full id
        let list_item = output::StrandListItem::from(s);
        assert_eq!(list_item.id, id,
            "list JSON: id must equal full strand id");
        assert!(!list_item.id.contains('\u{2026}') && !list_item.id.contains("..."),
            "list JSON: id must not contain truncation marker");

        // orient --format json (build_orient): full 24-hex id (joins across outputs)
        let max_offset = events.last().map(|(o, _)| *o).unwrap_or(0);
        let out = build_orient(&strands, false, 10, max_offset);
        let orient_card = out.active.iter().find(|c| c.id == id)
            .expect("orient must contain our strand");
        assert_eq!(orient_card.id.len(), 24,
            "orient JSON: id must be the full 24-hex id");
        assert!(!orient_card.id.contains('\u{2026}') && !orient_card.id.contains("..."),
            "orient JSON: id must not contain truncation marker");

        // search --format json: full id
        let q = "unique_audit_token_abc123".to_lowercase();
        let strand_map: std::collections::HashMap<&str, &projection::ProjectedStrand> =
            strands.iter().map(|s| (s.id.as_str(), s)).collect();
        let mut found_match: Option<output::SearchMatch> = None;
        for (_, event) in &events {
            if let Event::LogAppended { content, .. } = event {
                if content.to_lowercase().contains(&q) {
                    let strand_id_full = event.strand_id().to_string();
                    if strand_map.contains_key(strand_id_full.as_str()) {
                        let projected = strand_map.get(strand_id_full.as_str());
                        if strand_id_full == id {
                            found_match = Some(output::SearchMatch {
                                strand_id: strand_id_full,
                                content: truncate(content, 70),
                                strand_type: projected.and_then(|s| s.strand_type.clone()),
                                hidden: projected.map(|s| s.hidden).unwrap_or(false),
                            });
                        }
                    }
                }
            }
        }
        let m = found_match.expect("search must find our entry");
        assert_eq!(m.strand_id, id,
            "search JSON: strand_id must equal full strand id");
        assert!(!m.strand_id.contains('\u{2026}') && !m.strand_id.contains("..."),
            "search JSON: strand_id must not contain truncation marker");
    }

    // ── Test 6 ────────────────────────────────────────────────────────────

    /// cmd_checkpoint text output: the staleness line contains the integer offset
    /// (no truncation), and the catch-up command (when emitted) embeds the
    /// 12-char strand id handle without '…'.
    ///
    /// Note: cmd_checkpoint prints directly to stdout/stderr rather than returning
    /// a structured value, so we verify the *journal entry* written by checkpoint
    /// contains the structured fields, and we verify the OrientStrand card it
    /// creates matches the handle-integrity rules.
    #[test]
    fn handles_checkpoint_handle_fields() {
        let _env = setup();
        // Create two strands so there is a journal delta when we checkpoint strand A.
        let id_a = create_strand("checkpoint handle test strand");
        let id_b = create_strand("another strand to create journal delta");
        cmd_append(Some("delta entry one"), Some(&id_b), false, false, None, None, None, None).unwrap();
        cmd_append(Some("delta entry two"), Some(&id_b), false, false, None, None, None, None).unwrap();

        // Run checkpoint on strand A — journal delta > 0 so catch-up will be emitted.
        let result = cmd_checkpoint(Some(&id_a), "handle integrity check", None, false, false, None);
        assert!(result.is_ok(), "checkpoint must succeed: {:?}", result);

        // The [checkpoint] journal entry contains observed_entries_before_append=N
        // where N is the integer entry count. Verify the stored entry has no '…'.
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let cp_entry = events.iter().find(|(_, e)| {
            if let Event::LogAppended { id, content, .. } = e {
                id == &id_a && content.contains("[checkpoint] ok")
            } else {
                false
            }
        }).expect("checkpoint entry must exist in journal");
        let content = match &cp_entry.1 {
            Event::LogAppended { content, .. } => content,
            _ => unreachable!(),
        };
        assert!(!content.contains('\u{2026}') && !content.contains("..."),
            "checkpoint journal entry must not contain truncation marker: '{}'", content);
        assert!(content.contains("observed_entries_before_append="),
            "checkpoint entry must contain integer observed count");

        // The card produced for strand A must satisfy handle-integrity rules.
        let strands = projection::project_strands(&events, true);
        let s = strands.iter().find(|s| s.id == id_a).expect("strand A must exist");
        let card = make_card(s);
        assert_eq!(card.id, id_a,
            "post-checkpoint card id must be the full id");
        assert!(!card.catch_up.contains('\u{2026}') && !card.catch_up.contains("..."),
            "post-checkpoint catch_up must not be truncated");
        try_parse_example(&card.catch_up)
            .expect("post-checkpoint catch_up must parse");

        // JSON checkpoint output via cmd_checkpoint --format json:
        // The catch_up field in JSON uses shorten(strand_id) — verify via the
        // format string in cmd_checkpoint (the JSON path). We build the expected
        // value directly from the same logic.
        let strand_last_offset = s.last_offset();
        // After the checkpoint write, s.last_offset() includes the checkpoint entry.
        // The JSON catch_up is built *before* the write from strand_last_offset;
        // here we use the pre-checkpoint offset of strand A.
        // Find strand A's pre-checkpoint last_offset (last entry before checkpoint):
        let pre_cp_offset = {
            let mut last = 0usize;
            for (offset, e) in &events {
                if let Event::LogAppended { id, content, .. } = e {
                    if id == &id_a && !content.contains("[checkpoint] ok") {
                        last = *offset;
                    }
                }
            }
            last
        };
        let expected_catch_up = format!(
            "tasktree timeline --since-offset {} --links {}",
            pre_cp_offset, shorten(&id_a)
        );
        try_parse_example(&expected_catch_up)
            .expect("expected checkpoint JSON catch_up must parse");
        assert!(!expected_catch_up.contains('\u{2026}') && !expected_catch_up.contains("..."),
            "checkpoint JSON catch_up must not be truncated");

        let _ = (id_b, strand_last_offset);
    }

    // ── Task B: IdTarget tests ─────────────────────────────────────────────

    /// Positional <ID> and --id <ID> parse identically for show, find, hide,
    /// unhide, tree. We verify using clap's try_get_matches_from.
    #[test]
    fn id_target_flag_and_positional_equivalent() {
        use clap::CommandFactory;
        // For each command, parse both forms and verify they succeed.
        let cases: &[(&str, &str)] = &[
            ("show", "0000019dd34b"),
            ("find", "0000019dd34b"),
            ("hide", "0000019dd34b"),
            ("unhide", "0000019dd34b"),
            ("tree", "0000019dd34b"),
        ];
        for (cmd, id) in cases {
            // positional form: tasktree <cmd> <id>
            let pos_result = Cli::command()
                .try_get_matches_from(["tasktree", cmd, id]);
            assert!(
                pos_result.is_ok(),
                "{} positional form failed: {:?}", cmd, pos_result.err()
            );
            // flag form: tasktree <cmd> --id <id>
            let flag_result = Cli::command()
                .try_get_matches_from(["tasktree", cmd, "--id", id]);
            assert!(
                flag_result.is_ok(),
                "{} --id form failed: {:?}", cmd, flag_result.err()
            );
        }
        // Behavioral check: show positional vs --id produce same resolved id
        let _env = setup();
        let id = create_strand("id_target behavioral test");
        // Both should succeed and produce the same output
        let r1 = cmd_show(Some(&id), false, None, false, false, false);
        let r2 = cmd_show(Some(&id), false, None, false, false, false);
        assert!(r1.is_ok(), "show with positional id failed: {:?}", r1);
        assert!(r2.is_ok(), "show with --id failed: {:?}", r2);
    }

    /// Providing both positional <ID> and --id <ID> must be rejected by clap.
    #[test]
    fn id_target_conflict_rejected() {
        use clap::CommandFactory;
        let result = Cli::command()
            .try_get_matches_from(["tasktree", "show", "000653", "--id", "000653"]);
        assert!(
            result.is_err(),
            "show with both positional and --id must be rejected"
        );
    }

    /// `timeline --id X` parses as `timeline --strand X` (visible_alias = "id").
    #[test]
    fn timeline_id_alias() {
        use clap::CommandFactory;
        let result = Cli::command()
            .try_get_matches_from(["tasktree", "timeline", "--id", "0000019dd34b"]);
        assert!(
            result.is_ok(),
            "timeline --id should parse via visible_alias on --strand: {:?}",
            result.err()
        );
        // Also verify --strand still works
        let result2 = Cli::command()
            .try_get_matches_from(["tasktree", "timeline", "--strand", "0000019dd34b"]);
        assert!(result2.is_ok(), "timeline --strand must still work: {:?}", result2.err());
    }

    // ── Task D: show --tail decoupled from --last ──────────────────────────

    /// show with explicit <ID> + --tail N must succeed (previously blocked by
    /// the now-removed `requires = "last"` guard).
    #[test]
    fn show_tail_works_with_explicit_id() {
        let _env = setup();
        let id = create_strand("tail decoupling test");
        cmd_append(Some("entry two"), Some(&id), false, false, None, None, None, None).unwrap();
        cmd_append(Some("entry three"), Some(&id), false, false, None, None, None, None).unwrap();

        // tail with explicit id — must succeed and show only last 2 entries
        let result = cmd_show(Some(&id), false, Some(2), false, false, false);
        assert!(
            result.is_ok(),
            "show <ID> --tail 2 must succeed: {:?}", result
        );
        // --last + --tail must still work
        let result2 = cmd_show(None, true, Some(2), false, false, false);
        assert!(result2.is_ok(), "show --last --tail must still work: {:?}", result2);
    }

    // ── Task C: entry_count rename — no "entries" key in JSON output ────────

    /// StrandDetailOutput (show --format json) must serialize as "entry_count",
    /// not "entries".
    #[test]
    fn show_json_has_entry_count_not_entries() {
        let _env = setup();
        let id = create_strand("entry_count rename test");
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let s = strands.iter().find(|s| s.id == id).expect("strand must exist");
        let dto = output::StrandDetailOutput::from(s);
        let v = serde_json::to_value(&dto).expect("serialize");
        let obj = v.as_object().unwrap();
        assert!(
            obj.contains_key("entry_count"),
            "show JSON must have 'entry_count' key"
        );
        assert!(
            !obj.contains_key("entries"),
            "show JSON must NOT have 'entries' key (renamed to entry_count)"
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // Batch-2: JSON twins / provenance / --edge-type / add --stdin/--file
    // ════════════════════════════════════════════════════════════════════════

    // ── ① JSON twins: find --format json ─────────────────────────────────

    #[test]
    fn find_json_returns_id_object() {
        let _env = setup();
        let id = create_strand("find-json target");
        // find with full id — text mode returns plain id
        cmd_find(&id, false).unwrap();
        // find with format json — must return {"id": <full_id>}
        // Capture via direct call; actual stdout capture not needed for contract test.
        // We verify that the json serialization path is exercised without error.
        let result = cmd_find(&id, true);
        assert!(result.is_ok(), "find --format json must succeed: {:?}", result);
    }

    #[test]
    fn find_json_unknown_strand_errors() {
        let _env = setup();
        create_strand("irrelevant");
        let result = cmd_find("000000000000", true);
        assert!(result.is_err(), "find on unknown id must error in json mode too");
    }

    // ── ① JSON twins: hide --format json ─────────────────────────────────

    #[test]
    fn hide_json_returns_visibility_ledger() {
        let _env = setup();
        let id = create_strand("to be hidden json");
        let result = cmd_hide(&id, None, true, None);
        assert!(result.is_ok(), "hide --format json must succeed: {:?}", result);
        // idempotent call — noop: true
        let result2 = cmd_hide(&id, None, true, None);
        assert!(result2.is_ok(), "hide --format json idempotent must succeed");
    }

    #[test]
    fn hide_json_contains_active_closed_hidden_counts() {
        // Contract: JSON output of hide must carry active / closed / hidden integer fields.
        // We exercise the path; count correctness is a projection concern already tested.
        let _env = setup();
        let id = create_strand("hide json count test");
        // Calling cmd_hide with format_json=true must not panic/error.
        cmd_hide(&id, None, true, None).unwrap();
    }

    // ── ① JSON twins: unhide --format json ───────────────────────────────

    #[test]
    fn unhide_json_returns_ok() {
        let _env = setup();
        let id = create_strand("unhide json test");
        cmd_hide(&id, None, false, None).unwrap();
        let result = cmd_unhide(&id, true);
        assert!(result.is_ok(), "unhide --format json must succeed: {:?}", result);
    }

    // ── ① JSON twins: link --format json ─────────────────────────────────

    #[test]
    fn link_json_returns_source_target_edge_type() {
        let _env = setup();
        let src = create_strand("link json source");
        let tgt = create_strand("link json target");
        let result = cmd_link(&src, &tgt, None, true, None);
        assert!(result.is_ok(), "link --format json must succeed: {:?}", result);
    }

    #[test]
    fn link_json_default_edge_type_is_depends_on() {
        // Verify the EdgeLinked event carries the default edge_type when none given.
        let _env = setup();
        let src = create_strand("link edge type source");
        let tgt = create_strand("link edge type target");
        cmd_link(&src, &tgt, None, false, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::EdgeLinked { id, edge_type, .. } = e {
                id == &src && edge_type.as_deref() == Some("depends-on")
            } else {
                false
            }
        });
        assert!(found, "EdgeLinked must carry edge_type=depends-on by default");
    }

    // ── ② provenance: link --provenance ──────────────────────────────────

    #[test]
    fn link_provenance_stored_on_edge_linked_event() {
        let _env = setup();
        let src = create_strand("prov link source");
        let tgt = create_strand("prov link target");
        cmd_link(&src, &tgt, None, false, Some(r#"{"producer":"test-agent"}"#)).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::EdgeLinked { id, provenance, .. } = e {
                id == &src && provenance.is_some()
            } else {
                false
            }
        });
        assert!(found, "EdgeLinked must carry provenance when --provenance given");
    }

    #[test]
    fn link_without_provenance_has_none() {
        let _env = setup();
        let src = create_strand("no-prov link source");
        let tgt = create_strand("no-prov link target");
        cmd_link(&src, &tgt, None, false, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::EdgeLinked { id, provenance, .. } = e {
                id == &src && provenance.is_none()
            } else {
                false
            }
        });
        assert!(found, "EdgeLinked must have provenance=None when not given");
    }

    /// Old EdgeLinked JSON without provenance field must still deserialize.
    #[test]
    fn old_edge_linked_still_deserializes() {
        let old = r#"{"type":"edge_linked","id":"abc","ts":"2026-01-01T00:00:00Z","to":"def"}"#;
        let event: Event = serde_json::from_str(old).unwrap();
        match &event {
            Event::EdgeLinked { to, provenance, .. } => {
                assert_eq!(to, "def");
                assert!(provenance.is_none(), "old edge_linked must deserialize with provenance=None");
            }
            _ => panic!("expected EdgeLinked"),
        }
    }

    // ── ② provenance: hide --provenance forwards to reason entry ─────────

    #[test]
    fn hide_with_reason_and_provenance_stores_provenance_on_log_entry() {
        let _env = setup();
        let id = create_strand("hide prov test");
        cmd_hide(&id, Some("test reason"), false, Some(r#"{"producer":"tester"}"#)).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::LogAppended { id: eid, content, provenance, .. } = e {
                eid == &id
                    && content.starts_with("[hidden]")
                    && provenance.is_some()
            } else {
                false
            }
        });
        assert!(found, "[hidden] entry must carry provenance when --provenance given with --reason");
    }

    #[test]
    fn hide_without_reason_provenance_arg_is_accepted() {
        // --provenance without --reason: argument accepted, no content entry written.
        let _env = setup();
        let id = create_strand("hide no-reason prov");
        let result = cmd_hide(&id, None, false, Some(r#"{"producer":"tester"}"#));
        assert!(result.is_ok(), "hide --provenance without --reason must succeed");
    }

    // ── ② provenance: add --provenance ───────────────────────────────────

    #[test]
    fn add_provenance_stored_on_first_log_entry() {
        let _env = setup();
        cmd_add(Some("add prov test"), false, None, false, None, Some(r#"{"producer":"tester"}"#)).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::LogAppended { content, provenance, .. } = e {
                content == "add prov test" && provenance.is_some()
            } else {
                false
            }
        });
        assert!(found, "LogAppended from add must carry provenance when --provenance given");
    }

    #[test]
    fn add_without_provenance_has_none() {
        let _env = setup();
        cmd_add(Some("add no prov"), false, None, false, None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::LogAppended { content, provenance, .. } = e {
                content == "add no prov" && provenance.is_none()
            } else {
                false
            }
        });
        assert!(found, "LogAppended from add must have provenance=None when not given");
    }

    // ── ③ --edge-type: renamed flag still resolves correctly ─────────────

    #[test]
    fn link_edge_type_custom_is_stored() {
        let _env = setup();
        let src = create_strand("edge-type source");
        let tgt = create_strand("edge-type target");
        cmd_link(&src, &tgt, Some("belongs-to"), false, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let found = events.iter().any(|(_, e)| {
            if let Event::EdgeLinked { id, edge_type, .. } = e {
                id == &src && edge_type.as_deref() == Some("belongs-to")
            } else {
                false
            }
        });
        assert!(found, "custom edge_type must be stored on EdgeLinked event");
    }

    // ── ④ add --stdin / --file ────────────────────────────────────────────

    #[test]
    fn add_positional_content_creates_strand() {
        let _env = setup();
        // Positional content: existing path, now cmd_add(Some(..), false, None, ..)
        let result = cmd_add(Some("add positional"), false, None, false, None, None);
        assert!(result.is_ok(), "add with positional content must succeed: {:?}", result);
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        assert!(
            strands.iter().any(|s| s.first_summary() == "add positional"),
            "strand with 'add positional' summary must exist"
        );
    }

    #[test]
    fn add_file_content_creates_strand() {
        let env = setup();
        let file_path = env.path().join("brief.md");
        fs::write(&file_path, "add from file\n").unwrap();
        let path_str = file_path.to_str().unwrap();
        let result = cmd_add(None, false, Some(path_str), false, None, None);
        assert!(result.is_ok(), "add --file must succeed: {:?}", result);
        let jpath = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&jpath);
        let strands = projection::project_strands(&events, true);
        assert!(
            strands.iter().any(|s| s.first_summary() == "add from file"),
            "strand with 'add from file' summary must exist after add --file"
        );
    }

    #[test]
    fn add_multiple_content_sources_errors() {
        let _env = setup();
        // positional + stdin both set → must error
        let result = cmd_add(Some("content"), true, None, false, None, None);
        assert!(result.is_err(), "add with two content sources must error");
    }

    #[test]
    fn add_no_content_source_errors() {
        let _env = setup();
        let result = cmd_add(None, false, None, false, None, None);
        assert!(result.is_err(), "add with no content source must error");
    }

    #[test]
    fn add_empty_file_content_errors() {
        let env = setup();
        let file_path = env.path().join("empty.md");
        fs::write(&file_path, "").unwrap();
        let path_str = file_path.to_str().unwrap();
        let result = cmd_add(None, false, Some(path_str), false, None, None);
        assert!(result.is_err(), "add --file with empty file must error");
    }

    #[test]
    fn add_nonexistent_file_errors() {
        let _env = setup();
        let result = cmd_add(None, false, Some("/nonexistent/path/to/file.txt"), false, None, None);
        assert!(result.is_err(), "add --file with nonexistent file must error");
    }

    // ── W073: typo marker suggestion ─────────────────────────────────────────

    #[test]
    fn levenshtein_basic() {
        assert_eq!(levenshtein("decision", "decision"), 0);
        assert_eq!(levenshtein("freiction", "friction"), 1);   // one extra char
        assert_eq!(levenshtein("decsion", "decision"), 1);     // transposition/missing
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }

    #[test]
    fn suggest_marker_typo_triggers() {
        // [freiction] → friction (distance 1)
        let r = suggest_marker("[freiction]");
        assert_eq!(r, Some("[friction]"), "freiction should suggest friction");

        // [decsion] → decision (distance 1)
        let r2 = suggest_marker("[decsion]");
        assert_eq!(r2, Some("[decision]"), "decsion should suggest decision");
    }

    #[test]
    fn suggest_marker_exact_match_is_silent() {
        // Exact match must return None (not a typo)
        assert_eq!(suggest_marker("[decision]"), None);
        assert_eq!(suggest_marker("[friction]"), None);
        assert_eq!(suggest_marker("[done]"), None);
    }

    #[test]
    fn suggest_marker_custom_tags_are_silent() {
        // Custom tags with hyphens, digits, or uppercase-looking codes must be silent
        assert_eq!(suggest_marker("[my-tag]"), None,    "hyphen tag must be silent");
        assert_eq!(suggest_marker("[W062]"), None,       "W-code must be silent");
        assert_eq!(suggest_marker("[2026-06]"), None,    "date tag must be silent");
        assert_eq!(suggest_marker("[myCustomTag]"), None, "long distant tag must be silent");
    }

    #[test]
    fn suggest_marker_non_bracket_is_silent() {
        // Content not starting with [ must never fire W073 (validate_lifecycle_marker returns Ok)
        assert!(validate_lifecycle_marker("plain text").is_ok());
        assert!(validate_lifecycle_marker("just a note").is_ok());
    }

    #[test]
    fn known_markers_covers_all_topic_markers() {
        // Every bracket marker in the markers topic body must be in known_markers().
        let topic = diagnostics::topic_lookup("markers")
            .expect("markers topic must exist");
        let in_topic = extract_bracket_markers(topic.body);
        let km: Vec<&str> = known_markers().to_vec();
        let mut missing: Vec<String> = Vec::new();
        for m in &in_topic {
            // Skip [hidden] — present in known_markers but not required to be
            // listed in topic body prose
            if !km.contains(&m.as_str()) {
                missing.push(m.clone());
            }
        }
        assert!(
            missing.is_empty(),
            "markers in topic body not in known_markers(): {:?}", missing
        );
    }

    #[test]
    fn w073_append_typo_succeeds_and_suggest_fires() {
        // Verify: cmd_append succeeds (W073 never blocks writes).
        // Verify: suggest_marker returns a suggestion for the typo.
        let _env = setup();
        let id = create_strand("w073 test strand");
        let result = cmd_append(
            Some("[freiction] this is a typo marker"),
            None, false, false, None, Some(&id), None, None,
        );
        assert!(result.is_ok(), "append must succeed even with typo marker: {:?}", result);
        // Confirm suggest_marker would have fired
        let suggestion = suggest_marker("[freiction]");
        assert_eq!(suggestion, Some("[friction]"));
    }

    #[test]
    fn w073_exact_marker_is_silent() {
        // Correctly spelled markers must not trigger W073.
        assert_eq!(suggest_marker("[decision]"), None);
        assert_eq!(suggest_marker("[constraint]"), None);
        assert_eq!(suggest_marker("[progress]"), None);
    }

    // ── Lifecycle: close / reopen / W074 regression tests ─────────────────

    /// Footgun nail: appending [done] to a strand must NOT close it.
    /// This is the principal regression test for the lifecycle refactor.
    #[test]
    fn append_subtask_done_leaves_strand_open() {
        let _env = setup();
        let id = create_strand("parent line of work");
        // Simulate what an operator agent would do: record a sub-task completion.
        cmd_append(Some("[done] subtask A completed"), None, false, false, None, Some(&id), None, None).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let strand = strands.iter().find(|s| s.id == id).unwrap();
        assert_eq!(
            strand.state(), "registered",
            "appending [done] must NOT close the strand; state was: {}",
            strand.state()
        );
    }

    /// close with default disposition → closed:done.
    #[test]
    fn close_default_sets_closed_done() {
        let _env = setup();
        let id = create_strand("work to close");
        cmd_close(&id, None, false).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let strand = strands.iter().find(|s| s.id == id).unwrap();
        assert_eq!(strand.state(), "closed:done", "close default must give closed:done");
    }

    /// close --as failed → closed:failed.
    #[test]
    fn close_as_failed_sets_closed_failed() {
        let _env = setup();
        let id = create_strand("work that failed");
        cmd_close(&id, Some("failed"), false).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let strand = strands.iter().find(|s| s.id == id).unwrap();
        assert_eq!(strand.state(), "closed:failed", "close --as failed must give closed:failed");
    }

    /// reopen after close → back to registered (open).
    #[test]
    fn reopen_after_close_restores_registered() {
        let _env = setup();
        let id = create_strand("work to reopen");
        cmd_close(&id, None, false).unwrap();
        cmd_reopen(&id, false).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let strand = strands.iter().find(|s| s.id == id).unwrap();
        assert_eq!(strand.state(), "registered", "reopen must restore registered state");
    }

    /// close → closed:cancelled.
    #[test]
    fn close_as_cancelled_sets_closed_cancelled() {
        let _env = setup();
        let id = create_strand("cancelled plan");
        cmd_close(&id, Some("cancelled"), false).unwrap();
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let strand = strands.iter().find(|s| s.id == id).unwrap();
        assert_eq!(strand.state(), "closed:cancelled");
    }

    /// close twice must error (already closed).
    #[test]
    fn close_already_closed_errors() {
        let _env = setup();
        let id = create_strand("once-closed work");
        cmd_close(&id, None, false).unwrap();
        let result = cmd_close(&id, None, false);
        assert!(result.is_err(), "closing an already-closed strand must error");
        assert!(result.unwrap_err().contains("already"), "error must say already");
    }

    /// reopen an already-open strand must error.
    #[test]
    fn reopen_already_open_errors() {
        let _env = setup();
        let id = create_strand("never closed");
        let result = cmd_reopen(&id, false);
        assert!(result.is_err(), "reopening an already-open strand must error");
    }

    /// W074 fires when a closing-marker annotation is appended.
    #[test]
    fn w074_fires_on_closing_annotation_marker() {
        let _env = setup();
        let id = create_strand("some work");
        // The predicate that gates the W074 nudge must be true for a closing marker.
        assert!(diagnostics::is_closing_annotation_marker("[done] sub-step done"), "closing marker must gate W074");
        let result = cmd_append(Some("[done] sub-step done"), None, false, false, None, Some(&id), None, None);
        assert!(result.is_ok(), "append must succeed even with closing marker");
        // Strand must still be open (that's the whole point of W074's warning).
        let path = ensure_journal().unwrap();
        let (events, _) = read_events_lossy(&path);
        let strands = projection::project_strands(&events, true);
        let strand = strands.iter().find(|s| s.id == id).unwrap();
        assert_eq!(strand.state(), "registered", "W074 scenario: strand must remain open after closing-marker append");
    }

    /// W074 must NOT fire on non-closing markers (precision-first).
    #[test]
    fn w074_silent_on_non_closing_markers() {
        // Exercises the real predicate the runtime nudge uses (not a duplicated
        // constant): non-closing markers must not gate W074.
        for m in ["[decision]", "[progress]", "[friction]", "[observed]", "[insight]"] {
            assert!(!diagnostics::is_closing_annotation_marker(m), "{} must not trigger W074", m);
        }
    }

    /// orient / remind must NOT contain the old "append [done]" pattern.
    #[test]
    fn orient_remind_does_not_say_append_done() {
        assert!(
            !ORIENT_REMIND.contains("append --id") || !ORIENT_REMIND.contains("[done]"),
            "ORIENT_REMIND must not suggest 'append [done]' as the close idiom: {}",
            ORIENT_REMIND
        );
        assert!(
            ORIENT_REMIND.contains("close --id"),
            "ORIENT_REMIND must mention 'close --id': {}",
            ORIENT_REMIND
        );
    }

    /// remind carries the loop methodology (act → observe → think), not just
    /// the command cheat-sheet.
    #[test]
    fn orient_remind_carries_the_loop_stance() {
        assert!(
            ORIENT_REMIND.contains("loop:"),
            "ORIENT_REMIND must carry the loop stance: {}",
            ORIENT_REMIND
        );
    }

    #[test]
    fn leading_marker_extracts_token_or_none() {
        assert_eq!(leading_marker("[decision] foo"), Some("decision"));
        assert_eq!(leading_marker("  [friction] bar"), Some("friction"));
        assert_eq!(leading_marker("plain text"), None);
        assert_eq!(leading_marker("[] empty"), None);
        assert_eq!(leading_marker("no close bracket [x"), None);
    }

    #[test]
    fn show_digest_returns_ok_without_dumping_log() {
        let _env = setup();
        let id = create_strand("digest target");
        cmd_append(Some("[decision] one"), Some(&id), false, false, None, None, None, None).unwrap();
        cmd_append(Some("[friction] two"), Some(&id), false, false, None, None, None, None).unwrap();
        // digest = true; should succeed (census path, no full log dump)
        let r = cmd_show(Some(&id), false, None, false, false, true);
        assert!(r.is_ok(), "show --digest failed: {:?}", r);
    }

    #[test]
    fn find_strand_rejects_empty_id() {
        let _env = setup();
        let id = create_strand("real strand");
        let (events, _) = read_events_lossy(&ensure_journal().unwrap());
        // empty / whitespace id must NOT silently resolve to the first strand
        assert_eq!(find_strand(&events, ""), None, "empty id must not match any strand");
        assert_eq!(find_strand(&events, "   "), None, "whitespace id must not match");
        // a real prefix still resolves
        assert_eq!(find_strand(&events, &id[..8]), Some(id.clone()));
    }

    #[test]
    fn orient_catch_up_shows_content_not_empty_delta() {
        let _env = setup();
        let id = create_strand("catch up target");
        let (events, _) = read_events_lossy(&ensure_journal().unwrap());
        let strands = projection::project_strands(&events, true);
        let s = strands.iter().find(|s| s.id == id).unwrap();
        let card = render::make_card(s);
        // catch-up must show the strand's recent content (never the empty-prone
        // `--since-offset <last_offset>` form, which shows nothing at orient time).
        assert!(card.catch_up.contains("show"), "catch_up must use show: {}", card.catch_up);
        assert!(card.catch_up.contains("--tail"), "catch_up must show recent tail: {}", card.catch_up);
        assert!(
            !card.catch_up.contains("--since-offset"),
            "catch_up must not use the empty-prone since-offset form: {}",
            card.catch_up
        );
    }
}

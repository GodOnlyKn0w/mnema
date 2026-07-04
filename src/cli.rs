use crate::commands::context::*;
use crate::commands::doctor::*;
use crate::commands::explain::cmd_explain;
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
  find          Resolve a strand id
  tree          Strand forest (belongs-to nesting)
  depends       depends-on analysis: blockers / readiness / critical path
  current       Latest effective subject binding
  agent-context Machine-readable active-strand context
  context       Project a typed context slice

做 / change:
  add           Create a new strand
  append        Append an entry to a strand
  close         Close a strand (close effect entry)
  reopen        Reopen a closed strand (reopen effect entry)
  checkpoint    Record context before an irreversible action
  link          Link strands (belongs-to / depends-on)
  unlink        Remove a link (unlink effect entry; projection drops the edge)
  bind          Record a subject binding

管 / manage:
  init          Initialize .tasktree/ journal
  hide          Hide a strand from active orient (parked, revivable)
  unhide        Unhide a strand
  doctor        Diagnose journal integrity
  export        Export journal as standalone audit artifact
  cutover-v2    Rewrite/import current journal into pure v2 form
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
    /// Create a new strand with first log entry from stdin
    #[command(after_help = "\
Content source:
  stdin               First log entry content. Piped input is required.

Rules:
  Entry content is never read from positional arguments, --stdin, or --file.
  Empty stdin content is rejected.

Examples:
  echo \"start a new line of work\" | tasktree add
  echo \"child line of work\" | tasktree add --parent <PARENT>
  echo \"derived matter\" | tasktree add --parent <PARENT> --from <REF>")]
    Add {
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Parent strand id. Creates a belongs-to edge from the new child to this parent.
        #[arg(long = "parent", visible_alias = "belongs-to", value_name = "PARENT")]
        parent: Option<String>,
        /// Source record this line derives from: a strand id/prefix (pins its
        /// latest entry) or an entry-hash prefix (pins that exact entry).
        /// Stored as a ref on the new line's first entry. Orthogonal to
        /// --parent: derivation may carry either, both, or neither.
        #[arg(long = "from", value_name = "REF")]
        from: Option<String>,
        /// Strand type: task, dag, why, session (default: auto-detect)
        #[arg(long = "type", value_name = "TYPE")]
        strand_type: Option<String>,
        /// Optional provenance JSON object. Stored on the initial LogAppended entry.
        #[arg(long = "provenance", value_name = "JSON")]
        provenance: Option<String>,
    },
    /// Append stdin content to a strand, or create a new strand from stdin.
    #[command(after_help = "\
Invocation forms:
  tasktree append [--id <ID> | --new]

Content source:
  stdin               Log content. Piped input is required.

Target (choose at most one):
  (none)              Append to most recently active strand
  --id <ID>           Append to a specific strand
  --new               Create a new strand from the content

Rules:
  Entry content is never read from positional arguments, --stdin, or --file.
  --new and --id are mutually exclusive.
  Empty stdin content is rejected.

Examples:
  echo \"short note\" | tasktree append
  echo \"long note\" | tasktree append --id 0000019dd34b
  echo \"new strand title\" | tasktree append --new
  echo \"[metric] win_count=26\" | tasktree append --id 0000019dd34b --provenance '{\"producer\":\"pi\",\"model\":\"gpt-5\"}'

Markers (optional bracket prefix on the first line):
  Marker vocabulary: tasktree explain markers

Provenance:
  --provenance <JSON>  Optional structured metadata. Must be a JSON
                       object. Stored on the LogAppended event, not in
                       the entry text. Older journals ignore it.
  --seen-offset <N>    Caller-declared last observed offset for the target
                       strand. If stale, emits W076 but still writes.")]
    Append {
        /// Create a new strand from stdin content
        #[arg(short, long)]
        new: bool,
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
        /// Pin a rationale. REF is a strand id/prefix (stores that line's
        /// latest entry hash — "its current conclusion") or an entry-hash
        /// prefix (pins that exact entry). During the v2 migration, a legacy
        /// ref=<id>@<offset> citation-frontier pin is also stored.
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
    #[command(after_help = "\
Examples:
  tasktree show 0000019dd34b --tail 8
  tasktree show --entry 3dfc13241d55 --deref 2
  tasktree show --entry 3dfc13241d55 --after 3")]
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
        /// Show one entry by hash prefix instead of a strand, expanding its
        /// rationale refs (see --deref). Cited entries arrive with their home
        /// line, position, and whether that line has since advanced; --before/
        /// --after read the neighbourhood on that line without pulling it whole.
        #[arg(long = "entry", value_name = "HASH")]
        entry: Option<String>,
        /// With --entry: expand refs N hops (default 1; 0 = list refs only).
        /// Refs beyond the boundary are listed with their expansion cost.
        #[arg(long = "deref", value_name = "N", requires = "entry")]
        deref: Option<usize>,
        /// With --entry: show K entries preceding each pulled entry on its
        /// own line — the local deliberation it may lean on
        #[arg(long = "before", value_name = "K", requires = "entry")]
        before: Option<usize>,
        /// With --entry: show K entries following each pulled entry on its
        /// own line — the re-look for (advanced): what the cited line did
        /// after the citation
        #[arg(long = "after", value_name = "K", requires = "entry")]
        after: Option<usize>,
        /// Filter log entries to one writer (matches provenance.producer).
        /// The narrowing dimension for multi-writer journals.
        #[arg(long = "producer", value_name = "NAME")]
        producer: Option<String>,
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
        /// Stored on the link effect entry.
        #[arg(long = "provenance", value_name = "JSON")]
        provenance: Option<String>,
    },
    /// Remove a directed link between two strands (writes an unlink effect entry)
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
        /// Optional provenance JSON object. Stored on the unlink effect entry.
        #[arg(long = "provenance", value_name = "JSON")]
        provenance: Option<String>,
    },
    /// Hide a strand from default list view
    Hide {
        #[command(flatten)]
        target: IdTarget,
        /// Reason for hiding (optional). If provided, stored as '[hidden] <reason>' content on the hide effect entry.
        #[arg(long)]
        reason: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Optional provenance JSON object. Stored on the hide effect entry.
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

    /// Close a strand (write a close effect entry)
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

    /// Reopen a closed strand (write a reopen effect entry)
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
        /// Optional provenance JSON object. Stored on the SubjectBound event.
        #[arg(long = "provenance", value_name = "JSON")]
        provenance: Option<String>,
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
    /// Rewrite/import the current journal into pure v2 form
    #[command(
        name = "cutover-v2",
        after_help = "\
Default is a dry run. --apply performs the cutover in .tasktree/:\n  - moves journal.jsonl to journal.v1.jsonl\n  - writes a new pure-v2 journal.jsonl\n  - writes migration-v1-to-v2.json with old id/offset to new hash mappings\n\nExamples:\n  tasktree cutover-v2\n  tasktree cutover-v2 --format json\n  tasktree cutover-v2 --apply"
    )]
    CutoverV2 {
        /// Apply the cutover. Without this flag, only report the plan.
        #[arg(long)]
        apply: bool,
        /// Archive path for the pre-cutover journal (default: .tasktree/journal.v1.jsonl)
        #[arg(long, value_name = "PATH")]
        archive: Option<String>,
        /// Mapping output path (default: .tasktree/migration-v1-to-v2.json)
        #[arg(long = "map", value_name = "PATH")]
        map: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
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
  continue a line   echo \"[decision] ...\" | tasktree append --id <ID>
  new matter        echo \"<summary>\" | tasktree add
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
    Journal {
        /// Treat advisory warnings as blocking issues
        #[arg(long)]
        strict: bool,
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
    // Journal Core surfaces write-path warnings through an injected sink; the
    // CLI is the presentation layer, so it installs the stderr presenter here.
    crate::journal::set_journal_warning_sink(|message| {
        eprintln!("[tasktree] warning: {}", message);
    });
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
    if let Commands::Checkpoint {
        id,
        action,
        tail,
        format,
        include_hidden,
        provenance,
        seen_offset,
    } = command
    {
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
        Commands::Add {
            format,
            parent,
            from,
            strand_type,
            provenance,
        } => {
            let fmt = format.as_deref() == Some("json");
            cmd_add_from_stdin(
                fmt,
                parent.as_deref(),
                from.as_deref(),
                strand_type.as_deref(),
                provenance.as_deref(),
            )
        }
        Commands::Append {
            new,
            explicit_id,
            format,
            provenance,
            seen_offset,
            why,
        } => cmd_append_from_stdin(
            *new,
            explicit_id.as_deref(),
            format.as_deref(),
            provenance.as_deref(),
            *seen_offset,
            why.as_deref(),
        ),
        Commands::List {
            all,
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
                *all,
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
            entry,
            deref,
            before,
            after,
            producer,
            format,
            locked,
        } => {
            let fmt = format.as_deref() == Some("json");
            if let Some(prefix) = entry {
                if target.get().is_some()
                    || *last
                    || *digest
                    || tail.is_some()
                    || *locked
                    || producer.is_some()
                {
                    return Err(
                        "--entry reads one entry by hash; it does not combine with strand \
                         view flags (a strand id, --last, --tail, --digest, --locked, \
                         --producer)"
                            .to_string(),
                    );
                }
                cmd_show_entry(
                    prefix,
                    deref.unwrap_or(1),
                    before.unwrap_or(0),
                    after.unwrap_or(0),
                    fmt,
                )
            } else {
                cmd_show(
                    target.get(),
                    *last,
                    *tail,
                    fmt,
                    *locked,
                    *digest,
                    producer.as_deref(),
                )
            }
        }
        Commands::Search {
            query,
            format,
            include_hidden,
        } => {
            let fmt = format.as_deref() == Some("json");
            cmd_search(query, fmt, *include_hidden)
        }
        Commands::Find { target, format } => match target.get() {
            Some(id) => cmd_find(id, format.as_deref() == Some("json")),
            None => Err("missing strand id: pass <ID> or --id <ID>".to_string()),
        },
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
            let output = cmd_explain(code, is_json);
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
                DoctorTarget::Journal { strict } => cmd_doctor_journal(*strict),
            };
            match result {
                Ok(true) => Err("journal issues detected".to_string()),
                Ok(false) => Ok(()),
                Err(e) => Err(format!("journal unreadable: {}", e)),
            }
        }

        Commands::Export { out } => cmd_export(out),

        Commands::CutoverV2 {
            apply,
            archive,
            map,
            format,
        } => cmd_cutover_v2(
            *apply,
            archive.as_deref(),
            map.as_deref(),
            format.as_deref() == Some("json"),
        ),
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

        Commands::AgentContext {
            format,
            include_hidden,
        } => cmd_agent_context(format.as_deref(), *include_hidden),

        Commands::Context {
            context_type,
            covers,
            since_offset,
            format,
            exclude_friction,
            include_hidden,
            include_observations,
        } => cmd_context(
            context_type.as_deref(),
            &covers,
            *since_offset,
            format.as_deref(),
            *exclude_friction,
            *include_hidden,
            *include_observations,
        ),

        Commands::Bind {
            subject_type,
            subject_id,
            id,
            stdin,
            format,
            provenance,
        } => {
            let fmt = format.as_deref() == Some("json");
            cmd_bind_with_provenance(
                subject_type.as_deref(),
                subject_id.as_deref(),
                id.as_deref(),
                *stdin,
                fmt,
                provenance.as_deref(),
            )
        }
        Commands::Current {
            subject_type,
            subject_id,
            format,
        } => {
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

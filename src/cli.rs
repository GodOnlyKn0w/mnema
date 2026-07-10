use crate::commands::doctor::*;
use crate::commands::explain::cmd_explain;
use crate::commands::manage::*;
use crate::commands::query::*;
use crate::commands::write::*;
use crate::diagnostics;
use crate::journal::JOURNAL_DIR;
use clap::{Parser, Subcommand, error::ErrorKind};
use std::ffi::OsString;
use std::path::PathBuf;

fn version_info() -> &'static str {
    concat!(
        env!("CARGO_PKG_VERSION"),
        "\njournal schema: mnema-journal-v2",
        "\ncommit: ",
        env!("MNEMA_COMMIT"),
        "\nbuild: ",
        env!("MNEMA_BUILD_PROFILE"),
    )
}

#[derive(Parser)]
#[command(
    name = "mnema",
    version = version_info(),
    after_help = "\
loop: 做一步 -> 看现实变 -> 再想。命令按 loop 阶分组：

  orient        Session entry: active strand menu + catch-up commands

看 / read:
  list          List strands
  show          Show one strand (--digest one-glance, --tail N recent)
  timeline      Chronological entries across strands (+linked)
  search        Full-text search (entry hashes; --marker filter)
  find          Resolve a strand id
  pick          Pick a strand from an arrow-key menu; append reads body from stdin
  tree          Strand forest (belongs-to nesting)
  depends       depends-on upstream review context

做 / change:
  add           Create a new strand
  append        Append an entry to a strand
  close         Close a strand (close effect entry)
  reopen        Reopen a closed strand (reopen effect entry)
  checkpoint    Record context before an irreversible action
  link          Link strands (belongs-to / depends-on)
  unlink        Remove a link (unlink effect entry; projection drops the edge)

管 / manage:
  init          Initialize .mnema/ journal
  hide          Hide a strand from active orient (parked, revivable)
  unhide        Unhide a strand
  doctor        Diagnose journal integrity (journal | edges)
  export        Export journal as standalone audit artifact
  cutover-v2    Rewrite/import current journal into pure v2 form
  cutover-v3    Migrate pure v2 journal into activated v3 (manifest commit)
  explain       Explain a diagnostic code or topic (markers, json, grammar, writing, ...)

Run:  mnema <command> --help"
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

/// Unified target resolution for read/append commands: an explicit id
/// (positional or --id) passes through as-is (the command resolves the prefix
/// itself); otherwise (--last, or no target at all) fall through to the most
/// recently active strand. Returns the resolved strand id string to hand to
/// the command. Used so `show`, `find`, `hide`, `unhide`, `tree`, `depends`
/// share one convention: id / --id / --last, defaulting to most-recent.
fn resolve_read_target(target: &IdTarget) -> Result<String, String> {
    match target.get() {
        Some(s) => Ok(s.to_string()),
        None => {
            let path = crate::journal::ensure_journal()?;
            let (events, _) = crate::journal::read_events_lossy(&path);
            resolve_most_recent_strand(&crate::projection::project_strands(&events, true))
                .map(|s| s.id.clone())
                .ok_or_else(|| {
                    "no active strand to default to — pass <ID>, --id, or --last".to_string()
                })
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize .mnema/ directory and journal
    Init,
    /// Create a new strand with first log entry from stdin
    #[command(after_help = "\
Content source:
  stdin               First log entry content. Piped input is required.

Rules:
  Entry content is never read from positional arguments, --stdin, or --file.
  Empty stdin content is rejected.

Examples:
  echo \"start a new line of work\" | mnema add
  echo \"child line of work\" | mnema add --parent <PARENT>
  echo \"derived matter\" | mnema add --parent <PARENT> --from <REF>")]
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
        /// Human alias for the new strand. Must be unique and not pure hex.
        #[arg(long = "slug", value_name = "SLUG")]
        slug: Option<String>,
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
  mnema append [--id <ID> | --new]

Content source:
  stdin               Log content. Piped input is required.

Target (choose at most one):
  (none)              Append to most recently active strand
  --last              Append to most recently active strand (explicit form of none)
  --id <ID>           Append to a specific strand
  --new               Create a new strand from the content

Rules:
  Entry content is never read from positional arguments, --stdin, or --file.
  --new, --id, and --last are mutually exclusive.
  Empty stdin content is rejected.
  Explicit --id can append to a closed strand; that still writes and emits W059.

Examples:
  echo \"short note\" | mnema append
  echo \"short note\" | mnema append --last
  echo \"long note\" | mnema append --id 0000019dd34b
  echo \"new strand title\" | mnema append --new
  echo \"[metric] win_count=26\" | mnema append --id 0000019dd34b --provenance '{\"producer\":\"pi\",\"model\":\"gpt-5\"}'

Markers (optional bracket prefix on the first line):
  Marker vocabulary: mnema explain markers

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
        /// Append to the most recently active strand — the explicit form of
        /// omitting --id. Conflicts with --id and --new.
        #[arg(long, conflicts_with_all = ["explicit_id", "new"])]
        last: bool,
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
        /// prefix (pins that exact entry).
        #[arg(long = "why", value_name = "REF")]
        why: Option<String>,
    },
    /// Record context before an irreversible or state-closing action
    #[command(after_help = "\
Invocation forms:
  mnema checkpoint --action \"<action and reason>\"
  mnema checkpoint --last --action \"<action and reason>\"
  mnema checkpoint --id <STRAND_ID> --action \"<action and reason>\"
  mnema checkpoint --id <STRAND_ID> --tail 30 --format json --action \"<action and reason>\"

Required:
  --action <TEXT>    Agent-supplied action and reason. Recorded, not classified.

Target:
  --id <STRAND_ID>   Use explicit strand. Prefer this for git commits and destructive actions.
  --last             Resolve to most recently active strand (explicit form of omitting --id).
  omitted --id       Resolve to most recently active strand; stdout shows resolved_by.

Output:
  default            Human-readable stdout + journal append. The strand line
                     includes entry count and state for at-a-glance confirmation.
  --format json      Machine-readable stdout + journal append. Includes a
                     \"result\" field with the updated strand card (OrientStrand).

  staleness          Always printed: age of strand's last entry + journal delta
                     since that entry. Catch-up command shown when delta > 0.
  catch-up           mnema timeline --since-offset <N> --links <STRAND_ID>
                     (emitted verbatim when journal delta > 0)
  warnings           W071 (closed strand) and W076 (--seen-offset behind target
                     last_offset) fire as scar
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
JSON shape: mnema explain json")]
    Checkpoint {
        /// Strand ID (prefix match). Prefer explicit --id for commits and destructive actions.
        #[arg(long = "id", value_name = "STRAND_ID")]
        id: Option<String>,
        /// Resolve to the most recently active strand — the explicit form of
        /// omitting --id. Conflicts with --id.
        #[arg(long, conflicts_with = "id")]
        last: bool,
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
    #[command(after_help = "\
Examples:
  mnema list
  mnema list --under <ID>
  mnema list --under <ID> --format json
  mnema list --stale 2h

--under X: same fields/schema as journal list; candidate set is SubtreeScope(X)
(X plus belongs-to descendants). Collection queries share this flag.")]
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
        /// Filter to registered strands silent for duration (s/m/h/d, e.g. 2h).
        /// Handoff candidates only — closed strands are excluded.
        #[arg(long, value_name = "DURATION")]
        stale: Option<String>,
        /// Filter to registered strands with last entry offset <= N (silent).
        /// Same handoff intent as --stale; closed strands are excluded.
        #[arg(long, value_name = "N", conflicts_with = "since_offset")]
        stale_offset: Option<usize>,
        /// Filter to strands with last entry offset > N (updated since)
        #[arg(long, value_name = "N", conflicts_with = "stale_offset")]
        since_offset: Option<usize>,
        /// Restrict to belongs-to subtree rooted at ID (SubtreeScope)
        #[arg(long, value_name = "ID")]
        under: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
    /// Show full details of one strand
    #[command(after_help = "\
Examples:
  mnema show 0000019dd34b --tail 8
  mnema show --entry 3dfc13241d55 --deref 2
  mnema show --entry 3dfc13241d55 --after 3")]
    Show {
        #[command(flatten)]
        target: IdTarget,
        /// Show the most recently active strand instead of specifying an id
        /// (explicit form of giving no id)
        #[arg(long, conflicts_with_all = ["id_pos", "id_flag"])]
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
    /// Full-text search across all log content (entry-level hits with hashes)
    #[command(after_help = "\
Examples:
  mnema search friction
  mnema search --marker friction
  mnema search win_count --marker metric --format json
  mnema search --marker decision --format json
  mnema search friction --under <ID>
  mnema search --marker decision --under <ID> --format json

Each hit is entry-level: entry hash prefix + marker + first line.
Use the hash for fixes=<hash> / --why <hash>. Filter with --marker
(friction/decision/metric/… — see mnema explain markers).
--under X: search only inside SubtreeScope(X) (same schema, smaller candidate set).")]
    Search {
        /// Search query (substring match, case-insensitive). Optional when --marker is set.
        #[arg(default_value = "")]
        query: String,
        /// Filter to entries whose leading marker matches NAME (with or without brackets)
        #[arg(long, value_name = "NAME")]
        marker: Option<String>,
        /// Include hidden strands in the result set (default: exclude)
        #[arg(long)]
        include_hidden: bool,
        /// Restrict to belongs-to subtree rooted at ID (SubtreeScope)
        #[arg(long, value_name = "ID")]
        under: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
    /// Resolve a prefix to full strand ID
    Find {
        #[command(flatten)]
        target: IdTarget,
        /// Resolve the most recently active strand (explicit form of giving no id)
        #[arg(long, conflicts_with_all = ["id_pos", "id_flag"])]
        last: bool,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
    /// Pick a strand, then run a read/manage command on it
    #[command(after_help = "\
Examples:
  mnema pick show
  mnema pick tree
  echo \"short note\" | mnema pick append
  mnema pick --print-id
  mnema pick --under <ID> show

Rules:
  Opens an arrow-key menu (up/down to move, type to filter, Enter to select,
  Esc to cancel). In non-TTY contexts it exits with an error instead of
  waiting for input. append selects a strand interactively and reads its body
  from stdin: echo ... | mnema pick append.
--under X: only strands in SubtreeScope(X) appear in the menu.")]
    Pick {
        /// Command to run with the selected strand: show, tree, depends, append, close, reopen, hide, unhide
        #[arg(value_name = "COMMAND", default_value = "show")]
        command: String,
        /// Print the selected canonical full strand id instead of running a command
        #[arg(long = "print-id")]
        print_id: bool,
        /// Include closed and hidden strands (default: only active + visible; --all is an alias)
        #[arg(long = "include-hidden", alias = "all")]
        include_hidden: bool,
        /// Restrict candidates to belongs-to subtree rooted at ID (SubtreeScope)
        #[arg(long, value_name = "ID")]
        under: Option<String>,
    },
    /// Create a directed link between two strands
    #[command(after_help = "\
Direction (read SOURCE first): the edge always points from SOURCE to TARGET.

  belongs-to   SOURCE belongs to TARGET — source is the child, target is the
               parent. tree and orient --tree nest SOURCE under TARGET.
  depends-on   SOURCE depends on TARGET (review upstream). [default]

  (why is no longer a link (D2): a reason is an entry rationale, not a
   strand edge. Record the reason in the entry text itself.)

Examples:
  mnema link <CHILD> <PARENT> --edge-type belongs-to
               (CHILD nests under PARENT in tree / orient --tree)
  mnema link <TASK> <UPSTREAM> --edge-type depends-on
               (TASK cites UPSTREAM for review context)

Forest projection (how belongs-to nests): mnema explain card
JSON shape: mnema explain json")]
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
  mnema unlink <TASK> <UPSTREAM> --edge-type depends-on")]
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
        /// Hide the most recently active strand (explicit form of giving no id)
        #[arg(long, conflicts_with_all = ["id_pos", "id_flag"])]
        last: bool,
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
        /// Unhide the most recently active strand (explicit form of giving no id)
        #[arg(long, conflicts_with_all = ["id_pos", "id_flag"])]
        last: bool,
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

Reason (optional): pipe a closing note on stdin; it rides on the close entry.

Target: close is a lifecycle-closing action, so it never defaults — name the
strand explicitly, positional <ID> or --id <ID>. No --last, no implicit
most-recent.

Examples:
  mnema close <ID>
  mnema close --id <ID> --as failed
  echo \"verified in staging\" | mnema close --id <ID> --as verified")]
    Close {
        #[command(flatten)]
        target: IdTarget,
        /// Disposition: done (default), failed, cancelled, merged, verified
        #[arg(long = "as", value_name = "DISPOSITION")]
        disposition: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },

    /// Reopen a closed strand (write a reopen effect entry)
    ///
    /// Moves the strand back to open/registered state. Reopen means undoing an
    /// erroneous close (CORPUS §6); pipe a reason on stdin to say why.
    #[command(after_help = "\
Target: name the strand explicitly, positional <ID> or --id <ID>. Like close,
reopen never defaults to most-recent.

Examples:
  mnema reopen <ID>
  echo \"closed by mistake, still active\" | mnema reopen --id <ID>")]
    Reopen {
        #[command(flatten)]
        target: IdTarget,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },

    /// Explain a diagnostic code or encyclopaedia topic
    ///
    /// Namespace rule: diagnostic codes begin with an uppercase letter
    /// (W068, E053); topics are all-lowercase (card, markers, retry, json, jq, grammar, writing, collaboration, delegation).
    /// The two namespaces are mechanically disjoint.
    #[command(after_help = "\
Namespaces:
  Diagnostic codes   uppercase-initial: W068, E053, w068 (case-insensitive)
  Topics             all-lowercase:     card, markers, retry, json, jq, grammar, writing, collaboration, delegation

Topics:
  card      卡片：统一输出文法单元（格式、字段、回显语义）
  markers   Marker 词表（[decision]、[done] 等前缀规范）
  retry     重试语义：哪些命令可盲目重试
  json      JSON 形态索引：各读命令 --format json 的顶层字段
  jq        jq 整型：把 --format json 输出切成你要的形
  grammar   文法契约：全 CLI 一致的参数与命名规则
  writing   写入时机、entry 模板、临时 journal 演练脚本
  collaboration  协作 forest：多路工作在 journal 里的形状
  delegation     递归异步委派：child strand、refs、显式 id 与验收时机

Examples:
  mnema explain W068
  mnema explain card
  mnema explain json
  mnema explain markers
  mnema explain retry
  mnema explain grammar
  mnema explain writing
  mnema explain collaboration
  mnema explain delegation
  mnema explain W068 --format json
  mnema explain card --json")]
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
Default is a dry run. --apply performs the cutover in .mnema/:\n  - moves journal.jsonl to journal.v1.jsonl\n  - writes a new pure-v2 journal.jsonl\n  - writes migration-v1-to-v2.json with old id/offset to new hash mappings\n\nExamples:\n  mnema cutover-v2\n  mnema cutover-v2 --format json\n  mnema cutover-v2 --apply"
    )]
    CutoverV2 {
        /// Apply the cutover. Without this flag, only report the plan.
        #[arg(long)]
        apply: bool,
        /// Archive path for the pre-cutover journal (default: .mnema/journal.v1.jsonl)
        #[arg(long, value_name = "PATH")]
        archive: Option<String>,
        /// Mapping output path (default: .mnema/migration-v1-to-v2.json)
        #[arg(long = "map", value_name = "PATH")]
        map: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
    /// Migrate pure v2 journal into activated v3 form
    #[command(
        name = "cutover-v3",
        after_help = "\
Default is a dry run (plan + projection equivalence, no durable writes).\n\
--apply prepares history/v3 artifacts and atomically installs active-journal.json.\n\
Failure never activates. Repeat --apply is resume/noop when artifacts match.\n\
\nExamples:\n  mnema cutover-v3\n  mnema cutover-v3 --format json\n  mnema cutover-v3 --apply"
    )]
    CutoverV3 {
        /// Apply the cutover (prepare artifacts + activate). Without this flag, only report the plan.
        #[arg(long)]
        apply: bool,
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
        /// Restrict to events from strands in SubtreeScope(ID) (belongs-to descendants + root)
        #[arg(long = "under", value_name = "ID", conflicts_with_all = ["strand", "links"])]
        under: Option<String>,
        /// Maximum events to return (from the start of the filtered window)
        #[arg(long, value_name = "N", conflicts_with = "tail")]
        limit: Option<usize>,
        /// Return only the last N events (recent tail of the filtered window)
        #[arg(long, value_name = "N", conflicts_with = "limit")]
        tail: Option<usize>,
    },
    /// Session-start orientation: menu of active strands with catch-up commands
    #[command(after_help = "\
Pure read: orient never writes to the journal.

Examples:
  mnema orient
  mnema orient --id <ID>
  mnema orient --id <ID> --tree
  mnema orient --format json

Output per active strand:
  handle        Strand id (use with --id)
  summary       First entry (what this line of work is)
  last          Most recent entry (where it left off)
  catch-up      Ready-to-run command showing this strand's recent content
                window: mnema show --id <ID> --tail 8

After orienting:
  writing guide     mnema explain writing
  continue a line   echo \"[decision] ...\" | mnema append --id <ID>
  new matter        echo \"<summary>\" | mnema add
  matter concluded  mnema close --id <ID> [--as done|failed|cancelled|merged|verified]
                    (default: done; reopen with mnema reopen --id <ID>)
  before anything irreversible
                    mnema checkpoint --id <ID> --action \"<what and why>\"

Closed strands are folded to a count; retrieve with mnema list.
Hidden strands are folded to a count; retrieve with mnema list --all.

--id X: dedicated delegated entry — candidate set is SubtreeScope(X)
  (X plus belongs-to descendants). Same set as collection queries' --under X.
--tree: render active strands as a belongs-to forest. Strands that declare
  a belongs-to edge to another active strand are indented under their parent;
  parallel siblings under the same parent are visible as a group.
  Default orient (no --tree) is unchanged: flat list ordered by last_offset.
Exit codes:
  0 ok
  1 journal missing or unreadable
JSON shape: mnema explain json")]
    Orient {
        /// Scope menu to belongs-to subtree rooted at ID (SubtreeScope; dedicated entry)
        #[arg(long, value_name = "ID")]
        id: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Include hidden strands in the menu (default: exclude)
        #[arg(long)]
        include_hidden: bool,
        /// Maximum strands in the menu, most recent first (default: adaptive by journal maturity)
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
        /// Root the tree at the most recently active strand (explicit form of giving no id)
        #[arg(long, conflicts_with_all = ["id_pos", "id_flag"])]
        last: bool,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
    /// Inspect depends-on upstreams as review context
    #[command(after_help = "\
depends-on is an attention edge for review and handoff context, not an
execution gate. Output lists upstream lifecycle facts and show handles;
lifecycle is evidence, not a verdict. Built on the typed depends-on projection (F3).
Does not compute ready/blocker/critical-path.

Single strand (default): one task's direct depends-on upstreams.
--under X: scoped set — same per-strand facts for every strand in
SubtreeScope(X) (X plus belongs-to descendants).

Examples:
  mnema depends <TASK>
  mnema depends <TASK> --format json
  mnema depends --under <ID>
  mnema depends --under <ID> --format json")]
    Depends {
        #[command(flatten)]
        target: IdTarget,
        /// Review the most recently active strand (explicit form of giving no id)
        #[arg(long, conflicts_with_all = ["id_pos", "id_flag", "under"])]
        last: bool,
        /// List depends-on upstream facts for each strand in SubtreeScope(ID)
        #[arg(long, value_name = "ID", conflicts_with_all = ["id_pos", "id_flag", "last"])]
        under: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },
}

#[derive(Subcommand)]
enum DoctorTarget {
    /// Check journal integrity (always JournalScope — parse/hash/anchor
    /// integrity cannot be narrowed by --under/--id; scope must not hide
    /// container damage)
    Journal,
    /// Edge-discipline self-check: open unfixed [friction] + [decision] without --why
    #[command(after_help = "\
Examples:
  mnema doctor edges
  mnema doctor edges --format json
  mnema doctor edges --since 1200
  mnema doctor edges --under <ID>
  mnema doctor edges --id <ID>

Read-only projection of the tool's own edge discipline:
  (a) unfixed [friction]: any [friction] not targeted by any
      [fixed] fixes=<hash> — home-strand open/closed does not matter
      (dual count: total / of which on active strands)
  (b) [decision] entries recorded without a --why ref
      (--since N skips decisions at offset <= N; pre-policy stock)
JSON twin: open_frictions[] / decisions_without_why[] with entry_id;
  open_friction_count + open_friction_active_count.
Same findings schema always; --under X / --id X only shrink the candidate
set (JournalScope default; SubtreeScope / single strand). Fix knowledge
still uses the full journal. doctor journal integrity stays JournalScope.")]
    Edges {
        /// Output format: text (default) or json
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Skip [decision] entries at journal offset <= N (legacy pre-policy
        /// stock). Unfixed frictions are never skipped by this floor.
        #[arg(long, value_name = "N")]
        since: Option<usize>,
        /// Restrict findings to strands in SubtreeScope(ID)
        #[arg(long, value_name = "ID", conflicts_with = "id")]
        under: Option<String>,
        /// Restrict findings to a single strand
        #[arg(long, value_name = "ID", conflicts_with = "under")]
        id: Option<String>,
    },
}

/// NOTE: Strand sort key is `max(log_appended.ts)` per strand.
fn cmd_init() -> Result<(), String> {
    use crate::activation::{
        ACTIVE_MANIFEST_SCHEMA, ActivationOriginV3, ActiveJournalManifestV3, JournalArtifactV3,
        activate_initial_v3, load_active_manifest,
    };
    use crate::cutover_v3::TARGET_V3_REL;
    use crate::journal::sha256_bytes;
    use crate::journal_v3::{make_anchor, write_records_prepared};

    let dir = PathBuf::from(JOURNAL_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create .mnema/: {}", e))?;
    let lock_path = dir.join("journal.lock");
    if !lock_path.exists() {
        std::fs::write(&lock_path, "").map_err(|e| format!("cannot create journal.lock: {}", e))?;
    }
    let journal_id = crate::journal::ensure_journal_id_in(&dir)?.to_ascii_lowercase();

    // Already activated: idempotent success.
    if load_active_manifest(&dir)?.is_some() {
        println!("Initialized empty mnema in .mnema/ (already active)");
        println!("  journal-id: {}", journal_id);
        return Ok(());
    }

    // Legacy v2 source present without manifest: keep transitional v2 layout.
    let legacy = dir.join("journal.jsonl");
    if legacy.exists() {
        let meta = std::fs::metadata(&legacy)
            .map_err(|e| format!("stat legacy journal: {e}"))?;
        if meta.len() > 0 {
            println!("Initialized empty mnema in .mnema/");
            println!("  journal-id: {}", journal_id);
            println!("  note: legacy journal.jsonl present; run mnema cutover-v3 --apply to activate v3");
            return Ok(());
        }
    }

    // Fresh directory: create pure v3 journal (final anchor over 0 records) +
    // ActivationOriginV3::Fresh. Never stage a v2 file then self-migrate.
    std::fs::create_dir_all(dir.join("journals"))
        .map_err(|e| format!("create journals dir: {e}"))?;
    let target = dir.join(TARGET_V3_REL);
    let records = vec![make_anchor(
        &journal_id,
        &[],
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
    )?];
    let target_sha = if target.exists() {
        let bytes = std::fs::read(&target).map_err(|e| format!("read existing v3: {e}"))?;
        sha256_bytes(&bytes)
    } else {
        write_records_prepared(&target, &journal_id, &records)?
    };

    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    hasher.update(b"mnema.fresh-activation.v1\0");
    hasher.update(journal_id.as_bytes());
    let fresh_id = hex::encode(hasher.finalize());

    let manifest = ActiveJournalManifestV3 {
        schema: ACTIVE_MANIFEST_SCHEMA.to_string(),
        journal_id: journal_id.clone(),
        active: JournalArtifactV3 {
            schema: "v3".to_string(),
            path: TARGET_V3_REL.to_string(),
            sha256: target_sha,
        },
        history: Vec::new(),
        origin: ActivationOriginV3::Fresh { id: fresh_id },
    };
    activate_initial_v3(&dir, &manifest)?;

    println!("Initialized empty mnema in .mnema/ (v3 fresh)");
    println!("  journal-id: {}", journal_id);
    println!("  active: {}", TARGET_V3_REL);
    Ok(())
}

// ── exit strategy ──
// Commands return typed-by-prefix error strings; this adapter is the single
// place that turns them into process output and exit codes.
pub(crate) fn main() {
    // Journal Core surfaces write-path warnings through an injected sink; the
    // CLI is the presentation layer, so it installs the stderr presenter here.
    crate::journal::set_journal_warning_sink(|message| {
        eprintln!("[mnema] warning: {}", message);
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
    let args: Vec<OsString> = std::env::args_os().collect();
    match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(err) => {
            let kind = err.kind();
            let code = match kind {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
                _ => 3,
            };
            let recovery = parse_error_recovery_hint(&args, kind);
            let _ = err.print();
            if let Some(recovery) = recovery {
                eprintln!("\n{}", recovery);
            }
            std::process::exit(code);
        }
    }
}

pub(crate) fn parse_error_recovery_hint(args: &[OsString], kind: ErrorKind) -> Option<String> {
    if matches!(kind, ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
        return None;
    }
    let tokens: Vec<String> = args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let (command_index, command) = first_cli_command(&tokens)?;
    if matches!(command, "add" | "append") {
        if let Some(hint) = stdin_body_recovery(command, &tokens[command_index + 1..]) {
            return Some(hint);
        }
    }
    if !is_known_cli_command(command) {
        return nearest_cli_command(command).map(|nearest| {
            format!(
                "try this:
  mnema {} --help",
                nearest
            )
        });
    }
    canonical_cli_shape(command).map(|shape| {
        format!(
            "standard shape:
  {}",
            shape
        )
    })
}

fn first_cli_command(tokens: &[String]) -> Option<(usize, &str)> {
    let mut i = 1usize;
    while i < tokens.len() {
        let token = tokens[i].as_str();
        if token == "-C" || token == "--chdir" {
            i += 2;
            continue;
        }
        if token.starts_with("--chdir=") || (token.starts_with("-C") && token.len() > 2) {
            i += 1;
            continue;
        }
        if token.starts_with('-') {
            i += 1;
            continue;
        }
        return Some((i, token));
    }
    None
}

/// Strand-id prefix shape for rescue: all hex, length >= 4.
/// Aligns with what mnema can actually resolve (find/show accept short
/// prefixes). Threshold was >=8 and missed real prefixes like `abc123`
/// (6 hex), folding them into the entry body and teaching dirty writes.
fn looks_like_strand_id_token(token: &str) -> bool {
    token.len() >= 4 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

fn flag_list_has_id(kept: &[String]) -> bool {
    kept.iter().any(|t| t == "--id" || t.starts_with("--id="))
}

fn stdin_body_recovery(command: &str, rest: &[String]) -> Option<String> {
    let mut kept: Vec<String> = Vec::new();
    let mut body: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < rest.len() {
        let token = rest[i].as_str();
        if token.starts_with("--") {
            kept.push(rest[i].clone());
            if !token.contains('=') && stdin_command_flag_takes_value(command, token) {
                if let Some(value) = rest.get(i + 1) {
                    kept.push(value.clone());
                    i += 2;
                    continue;
                }
            }
            i += 1;
            continue;
        }
        if token.starts_with('-') {
            kept.push(rest[i].clone());
            if stdin_command_flag_takes_value(command, token) {
                if let Some(value) = rest.get(i + 1) {
                    kept.push(value.clone());
                    i += 2;
                    continue;
                }
            }
            i += 1;
            continue;
        }
        body.push(rest[i].clone());
        i += 1;
    }
    if body.is_empty() {
        return None;
    }

    // append: leading id-shaped token is a mis-placed strand target → --id.
    // Never echo that token as body (teaches dirty write to wrong line).
    if command == "append" && looks_like_strand_id_token(&body[0]) {
        let id = body[0].as_str();
        let text = body[1..].join(" ");
        let mut corrected = format!("mnema {}", command);
        for token in &kept {
            corrected.push(' ');
            corrected.push_str(token);
        }
        if !flag_list_has_id(&kept) {
            corrected.push_str(" --id ");
            corrected.push_str(id);
        }
        if text.is_empty() {
            // Pure id: point at the line; body still comes from stdin.
            return Some(format!(
                "try this:
  {}",
                corrected
            ));
        }
        return Some(format!(
            "try this:
  echo {} | {}",
            echo_double_quoted(&text),
            corrected
        ));
    }

    // add has no target id; strip id-shaped tokens so rescue never
    // suggests writing a strand hash as the new strand's summary.
    let body_text = if command == "add" {
        let text: Vec<&str> = body
            .iter()
            .map(|s| s.as_str())
            .filter(|t| !looks_like_strand_id_token(t))
            .collect();
        if text.is_empty() {
            return None;
        }
        text.join(" ")
    } else {
        body.join(" ")
    };

    let mut corrected = format!("mnema {}", command);
    for token in kept {
        corrected.push(' ');
        corrected.push_str(&token);
    }
    Some(format!(
        "try this:
  echo {} | {}",
        echo_double_quoted(&body_text),
        corrected
    ))
}

fn stdin_command_flag_takes_value(command: &str, flag: &str) -> bool {
    let flag = flag.split_once('=').map(|(name, _)| name).unwrap_or(flag);
    match command {
        "add" => matches!(
            flag,
            "--format"
                | "--parent"
                | "--belongs-to"
                | "--from"
                | "--slug"
                | "--type"
                | "--provenance"
        ),
        "append" => matches!(
            flag,
            "--format" | "-f" | "--id" | "--provenance" | "--seen-offset" | "--why"
        ),
        _ => false,
    }
}

fn echo_double_quoted(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", escaped)
}

fn canonical_cli_shape(command: &str) -> Option<&'static str> {
    match command {
        "add" => Some(r#"echo "<summary>" | mnema add"#),
        "append" => Some(r#"echo "<entry>" | mnema append --id <ID>"#),
        "checkpoint" => Some(r#"mnema checkpoint --id <ID> --action "<action and reason>""#),
        "close" => Some("mnema close --id <ID>"),
        "depends" => Some("mnema depends --id <ID>"),
        "doctor" => Some("mnema doctor journal"),
        "export" => Some("mnema export --out <PATH>"),
        "find" => Some("mnema find <ID>"),
        "hide" => Some("mnema hide --id <ID>"),
        "init" => Some("mnema init"),
        "link" => Some("mnema link <SOURCE> <TARGET> --edge-type depends-on"),
        "list" => Some("mnema list"),
        "orient" => Some("mnema orient"),
        "pick" => Some("mnema pick show"),
        "reopen" => Some("mnema reopen --id <ID>"),
        "search" => Some("mnema search <query>"),
        "show" => Some("mnema show --id <ID>"),
        "timeline" => Some("mnema timeline"),
        "tree" => Some("mnema tree --id <ID>"),
        "unlink" => Some("mnema unlink <SOURCE> <TARGET> --edge-type depends-on"),
        "unhide" => Some("mnema unhide --id <ID>"),
        "cutover-v2" => Some("mnema cutover-v2 --out <PATH>"),
        "cutover-v3" => Some("mnema cutover-v3 --apply"),
        "explain" => Some("mnema explain grammar"),
        _ => None,
    }
}

fn is_known_cli_command(command: &str) -> bool {
    canonical_cli_shape(command).is_some()
}

fn nearest_cli_command(command: &str) -> Option<&'static str> {
    const COMMANDS: &[&str] = &[
        "add",
        "append",
        "checkpoint",
        "close",
        "depends",
        "doctor",
        "export",
        "find",
        "hide",
        "init",
        "link",
        "list",
        "orient",
        "pick",
        "reopen",
        "search",
        "show",
        "timeline",
        "tree",
        "unlink",
        "unhide",
        "cutover-v2",
        "cutover-v3",
        "explain",
    ];
    COMMANDS
        .iter()
        .copied()
        .map(|candidate| (candidate, edit_distance(command, candidate)))
        .filter(|(_, distance)| *distance <= 2)
        .min_by_key(|(_, distance)| *distance)
        .map(|(candidate, _)| candidate)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr = vec![0usize; b_chars.len() + 1];
    for (i, ca) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b_chars.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_chars.len()]
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
        last: _,
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
            slug,
            strand_type,
            provenance,
        } => {
            let fmt = format.as_deref() == Some("json");
            cmd_add_from_stdin(
                fmt,
                parent.as_deref(),
                from.as_deref(),
                slug.as_deref(),
                strand_type.as_deref(),
                provenance.as_deref(),
            )
        }
        Commands::Append {
            new,
            explicit_id,
            last: _,
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
            under,
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
                under.as_deref(),
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
            marker,
            format,
            include_hidden,
            under,
        } => {
            let fmt = format.as_deref() == Some("json");
            cmd_search(
                query,
                fmt,
                *include_hidden,
                marker.as_deref(),
                under.as_deref(),
            )
        }
        Commands::Find {
            target,
            last: _,
            format,
        } => {
            let id = resolve_read_target(target)?;
            cmd_find(&id, format.as_deref() == Some("json"))
        }
        Commands::Pick {
            command,
            print_id,
            include_hidden,
            under,
        } => cmd_pick(command, *print_id, *include_hidden, under.as_deref()),
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
            last: _,
            reason,
            format,
            provenance,
        } => {
            let id = resolve_read_target(target)?;
            cmd_hide(
                &id,
                reason.as_deref(),
                format.as_deref() == Some("json"),
                provenance.as_deref(),
            )
        }
        Commands::Unhide {
            target,
            last: _,
            format,
        } => {
            let id = resolve_read_target(target)?;
            cmd_unhide(&id, format.as_deref() == Some("json"))
        }

        Commands::Close {
            target,
            disposition,
            format,
        } => {
            let id = target.get().ok_or(
                "close needs an explicit target: <ID> or --id <ID> (no --last, no default)",
            )?;
            let fmt = format.as_deref() == Some("json");
            // Optional author reason via a pipe; a bare close (no pipe) still works.
            let reason = crate::util::read_stdin_if_piped();
            cmd_close(id, disposition.as_deref(), reason.as_deref(), fmt)
        }

        Commands::Reopen { target, format } => {
            let id = target.get().ok_or(
                "reopen needs an explicit target: <ID> or --id <ID> (no --last, no default)",
            )?;
            let fmt = format.as_deref() == Some("json");
            let reason = crate::util::read_stdin_if_piped();
            cmd_reopen(id, reason.as_deref(), fmt)
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
            tail,
            under,
        } => cmd_timeline(
            *since_offset,
            since_ts.as_deref(),
            *until_offset,
            until_ts.as_deref(),
            strand.as_deref(),
            links.as_deref(),
            format.as_deref(),
            *limit,
            *tail,
            under.as_deref(),
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
                DoctorTarget::Journal => cmd_doctor_journal(),
                DoctorTarget::Edges {
                    format,
                    since,
                    under,
                    id,
                } => cmd_doctor_edges(
                    format.as_deref() == Some("json"),
                    *since,
                    under.as_deref(),
                    id.as_deref(),
                ),
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
        Commands::CutoverV3 { apply, format } => {
            cmd_cutover_v3(*apply, format.as_deref() == Some("json"))
        }
        Commands::Tree {
            target,
            last: _,
            format,
        } => {
            let id = resolve_read_target(target)?;
            cmd_tree(&id, format.as_deref())
        }

        Commands::Depends {
            target,
            last: _,
            under,
            format,
        } => {
            if let Some(root) = under.as_deref() {
                cmd_depends_under(root, format.as_deref())
            } else {
                let id = resolve_read_target(target)?;
                cmd_depends(&id, format.as_deref())
            }
        }

        Commands::Orient {
            id,
            format,
            include_hidden,
            limit,
            tree,
        } => cmd_orient(
            format.as_deref(),
            *include_hidden,
            *limit,
            *tree,
            id.as_deref(),
        ),

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

//! Unified diagnostic catalog — single source of truth for all diagnostic codes.
//!
//! Every code emitted by any producer (currently: lifecycle, health) MUST
//! have an entry here. The `mnema explain` command queries this catalog.
//!
//! # Catalog closure contract
//!
//! Adding a new diagnostic code without a corresponding catalog entry is a bug.
//! Closure is two-way:
//!   1. Every emitted code must resolve via `mnema explain --json <code>`
//!      with `ok: true` (no orphan emissions).
//!   2. Every catalog entry should have a live producer (no dead codes lying
//!      about checks that no longer run).
//!
//! # Code permanence
//!
//! Codes are permanent vocabulary: once a code has shipped, its number is
//! never reused for a different meaning (journals reference codes; reuse
//! makes history lie). 2026-06: 16 codes belonging to an external workflow
//! (gate/shuttle/covers/DAG/story — producers outside this repo) were
//! removed; see git history and `test_removed_workflow_codes_stay_removed`.

// ── Topic catalog (L3 encyclopaedia layer) ──────────────────

/// One encyclopaedia topic reachable via `mnema explain <name>`.
/// Topic names are lowercase. Diagnostics include W-codes and lowercase named
/// migration/activation keys. Lookup checks diagnostics before topics; catalog
/// closure tests prevent duplicate keys. Topic lookup itself is exact.
pub struct TopicInfo {
    pub name: &'static str,
    pub title: &'static str,
    pub body: &'static str,
}

static TOPICS: &[TopicInfo] = &[
    TopicInfo {
        name: "card",
        title: "Card — the shared output unit",
        body: r#"The same card shape appears after writes, in orient, and in JSON result fields.

Text shape:
  handle   <id> [type] | <n> entries | <state>
  summary  first entry, describing the strand's subject
  last:    latest entry (only when entry_count > 1)
  scar     transient W-code diagnostics produced by this write, when any

The state is registered or closed:<disposition>. Only close/reopen changes
lifecycle; append markers are annotations.

A write card is prepaid verification: add, append, checkpoint, hide, unhide,
link, close, and reopen acknowledge the affected strand without requiring an
immediate show/orient round trip.

JSON card fields (OrientStrand, write result, orient active[]):
  id / slug       full strand id and optional human alias
  strand_type     optional task/dag/why/session type
  entry_count     number of log entries
  summary         first entry, truncated to 70 characters
  last_entry      latest entry, truncated to 70 characters
  last_offset     journal offset of the strand's latest event
  catch_up        ready command for a bounded recent window
  lifecycle       registered or closed:<disposition>

See mnema explain json: complete JSON shape index."#,
    },
    TopicInfo {
        name: "markers",
        title: "Markers — structured entry annotations",
        body: r#"A marker is a machine-readable bracketed prefix on an entry's first line.
Markers annotate content; they never change strand lifecycle. Use close/reopen
for lifecycle transitions.

judgment:    [decision] [constraint] [friction] [fixed] [lesson] [insight]
observation: [observed] [check] [progress] [deliverable] [metric]
planning:    [deadline] <text> by=YYYY-MM-DD  (or by=<RFC3339>)
structure:   [covers] [guide] [skill] [task] [session]
annotation:  [done] [verified] [cancelled] [failed] [merged] [ended]
             [dispatched] [registered]
system:      [checkpoint] [hidden] [waiting:human] [grill]

Common meanings:
  [decision]    a decision made
  [constraint]  a constraint that must hold
  [friction]    an unresolved problem
  [fixed]       a repair; fixes=<entry-prefix> identifies its friction
  [observed]    an observed fact
  [progress]    progress; [deliverable] a delivered artifact
  [metric]      a recorded measurement, conventionally name=value
  [deadline]    a deadline with by=<date-or-RFC3339>
  [done]        completion annotation only; use mnema close: lifecycle command
  [checkpoint]  written by mnema checkpoint --action <TEXT>; do not add manually

Unknown bracketed prefixes pass through. W073 flags likely misspellings; W074
redirects lifecycle-like annotations to mnema close."#,
    },
    TopicInfo {
        name: "retry",
        title: "Retry semantics — inspect before repeating writes",
        body: r#"Safe blind retries (idempotent):
  hide     already hidden is an explicit no-op
  unhide   already visible is an explicit no-op
  init     preserves an existing journal and journal id

Do not retry blindly:
  append      writes another LogAppended event
  add         creates another strand
  checkpoint  writes another checkpoint entry
  link        writes another link effect entry
  unlink      writes another unlink effect entry
  close       writes another close effect entry
  reopen      writes another reopen effect entry

Inspect command-specific state before repeating cutover-v2/cutover-v3 --apply.
Their default dry runs are read-only. export writes an external path: inspect
the destination before retrying. Read commands, explain, doctor, and dry-run
cutovers do not mutate the journal.

After a timeout, inspect show, orient, or timeline to determine whether the
event was recorded before deciding to retry."#,
    },
    TopicInfo {
        name: "json",
        title: "JSON shape index — public top-level fields",
        body: r#"show (StrandDetailOutput): id / slug / hidden / summary / entry_count / status / state_marker / state_offset / last_entry_offset /
  edges / belongs_to_edges / depends_on_edges / strand_branch / events; events[].entry is content; last_entry_offset feeds --seen-offset
list (StrandListOutput.strands[], StrandListItem): id / slug / entry_count / first_summary / last_summary / hidden / strand_type /
  edges / belongs_to_edges / depends_on_edges / status / state_marker /
  state_offset / last_entry_ts / last_entry_offset
orient (OrientOutput): max_offset / active / closed_count / hidden_count / integrity / notices / since_command / delegation_command / remind / pause / stale_count / stale_command / scope
  active[] uses the card shape; scope context lists unexpanded parent/depends-on/ref exits and read commands
search (SearchOutput): matches / count / query / marker; matches[]: strand_id / content / strand_type / hidden / entry_id / marker
doctor edges (EdgesOutput): open_frictions / decisions_without_why / open_friction_count / open_friction_active_count / decision_without_why_count
  items: entry_id / strand_id / marker / content / offset; under/id only narrows candidates; fixes= resolution remains journal-wide
  --since N skips existing decisions at offset <= N; doctor journal integrity always uses JournalScope
timeline (TimelineOutput): timeline / truncated / count / max_offset / scope / window
  timeline[]: journal_offset / ts / strand_id / strand_type / kind / ts_skew; scope={kind,root,membership}; window supports safe continuation even with no hits
append: seen_offset / seen_gap / warnings / closed_target / result / resolved_by / active_count
checkpoint: seen_offset / seen_gap / warnings / result
add: id / status / provenance / slug / parent_id / edge_type / result
find: id
hide / unhide: strand_id / status / noop / active_count / closed_count / hidden_count / result (card)
link: source_id / target_id / edge_type / status / result.source / result.target (cards)
unlink: source_id / target_id / edge_type / status / result.source / result.target (cards)
close / reopen: strand_id / status / disposition / result (card)
tree: recursive TreeOutput node with id / summary / children
pick: delegates to the selected command; --print-id emits text only
export: text acknowledgement only; writes the requested audit artifact
doctor journal: text integrity report only; exit 2 means unreadable/corrupt journal
cutover-v2: applied / source_journal / archive_journal / map+certificate / source_event_count / imported_event_count / strand_count / entry_count / anchor_count / unresolved_ref_count
cutover-v3: applied / outcome / migration_id / source|history|target / map+certificate / counts / projection_ok
depends (DependsOutput): id / summary / upstream_count / registered_upstream_count / upstreams[]
  upstreams[]: id / lifecycle / summary / last_entry / show_command; scoped DependsScopeOutput: root_id / count / strands[]
See mnema explain card: result cards. See mnema explain jq: projections."#,
    },
    TopicInfo {
        name: "jq",
        title: "jq projections — consume public JSON without scraping text",
        body: r#"JSON exposes spatial (tree) and temporal (timeline) projections; jq
shapes them. It cannot recover structure buried in prose, so write parseable
markers and name=value metrics when later extraction matters. See
mnema explain json: top-level fields.

Get a strand id:
  echo "..." | mnema add --format json | jq -r .id

Get entry content:
  mnema show --id <ID> --format json | jq -r '.events[].entry'

Select entries by marker:
  mnema show --id <ID> --format json | jq -r '.events[] | select(.entry | startswith("[friction]")) | .entry'
  Prefer startswith; regex backslash quoting is fragile across shells.

Extract a numeric metric series:
  echo "[metric] win_count=26" | mnema append --id <ID>
  mnema show --id <ID> --format json | jq '[.events[].entry | capture("win_count=(?<v>[0-9]+)") | .v | tonumber]'

Filter numeric fields:
  mnema list --format json | jq '.strands[] | select(.entry_count > 10) | .id'

Build a compact current-state view (last_offset feeds the next --seen-offset):
  mnema orient --format json | jq -r '.active[] | "\(.id[0:12]) n=\(.last_offset) :: \(.last_entry)"'

Project a compact timeline:
  mnema timeline --format json | jq '.timeline[] | {ts, strand_id, kind}'"#,
    },
    TopicInfo {
        name: "writing",
        title: "Writing — timing, entry shapes, and a disposable drill",
        body: r#"These are synthetic examples, not facts about the host project.

Write when:
  a plan forms: record the decision, evidence, and verification anchor;
  evidence changes judgment: record the observation and displaced assumption;
  work closes or an irreversible action approaches: checkpoint first.

Entry templates:
  [decision] <claim>; anchor=<file>:<line>; verify=<command>
  [observed] <fact>; source=<command>; changes=<assumption>
  [friction] <blocked thing>; at=<file>:<line>; tried=<command>
  [fixed] fixes=<entry-hash> <what changed>; verified=<command>
  [deliverable] <files changed>; build=<command>; test=<command>

Disposable journal drill (replace placeholders with prior output):
  tmp=<tmp>
  mnema -C <tmp> init
  printf '%s\n' '[task] synthetic writing drill; not host facts' | mnema -C <tmp> add --format json
  printf '%s\n' '[decision] choose <option>; anchor=<file>:<line>; verify=<command>' | mnema -C <tmp> append --id <ID>
  printf '%s\n' '[friction] <blocked thing>; at=<file>:<line>; tried=<command>' | mnema -C <tmp> append --id <ID> --format json
  printf '%s\n' '[fixed] fixes=<entry-hash> <what changed>; verified=<command>' | mnema -C <tmp> append --id <ID>
  mnema -C <tmp> checkpoint --id <ID> --action "before irreversible <action>; reason=<reason>"
  printf '%s\n' '[deliverable] changed=<file>; build=<command>; test=<command>' | mnema -C <tmp> append --id <ID>
  mnema -C <tmp> close --id <ID> --as done
  mnema -C <tmp> show --id <ID>
  mnema -C <tmp> timeline --links <ID>"#,
    },
    TopicInfo {
        name: "collaboration",
        title: "Collaboration forest — representing parallel work",
        body: r#"Mnema records journal-side collaboration structure; launching workers belongs
to the surrounding harness.

Structure:
  Use one strand per work lane. Create derived work with
  mnema add --parent <PARENT>, producing CHILD belongs-to PARENT.
  Identify the worker/lane in its first entry; keep vendor launch details out
  of Core semantics. TASK depends-on UPSTREAM records review context, not a gate.

Discipline:
  Deliverables land on the worker's strand; outer stdout may point to it.
  Workers close with mnema close --id <ID> --as done|failed; [done] is only an annotation.
  Coordinators inspect child entries and close state instead of trusting stdout.
  The parent receives the final synthesis entry.

Delegate parallel reviews, scans, and independent cross-checks. Keep inherently
serial implementation on the current strand.

Reads:
  mnema tree --id <PARENT>
  mnema depends --id <TASK>
  mnema depends --under <PARENT>
  mnema doctor edges --under <PARENT>"#,
    },
    TopicInfo {
        name: "delegation",
        title: "Delegation — strands as recursive asynchronous handoff",
        body: r#"The same strand semantics apply at every depth; there is no special top-level
or second-order worker type.

1. Create one child per lane:
   echo "<task-specific instruction>" | mnema add --parent <PARENT>
2. Keep the body task-specific. Attach 0..N refs for context; refs do not expand by default.
3. The worker enters from its strand, appends progress/evidence/conclusions, then closes it.
4. Delegation is asynchronous: continue useful work without polling; inspect close,
   entries, diff, and tests at a natural synthesis or acceptance point.
5. Further delegation still uses add --parent plus refs; the task/harness authorizes fan-out.
6. Concurrent writers use explicit full strand ids, not --last.
7. Process exit and stdout claims are not task verdicts; journal evidence is the handoff.
8. The harness owns processes, models, worktrees, timeouts, and retries.

Entry:       mnema orient --id <CHILD>
Incremental: mnema timeline --since-offset <N> --under <CHILD> --scope-at-event
Subtree:     mnema tree --id <PARENT>; mnema depends --under <PARENT>"#,
    },
    TopicInfo {
        name: "grammar",
        title: "Grammar — cross-command arguments and naming",
        body: r#"Single-strand targets accept positional <ID> or --id <ID>; --last names the
most recent active strand. show/find/hide/unhide/tree/depends/append/checkpoint
default to --last; close/reopen require a target. timeline --id aliases --strand.
add/append content always comes from stdin.

Canonical flags:
  --include-hidden   include hidden strands; mnema list --all: compatibility alias
  --format json      canonical machine output; explain --json is a shortcut
  --provenance       structured writer metadata
  --seen-offset <N>  caller's last observed target offset
  --tail <N>         display bound only; never changes journal facts
  --under <ID>       SubtreeScope for collection queries and doctor edges
  mnema orient --id <ID>: delegated entry with that subtree candidate set
  --edge-type        link relation; --type is deprecated
  --ref <REF>        repeatable evidence/source refs in authored order
  --scope-at-event   event-time membership for scoped timeline queries

REF accepts a strand prefix (latest entry) or entry-hash prefix (exact entry).
Read it with mnema show --entry <HASH>; optional --deref/--before/--after expand it.
JSON: plural nouns are arrays; counts use count/*_count; own identity is id;
foreign identities use <noun>_id. id and strand_id are full 64-hex values.
Cross-journal writing convention (not parsed): <journal-id>:<strand>:<entry>.
mnema doctor journal: journal id is 64 hex; strand/entry components are >=8 hex.
Writes accept --provenance and, where offered, --format json; see mnema explain card.
Global -C <DIR>/--chdir changes journal discovery and the relative-path base.
Exit codes: 0 success; 1 command failure; 2 corrupt journal; 3 invalid arguments.
Exceptions: doctor nests subcommands; pick is interactive; add/append have no
content positional/--stdin/--file; export uses --out; cutover uses --apply."#,
    },
];

/// Exact lowercase match (topic names are always all-lowercase).
pub fn topic_lookup(name: &str) -> Option<&'static TopicInfo> {
    TOPICS.iter().find(|t| t.name == name)
}

pub fn topics() -> &'static [TopicInfo] {
    TOPICS
}

// ── Data model ──────────────────────────────────────────────

/// Fixed recovery kinds. Each diagnostic must use one of these.
/// Non-Manual variants are reserved for future executable recoveries; output
/// serialises the full vocabulary even though the catalog currently uses Manual.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum RecoveryKind {
    /// Verify a task's completion.
    Verify,
    /// Modify existing code or documentation.
    Edit,
    /// Structural reorganisation or rename.
    MoveOrRename,
    /// Create a [covers] strand for a protocol file.
    CreateCoverStrand,
    /// Append a marker entry to an existing strand.
    AppendMarker,
    /// Dispatch a registered task.
    Dispatch,
    /// Cancel a stale task.
    Cancel,
    /// No mechanical recovery exists — human must decide.
    Manual,
}

/// Machine-readable recovery action (catalog — &'static str).
#[derive(Debug, Clone)]
pub struct RecoveryInfo {
    pub kind: RecoveryKind,
    pub command_str: &'static str,
    pub executable: bool,
    pub requires_human: bool,
}

/// One diagnostic code in the catalog.
#[derive(Debug, Clone)]
pub struct DiagnosticInfo {
    pub code: &'static str,
    pub severity: Severity,
    pub category: &'static str,
    pub title: &'static str,
    pub finding: &'static str,
    pub impact: &'static str,
    pub recovery: RecoveryInfo,
    pub producer: &'static str,
}

#[derive(Debug, Clone)]
pub enum Severity {
    #[allow(dead_code)] // reserved for future E-severity codes
    Error,
    Warning,
}

// ── Catalog ─────────────────────────────────────────────────

static CATALOG: &[DiagnosticInfo] = &[
    DiagnosticInfo {
        code: "migration-source-invalid",
        severity: Severity::Error,
        category: "integrity",
        title: "v2 source cannot be represented by v3 invariants",
        finding: "A source record has an invalid chain, identity, timestamp, relation, or payload for strict v3 conversion.",
        impact: "No activation occurs; the source remains authoritative.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "inspect the reported source offset, repair or explicitly adjudicate it, then rerun: mnema cutover-v3",
            executable: false,
            requires_human: true,
        },
        producer: "cutover-v3",
    },
    DiagnosticInfo {
        code: "migration-source-changed",
        severity: Severity::Error,
        category: "concurrency",
        title: "v2 source changed after planning",
        finding: "The exclusive-lock recheck found source bytes different from the prepared plan.",
        impact: "No activation occurs, preventing an out-of-date plan from committing.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "rerun planning against the current source: mnema cutover-v3",
            executable: false,
            requires_human: false,
        },
        producer: "cutover-v3",
    },
    DiagnosticInfo {
        code: "migration-map-incomplete",
        severity: Severity::Error,
        category: "integrity",
        title: "migration disposition or projection is incomplete",
        finding: "Not every source record/identity has one disposition, or the mapped v3 projection differs from v2.",
        impact: "No activation occurs because the migration proof is incomplete.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "inspect the reported mismatch and repair the converter before retrying",
            executable: false,
            requires_human: true,
        },
        producer: "cutover-v3",
    },
    DiagnosticInfo {
        code: "migration-id-collision",
        severity: Severity::Error,
        category: "integrity",
        title: "distinct source identities collided",
        finding: "Two v2 strand identities mapped to the same v3 genesis identity.",
        impact: "No activation occurs; silently merging histories is forbidden.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "audit canonical identity and deterministic import seed construction",
            executable: false,
            requires_human: true,
        },
        producer: "cutover-v3",
    },
    DiagnosticInfo {
        code: "migration-artifact-conflict",
        severity: Severity::Error,
        category: "integrity",
        title: "prepared migration artifact conflicts",
        finding: "A target, history, map, certificate, or manifest path already contains different bytes for this migration.",
        impact: "No conflicting artifact is overwritten and activation does not advance.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "inspect .mnema/history, .mnema/journals, and active-journal.json before retrying",
            executable: false,
            requires_human: true,
        },
        producer: "cutover-v3",
    },
    DiagnosticInfo {
        code: "legacy-history-write-forbidden",
        severity: Severity::Error,
        category: "resolution",
        title: "write resolved to frozen legacy history",
        finding: "A legacy Event append was attempted after the active manifest selected v3.",
        impact: "The v2 shadow/history is unchanged; only the active v3 journal is writable.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "resolve the v3 strand/entry identity through the migration map and retry the normal command",
            executable: false,
            requires_human: false,
        },
        producer: "journal-v3",
    },
    DiagnosticInfo {
        code: "legacy-shadow-diverged",
        severity: Severity::Warning,
        category: "compatibility",
        title: "ignored legacy shadow diverged after v3 activation",
        finding: "The legacy journal.jsonl differs from frozen v2 history, or unexpectedly exists under a fresh v3 origin; an old binary may have written facts outside the active journal.",
        impact: "Active v3 integrity is unchanged, but intended facts in the shadow are invisible to normal reads and writes.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "upgrade the PATH binary, inspect the legacy shadow delta, and explicitly append any intended facts to v3",
            executable: false,
            requires_human: true,
        },
        producer: "doctor-journal-v3",
    },
    DiagnosticInfo {
        code: "atomic-activation-failed",
        severity: Severity::Error,
        category: "concurrency",
        title: "active manifest commit failed",
        finding: "Prepared and verified v3 artifacts exist, but the no-replace manifest commit did not complete.",
        impact: "v2 remains authoritative; prepared artifacts are safe resume points.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "inspect the reported filesystem error and rerun: mnema cutover-v3 --apply",
            executable: false,
            requires_human: false,
        },
        producer: "activation-v3",
    },
    DiagnosticInfo {
        code: "activation-durability-uncertain",
        severity: Severity::Warning,
        category: "io",
        title: "activation committed but directory sync is uncertain",
        finding: "The active manifest is installed and readable, but the post-commit directory durability sync failed.",
        impact: "v3 is already authoritative; reporting this as an uncommitted failure would invite a dangerous retry assumption.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "run mnema doctor journal, sync or back up the filesystem, and treat v3 as active",
            executable: false,
            requires_human: false,
        },
        producer: "activation-v3",
    },
    // ── Lifecycle: E053/E056 reserved, not removed ──────
    // Completion-pair checks (done↔verified) are parked until the marker
    // vocabulary stabilises — paired markers are coming, and these two
    // numbers stay reserved for that semantics. Their old recovery
    // commands referenced shuttle and must be rewritten on revival.
    //
    // E053  done without verified   (pair check, fire only if the strand
    //                                ever used [verified])
    // E056  verified without done   (inverse pair check)
    //
    // E055/E057/E058 (dispatch artifact / dispatched stale / registered
    // stale) were removed 2026-06 with the external workflow codes — the
    // dispatch concept belongs to that workflow, not to the journal.

    // ── Lifecycle (W codes) ─────────────────────────────
    DiagnosticInfo {
        code: "W068",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "deadline overdue",
        finding: "A task has a [deadline] entry whose by= time has passed, and the strand carries no close effect or legacy closing marker.",
        impact: "The task is overdue; downstream schedule assumptions are invalid.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "re-read the deadline and current state: mnema show --id <STRAND_ID>",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },
    DiagnosticInfo {
        code: "W071",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "checkpoint on closed strand",
        finding: "The checkpoint target strand is not in the registered state — it has already been closed with a marker such as [done], [cancelled], or [failed].",
        impact: "The checkpoint is almost certainly targeting the wrong strand — irreversible actions should be anchored to an open strand.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "confirm the target with mnema list; the checkpoint may belong on a successor strand",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },
    DiagnosticInfo {
        code: "W059",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "append on closed strand",
        finding: "An explicit append --id targeted a strand whose lifecycle state is closed:<disposition>.",
        impact: "The append still writes to that closed strand. If this is a new result, start a successor with `mnema add --from <ID>` and refer back to the closed line. If the strand was closed by mistake, reopen it with `mnema reopen --id <ID>` before continuing.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "new result: mnema add --from <ID>; wrong close: mnema reopen --id <ID>",
            executable: false,
            requires_human: true,
        },
        producer: "append",
    },
    DiagnosticInfo {
        code: "W073",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "unknown marker — possible typo",
        finding: "The appended content starts with a bracket word (e.g. [freiction]) that is not in the known marker vocabulary but is within edit distance 2 of a known marker.",
        impact: "The entry was written as plain content, not a structured marker — it will be invisible to projections that filter by marker type.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "check vocabulary: mnema explain markers",
            executable: false,
            requires_human: true,
        },
        producer: "append",
    },
    DiagnosticInfo {
        code: "W074",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "closing marker appended — strand lifecycle unchanged",
        finding: "The appended entry starts with a closing annotation marker ([done], [failed], [cancelled], [merged], or [verified]). Since lifecycle-from-marker semantics were removed, these markers are annotations only — the strand's lifecycle state was NOT changed by this append.",
        impact: "If the intent was to close the strand, it remains open. Downstream tools that filter on lifecycle state (list --state done, orient closed_count) will not see this strand as closed.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "re-check whether it should be closed: mnema show --id <STRAND_ID>",
            executable: false,
            requires_human: false,
        },
        producer: "append",
    },
    DiagnosticInfo {
        code: "W075",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "dangling fix reference — fixes= prefix unmatched",
        finding: "A [fixed] entry carries a fixes=<prefix> token (prefix >= 8 hex chars) that does not match any [friction] entry's entry id (or a pre-retirement append_id) in the same strand. The prefix either points to a nonexistent entry or to an entry that is not a [friction].",
        impact: "The [fixed] entry is not folded and its intended friction target remains exposed as an unresolved live debt. The pairing was silently skipped.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "re-check the fixes= prefix: mnema show --id <STRAND_ID>",
            executable: false,
            requires_human: true,
        },
        producer: "context",
    },
    DiagnosticInfo {
        code: "W076",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "seen offset behind strand",
        finding: "A write command was passed --seen-offset <N>, and N is behind the target strand's current last_offset before the write.",
        impact: "The caller is writing after the strand changed behind its last observed position; its local view may be stale. W076 is a transient write-time signal (rides the append/checkpoint echo on stderr + JSON warnings[]/seen_gap, exit 0). By design it is NOT persisted as a scar and will NOT reappear in a later `show` — scars are lifecycle state (close/reopen), not diagnostics, and recording a read cursor would violate ADR-0003. Capture the evidence on the spot from the write echo; do not audit it via show.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "mnema timeline --since-offset <N> --links <STRAND_ID>",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },
];

// ── Lookup ──────────────────────────────────────────────────

pub fn lookup(code: &str) -> Option<&'static DiagnosticInfo> {
    CATALOG.iter().find(|d| d.code.eq_ignore_ascii_case(code))
}

/// Full catalog access for closure checks (examples-as-contract CI and
/// the two-way closure tests: every emitted code resolves, every entry
/// has a live producer).
#[cfg(test)]
pub fn catalog() -> &'static [DiagnosticInfo] {
    CATALOG
}

mod runtime;
pub(crate) use runtime::*;

#[cfg(test)]
pub fn all_codes() -> Vec<&'static str> {
    CATALOG.iter().map(|d| d.code).collect()
}

mod audit;
pub use audit::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::explain::cmd_explain;
    #[test]
    fn audit_journal_reports_edge_validity_from_graph_module() {
        use crate::event::Event;
        let ts = "2026-01-01T00:00:00Z".to_string();
        let events = vec![
            Event::StrandCreated {
                id: "task".to_string(),
                ts: ts.clone(),
                strand_type: None,
                slug: None,
            },
            Event::LogAppended {
                id: "task".to_string(),
                ts: ts.clone(),
                content: "task summary".to_string(),
                effect: None,
                prev_entry_id: None,
                entry_id: None,
                refs: Vec::new(),
                ref_: None,
                append_id: None,
                git: None,
                provenance: None,
            },
            Event::StrandCreated {
                id: "parent_a".to_string(),
                ts: ts.clone(),
                strand_type: None,
                slug: None,
            },
            Event::LogAppended {
                id: "parent_a".to_string(),
                ts: ts.clone(),
                content: "parent a".to_string(),
                effect: None,
                prev_entry_id: None,
                entry_id: None,
                refs: Vec::new(),
                ref_: None,
                append_id: None,
                git: None,
                provenance: None,
            },
            Event::StrandCreated {
                id: "parent_b".to_string(),
                ts: ts.clone(),
                strand_type: None,
                slug: None,
            },
            Event::LogAppended {
                id: "parent_b".to_string(),
                ts: ts.clone(),
                content: "parent b".to_string(),
                effect: None,
                prev_entry_id: None,
                entry_id: None,
                refs: Vec::new(),
                ref_: None,
                append_id: None,
                git: None,
                provenance: None,
            },
            Event::EdgeLinked {
                id: "task".to_string(),
                ts: ts.clone(),
                to: "parent_a".to_string(),
                edge_type: Some("belongs-to".to_string()),
                provenance: None,
            },
            Event::EdgeLinked {
                id: "task".to_string(),
                ts,
                to: "parent_b".to_string(),
                edge_type: Some("belongs-to".to_string()),
                provenance: None,
            },
        ];

        let audit = audit_journal(&events, chrono::Utc::now());
        let edge_section = audit
            .lint_sections
            .iter()
            .find(|section| section.name == "edge-validity")
            .expect("edge-validity section");

        assert_eq!(edge_section.count(), 1);
        assert!(edge_section.findings[0].contains("belongs-to"));
        assert!(edge_section.findings[0].contains("task"));
    }

    #[test]
    fn audit_reports_ref_target_advanced_position_fact() {
        use crate::event::{Event, make_log_appended_entry, make_strand_created};
        let (basis_created, basis_first) = make_strand_created("basis line", None);
        let basis_id = match &basis_created {
            Event::StrandCreated { id, .. } => id.clone(),
            _ => unreachable!(),
        };
        let basis_first_hash = match &basis_first {
            Event::LogAppended { entry_id, .. } => entry_id.clone().unwrap(),
            _ => unreachable!(),
        };
        let (consumer_created, consumer_first) = make_strand_created("consumer line", None);
        let consumer_id = match &consumer_created {
            Event::StrandCreated { id, .. } => id.clone(),
            _ => unreachable!(),
        };
        let consumer_first_hash = match &consumer_first {
            Event::LogAppended { entry_id, .. } => entry_id.clone().unwrap(),
            _ => unreachable!(),
        };
        let citing = make_log_appended_entry(
            &consumer_id,
            Some(&consumer_first_hash),
            "[decision] built on the basis entry",
            vec![basis_first_hash.clone()],
            None,
            None,
        );
        let basis_update = make_log_appended_entry(
            &basis_id,
            Some(&basis_first_hash),
            "basis moved on",
            Vec::new(),
            None,
            None,
        );

        // Cited line has nothing after the citation: no fact to report.
        let quiet = vec![
            basis_created.clone(),
            basis_first.clone(),
            consumer_created.clone(),
            consumer_first.clone(),
            citing.clone(),
        ];
        let audit = audit_journal(&quiet, chrono::Utc::now());
        let section = audit
            .lint_sections
            .iter()
            .find(|s| s.name == "ref-target-advanced")
            .expect("ref-target-advanced section");
        assert_eq!(section.count(), 0);

        // Cited line gains an entry after the citation: position fact reported.
        let advanced = vec![
            basis_created,
            basis_first,
            consumer_created,
            consumer_first,
            citing,
            basis_update,
        ];
        let audit = audit_journal(&advanced, chrono::Utc::now());
        let section = audit
            .lint_sections
            .iter()
            .find(|s| s.name == "ref-target-advanced")
            .unwrap();
        assert_eq!(section.count(), 1);
        assert!(section.findings[0].contains("ref-target-advanced"));
        assert!(
            section.findings[0].contains("may warrant review"),
            "fact is reported, judgment stays with the reader"
        );
    }

    #[test]
    fn test_lookup_known_code() {
        let info = lookup("W068").expect("W068 should be known");
        assert_eq!(info.code, "W068");
        assert_eq!(info.title, "deadline overdue");
        assert!(matches!(info.severity, Severity::Warning));
    }

    #[test]
    fn test_lookup_case_insensitive() {
        let info = lookup("w068").expect("w068 should be known");
        assert_eq!(info.code, "W068");
    }

    #[test]
    fn test_lookup_unknown_code() {
        assert!(lookup("E999").is_none());
    }

    #[test]
    fn test_explain_json_known() {
        let output = cmd_explain("W068", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(v["ok"], true);
        assert_eq!(v["code"], "W068");
        assert!(v["recovery"]["kind"].as_str().is_some());
        assert!(v["recovery"]["command"].as_str().is_some());
    }

    #[test]
    fn test_explain_json_unknown() {
        let output = cmd_explain("E999", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(v["ok"], false);
        // new error key is "error" with updated message
        assert!(
            v["error"]
                .as_str()
                .unwrap_or("")
                .contains("unknown code or topic")
        );
    }

    #[test]
    fn test_explain_text_known() {
        let output = cmd_explain("W068", false);
        assert!(output.contains("W068"));
        assert!(output.contains("deadline"));
    }

    #[test]
    fn test_explain_text_unknown() {
        let output = cmd_explain("XYZ", false);
        assert!(output.contains("unknown code or topic"));
    }

    // ── Topic catalog tests ─────────────────────────────────

    #[test]
    fn explain_topics_resolve() {
        // All topics resolve in both text and JSON modes.
        for name in [
            "card",
            "markers",
            "retry",
            "json",
            "jq",
            "grammar",
            "writing",
            "collaboration",
        ] {
            let text = cmd_explain(name, false);
            assert!(
                !text.contains("unknown code or topic"),
                "topic {} failed text: {}",
                name,
                text
            );

            let json_out = cmd_explain(name, true);
            let v: serde_json::Value = serde_json::from_str(&json_out)
                .unwrap_or_else(|_| panic!("topic {} json not valid JSON: {}", name, json_out));
            assert_eq!(v["ok"], true, "topic {} json ok must be true", name);
            assert_eq!(v["topic"], name, "topic {} json name mismatch", name);
            assert!(
                v["title"].as_str().is_some(),
                "topic {} missing title",
                name
            );
            assert!(v["body"].as_str().is_some(), "topic {} missing body", name);
        }

        // Unknown input shows error AND lists "card" (no dead ends)
        let err_text = cmd_explain("nonexistent_topic", false);
        assert!(
            err_text.contains("unknown code or topic"),
            "expected error in: {}",
            err_text
        );
        assert!(
            err_text.contains("card"),
            "error must list available topics, missing 'card': {}",
            err_text
        );

        let err_json = cmd_explain("nonexistent_topic", true);
        let v: serde_json::Value =
            serde_json::from_str(&err_json).expect("error JSON must be valid");
        assert_eq!(v["ok"], false);
        // available_topics array must contain "card"
        let topics_arr = v["available_topics"]
            .as_array()
            .expect("available_topics must be array");
        assert!(
            topics_arr.iter().any(|x| x == "card"),
            "available_topics must include card"
        );
    }

    #[test]
    fn delegation_topic_teaches_async_core_boundary_without_vendor_commands() {
        let topic = topic_lookup("delegation").expect("delegation topic");
        assert!(topic.body.contains("without polling"));
        assert!(topic.body.contains("0..N refs"));
        assert!(topic.body.contains("harness"));
        for vendor in ["grok --", "codex exec", "claude -p"] {
            assert!(
                !topic.body.contains(vendor),
                "vendor command leaked: {vendor}"
            );
        }
    }

    #[test]
    fn explain_code_lookup_unchanged() {
        // W068/w068 still route to diagnostic catalog (not topic lookup).
        let upper = cmd_explain("W068", true);
        let v: serde_json::Value = serde_json::from_str(&upper).expect("valid JSON");
        assert_eq!(v["ok"], true);
        assert_eq!(v["code"], "W068");

        let lower = cmd_explain("w068", true);
        let v2: serde_json::Value = serde_json::from_str(&lower).expect("valid JSON");
        assert_eq!(v2["ok"], true);
        assert_eq!(v2["code"], "W068");
    }

    #[test]
    fn topic_body_line_count_at_most_30() {
        for topic in topics() {
            let lines = topic.body.lines().count();
            assert!(
                lines <= 30,
                "topic '{}' body has {} lines (max 30)",
                topic.name,
                lines
            );
        }
    }

    #[test]
    fn card_topic_fields_match_serialization() {
        // Build a minimal OrientStrand and check its serde keys all appear in
        // the card topic body.
        use crate::output::OrientStrand;
        let sample = OrientStrand {
            id: "abc123".to_string(),
            slug: None,
            strand_type: None,
            entry_count: 1,
            summary: "test".to_string(),
            last_entry: "test".to_string(),
            last_offset: 0,
            catch_up: "mnema timeline --since-offset 0 --links abc123".to_string(),
            lifecycle: "registered".to_string(),
        };
        let v = serde_json::to_value(&sample).expect("serialize OrientStrand");
        let keys: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        let topic = topic_lookup("card").expect("card topic must exist");
        for key in &keys {
            assert!(
                topic.body.contains(key.as_str()),
                "card topic body missing OrientStrand field: {}",
                key
            );
        }
    }

    #[test]
    fn json_topic_fields_match_serialization() {
        use crate::output::{
            DependsOutput, DependsScopeOutput, EdgesOutput, OrientOutput, SearchOutput,
            StrandDetailOutput, StrandListItem, TimelineOutput,
        };
        let topic = topic_lookup("json").expect("json topic must exist");

        // show → StrandDetailOutput
        let show_sample = StrandDetailOutput {
            id: "a".to_string(),
            slug: None,
            hidden: false,
            summary: "s".to_string(),
            entry_count: 0,
            status: "registered".to_string(),
            state_marker: None,
            state_offset: 0,
            last_entry_offset: 0,
            edges: vec![],
            belongs_to_edges: vec![],
            depends_on_edges: vec![],
            strand_branch: None,
            events: vec![],
        };
        let v = serde_json::to_value(&show_sample).expect("serialize StrandDetailOutput");
        for key in v.as_object().unwrap().keys() {
            assert!(
                topic.body.contains(key.as_str()),
                "json topic missing show field: {}",
                key
            );
        }

        // list → StrandListItem
        let list_sample = StrandListItem {
            id: "a".to_string(),
            slug: None,
            entry_count: 0,
            first_summary: "s".to_string(),
            last_summary: "s".to_string(),
            hidden: false,
            strand_type: None,
            edges: vec![],
            belongs_to_edges: vec![],
            depends_on_edges: vec![],
            status: "registered".to_string(),
            state_marker: None,
            state_offset: 0,
            last_entry_ts: "".to_string(),
            last_entry_offset: 0,
        };
        let v = serde_json::to_value(&list_sample).expect("serialize StrandListItem");
        for key in v.as_object().unwrap().keys() {
            assert!(
                topic.body.contains(key.as_str()),
                "json topic missing list field: {}",
                key
            );
        }

        // orient → OrientOutput (check top-level fields)
        let orient_sample = OrientOutput {
            max_offset: 0,
            active: vec![],
            closed_count: 0,
            hidden_count: 0,
            integrity: "".to_string(),
            notices: vec![],
            since_command: "mnema timeline --since-offset 0".to_string(),
            delegation_command: "mnema explain delegation".to_string(),
            remind: "".to_string(),
            pause: "".to_string(),
            stale_count: 0,
            stale_command: "mnema list --stale 2h".to_string(),
            scope: crate::output::OrientScopeOutput::journal(),
        };
        let v = serde_json::to_value(&orient_sample).expect("serialize OrientOutput");
        for key in v.as_object().unwrap().keys() {
            assert!(
                topic.body.contains(key.as_str()),
                "json topic missing orient field: {}",
                key
            );
        }

        // search → SearchOutput
        let search_sample = SearchOutput {
            matches: vec![],
            count: 0,
            query: "q".to_string(),
            marker: None,
        };
        let v = serde_json::to_value(&search_sample).expect("serialize SearchOutput");
        for key in v.as_object().unwrap().keys() {
            assert!(
                topic.body.contains(key.as_str()),
                "json topic missing search field: {}",
                key
            );
        }

        // timeline → TimelineOutput
        let timeline_sample = TimelineOutput {
            timeline: vec![],
            truncated: false,
            count: 0,
            max_offset: 0,
            scope: crate::output::TimelineScopeOutput {
                kind: "journal".to_string(),
                root: None,
                membership: "not-applicable".to_string(),
            },
            window: crate::output::TimelineWindowOutput {
                since_offset: None,
                since_ts: None,
                until_offset: None,
                until_ts: None,
                observed_through: 0,
                next_since_offset: 0,
            },
        };
        let v = serde_json::to_value(&timeline_sample).expect("serialize TimelineOutput");
        for key in v.as_object().unwrap().keys() {
            assert!(
                topic.body.contains(key.as_str()),
                "json topic missing timeline field: {}",
                key
            );
        }

        // depends → DependsOutput
        let depends_sample = DependsOutput {
            id: "task".to_string(),
            summary: "task".to_string(),
            upstream_count: 1,
            registered_upstream_count: 1,
            upstreams: vec![],
        };
        let v = serde_json::to_value(&depends_sample).expect("serialize DependsOutput");
        for key in v.as_object().unwrap().keys() {
            assert!(
                topic.body.contains(key.as_str()),
                "json topic missing depends field: {}",
                key
            );
        }

        // depends --under → DependsScopeOutput
        let depends_scope_sample = DependsScopeOutput {
            root_id: "root".to_string(),
            count: 0,
            strands: vec![],
        };
        let v = serde_json::to_value(&depends_scope_sample).expect("serialize DependsScopeOutput");
        for key in v.as_object().unwrap().keys() {
            assert!(
                topic.body.contains(key.as_str()),
                "json topic missing depends --under field: {}",
                key
            );
        }

        // doctor edges → EdgesOutput
        let edges_sample = EdgesOutput {
            open_frictions: vec![],
            decisions_without_why: vec![],
            open_friction_count: 0,
            open_friction_active_count: 0,
            decision_without_why_count: 0,
        };
        let v = serde_json::to_value(&edges_sample).expect("serialize EdgesOutput");
        for key in v.as_object().unwrap().keys() {
            assert!(
                topic.body.contains(key.as_str()),
                "json topic missing edges field: {}",
                key
            );
        }
    }

    #[test]
    fn test_all_codes_present() {
        let codes = all_codes();
        assert!(codes.contains(&"W059"));
        assert!(codes.contains(&"W068"));
        assert!(codes.contains(&"W071"));
        assert!(codes.contains(&"W073"));
        assert!(codes.contains(&"W074"));
        assert!(codes.contains(&"W075"));
        assert!(codes.contains(&"W076"));
        assert!(codes.contains(&"migration-source-invalid"));
        assert!(codes.contains(&"migration-source-changed"));
        assert!(codes.contains(&"migration-map-incomplete"));
        assert!(codes.contains(&"migration-id-collision"));
        assert!(codes.contains(&"migration-artifact-conflict"));
        assert!(codes.contains(&"legacy-history-write-forbidden"));
        assert!(codes.contains(&"legacy-shadow-diverged"));
        assert!(codes.contains(&"atomic-activation-failed"));
        assert!(codes.contains(&"activation-durability-uncertain"));
        assert_eq!(
            codes.len(),
            16,
            "catalog size changed — update this test deliberately"
        );
    }

    #[test]
    fn test_removed_workflow_codes_stay_removed() {
        // 18 codes were removed 2026-06 — they live in git history. Their
        // numbers must never be reused for new meanings:
        //   16 external-workflow codes (gate/shuttle/covers/DAG/story),
        //   E055/E057/E058 (dispatch concept left with that workflow),
        //   W066 (v0 migration finished — journal scan found no residue).
        // E053/E056 are NOT in this list: reserved (commented out in the
        // catalog) for completion-pair semantics once markers stabilise.
        // W062/W069/W070 were removed 2026-07 as semantic-subtraction codes
        // (health/concurrency/producer guards — judgment left to agents).
        // W071 was previously in this list as a removed external-workflow code;
        // it has been revived for checkpoint closed-strand guard — see git history.
        for code in [
            "E047", "W058", "W062", "W065", "W067", "W069", "W070", "W072", "E081", "W081", "E082",
            "W082", "E083", "W083", "E084", "W085", "E055", "E057", "E058", "W066",
        ] {
            assert!(lookup(code).is_none(), "removed code {} reappeared", code);
        }
    }

    #[test]
    fn test_reserved_codes_not_yet_revived() {
        // E053/E056 are parked until paired completion markers stabilise.
        // When they come back, delete this test and re-add them to
        // test_all_codes_present.
        assert!(lookup("E053").is_none());
        assert!(lookup("E056").is_none());
    }

    #[test]
    fn test_explain_json_recovery_fields() {
        let output = cmd_explain("W071", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        let recovery = &v["recovery"];
        assert_eq!(recovery["executable"], false);
        assert_eq!(recovery["requires_human"], true);
        assert!(recovery["command"].as_str().unwrap().contains("mnema list"));
    }

    #[test]
    fn test_w073_can_explain() {
        let info = lookup("W073").expect("W073 should be in catalog");
        assert_eq!(info.code, "W073");
        assert_eq!(info.title, "unknown marker — possible typo");
        assert!(matches!(info.severity, Severity::Warning));
        assert_eq!(info.category, "lifecycle");
        assert_eq!(info.producer, "append");
        let output = cmd_explain("W073", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(v["ok"], true);
        assert_eq!(v["code"], "W073");
        assert_eq!(v["recovery"]["executable"], false);
        assert_eq!(v["recovery"]["requires_human"], true);
    }

    #[test]
    fn test_w075_can_explain() {
        let info = lookup("W075").expect("W075 should be in catalog");
        assert_eq!(info.code, "W075");
        assert_eq!(
            info.title,
            "dangling fix reference — fixes= prefix unmatched"
        );
        assert!(matches!(info.severity, Severity::Warning));
        assert_eq!(info.category, "lifecycle");
        assert_eq!(info.producer, "context");
        let output = cmd_explain("W075", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(v["ok"], true);
        assert_eq!(v["code"], "W075");
        assert_eq!(v["recovery"]["executable"], false);
        assert_eq!(v["recovery"]["requires_human"], true);
    }

    #[test]
    fn test_no_duplicate_codes() {
        use std::collections::HashSet;
        let codes: Vec<&str> = CATALOG.iter().map(|d| d.code).collect();
        let unique: HashSet<&str> = codes.iter().copied().collect();
        assert_eq!(
            codes.len(),
            unique.len(),
            "duplicate diagnostic codes found"
        );
    }

    #[test]
    fn test_w059_can_explain() {
        let info = lookup("W059").expect("W059 should be in catalog");
        assert_eq!(info.code, "W059");
        assert_eq!(info.title, "append on closed strand");
        assert!(matches!(info.severity, Severity::Warning));
        assert_eq!(info.category, "lifecycle");
        assert_eq!(info.producer, "append");
        let output = cmd_explain("W059", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(v["ok"], true);
        assert_eq!(v["code"], "W059");
        assert_eq!(v["recovery"]["executable"], false);
        assert_eq!(v["recovery"]["requires_human"], true);
    }

    #[test]
    fn test_w071_can_explain() {
        let info = lookup("W071").expect("W071 should be in catalog");
        assert_eq!(info.code, "W071");
        assert_eq!(info.title, "checkpoint on closed strand");
        assert!(matches!(info.severity, Severity::Warning));
        assert_eq!(info.category, "lifecycle");
        let output = cmd_explain("W071", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(v["ok"], true);
        assert_eq!(v["code"], "W071");
        assert_eq!(v["recovery"]["executable"], false);
        assert_eq!(v["recovery"]["requires_human"], true);
    }

    #[test]
    fn test_w076_can_explain() {
        let info = lookup("W076").expect("W076 should be in catalog");
        assert_eq!(info.code, "W076");
        assert_eq!(info.title, "seen offset behind strand");
        assert!(matches!(info.severity, Severity::Warning));
        assert_eq!(info.category, "lifecycle");
        let output = cmd_explain("W076", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(v["ok"], true);
        assert_eq!(v["code"], "W076");
        assert_eq!(v["recovery"]["executable"], false);
        assert_eq!(v["recovery"]["requires_human"], true);
    }

    #[test]
    fn w076_seen_offset_gap_and_catch_up_are_precise() {
        let id = "0000019dd34b111111111111";
        let warning = check_w076_seen_offset(id, Some(2), 5).expect("stale seen offset");
        assert_eq!(warning.code, "W076");
        assert_eq!(warning.seen_offset, 2);
        assert_eq!(warning.strand_last_offset, 5);
        assert_eq!(warning.seen_gap, 3);
        assert!(warning.catch_up.contains("--since-offset 2"));
        assert!(warning.catch_up.contains("0000019dd34b"));

        assert!(check_w076_seen_offset(id, Some(5), 5).is_none());
        assert!(check_w076_seen_offset(id, Some(9), 5).is_none());
        assert!(check_w076_seen_offset(id, None, 5).is_none());
    }
}

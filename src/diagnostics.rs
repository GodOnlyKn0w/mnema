//! Unified diagnostic catalog — single source of truth for all diagnostic codes.
//!
//! Every code emitted by any producer (currently: lifecycle, health) MUST
//! have an entry here. The `tasktree explain` command queries this catalog.
//!
//! # Catalog closure contract
//!
//! Adding a new diagnostic code without a corresponding catalog entry is a bug.
//! Closure is two-way:
//!   1. Every emitted code must resolve via `tasktree explain --json <code>`
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

/// One encyclopaedia topic reachable via `tasktree explain <name>`.
/// Namespace rule: topic names are all-lowercase; diagnostic codes begin
/// with an uppercase letter (W/E). The two namespaces are mechanically
/// disjoint — no case-folding is applied to topics.
pub struct TopicInfo {
    pub name: &'static str,
    pub title: &'static str,
    pub body: &'static str,
}

static TOPICS: &[TopicInfo] = &[
    TopicInfo {
        name: "card",
        title: "卡片——统一输出文法单元",
        body: r#"卡片是所有写命令写后回显、orient 菜单、--format json result 字段共享的形态。

文本格式（四行结构）：
  把手行   <id> [type] | <n> entries | <state>
  首条     <summary>（第一条日志，概述这条线的主题）
  last:    <last_entry>（最近一条日志；entries>1 时出现）
  疤痕行   仅当命令产生 W 码时追加（如 W070、W071、W076）
           （W 码=写时瞬态诊断：骑写回显，不入账/不成疤/show 不复显，须当场捕证。ADR-0003）

把手行中的 <state> 显示生命周期（lifecycle），格式：
  open:   registered（未关闭）
  closed: closed:<disposition>（如 closed:done、closed:failed）
  生命周期由 tasktree close / reopen 命令改变，append 的 marker 是注解。

语义：
  回显即预付的验证——写后输出卡片，调用方无需再跑 show/orient 确认。
  所有写命令（append/add/checkpoint/bind/hide/unhide/link/close/reopen）
  都在写后回显受影响线的卡片。

JSON 形态（OrientStrand，写命令 result 字段 / orient active[]）：
  - id:           全宽 strand id（24 hex，跨输出可直接 join）
  - strand_type:  线的类型，可为 null（task/dag/why/session）
  - entry_count:  日志条目计数
  - summary:      第一条日志截断到 70 字符
  - last_entry:   最近一条日志截断到 70 字符
  - last_offset:  该线最近事件的 journal offset
  - catch_up:     就绪的 timeline 追赶命令
  - lifecycle:    生命周期（"registered" 或 "closed:<disposition>"）

JSON shape 索引见 tasktree explain json"#,
    },
    TopicInfo {
        name: "markers",
        title: "Marker 词表——append 条目前缀规范",
        body: r#"Marker 是 append 条目首行的方括号前缀，机器可解析。
Marker 是注解（annotation），不改变线的生命周期。
生命周期由 close / reopen 命令控制，不由 marker 控制。

judgment:    [decision] [constraint] [friction] [fixed] [lesson] [insight]
observation: [observed] [check] [progress] [deliverable] [metric]
planning:    [deadline] <text> by=YYYY-MM-DD  （或 by=<RFC3339>）
structure:   [covers] [guide] [skill] [task] [session]
annotation:  [done] [verified] [cancelled] [failed] [merged] [ended]
             [dispatched] [registered]
system:      [checkpoint] [hidden] [waiting:human] [grill]

Marker 语义（一行一条）：
  [decision]    已做的决定
  [constraint]  必须遵守的约束
  [friction]    阻力 / 未解决的问题
  [fixed]       已修复；可用 fixes=<append_id前缀≥8位> 指定目标 friction
  [lesson]      学到的教训
  [insight]     洞见
  [observed]    观察到的事实
  [progress]    进展 / [deliverable] 交付物
  [metric]      落账的测量值；约定写 name=val（如 [metric] win_count=26）
                可被 jq capture 抽成序列，见 tasktree explain jq
  [deadline]    截止日期（by= 字段必须是日期或 RFC3339）
  [done]        完成注解（仅注解，不关闭线；关闭用 close --id <ID>）
  [checkpoint]  由 tasktree checkpoint 命令写入，勿手动添加

未知方括号前缀一律透传（不拒写）；拼错收 W073；
追加关闭类 marker 后收 W074 提醒改用 close 命令。"#,
    },
    TopicInfo {
        name: "retry",
        title: "重试语义——哪些命令可盲目重试",
        body: r#"命令重试安全性（基于源码核实）：

可盲目重试（幂等）：
  hide     已隐藏时显式 no-op（不写事件，输出"already hidden"）
  unhide   已可见时显式 no-op（不写事件，输出"already visible"）
  init     已存在时跳过文件创建；总是打印初始化消息；目录幂等

不可盲目重试（有副作用）：
  bind     append-only；重复调用写入新的 SubjectBound 事件；
           后绑定对 current 投影生效（覆盖语义在投影层，
           不在写入层）；超时后先 current 查账再决定
  append   重复写入新的 LogAppended 事件；
           超时后先 show/orient 查账再决定
  add      每次创建新 strand；不检查内容重复
  checkpoint  重复写入新的 checkpoint 条目；
              超时后先 timeline 查账再决定
  link     重复写入新的 EdgeLinked 事件；投影去重与否取决于下游

通用原则：超时后先查账（show/orient/timeline），
确认事件是否已写入，再决定是否重试。"#,
    },
    TopicInfo {
        name: "json",
        title: "JSON 形态索引——各读命令 --format json 的顶层字段",
        body: r#"show（StrandDetailOutput）：
  id / hidden / summary / entry_count / status / state_marker / state_offset / last_entry_offset /
  edges / belongs_to_edges / depends_on_edges / strand_branch / events
  ※ events[].entry=日志行；last_entry_offset=下次 --seen-offset；belongs_to_edges=父 / depends_on_edges=阻塞者(F3)

list（StrandListOutput.strands[]，StrandListItem）：
  id / entry_count / first_summary / last_summary / hidden / strand_type /
  edges / belongs_to_edges / depends_on_edges / status / state_marker /
  state_offset / last_entry_ts / last_entry_offset

orient（OrientOutput）：
  max_offset / active / closed_count / hidden_count / remind
  ※ active[] 是卡片数组（OrientStrand）见 tasktree explain card

search（SearchOutput）：
  matches / count / query
  ※ matches[] 每元素：strand_id / content / strand_type / hidden

timeline（TimelineOutput）：
  timeline / truncated / count / max_offset
  ※ timeline[] 每元素：journal_offset / ts / strand_id /
    strand_type / kind / ts_skew

append/checkpoint: seen_offset / seen_gap / warnings / result；add/find: id / status / result
hide / unhide: strand_id / status / noop /
  active_count / closed_count / hidden_count / result（卡片）
link: source_id / target_id / edge_type / status /
  result.source / result.target（卡片）
卡片/result 形态见 tasktree explain card
jq 整型（切 JSON 成你要的形）见 tasktree explain jq"#,
    },
    TopicInfo {
        name: "jq",
        title: "jq 整型——把 JSON 投影切成你要的形",
        body: r#"JSON 是空间(tree)/时间(timeline)两视角投影，jq 是塑形层。
边界：jq 只塑形结构够的内容——埋在散文里的数/状态它抓不动，
故"写得可解析"是前提（marker 前缀、name=val），不是 tasktree 多建命令。
（orient 开场 remind 的 read/extract 段即指向此页。）
顶层字段见 tasktree explain json。常用：

取 strand id（免脆弱解析，取代手搓字符串切割）：
  tasktree add "..." --format json | jq -r .id

取日志行：
  tasktree show --id <ID> --format json | jq -r '.events[].entry'

按 marker 聚条目（marker 是 .entry 前缀，取代 show 文字墙 + grep）：
  tasktree show --id <ID> --format json | jq -r '.events[] | select(.entry | startswith("[friction]")) | .entry'
  坑：用 startswith；勿用 test("^\[...")——shell 里反斜杠转义会炸。

抽数字轨迹（先按约定写 [metric] name=val，再 capture 出序列）：
  tasktree append --id <ID> "[metric] win_count=26"
  tasktree show --id <ID> --format json | jq '[.events[].entry | capture("win_count=(?<v>[0-9]+)") | .v | tonumber]'

数值筛选（offset / count / entry_count 是数，可比较）：
  tasktree list --format json | jq '.strands[] | select(.entry_count > 10) | .id'

中途现状合成（"我在哪"：活线 + 各自 last_offset 即下次 --seen-offset 的 N）：
  tasktree orient --format json | jq -r '.active[] | "\(.id[0:12]) n=\(.last_offset) :: \(.last_entry)"'

时间线切成精简视图：
  tasktree timeline --format json | jq '.timeline[] | {ts, strand_id, kind}'"#,
    },
    TopicInfo {
        name: "grammar",
        title: "文法契约——全 CLI 一致的参数与命名规则",
        body: r#"目标线：主对象用位置参数；位置被 content 占用的命令
（append/checkpoint/bind）用 --id；单 id 命令两种写法等价
（<ID> 与 --id <ID>）；timeline 的 --id 等价 --strand。

旗标词表（同一概念只有一个名字）：
  --include-hidden  含隐藏线（list 的 --all 是兼容别名）
  --format json     机器输出唯一正典（explain --json 是兼容快捷）
  --provenance / --seen-offset <N>  写命令出处 / 上次看到的目标线 offset
  --tail <N>        只限显示、不改账，对任何目标可用
  --edge-type       link 的边类型（--type 是 deprecated 别名）

JSON 命名法：
  复数名词 = 数组（events / matches / strands / active / timeline）
  计数 = count 或 *_count（entry_count / closed_count / hidden_count）
  自身身份 = id；引用他者 = <noun>_id（如 search 的 strand_id）
  id / strand_id 一律全宽 24 hex，跨输出可 join
  （append_id 例外：64 hex 内容哈希，不是 strand 把手）

写命令三件套：写 journal 必收 --provenance、必有 --format json
孪生、写后回显卡片（见 tasktree explain card）。
（孪生与 provenance 的覆盖缺口见一致性 CI 豁免表，按批清偿。）

全局旗标：
  -C <DIR> / --chdir  如同在 DIR 启动；journal 解析与相对路径随之；DIR 不存在 → exit 3。
exit code：0 成功 / 1 解析失败 / 2 写入失败 / 3 参数非法。

永久豁免（点名豁免，防"看起来漏了"的二次猜测）：
  doctor 子命令风格（doctor journal）
  export --out <PATH>（主对象用旗标）
  append [CONTENT] [ID] 的 LEGACY 第二位置参数"#,
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
#[derive(Debug, Clone)]
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
    Error,
    Warning,
}

// ── Catalog ─────────────────────────────────────────────────

static CATALOG: &[DiagnosticInfo] = &[
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
        finding: "A task has a [deadline] entry whose by= time has passed, and the strand carries no closing marker ([verified] [done] [cancelled] [failed] [merged] [ended]).",
        impact: "The task is overdue; downstream schedule assumptions are invalid.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "verify or cancel the task; update the deadline if re-planned",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },
    DiagnosticInfo {
        code: "W069",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "concurrent marker write",
        finding: "The same marker type was written by two or more different agents on the same task.",
        impact: "Concurrent state transitions may conflict — the task's true state is ambiguous.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "review agents' actions and decide which one should continue",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },
    DiagnosticInfo {
        code: "W070",
        severity: Severity::Warning,
        category: "lifecycle",
        title: "strand moved under you",
        finding: "The checkpoint's provenance.producer differs from the producer of the last LogAppended entry on the target strand. Both producers must be present and non-empty for this check to fire; if either is absent the check is silently skipped.",
        impact: "You may be checkpointing a strand that was last touched by a different agent — your view of the strand's state may be stale.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "tasktree timeline --since-offset <OFFSET> --links <STRAND_ID>",
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
            command_str: "confirm the target with tasktree list; the checkpoint may belong on a successor strand",
            executable: false,
            requires_human: true,
        },
        producer: "lifecycle",
    },
    // ── Health (W062) ───────────────────────────────────
    DiagnosticInfo {
        code: "W062",
        severity: Severity::Warning,
        category: "health",
        title: "contradictory decision/constraint",
        finding: "A [decision] and [constraint] with the same keyword were written within 10 minutes from different strands.",
        impact: "The decision and constraint may conflict — the governance signal is ambiguous.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Manual,
            command_str: "review both entries and resolve the contradiction; append a clarifying entry",
            executable: false,
            requires_human: true,
        },
        producer: "health",
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
            command_str: "check vocabulary: tasktree explain markers",
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
            kind: RecoveryKind::AppendMarker,
            command_str: "tasktree close --id <STRAND_ID> [--as done|failed|cancelled|merged|verified]",
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
        finding: "A [fixed] entry carries a fixes=<prefix> token (prefix >= 8 hex chars) that does not match any [friction] entry's append_id in the same strand. The prefix either points to a nonexistent entry or to an entry that is not a [friction].",
        impact: "The [fixed] entry is not folded and its intended friction target remains exposed as an unresolved live debt. The pairing was silently skipped.",
        recovery: RecoveryInfo {
            kind: RecoveryKind::Edit,
            command_str: "check the fixes= prefix against tasktree show --id <STRAND_ID> and correct it",
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
            command_str: "tasktree timeline --since-offset <N> --links <STRAND_ID>",
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
pub fn catalog() -> &'static [DiagnosticInfo] {
    CATALOG
}

/// Routing order:
///   1. Diagnostic code lookup (case-insensitive; W062, w062, etc.)
///   2. Topic lookup (input lowercased; card, markers, retry, json, grammar)
///   3. Error with available-topics list and diagnostic-code hint
pub fn cmd_explain(input: &str, format_json: bool) -> String {
    // ── 1. Diagnostic code (case-insensitive) ──────────────
    if let Some(info) = lookup(input) {
        let output = crate::output::ExplainSuccessOutput::from(info);
        return if format_json {
            serde_json::to_string_pretty(&output).unwrap_or_else(|e| {
                format!(
                    r#"{{"ok":false,"code":"{}","error":"serialization failed: {}"}}"#,
                    input, e
                )
            })
        } else {
            format!(
                "{}\n  severity: {}\n  category: {}\n  title: {}\n\n  finding: {}\n\n  impact: {}\n\n  recovery:\n    kind: {:?}\n    command: {}\n    executable: {}\n    requires_human: {}\n\n  producer: {}",
                info.code,
                match info.severity {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                },
                info.category,
                info.title,
                info.finding,
                info.impact,
                info.recovery.kind,
                info.recovery.command_str,
                info.recovery.executable,
                info.recovery.requires_human,
                info.producer,
            )
        };
    }

    // ── 2. Topic (exact lowercase match) ───────────────────
    let lowered = input.to_lowercase();
    if let Some(topic) = topic_lookup(&lowered) {
        let output = crate::output::ExplainTopicOutput::from(topic);
        return if format_json {
            serde_json::to_string_pretty(&output).unwrap_or_else(|e| {
                format!(
                    r#"{{"ok":false,"topic":"{}","error":"serialization failed: {}"}}"#,
                    input, e
                )
            })
        } else {
            format!("{}\n\n{}", topic.title, topic.body)
        };
    }

    // ── 3. Unknown ─────────────────────────────────────────
    let available_topics: Vec<&str> = TOPICS.iter().map(|t| t.name).collect();
    if format_json {
        let error_output = crate::output::ExplainUnknownOutput::new(input, available_topics);
        serde_json::to_string_pretty(&error_output).unwrap_or_else(|_| {
            format!(
                r#"{{"ok":false,"input":"{}","error":"unknown code or topic"}}"#,
                input
            )
        })
    } else {
        format!(
            "unknown code or topic: {}\n  topics: {}\n  diagnostic codes: tasktree explain W062 etc",
            input,
            available_topics.join(", "),
        )
    }
}

mod runtime;
pub(crate) use runtime::*;

pub fn all_codes() -> Vec<&'static str> {
    CATALOG.iter().map(|d| d.code).collect()
}

pub fn catalog_size() -> usize {
    CATALOG.len()
}

mod audit;
pub use audit::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audit_journal_reports_edge_validity_from_graph_module() {
        use crate::event::Event;
        let ts = "2026-01-01T00:00:00Z".to_string();
        let events = vec![
            Event::StrandCreated {
                id: "task".to_string(),
                ts: ts.clone(),
                strand_type: None,
            },
            Event::LogAppended {
                id: "task".to_string(),
                ts: ts.clone(),
                content: "task summary".to_string(),
                ref_: None,
                append_id: None,
                git: None,
                provenance: None,
            },
            Event::StrandCreated {
                id: "parent_a".to_string(),
                ts: ts.clone(),
                strand_type: None,
            },
            Event::LogAppended {
                id: "parent_a".to_string(),
                ts: ts.clone(),
                content: "parent a".to_string(),
                ref_: None,
                append_id: None,
                git: None,
                provenance: None,
            },
            Event::StrandCreated {
                id: "parent_b".to_string(),
                ts: ts.clone(),
                strand_type: None,
            },
            Event::LogAppended {
                id: "parent_b".to_string(),
                ts: ts.clone(),
                content: "parent b".to_string(),
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
        let output = cmd_explain("W069", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(v["ok"], true);
        assert_eq!(v["code"], "W069");
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
        let output = cmd_explain("W062", false);
        assert!(output.contains("W062"));
        assert!(output.contains("contradictory"));
    }

    #[test]
    fn test_explain_text_unknown() {
        let output = cmd_explain("XYZ", false);
        assert!(output.contains("unknown code or topic"));
    }

    // ── Topic catalog tests ─────────────────────────────────

    #[test]
    fn explain_topics_resolve() {
        // All four topics resolve in both text and JSON modes.
        for name in ["card", "markers", "retry", "json"] {
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
    fn explain_code_lookup_unchanged() {
        // W062/w062 still route to diagnostic catalog (not topic lookup).
        let upper = cmd_explain("W062", true);
        let v: serde_json::Value = serde_json::from_str(&upper).expect("valid JSON");
        assert_eq!(v["ok"], true);
        assert_eq!(v["code"], "W062");

        let lower = cmd_explain("w062", true);
        let v2: serde_json::Value = serde_json::from_str(&lower).expect("valid JSON");
        assert_eq!(v2["ok"], true);
        assert_eq!(v2["code"], "W062");
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
            strand_type: None,
            entry_count: 1,
            summary: "test".to_string(),
            last_entry: "test".to_string(),
            last_offset: 0,
            catch_up: "tasktree timeline --since-offset 0 --links abc123".to_string(),
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
            OrientOutput, SearchOutput, StrandDetailOutput, StrandListItem, TimelineOutput,
        };
        let topic = topic_lookup("json").expect("json topic must exist");

        // show → StrandDetailOutput
        let show_sample = StrandDetailOutput {
            id: "a".to_string(),
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
            remind: "".to_string(),
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
        };
        let v = serde_json::to_value(&timeline_sample).expect("serialize TimelineOutput");
        for key in v.as_object().unwrap().keys() {
            assert!(
                topic.body.contains(key.as_str()),
                "json topic missing timeline field: {}",
                key
            );
        }
    }

    #[test]
    fn test_all_codes_present() {
        let codes = all_codes();
        assert!(codes.contains(&"W062"));
        assert!(codes.contains(&"W068"));
        assert!(codes.contains(&"W069"));
        assert!(codes.contains(&"W070"));
        assert!(codes.contains(&"W071"));
        assert!(codes.contains(&"W073"));
        assert!(codes.contains(&"W074"));
        assert!(codes.contains(&"W075"));
        assert!(codes.contains(&"W076"));
        assert_eq!(
            codes.len(),
            9,
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
        // W070/W071 were previously in this list as removed external-workflow
        // codes; they have been revived with new lifecycle semantics for the
        // checkpoint command (strand-moved and closed-strand guards) — see
        // git history for the old meanings.
        for code in [
            "E047", "W058", "W065", "W067", "W072", "E081", "W081", "E082", "W082", "E083", "W083",
            "E084", "W085", "E055", "E057", "E058", "W066",
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
        let output = cmd_explain("W062", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        let recovery = &v["recovery"];
        assert_eq!(recovery["executable"], false);
        assert_eq!(recovery["requires_human"], true);
        assert!(
            recovery["command"]
                .as_str()
                .unwrap()
                .contains("contradiction")
        );
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
    fn test_w070_can_explain() {
        let info = lookup("W070").expect("W070 should be in catalog");
        assert_eq!(info.code, "W070");
        assert_eq!(info.title, "strand moved under you");
        assert!(matches!(info.severity, Severity::Warning));
        assert_eq!(info.category, "lifecycle");
        let output = cmd_explain("W070", true);
        let v: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(v["ok"], true);
        assert_eq!(v["code"], "W070");
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

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
  疤痕行   仅当命令产生 W 码时追加（如 W071、W076）
           （W 码=写时瞬态诊断：骑写回显，不入账/不成疤/show 不复显，须当场捕证。ADR-0003）

把手行中的 <state> 显示生命周期（lifecycle），格式：
  open:   registered（未关闭）
  closed: closed:<disposition>（如 closed:done、closed:failed）
  生命周期由 mnema close / reopen 命令改变，append 的 marker 是注解。

语义：
  回显即预付的验证——写后输出卡片，调用方无需再跑 show/orient 确认。
  所有写命令（append/add/checkpoint/hide/unhide/link/close/reopen）
  都在写后回显受影响线的卡片。

JSON 形态（OrientStrand，写命令 result 字段 / orient active[]）：
  - id / slug:    全宽 strand id；slug 为人类别名，可为 null，机器 join 仍用 id
  - strand_type:  线的类型，可为 null（task/dag/why/session）
  - entry_count:  日志条目计数
  - summary:      第一条日志截断到 70 字符
  - last_entry:   最近一条日志截断到 70 字符
  - last_offset:  该线最近事件的 journal offset
  - catch_up:     就绪的近内容窗口命令（mnema show --id <ID> --tail 8）
  - lifecycle:    生命周期（"registered" 或 "closed:<disposition>"）

JSON shape 索引见 mnema explain json"#,
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
  [fixed]       已修复；可用 fixes=<entry哈希前缀≥8位> 指定目标 friction
  [lesson]      学到的教训
  [insight]     洞见
  [observed]    观察到的事实
  [progress]    进展 / [deliverable] 交付物
  [metric]      落账的测量值；约定写 name=val（如 [metric] win_count=26）
                可被 jq capture 抽成序列，见 mnema explain jq
  [deadline]    截止日期（by= 字段必须是日期或 RFC3339）
  [done]        完成注解（仅注解，不关闭线；关闭用 mnema close --id <ID>）
  [checkpoint]  由 mnema checkpoint 命令写入，勿手动添加

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
  init     已存在时跳过文件创建与 journal-id 覆写；总是打印初始化消息；目录幂等

不可盲目重试（有副作用）：
  append   重复写入新的 LogAppended 事件；
           超时后先 show/orient 查账再决定
  add      每次创建新 strand；不检查内容重复
  checkpoint  重复写入新的 checkpoint 条目；
              超时后先 timeline 查账再决定
  link     重复写入新的 link effect entry；legacy EdgeLinked 仍按投影折叠

通用原则：超时后先查账（show/orient/timeline），
确认事件是否已写入，再决定是否重试。"#,
    },
    TopicInfo {
        name: "json",
        title: "JSON 形态索引——各读命令 --format json 的顶层字段",
        body: r#"show（StrandDetailOutput）：
  id / slug / hidden / summary / entry_count / status / state_marker / state_offset / last_entry_offset /
  edges / belongs_to_edges / depends_on_edges / strand_branch / events
  ※ events[].entry=日志行；last_entry_offset=下次 --seen-offset；belongs_to_edges=父 / depends_on_edges=上游(F3)
list（StrandListOutput.strands[]，StrandListItem）：
  id / slug / entry_count / first_summary / last_summary / hidden / strand_type /
  edges / belongs_to_edges / depends_on_edges / status / state_marker /
  state_offset / last_entry_ts / last_entry_offset
orient（OrientOutput）：
  max_offset / active / closed_count / hidden_count / integrity / notices / since_command / delegation_command / remind / pause / stale_count
  ※ active[] 卡片见 card；stale_count=活跃且末条 silent≥2h（指针 list --stale 2h）
search（SearchOutput）：
  matches / count / query / marker
  ※ matches[]：strand_id / content / strand_type / hidden / entry_id / marker（entry_id=全哈希供 fixes=/--why；marker null=未筛）
doctor edges（EdgesOutput）：
  open_frictions / decisions_without_why / open_friction_count / open_friction_active_count / decision_without_why_count
  ※ 项：entry_id / strand_id / marker / content / offset；under/id 仅缩候选集；fixes= 仍扫全 journal
  ※ unfixed friction=无 fixes= 指它（不按 home strand 开闭过滤）；active_count=其中 registered 线上
  ※ decision 无 --why；--since N 只跳过 offset<=N 的存量 decision；doctor journal integrity 始终 JournalScope
timeline（TimelineOutput）：
  timeline / truncated / count / max_offset
  ※ timeline[]：journal_offset / ts / strand_id / strand_type / kind / ts_skew
append: seen_offset / seen_gap / warnings / closed_target / result / resolved_by / active_count；checkpoint: seen_offset / seen_gap / warnings / result
add: id / status / provenance / slug / parent_id / edge_type / result；find: id
hide / unhide: strand_id / status / noop / active_count / closed_count / hidden_count / result（卡片）
link: source_id / target_id / edge_type / status / result.source / result.target（卡片）
cutover-v2: applied / source_journal / archive_journal / map+certificate / source_event_count / imported_event_count / strand_count / entry_count / anchor_count / unresolved_ref_count；cutover-v3: applied / outcome / migration_id / source|history|target / map+certificate / counts / projection_ok
depends（DependsOutput）：id / summary / upstream_count / registered_upstream_count / upstreams[]
  ※ upstreams[]：id / lifecycle / summary / last_entry / show_command；under-scope（DependsScopeOutput）：root_id / count / strands[]
卡片/result 形态见 mnema explain card；jq 整型见 mnema explain jq"#,
    },
    TopicInfo {
        name: "jq",
        title: "jq 整型——把 JSON 投影切成你要的形",
        body: r#"JSON 是空间(tree)/时间(timeline)两视角投影，jq 是塑形层。
边界：jq 只塑形结构够的内容——埋在散文里的数/状态它抓不动，
故"写得可解析"是前提（marker 前缀、name=val），不是 mnema 多建命令。
（orient 开场 remind 的 read/extract 段即指向此页。）
顶层字段见 mnema explain json。常用：

取 strand id（免脆弱解析，取代手搓字符串切割）：
  echo "..." | mnema add --format json | jq -r .id

取日志行：
  mnema show --id <ID> --format json | jq -r '.events[].entry'

按 marker 聚条目（marker 是 .entry 前缀，取代 show 文字墙 + grep）：
  mnema show --id <ID> --format json | jq -r '.events[] | select(.entry | startswith("[friction]")) | .entry'
  坑：用 startswith；勿用 test("^\[...")——shell 里反斜杠转义会炸。

抽数字轨迹（先按约定写 [metric] name=val，再 capture 出序列）：
  echo "[metric] win_count=26" | mnema append --id <ID>
  mnema show --id <ID> --format json | jq '[.events[].entry | capture("win_count=(?<v>[0-9]+)") | .v | tonumber]'

数值筛选（offset / count / entry_count 是数，可比较）：
  mnema list --format json | jq '.strands[] | select(.entry_count > 10) | .id'

中途现状合成（"我在哪"：活线 + 各自 last_offset 即下次 --seen-offset 的 N）：
  mnema orient --format json | jq -r '.active[] | "\(.id[0:12]) n=\(.last_offset) :: \(.last_entry)"'

时间线切成精简视图：
  mnema timeline --format json | jq '.timeline[] | {ts, strand_id, kind}'"#,
    },
    TopicInfo {
        name: "writing",
        title: "写入范例——时机、形状、临时演练",
        body: r#"这是合成示例，不描述宿主项目事实；具体内容只用占位符。

什么时候写：
  方案成形时：写决定、依据、验证锚点。
  判断被现实改变时：写新观察和被推翻的假设。
  收口或不可逆动作前：先 checkpoint，再 close 或执行动作。

entry 形状模板：
  [decision] <claim>; anchor=<file>:<line>; verify=<command>
  [observed] <fact>; source=<command>; changes=<assumption>
  [friction] <blocked thing>; at=<file>:<line>; tried=<command>
  [fixed] fixes=<entry-hash> <what changed>; verified=<command>
  [deliverable] <files changed>; build=<command>; test=<command>

临时 journal 演练（把 <tmp>/<ID>/<entry-hash> 换成上一步输出）：
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
        title: "协作 forest——多路工作在 journal 里的形状",
        body: r#"协作只记录 journal 侧结构；怎么启动执行者属于外层约定。

结构：
  每路工作一条 strand；派生工作用 mnema add --parent <母线>，建 belongs-to 子线。
  子线 entry 首行自报身份：谁派的哪一路；不要把外层启动细节写成工具规范。
  belongs-to 方向是子指父：CHILD belongs-to PARENT，tree 把 CHILD 缩进到 PARENT 下。
  depends-on 方向是任务指上游：TASK depends-on UPSTREAM，供追溯 review context。

纪律：
  交付物落在自己的 strand；外层 stdout 只留一个可追的 strand 指针。
  worker 收工用 mnema close --id <ID> --as done|failed，不用 [done] 改生命周期。
  协调者收工先读子线 entries 和 close 状态，不信外层 stdout 自报成功。
  母线最后写综合/收束 entry，把子线结论合并成可审计结果。

派发判据：
  能并行摊开的审查、扫描、交叉验证才拆成多条子线。
  串行实现、一次只能一路推进的改码，留在当前线自己做。

常用读法：
  mnema tree --id <母线>
  mnema depends --id <任务线>
  mnema depends --under <母线>
  mnema doctor edges --under <母线>"#,
    },
    TopicInfo {
        name: "delegation",
        title: "递归委派——strand 是异步交接面",
        body: r#"委派在任意深度使用同一套 strand 语义，不存在特殊的顶层/二阶 worker 类型。

1. 每一路先创建一个 child：echo "<task-specific instruction>" | mnema add --parent <PARENT>。
2. body 只写任务专属指令；背景通过 0..N refs 连接，refs 默认不展开。
3. worker 从自己的 strand 读取任务，把进展、证据与结论 append 回该线，收工用 close。
4. 委派是异步的：协调者启动 worker 后继续其他工作，不轮询；到综合/验收点再读 close、entries、diff 与 tests。
5. 任意深度继续委派时仍是 add --parent + refs；是否允许下派由任务/harness 授权。
6. 并发写显式使用完整 strand id，不把 --last 当多 agent 默认。
7. 进程退出和 stdout 自报不等于工单成败；journal 中的事实才是交接依据。
8. 进程、模型、worktree、超时与重试由 harness 管理，Core help 不绑定厂商启动命令。

入口：mnema orient --id <CHILD>
增量读取：mnema timeline --since-offset <N>
查看子树：mnema tree --id <PARENT>；mnema depends --under <PARENT>"#,
    },
    TopicInfo {
        name: "grammar",
        title: "文法契约——全 CLI 一致的参数与命名规则",
        body: r#"目标线：单 id 命令两种写法等价（位置 <ID> 与 --id <ID>）；"最近活跃线"统一用 --last。
读+追加命令（show/find/hide/unhide/tree/depends/append/checkpoint）缺省即 --last；
close/reopen 收口动作强制显式指名、禁 --last/缺省；正文只走 stdin，故 add/append 无位置参数；timeline 的 --id 等价 --strand。
旗标词表（同一概念只有一个名字）：
  --include-hidden  含隐藏线（checkpoint/pick 主名；--all 为兼容别名）
  mnema list --all  （list 的隐藏线开关例外：只认 --all）
  --format json     机器输出唯一正典（explain --json 是兼容快捷）
  --provenance / --seen-offset <N>  写命令出处 / 上次看到的目标线 offset
  --tail <N>        只限显示、不改账，对任何目标可用
  --under <ID>      集合查询的 SubtreeScope（list/search/timeline/pick/depends；doctor edges 同）
  mnema orient --id <ID>。委派入口专用写法，候选集同集合查询 --under（不是把 --under 写在 orient 上）
  doctor edges --id <ID>  单线候选集；与 --under 互斥。doctor journal integrity 始终 JournalScope，不可用 scope 隐藏容器损坏
  --edge-type       link 的边类型（--type 是 deprecated 别名）
  --why / --from    引依据/记来源：线前缀=其最新条，entry 哈希前缀=精确该条；读取用 mnema show --entry <HASH>（--deref 展开链，--before/--after 邻域）
JSON 命名法：复数名词=数组；计数=count/*_count；自身身份=id；引用他者=<noun>_id；id/strand_id 全宽 64 hex 可 join。
跨 journal 引用（书写约定，本版不解析、doctor 不校格式）：<journal-id>:<strand>:<entry>
  journal-id=64 hex 存 .mnema/journal-id.json（sidecar，不进哈希链；init 生成/旧仓 doctor 幂等补写，永不变）；strand/entry 为 ≥8 hex 前缀；整线可 <journal-id>:<strand>:；读 id：mnema doctor journal。
写命令三件套：写 journal 必收 --provenance、必有 --format json 孪生、写后回显卡片（见 mnema explain card）。
（孪生与 provenance 的覆盖缺口见一致性 CI 豁免表，按批清偿。）
全局旗标：-C <DIR> / --chdir  如同在 DIR 启动；journal 解析与相对路径随之；DIR 不存在 → exit 3。
exit code：0 成功 / 1 命令执行失败 / 2 journal 不可读或损坏 / 3 解析或参数非法。
永久豁免（点名豁免，防"看起来漏了"的二次猜测）：
  doctor 子命令风格（mnema doctor journal）；pick（交互选择器；机器入口用显式 --id 或 mnema pick --print-id）
  add/append 正文位置参数、--stdin、--file 已在 v2 迁移中移除
  mnema export --out <PATH>（主对象用旗标）；mnema cutover-v2 --apply（journal maintenance）"#,
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
        assert!(topic.body.contains("不轮询"));
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
        assert_eq!(
            codes.len(),
            7,
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

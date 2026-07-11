# CORPUS：mnema 语义模型

本文是 mnema 的规范语义。命令、投影和实现都服从这里定义的领域模型。

---

## 1. 目的与边界

mnema 是给人类和 LLM 共同消费的 append-only journal。它保存可验证的事实、关系和状态变化，并提供确定性的投影与查询。

核心分工只有三层：

| 层 | 负责 | 不负责 |
|---|---|---|
| journal core | 记录、校验、索引、投影、增量读取 | 判断任务含义、总结历史、替用户决策 |
| 人类或 LLM | 阅读事实、理解语义、作出判断、写回结论 | 假装历史中存在未写入的事实 |
| harness | 启动进程、选择模型、分配 worktree、管理超时与重试 | 成为 journal 的领域实体或改变关系含义 |

由此得到四条硬边界：

1. **机器记录事实，不猜语义。** 不根据自然语言判断“矛盾”“阻塞”“几乎肯定错误”或“应当交给谁”。
2. **journal 不保存生成摘要。** 若某段上游内容有价值，使用 `ref` 指向原始事实；若需要新的判断，就作为新的 entry 写入并承担其作者责任。
3. **投影必须可重放。** 同一组有效 entry 在同一版本语义下得到同一结果。
4. **协调不是调度。** Core 表达任务谱系和注意力关系；进程存活、心跳、权限、资源与模型路由留给 harness。

原始 entry 的截取、tail 或 digest 可以是传输优化，但不能冒充由机器生成的语义摘要，也不能成为唯一可恢复的信息。

---

## 2. 领域代数

### 2.1 Journal

Journal 是一组 strand 的持久化容器与查询作用域，不是一个 strand，也不是所有 strand 的虚拟父节点。

Journal 的物理追加单位是 `JournalRecord`：

```text
JournalRecord = Entry | Anchor
```

Entry 承载事实并形成 strand；Anchor 只证明 journal 级完整性。Anchor 不是 entry，不拥有 strand，也不进入任何 strand 的 `prev` 链。新的 journal-level record kind 必须有独立的不变量与消费投影，不能伪装成 entry。

它拥有自己的 journal identity，可通过 sidecar 等方式持久化；这个 identity 不进入 strand 的父子关系，也不占用 strand 的 hash/id 空间。

公开模型中不存在：

- journal root strand；
- `effective_parent`；
- “顶层 strand 属于 journal 这个父亲”的关系；
- root worker、coordinator worker 等深度相关实体类型。

### 2.2 Entry

Entry 是唯一进入 strand 的事实单元。概念字段如下：

| 字段 | 含义 |
|---|---|
| `id` | 由规范化内容计算得到的身份 |
| `strand_id` | 所属 strand；首 entry 的 strand id 等于自身 id |
| `prev` | 同 strand 的前一 entry |
| `kind` | 事实种类，如 note、decision、constraint、effect |
| `body` | 原始正文 |
| `refs[]` | 0..N 个显式引用 |
| `author` | 写入者声明的身份或来源 |
| `created_at` | 记录时间 |
| `payload` | kind 对应的结构化数据 |
| `strand` | hash view 中的 strand key：genesis seed 或既有 strand id |

Entry 不可原地修改。更正、撤销、迁移和关系变化都通过新 entry 表达。

### 2.3 Strand

Strand 是一条由 `prev` 串起的 entry 链：

```text
e0 <- e1 <- e2 <- ... <- en
```

`strand_id = e0.id`。Strand 的身份来自首 entry，不来自标题、当前父节点、任务角色或文件位置。

Strand 可以表达任务、调查、设计讨论、工单、证据线或任何需要独立历史的主题。mnema 不要求所有 strand 都是“任务”。

### 2.4 Scope

公开查询只需要两种递归作用域：

```text
JournalScope      = journal 中的全部 strand
SubtreeScope(X)   = X 加上 belongs-to 下的整棵后代子树
```

同一查询在两种 scope 下应保持同一字段与同一解释，只改变候选集合。深度和调用者角色不得改变语义。

---

## 3. 身份、完整性与溯源

### 3.1 Entry hash

概念上：

```text
entry_id = H(canonical(entry_without_id))
```

Canonical bytes 使用 RFC 8785/JCS，并把 I-JSON 边界作为写入前不变量：对象成员名在每一层都必须唯一；整数只能落在 IEEE-754 安全整数域，不能吞掉重复键或把超界整数静默舍入。时间统一为 UTC `Z`、最短 RFC3339 小数形式；hash、journal 与 ref 中的 256-bit id 统一为 64 位小写 hex。字段顺序、null/empty、时间和数值表示由协议固定，不依赖某个 serde 版本的偶然输出。规范化输入覆盖正文、kind、前驱、strand key、时间、作者、结构化 payload、provenance 和全部 refs。v3 envelope 与所有 typed payload 都拒绝未知字段；扩展必须先升级 schema，不能由旧 reader 静默吞掉。

Genesis 不能把尚未算出的 `strand_id` 放回自己的 hash。它使用带标签的 seed：

```text
genesis.strand   = {
  kind: genesis,
  seed: <256-bit>,
  slug: string | null,
  strand_type: string | null
}
entry_id         = H(canonical(genesis_without_id))
public strand_id = entry_id
```

`slug` 与 `strand_type` 是 strand 创建事实，只存在于 genesis key，并参与 genesis identity；后续 entry 只通过既有 strand id 指向它们。它们不能藏进任意 payload，也不能放在 journal 外的 sidecar，否则 clone、迁移和重放会丢失可解析别名或类型。

后续 entry 使用既有 strand identity：

```text
entry.strand = { kind: existing, id: <full genesis entry_id> }
```

新建 strand 的 genesis seed 由加密安全随机源产生。v2 导入使用确定性 seed：

```text
H("mnema.import.v2" || source_journal_id || old_strand_id)
```

因此同一来源的迁移可重复，而正文完全相同的两条旧 strand 不会被合并。导入来源进入 canonical identity；迁移清单保存完整的旧 id 到新 id 映射。

`belongs-to` 是后续 effect 建立的关系，不回写首 entry，因此移动 strand 不改变 strand id。

### 3.2 两层完整性

mnema 同时保护：

1. **strand 链完整性**：`prev`、hash 与 strand identity 能发现链内篡改；
2. **journal 完整性**：anchor 覆盖 journal 的粗粒度顺序与行集合，发现整行删除、重排或截断。

每个 anchor 明确记录 `covered_record_count`、`previous_anchor`、`covered_records_digest` 与当时的 sorted strand heads；anchor 自身 digest 还覆盖 canonical `created_at` 和上述全部字段。`covered_records_digest` 对 anchor 之前的 canonical record 序列逐条带长度承诺，因此不同 strand 的 entry 互换位置也会改变承诺。非空 journal 的最后一条必须是 anchor；写事务把领域 entry 与其后 anchor 作为同一持锁事务追加，strict replay 把缺最终 anchor 视为未锚定尾部或截断，而不是健康 journal。

Git commit、外部归档或签名可以作为 journal 之外的锚。Core 可以记录相关事实，但不把某一种版本控制工具写进领域语义。

### 3.3 Provenance

`author` 表示 entry 自述的来源，不是权限、所有权或排他写锁。多个 agent 可以在同一 journal 写入；生产者不同本身既不是冲突，也不是错误。

若 harness 需要记录模型、attempt、worktree 或进程信息，应以普通 provenance 字段或 entry 写入；Core 不据此推断谁“应该”接手。

---

## 4. 三种关系

三种关系不可混用，它们回答不同问题。

| 关系 | 方向 | 基数 | 回答的问题 |
|---|---|---:|---|
| `belongs-to` | child → parent | 每个 child 0..1 个 parent | 这条线位于哪棵任务/主题树中？ |
| `depends-on` | source → target | 0..N | 阅读 source 时，还应注意或审阅哪些上游线？ |
| `ref` | entry → target | 每个 entry 0..N | 这条新事实具体引用哪些既有事实或对象？ |

### 4.1 belongs-to：结构谱系

`belongs-to` 构成森林，必须满足：

- 一个 child 最多一个直接 parent；
- 不允许自环；
- 不允许多节点环；
- 顶层 strand 没有 parent；
- `unlink` 后 child 成为新的顶层根；
- link/unlink 不改变 strand identity。

`belongs-to` 是递归 scope 的唯一结构基础。关闭、隐藏或失败不会自动级联到父亲或孩子。

### 4.2 depends-on：注意力与审阅

`depends-on` 不是硬阻塞、调度 gate 或完成条件。它表达：消费 source 时，target 值得一并查看。

因此：

- 多节点环可以合法表达互相审阅；
- Core 不把环称作死锁，不据此阻止 close；
- 自环没有新增信息，可以拒绝；
- 执行门槛与调度状态属于 harness，不改变 `depends-on` 的含义。

### 4.3 ref：显式证据

一条 entry 可以有零个、一个或多个 ref。Ref 指向 strand、entry 或具有显式类型的外部对象。

默认输出只展示 ref 的身份与读取方式，不自动展开目标内容。消费者按需发现和读取，避免引用扇出把上下文失控地灌入当前任务。

Ref 是相关性的显式承诺，不是摘要，也不意味着结构隶属或执行依赖。

---

## 5. 状态、生命周期与历史

### 5.1 Marker 与 effect

业务内容和系统状态分开：

- 普通 entry 记录 note、decision、constraint、checkpoint 等事实；
- effect entry 记录 link、unlink、close、reopen、hide、unhide 等状态变化。

投影由重放 effect 得到当前状态。状态改变不删除历史。

### 5.2 非级联原则

以下操作默认只作用于显式目标：

- `close` 不关闭孩子或父亲；
- `hide` 不隐藏整棵子树；
- `unlink` 不修改后代结构；
- parent 已关闭而 child 仍开放，是可观察事实，不是错误；
- depends 环存在，是关系事实，不是错误。

若用户需要批量操作，CLI 必须显式给出集合 scope、预览和逐目标结果，不能借递归关系暗中级联。

### 5.3 关闭之后仍可追加

Close 表示一个生命周期判断，不封存底层链。向 closed strand 追加可以成功，同时返回中性的 lifecycle warning。消费者仍可通过后续 reopen 或新 strand 表达新的工作。

### 5.4 分叉与后继

需要独立追踪的新方向使用新 strand：

- `add --parent X` 在同一持锁快照与批次中建立 belongs-to 子线，对并发读写保持整体可见；
- 同一次 `add` 可带 0..N 个有序 `--ref`，把首条正文连接到具体证据；
- 对已结束或已由别路完成的工作，另开后继线，不覆盖旧结论。

机器不得自动生成“前情摘要”。子线入口应由任务正文、parent 关系和必要 refs 组成。

---

## 6. 递归委派

递归性是命令设计的校准标尺：任何位于 strand X 上的 agent，都可以在被授权时创建 X 的孩子；孩子上的 agent 也可以用完全相同的语义继续创建下一层。

```text
X
├── A
│   ├── A1
│   └── A2
└── B
    └── B1
```

这里没有“顶层 coordinator 类型”和“二阶 worker 类型”。只有 strand、关系与当前调用者选择的 scope。

### 6.1 创建契约

委派入口应支持一次原子表达：

```text
add
  body = 任务专属指令
  parent = X
  refs = [R1, R2, ...]
```

创建成功时，子 strand 与 belongs-to 关系要么一起可见，要么都不可见，避免孤儿窗口。Refs 不限一个，且默认不展开。

### 6.2 异步契约

Worker 是异步生产者：

1. 协调者创建 strand 并启动 worker；
2. worker 把进展、证据和结论写回自己的 strand；
3. 协调者继续处理其他可推进工作，不轮询进程；
4. 到综合、验收或收口时，再按需读取 close 状态、entries、diff 与测试结果。

进程退出不等于任务成功或失败；stdout 自报成功也不是交付依据。Journal 中的事实才是交接面。超时、重启和换模型由 harness 管理。

### 6.3 显式目标

递归并发下，写操作应优先使用显式 strand id。`--last` 只能作为单人、单写流中的便利语法，不能成为多 agent 教程的默认写法。

是否允许继续向下派遣是授权问题，由任务或 harness 决定；它不改变 mnema 命令的递归语义。

---

## 7. 查询与命令的 scope

### 7.1 统一的集合 scope

面向集合的查询采用一致的可选 scope：

```text
<query>                 # JournalScope
<query> --under X       # SubtreeScope(X)
```

`orient --id X` 是面向单个入口的写法，其 scope 是 `SubtreeScope(X)`。集合查询统一使用 `--under X`。

### 7.2 命令语义矩阵

| 命令 | 目标 scope | 递归语义 |
|---|---|---|
| `init` | journal | 创建容器和 identity；无 strand 深度 |
| `add` | 显式 parent 或顶层 | 在任意深度创建 strand；支持 0..N refs |
| `append` | 单一 strand | 任意深度相同；支持 0..N refs |
| `checkpoint` | 单一 strand | 任意深度相同，不承担摘要职责 |
| `show` | 单一 strand | 展示该线；refs 默认不展开 |
| `link` / `unlink` | 两个显式 strand | 只改变 belongs-to，不级联 |
| `hide` / `unhide` | 单一 strand | 只改变目标投影状态，不级联 |
| `close` / `reopen` | 单一 strand | 只改变目标生命周期，不级联 |
| `tree` | JournalScope 或 SubtreeScope | 展示 belongs-to 森林/子树 |
| `depends` | 单一 strand 或 scoped set | 展示注意力关系，不计算调度状态 |
| `list` | JournalScope，可 `--under` | 同一 schema，仅候选集合变化 |
| `search` | JournalScope，可 `--under` | 在 scope 内检索原始事实 |
| `pick` | JournalScope，可 `--under` | 在 scope 内选择候选，不替 LLM 判断 |
| `timeline` | JournalScope，可 `--under` | 在 scope 内按时间投影 |
| `orient` | JournalScope | 给主会话 journal 入口 |
| `orient --id X` | SubtreeScope(X) | 给任意深度委派入口，包含 X 的整棵子树 |
| `doctor` | journal、单线或子树 | integrity 检查可失败；关系事实只报告 |
| `find` | journal/federation | 定位 journal，不虚构跨 journal 父子树 |
| `export` | journal | 完整导出是 journal 级操作 |
| `help` / `explain` | CLI 语义 | 与 strand 深度无关 |

### 7.3 Orient

`orient` 不是自动摘要器，而是一个有界入口投影。

`orient` 的 journal scope 暴露：

- journal identity 与健康状态；
- 顶层 roots 和可继续读取的明确命令；
- 增量读取能力，例如 `since`；
- 委派能力的最短入口与 `explain delegation` 指针；
- 未展开的 refs/关系计数或指针。

`orient --id X` 使用同一 schema，并把候选集合换成 X 的整棵 belongs-to 子树。它暴露：

- X 的原始任务入口与当前生命周期；
- X 的直接关系和子树导航；
- 增量读取位置；
- 何时 append、何时 add child、何时 close 的简短操作提示；
- 未展开 refs 的身份和读取命令。

它不应注入 ancestors、siblings 或其他 strand 的生成摘要。需要背景时，消费者沿 parent、refs、search 或 depends 主动读取。

### 7.4 增量消费

增量读取是一等能力，不应藏在深层 help：

- cursor/offset 是位置事实，不是所有权；
- `since` 只返回位置之后的新事实；
- cursor 落后可以提示，但不阻止写入；
- cursor 不得因 agent 深度或角色改变含义。

`timeline --under X` 默认按查询时的当前子树过滤，适合浏览当前结构。精确回答“X 的子树自 offset N 后发生了什么”使用：

```text
mnema timeline --under X --since-offset N --scope-at-event
```

`--scope-at-event` 按每个事件发生时的 belongs-to 成员关系过滤：纳入使 strand 加入/离开子树的 link/unlink，纳入在域期间的事实，排除加入前和离开后的事实。该模式必须显式选择，不能由 `--since-offset` 暗中改变 `--under` 的含义。

机器输出必须自描述这次读取的解释边界：`timeline --format json` 的 `scope` 给出 selector、规范化 root 与 membership 模式；`window.observed_through` 给出本次实际观察到的 journal 水位，`window.next_since_offset` 给出不会倒退的下一 cursor。它们与“返回了几条”正交，空命中也必须推进已观察水位。

---

## 8. CLI 与能力发现

### 8.1 输入输出契约

- 结构化参数使用显式字段，不从正文猜关系。
- 文本和 JSON 是同一事实的两种投影，不能有不同语义。
- `--format` 等枚举在写入前严格校验；未知值不得静默回退。
- JSON 字段遵循只增不改不删；新增语义使用新增字段表达。
- stdin 适合承载长正文，argv 保留短标量和显式 id。

### 8.2 三层发现路径

能力发现采用以下层次：

```text
mnema --help
  -> mnema <command> --help
     -> mnema explain <topic|CODE>
```

职责分别是：

| 层 | 应包含 | 不应包含 |
|---|---|---|
| 顶层 help | 命令地图、核心概念、发现下一层的方法 | 每个工作流的长教程 |
| 命令 help | 语法、输入、输出、副作用、退出语义、可解析示例 | 跨命令概念全集 |
| explain | 关系、递归 scope、委派、诊断代码等跨命令语义 | 绑定某一家模型或 harness 的启动命令 |

`orient` 只给当下最必要的入口，不替代 help/explain。

### 8.3 `explain delegation`

委派帮助说明：

1. 为每一路创建一个 child strand；
2. body 只写任务专属指令，背景通过 0..N refs 连接；
3. refs 默认不展开，worker 按需读取；
4. worker 在自己的 strand append/close；
5. 委派是异步的，协调者继续工作，不轮询；
6. 任意深度使用同一套命令；
7. 并发写使用显式 id，不把 `--last` 当默认；
8. 进程与 worktree 管理由 harness 负责。

文档可以在仓库级花名册中列出 Codex、Grok、Claude 等调用方式，但 Core 的 help 不携带厂商命令。

### 8.4 示例的递归纪律

所有 help 示例都要经受这个问题：把示例里的 root 换成任意深度 strand，含义是否仍成立？

若答案是否定的，必须明确说明它是 journal-global 操作或显式语法例外，而不能暗示只有顶层协调者才能使用。

---

## 9. 诊断边界

错误和诊断只表达命令能否执行、硬不变量是否成立，以及值得调用者注意的客观事实。它们不判断任务语义，不承担调度，也不因 strand 深度或 agent 角色改变含义。

诊断类别、输出 envelope、code 注册表和 Doctor 契约见 [DIAGNOSTICS.md](./DIAGNOSTICS.md)。

---

## 10. Journal 边界与 federation

### 10.1 单一可写 journal

Journal 在单机上串行化 append：

- 同一 journal 可以有多个 agent/process 生产事实；
- 写入层负责锁与原子追加；
- Core 不承诺多机器同时写同一 journal 后自动合并；
- 进程间并发不改变 entry 和关系语义。


### 10.2 活动版本与历史

v3 journal 使用一个小型 manifest 作为唯一激活点：

```text
.mnema/
├── active-journal.json
├── journals/
│   └── <v3-journal>.jsonl
└── history/
    └── <v2-source>.jsonl
```

- `active-journal.json` 声明活动 schema、活动文件、journal identity 和 tagged `origin`：新建 journal 使用 `{kind:fresh,id}`，v2 cutover 使用 `{kind:migration,id,map_path,map_sha256,certificate_path,certificate_sha256}`；
- active artifact 的 `sha256` 承诺 manifest 激活瞬间已经验证并持久化的初始字节；激活后的 journal 继续 append-only 增长，当前完整性由 strict replay 与最终 anchor 判断，普通写入不得为了刷新该 hash 而重写 manifest；
- manifest 中 active artifact 必须位于 `journals/`，v2 history、map 与 certificate 必须位于 `history/`；路径统一使用 `/`、彼此唯一且不能逃逸目录；
- fresh origin 的 `history` 必须为空；migration origin 必须声明至少一个 v2 history artifact，不能为 fresh init 伪造 migration 证明；
- 普通读写只跟随 manifest 指向的 v3 文件；
- prepare 阶段把 v2 source 的同字节副本（或同文件系统硬链接）持久化到 `history/` 并校验 hash；不得在 manifest commit 前移动或删除旧活动路径，否则旧 resolver 会提前失去完整活动状态；
- manifest commit 后 `history/` 中的 v2 artifact 只提供显式历史读取和迁移证明；旧活动路径若仍存在只是 legacy shadow，默认读写忽略并由 Doctor 报告；
- 激活 manifest 前，v2 仍是完整活动状态；原子替换 manifest 后，v3 是完整活动状态；
- manifest 创建成功就是 commit point；若随后目录持久化同步失败，结果必须表达“已激活但耐久性未确认”，不能倒报为未激活；
- prepared artifacts、mapping、certificate 和 target 全部验证并持久化后才能替换 manifest；
- apply 在旧 journal 的同一独占写锁内完成最终 source digest 校验与 manifest commit，禁止 v2 append 穿越两者之间的窗口；
- 重复迁移根据 source digest 与 migration identity 收敛为 resume 或 noop，不产生第二个活动 journal；
- 旧 binary 产生的 legacy shadow 不参与 v3 解析，Doctor 将其报告为独立事实。

### 10.3 Federation

多个 journal 可以被发现、只读聚合或通过显式外部 ref 连接，但不能伪装成一棵具有共同虚拟父节点的 belongs-to 树。

跨 journal 连接包含：

- journal identity；
- 来源与读取地址；
- 引用目标的显式类型；
- 目标不可用时的可观察状态。

跨 journal 同步、冲突解决和写入路由属于独立协议，不由本地 scope 语法暗示完成。

### 10.4 Export

完整导出是 journal-global 操作，保留 journal identity、entry identity、关系、anchor 与原始顺序。导出不生成摘要，也不把多个 journal 改写成一棵 belongs-to 树。

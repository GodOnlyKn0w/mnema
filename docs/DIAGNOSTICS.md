# DIAGNOSTICS：错误与诊断契约

本文定义 mnema 的错误、warning、notice、Doctor 和诊断 code。所有命令、输出格式和调用深度使用同一套语义。

---

## 1. 判定边界

mnema 只诊断两类事实：

1. 命令无法执行，或执行会破坏硬不变量；
2. 命令已经执行，但出现值得调用者注意的客观事实。

mnema 不根据自然语言判断矛盾、阻塞、质量、所有权或任务成败，也不根据进程状态裁决 strand 状态。

以下事实不是错误：

- `depends-on` 多节点成环；
- closed parent 下仍有 open child；
- 多个 producer 写入同一 strand；
- cursor 落后；
- closed strand 收到新的 append 或 checkpoint；
- entry 的自然语言看起来与过去不一致。

---

## 2. 三类结果

| 结果 | 含义 | 退出语义 |
|---|---|---|
| hard error | 命令无法执行，或会破坏硬不变量 | 非零；写入前拒绝或原子回滚 |
| warning | 命令已执行，但有客观事实需要调用者注意 | 退出 0；明确 `written` |
| notice | 中性事实，适合查询、日志或 orient 展示 | 退出 0 |

同一个诊断在 JournalScope、SubtreeScope 和单一 strand 上具有相同 code、name、category、字段、写入语义和退出语义。Agent 的深度、模型和协调角色不参与诊断判定。

---

## 3. 诊断 envelope

文本和 JSON 输出陈述相同事实。JSON 诊断使用统一 envelope：

```json
{
  "code": "W076",
  "name": "seen-offset-behind",
  "category": "concurrency",
  "severity": "warning",
  "subject": { "strand_id": "..." },
  "scope": { "kind": "subtree", "root": "..." },
  "written": true,
  "retryable": false,
  "details": {
    "seen_offset": 41,
    "strand_last_offset": 45
  },
  "recovery": [
    "mnema since --id ... --after 41"
  ]
}
```

字段契约：

| 字段 | 语义 |
|---|---|
| `code` | 稳定的机器标识，号码不复用 |
| `name` | 稳定、可读、与角色无关的语义标识 |
| `category` | `syntax`、`resolution`、`integrity`、`concurrency`、`lifecycle`、`structure` 或 `reference` |
| `severity` | `error`、`warning` 或 `notice` |
| `subject` | 触发事实的显式对象 |
| `scope` | 本次检查使用的 JournalScope、SubtreeScope 或单一对象 |
| `written` | 本次命令是否已经持久化写入 |
| `retryable` | 在输入不变时重试是否可能成功 |
| `details` | code 对应的结构化事实，不要求调用者解析文案 |
| `recovery` | 读取、检查或显式重试命令，不代替人类/LLM 作语义决定 |

命令在写入前完成 grammar、枚举和硬不变量校验。未知 flag、未知 format 和非法参数组合不得静默回退。

---

## 4. Warning code 注册表

### W059 · append-target-closed

- 类别：`lifecycle`
- 触发：内容成功追加到 closed strand。
- 含义：目标在写入前处于 closed 生命周期；不评价追加是否正确。
- 结果：`written=true`，退出 0。
- Recovery：读取 close 原因；按人的判断 reopen、另开 child/successor，或保留现状。

### W068 · deadline-passed-open

- 类别：`lifecycle`
- 触发：已记录的 deadline 早于检查时间，strand 仍 open。
- 含义：只报告时间与生命周期事实，不推断任务逾期原因或下游影响。
- 结果：只读检查，`written=false`，退出 0。
- Recovery：读取相关 entry；由人类或 LLM 决定是否更新计划、close 或继续。

### W071 · checkpoint-target-closed

- 类别：`lifecycle`
- 触发：checkpoint 成功写入 closed strand。
- 含义：目标在写入前处于 closed 生命周期；checkpoint 仍是合法事实。
- 结果：`written=true`，退出 0。
- Recovery：读取生命周期；按需要 reopen 或把后续工作写到新 strand。

### W073 · marker-near-known

- 类别：`syntax`
- 触发：正文开头的 marker 与一个已知 marker 近似但不相同。
- 含义：可能存在拼写差异；Core 保留原文，不自动更正。
- 结果：`written=true`，退出 0。
- Recovery：检查原始 entry；需要更正时追加一条明确的新事实。

### W074 · closing-marker-no-lifecycle-effect

- 类别：`lifecycle`
- 触发：正文含关闭类 marker，但本次命令没有写入 close effect。
- 含义：内容注释与结构化生命周期是两件事，strand 状态没有因此改变。
- 结果：`written=true`，退出 0。
- Recovery：需要改变生命周期时显式执行 `mnema close --id <ID>`。

### W075 · fixes-target-unresolved

- 类别：`reference`
- 触发：`fixes` 声明指向的目标在本次 scope 中无法解析。
- 含义：保留引用声明，但不宣称目标已经被修复。
- 结果：envelope 明确 `written`；退出 0。
- Recovery：按 ref 原文定位目标，再追加可解析的显式引用。

### W076 · seen-offset-behind

- 类别：`concurrency`
- 触发：调用者声明的 seen offset 落后于目标 strand 写入前的 last offset。
- 含义：调用者尚未读取部分新 entry；它不是所有权冲突，也不阻止写入。
- 结果：`written=true`，退出 0。
- Recovery：使用 envelope 给出的 `since` 命令增量读取缺口。

---

## 5. Hard error 家族

Hard error 只覆盖不可执行条件和硬不变量：

| Name | 类别 | 简介 | 写入与重试 |
|---|---|---|---|
| `invalid-argument` | syntax | 参数组合、枚举值或输入形状不在公开 grammar 中 | 未写入；修正输入后重试 |
| `target-not-found` | resolution | 显式 strand、entry 或 ref 目标无法定位 | 未写入；读取候选后重试 |
| `target-ambiguous` | resolution | 简写解析到多个对象 | 未写入；使用完整 id 重试 |
| `journal-corrupt` | integrity | JSONL、hash、prev 或 anchor 无法通过校验 | 不继续写；先检查或恢复 journal |
| `belongs-self-link` | structure | child 与 parent 是同一个 strand | 未写入；选择不同目标 |
| `belongs-cycle` | structure | 新边会让 belongs-to 森林成环 | 未写入；调整结构关系 |
| `belongs-multiple-parent` | structure | child 已有直接 parent，调用未显式执行 reparent | 未写入；先 unlink 或显式 reparent |
| `expected-offset-mismatch` | concurrency | compare-and-append 的预期位置与当前位置不同 | 未写入；增量读取后重试 |
| `reference-invalid` | reference | ref 的类型、形状或标识不符合 grammar | 未写入；修正 ref 后重试 |
| `source-schema-unsupported` | integrity | cutover source 不是受支持且可严格验证的历史 schema | 未激活；使用对应迁移器或恢复 source |
| `migration-source-invalid` | integrity | source 存在断链、重复 identity、未解析本地 ref 或其他不可迁移事实 | 未激活；先修复或显式裁决历史事实 |
| `migration-source-changed` | concurrency | prepare 之后 source bytes/digest 发生变化 | 未激活；重新 prepare 后重试 |
| `migration-map-incomplete` | integrity | 不是每条 source record、identity 和 local ref 都有唯一处置 | 未激活；修复 converter |
| `migration-id-collision` | integrity | 两个不同 source identity 映射到同一 v3 identity | 未激活；修复 canonical identity 或 seed |
| `migration-artifact-conflict` | integrity | 同一 migration identity 已存在内容不同的 target、map 或 certificate | 未激活；检查 staging 与历史文件 |
| `active-schema-unsupported` | integrity | manifest 指向 Core 不支持的活动 schema | 拒绝读写；使用支持该 schema 的版本 |
| `legacy-history-write-forbidden` | resolution | mutation 目标解析到冻结的历史 identity | 未写入；通过 migration map 取得 v3 identity |
| `atomic-activation-failed` | concurrency | prepared v3 已验证，但原子替换 active manifest 失败 | 未激活；保留 artifacts，允许 resume |

`depends-on` 环不得产生 `belongs-cycle`；两种关系具有不同不变量。

---

## 6. Doctor

Doctor 使用两层输出：

1. **integrity**：检查 JSONL、hash、prev、anchor、唯一 parent 和 belongs-to 无环；失败返回 hard error 和非零退出码；
2. **facts**：报告 dangling ref、closed parent/open child、depends 环和未展开引用等客观事实；退出 0。

Doctor 不判断自然语言，不生成摘要，不计算调度状态，不自动修复。修复操作必须显式、可预览、可审计。

---

## 7. 能力发现

每个 code 和 hard error name 都可通过以下命令读取完整契约：

```text
mnema explain <CODE|name>
```

命令 help 只说明本命令可能产生哪些诊断，并链接到 `explain`；跨命令语义只在本注册表和 `explain` 中定义。

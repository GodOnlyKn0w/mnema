# journal v1 → v2 迁移指南

> **历史文档。** 当前默认格式是 v3；本文只保存已退役 v1→v2 cutover 的操作与证据，不是当前用户的迁移入口。当前格式边界见 [CORPUS](CORPUS.md#102-活动版本与历史)，文档导航见 [README](README.md)。

本文档说明如何把一份 v1 journal 迁移为纯 v2 形态。迁移由 `mnema
cutover-v2` 一条命令完成，本仓库自己的 journal 已于 2026-07-03 用同一
流程完成迁移（`.mnema/journal.v1.jsonl` 与
`.mnema/migration-v1-to-v2.json` 即当次产物）。

概念模型与迁移清单的设计依据见 [CORPUS.md](CORPUS.md) 第 10 节；
本文只管操作。

## 1. 迁移改什么

| # | v1 | v2 |
|---|---|---|
| 1 | 随机 strand id（24 hex） | 按线哈希链：entry id = hash(前驱 ‖ 内容)，线 id = 首条 entry 的哈希 |
| 2 | 引用地址 `线id@offset`（位置寻址） | entry 哈希（内容寻址，跨 journal 有效）；过渡期两者双存 |
| 3 | `StrandClosed`/`EdgeLinked` 等独立事件类型 | 统一为带 `effect` 的 entry（close/reopen/link/unlink/hide/unhide） |
| 4 | 正文可走位置参数/`--stdin`/`--file` | stdin 单通道（迁移前已在 CLI 落地，此处只涉及历史事件的翻译） |
| 5 | link 是独立的边事件 | link 是写在源线链上的 effect entry |
| 6 | 无 journal 级完整性 | 锚点事件（全部线头哈希清单+摘要），每次写入后自动追加，doctor 可验 |

迁移是**整本重写**：逐事件翻译成 v2 形态、重算全部哈希、追加首个锚点。
旧 journal 原样归档，不丢任何历史。

## 2. 迁移前检查

1. **停掉所有写入方**。迁移须在单写者窗口内执行（协调者本人），
   多 agent 并发写入期间不要迁移。
2. **确认 journal 健康**：

   ```bash
   mnema doctor journal
   ```

   有解析损坏先处理再迁——损坏行会被跳过（映射表里
   `source_event_count` 与 `imported_event_count` 的差即跳过数）。
3. 可选：手工再备一份 `.mnema/journal.jsonl`（`--apply` 本身会
   自动归档，见下）。

## 3. 干跑（默认行为）

不带 `--apply` 只报告计划、不落盘：

```bash
mnema cutover-v2
mnema cutover-v2 --format json   # 机器可读的计划报告
```

看三个数：源事件数、可导入数、未解析引用数（`unresolved_refs`）。
未解析引用不阻塞迁移，但值得先弄清来历。

## 4. 执行

```bash
mnema cutover-v2 --apply
```

在 `.mnema/` 产出三件：

| 文件 | 内容 |
|---|---|
| `journal.jsonl` | 新的纯 v2 journal（重写产物） |
| `journal.v1.jsonl` | 迁移前原件，逐字节归档（可用 `--archive <PATH>` 改位置） |
| `migration-v1-to-v2.json` | 新旧映射表（可用 `--map <PATH>` 改位置） |

## 5. 验证

```bash
mnema doctor journal    # 验哈希链 + 锚点
mnema orient            # 活跃线清单应与迁移前一致
mnema show <某条线前缀>  # 抽查内容完好
cargo test --release       # 在源码仓库内迁移时
```

## 6. 旧 id 换算

**全部 strand id 都会改变**（v2 线 id = 首条哈希）。散落在外部的旧 id
（脚本、笔记、其他系统）不会自动更新，用映射表换算：

```bash
# 旧 strand id → 新 strand id
jq -r '.strands["0006533183e644765ae00000"]' .mnema/migration-v1-to-v2.json

# 按旧 offset 找某条 entry 的新哈希（entries 数组含 old_offset/old_strand_id/new_strand_id/new_entry_id）
jq '.entries[] | select(.old_offset == 42)' .mnema/migration-v1-to-v2.json
```

映射表 schema 标识为 `tasktree-v2-cutover-map-v1`，字段：
`schema` / `source_event_count` / `imported_event_count` /
`strands`（旧线 id → 新线 id）/ `entries`（逐条映射）/
`unresolved_refs`。

## 7. 双轨兼容面（已于 2026-07-04 退役，v0.2.0）

过渡期曾维护三个兼容行为，现均已显式退役：

- ~~`--why`/`--from` 双写 legacy `ref=<线id>@<offset>` pin~~ →
  新写入只存 entry 哈希 refs。失效检测不再需要 pin：journal offset
  全局单调，"被引线越过引用时点"由位置直接推出（ref-target-advanced）。
- ~~`append_id`、`ref` 字段在 JSON 输出中保留~~ → 已从
  `--format json` 出口移除；写回执与 checkpoint 的条目把手改为
  `entry_id`。存储结构仍**读取容忍**退役前已落盘的旧字段（旧行
  原样保留、旧 journal 照常解析），只是不再写、不再输出。
- ~~show 文本对无 v2 数据的行回退 legacy 显示~~ → 只渲染 v2 把手。

配套：`[fixed] fixes=<前缀>` 配对把手改为 entry 哈希（对退役前写入
的旧 [friction] 行仍按其 append_id 兜底匹配）；audit 的 pin 基
`why-staleness` lint 节由哈希基 `ref-target-advanced` 取代。

仍然保留的 v1 读取面（属下一次 schema 迁移，非本次退役范围）：
v1 随机 id 行的虚拟 entry_id 投影、legacy 事件类型
（StrandClosed/EdgeLinked 等）的折叠读取、cutover-v2 翻译器本身。

## 8. 回滚

新 journal 不满意时（迁移后**尚未写入新内容**的前提下）：

```bash
mv .mnema/journal.jsonl .mnema/journal.v2.rejected.jsonl
mv .mnema/journal.v1.jsonl .mnema/journal.jsonl
```

若迁移后已有新写入，回滚会丢弃这些新条目——先用
`mnema timeline --since-offset <迁移点>` 导出再回滚。

## 9. 已知边界

- 锚点链从迁移那一刻开始，doctor 只能验证迁移点之后的完整性；
  迁移点之前的历史由 `journal.v1.jsonl` 归档 + 映射表作证。
- 哈希链防改历史，防不住有写权限者整本重算（CORPUS §3）；外部锚定
  （纳入 git 或私有备份仓）是另一层，须显式决策。
- 跨 journal 的旧引用（若有）不在映射表内，需持有方各自换算。

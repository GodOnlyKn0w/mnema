# journal v1 → v2 迁移指南

本文档说明如何把一份 v1 journal 迁移为纯 v2 形态。迁移由 `tasktree
cutover-v2` 一条命令完成，本仓库自己的 journal 已于 2026-07-03 用同一
流程完成迁移（`.tasktree/journal.v1.jsonl` 与
`.tasktree/migration-v1-to-v2.json` 即当次产物）。

概念模型与迁移清单的设计依据见 [CORPUS.md](../CORPUS.md) 第 10 节；
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
   tasktree doctor journal
   ```

   有解析损坏先处理再迁——损坏行会被跳过（映射表里
   `source_event_count` 与 `imported_event_count` 的差即跳过数）。
3. 可选：手工再备一份 `.tasktree/journal.jsonl`（`--apply` 本身会
   自动归档，见下）。

## 3. 干跑（默认行为）

不带 `--apply` 只报告计划、不落盘：

```bash
tasktree cutover-v2
tasktree cutover-v2 --format json   # 机器可读的计划报告
```

看三个数：源事件数、可导入数、未解析引用数（`unresolved_refs`）。
未解析引用不阻塞迁移，但值得先弄清来历。

## 4. 执行

```bash
tasktree cutover-v2 --apply
```

在 `.tasktree/` 产出三件：

| 文件 | 内容 |
|---|---|
| `journal.jsonl` | 新的纯 v2 journal（重写产物） |
| `journal.v1.jsonl` | 迁移前原件，逐字节归档（可用 `--archive <PATH>` 改位置） |
| `migration-v1-to-v2.json` | 新旧映射表（可用 `--map <PATH>` 改位置） |

## 5. 验证

```bash
tasktree doctor journal    # 验哈希链 + 锚点
tasktree orient            # 活跃线清单应与迁移前一致
tasktree show <某条线前缀>  # 抽查内容完好
cargo test --release       # 在源码仓库内迁移时
```

## 6. 旧 id 换算

**全部 strand id 都会改变**（v2 线 id = 首条哈希）。散落在外部的旧 id
（脚本、笔记、其他系统）不会自动更新，用映射表换算：

```bash
# 旧 strand id → 新 strand id
jq -r '.strands["0006533183e644765ae00000"]' .tasktree/migration-v1-to-v2.json

# 按旧 offset 找某条 entry 的新哈希（entries 数组含 old_offset/old_strand_id/new_strand_id/new_entry_id）
jq '.entries[] | select(.old_offset == 42)' .tasktree/migration-v1-to-v2.json
```

映射表 schema 标识为 `tasktree-v2-cutover-map-v1`，字段：
`schema` / `source_event_count` / `imported_event_count` /
`strands`（旧线 id → 新线 id）/ `entries`（逐条映射）/
`unresolved_refs`。

## 7. 迁移后的双轨兼容面

迁移产物是纯 v2，但工具在过渡期仍维护以下兼容行为，供尚未换算完
旧引用的消费者过渡：

- `--why`/`--from` 在写入 entry 哈希 refs 的同时，双写一条 legacy
  `ref=<线id>@<offset>` 引用前沿 pin；
- 旧字段 `append_id`、`ref` 在存储与 `--format json` 中保留（JSON
  契约字段只增不删）；
- show 文本视图优先渲染 v2 把手（短 entry 哈希、hash refs），仅在
  无 v2 数据时回退 legacy 显示。

这些兼容面将在后续大版本中退役，退役同样只能走一次显式迁移。

## 8. 回滚

新 journal 不满意时（迁移后**尚未写入新内容**的前提下）：

```bash
mv .tasktree/journal.jsonl .tasktree/journal.v2.rejected.jsonl
mv .tasktree/journal.v1.jsonl .tasktree/journal.jsonl
```

若迁移后已有新写入，回滚会丢弃这些新条目——先用
`tasktree timeline --since-offset <迁移点>` 导出再回滚。

## 9. 已知边界

- 锚点链从迁移那一刻开始，doctor 只能验证迁移点之后的完整性；
  迁移点之前的历史由 `journal.v1.jsonl` 归档 + 映射表作证。
- 哈希链防改历史，防不住有写权限者整本重算（CORPUS §3）；外部锚定
  （纳入 git 或私有备份仓）是另一层，须显式决策。
- 跨 journal 的旧引用（若有）不在映射表内，需持有方各自换算。

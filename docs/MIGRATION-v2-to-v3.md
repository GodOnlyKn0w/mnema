# journal v2 → v3 迁移

本文只负责当前 `cutover-v3` 的操作步骤。v3 identity、manifest、history 与原子激活语义以 [CORPUS §10](CORPUS.md#10-journal-边界与-federation) 为准；命令的实时参数与退出语义以 `mnema cutover-v3 --help` 为准。

## 迁移前

```powershell
mnema doctor journal
mnema cutover-v3
mnema cutover-v3 --format json
```

默认是 dry run：读取纯 v2 source，准备迁移计划并验证投影等价性，不写 durable state。先保存 JSON 报告并审阅 source identity、目标 artifact 与 equivalence 结果。

## 激活

```powershell
mnema cutover-v3 --apply
```

Apply 在同一独占写锁下重新确认 source，持久化 v2 history、mapping、certificate 和 v3 target，最后以原子替换 `active-journal.json` 作为 commit point。激活前 v2 完整可用；激活后默认读写只跟随 manifest 指向的 v3 journal。失败不得产生半激活状态。

重复执行 `--apply` 在 artifact 与 identity 一致时收敛为 resume/noop，不创建第二个活动 journal。

## 验证

```powershell
mnema doctor journal
mnema orient
mnema --version
```

确认：

- `doctor journal` 使用 v3 strict replay 且完整性健康；
- `orient` 的 strand、生命周期和关系投影与 dry-run equivalence 一致；
- `active-journal.json` 指向 `journals/` 下的 v3 artifact；
- v2 source、mapping 与 certificate 保留在 `history/`，不再作为默认写入目标；
- PATH 中二进制声明 `v3 read/write (default)`。

若旧 binary 在激活后继续写 legacy `journal.jsonl`，v3 活动 journal 不受其影响；`doctor` 会报告 legacy shadow。先升级 PATH，再审阅 shadow delta，把确需保留的事实显式写入 v3，不要拼接两份链。

## 回退边界

v3 激活不是通过覆盖历史来完成的。不要手工编辑 manifest、移动 artifacts 或改写 journal。需要调查时保留整个 `.mnema/`，使用 dry-run/doctor/export 获取证据；任何恢复动作都应先根据 [DIAGNOSTICS](DIAGNOSTICS.md) 的具体 code 判断，而不是猜测当前活动文件。

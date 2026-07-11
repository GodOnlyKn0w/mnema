# mnema Test Catalog

本文件是测试内容注册表：说明每组测试保护什么事实、从哪里运行、属于哪个本地 gate、需要什么隔离，以及产生什么证据。测试原则和新增测试的判据见 `docs/TESTING.md`；自动化入口落地后只认本表登记的 suite 名称。

## Gate vocabulary

| Lane | 用途 | 允许内容 | 时间目标 |
|---|---|---|---|
| `Fast` | 每次小改后的本地反馈 | 格式、编译、纯契约和小型黑盒 smoke | 尽量 ≤ 2 分钟 |
| `Full` | 提交/部署前权威 gate | release build、全部已注册 correctness suites | 当前观测约 8 分钟 |
| `Nightly` | 显式启动的扩展验证 | 大 seed、性能、fuzz、crash/failpoint | 不设交互等待目标，但必须有 timeout |

远程 CI 不在当前范围。`Direct` 与 `AsyncExec` 是同一 lane 的执行器，不是不同测试语义；二者必须产生同一 report schema。

## Current inventory

下表时间来自 2026-07-11 Windows release gate 的量级观测，仅用于 timeout 和分片，不是性能承诺。

| Suite id | Layer / protected claim | Exact entrypoint | Lane | Isolation | Observed | Evidence / owner |
|---|---|---|---|---|---:|---|
| `format` | Rust 格式稳定 | `cargo fmt --check` | Fast, Full | repo read-only | 秒级 | exit code；Rust source |
| `compile-release` | release profile 可编译；所有 test binaries 可链接 | `cargo test --release --no-run` | Fast, Full | 独占或独立 `CARGO_TARGET_DIR` | 约 1–3 分钟（冷） | Cargo log + TerminalEvent；Cargo.toml |
| `unit` | event/canonical/activation/v3 codec/projection/CLI/JSON/help/write/read contracts | `cargo test --release --bin mnema` | Full | Rust tests 使用自身 temp/CWD lock；不可触碰 repo `.mnema` | 382 tests，约 203 秒 | Rust test report；`src/**/*` + `src/tests/*` |
| `behavior` | release CLI 黑盒 scope、cursor、refs、并发完成态、manifest smoke | `cargo test --release --test behavior_harness` | Fast, Full | 每场景独立 temp project；固定 `NO_COLOR`/`TZ` | 6 tests，约 18 秒 | test report；`tests/behavior_harness.rs`, `tests/behavior/*` |
| `cli-recovery` | 错误 argv 的 exit/stderr 修复提示，不污染正文 | `cargo test --release --test cli_recovery` | Fast, Full | Cargo 提供 release binary；无 repo journal | 3 tests，<1 秒 | test report；`tests/cli_recovery.rs` |
| `v2-v3-compat` | 冻结 v2 bytes 的 source/migration/target identity、raw v3 records、迁移前后投影 | `cargo test --release --test v2_v3_compat` | Full | fixture 只读复制到 temp；fixture 强制 LF | 1 test，<1 秒 | golden hashes + report；`tests/fixtures/*`, `tests/v2_v3_compat.rs` |
| `v3-runtime` | fresh v3 写入、manifest、doctor、shadow、checkpoint、orient strict read | `cargo test --release --test v3_runtime` | Full | 每 test 独立 temp project | 7 tests，约 3 秒 | test report；`tests/v3_runtime.rs` |
| `generated-differential-ci` | 独立 scope model 对 current/event-time full replay 与 cursor 增量一致性 | `cargo test --release generated_scope_model_matches_full_and_incremental_replay` | Full | 纯内存、固定 seeds | 包含在 unit，约数秒 | failure seed/cursor；`src/tests/query_tests.rs` |
| `doctor-smoke` | 部署后二进制能严格读取本仓 journal | `mnema doctor journal` | Full（部署后） | 明确 `-C <repo>`；只读 | 秒级 | stdout/stderr + TerminalEvent；release wrapper |

## Planned inventory

计划项在实现前先登记，落地后必须把状态、入口和证据更新为 Current；不能仅在脚本里暗藏测试。

| Suite id | Status | Claim | Planned lane | Required isolation / artifact |
|---|---|---|---|---|
| `behavior-snapshots` | planned | reviewed stdout/stderr/exit JSON/text 不发生未审漂移 | Full | 每场景 temp；只替换声明动态值；checked-in snapshots + diff |
| `crash-atomicity` | planned | 写入/anchor/cutover/cache 提交点被杀后，只见旧态、完整新态或明确 integrity failure | Full（小集）, Nightly（矩阵） | test-only failpoints；独立 process containment；journal artifact |
| `concurrent-visibility` | planned | reader 在 parent+refs 与 anchor 批次中途看不到半状态 | Full | 多进程 writer+reader；独立 temp journal；事件时间线 |
| `performance-smoke` | planned | 100/1k/10k events 的核心读写路径无数量级退化 | Full（小规模） | 固定数据生成器；机器信息；JSON measurements |
| `performance-scale` | planned | 100k/1m events 的 p50/p95/p99、吞吐、冷暖 cache 曲线 | Nightly | 独占机器/target；不与 correctness shard 并发；baseline JSON |
| `differential-expanded` | planned | 扩大 seed、事件数、cursor、cache 状态 | Nightly | 固定 seed catalog；failure corpus |
| `fuzz-strict-input` | planned | strict JSON/canonical/v3 reader 对 hostile input 不 panic/hang | Nightly | 有界 corpus/time/memory；crash corpus |
| `fixture-typed-unlink` | planned | typed unlink + legacy tombstone 的 v2→v3 解释冻结 | Full | 新版本 fixture，不修改 compat-v1 |
| `fixture-retired-why` | planned | legacy why 只迁成 ref、不成为 live edge | Full | 新版本 fixture，不修改 compat-v1 |

## Local automation contract

统一入口计划为：

```powershell
./scripts/ci.ps1 -Mode Fast    -Executor Direct
./scripts/ci.ps1 -Mode Full    -Executor AsyncExec
./scripts/ci.ps1 -Mode Nightly -Executor AsyncExec
```

`scripts/ci.ps1` 负责选择本表 suite、组合结果和产出 `mnema.ci-report/v1`；`scripts/async-release-gate.ps1` 只把 suite 映射为 durable run。AsyncExec 记录进程事实，不解释测试成功，不重试，不理解 strand。

Full lane 固定先单路 `compile-release`，再并发 correctness shards，避免多个 Cargo 编译争夺 artifact lock：

```text
compile-release
  ├─ unit
  ├─ behavior
  ├─ cli-recovery
  ├─ v2-v3-compat
  └─ v3-runtime
```

每个 async run 的 RequestId 必须包含 repo、commit、lane、suite 和自动化 schema version；不同 worktree 必须使用独立 `CARGO_TARGET_DIR`。日志与 Handle/TerminalEvent 放在 `.artifacts/ci/<commit>/<run>/`，不得写入 `.mnema/`。

## Registration rule

新增或改变测试时同时更新本表：

1. 写清一个可证伪 claim，而不是“增加覆盖率”；
2. 给出稳定 suite id 与精确入口；
3. 标明 Fast/Full/Nightly 和最大 timeout；
4. 标明 journal、cwd、环境、target 与并发隔离；
5. 说明失败留下的最小复现证据；
6. 性能测试区分观测 baseline 与硬门槛；
7. fixture 和 snapshot 一旦发布不得原地重写。

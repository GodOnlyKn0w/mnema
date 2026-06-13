# AGENTS.md

tasktree 的源码仓库（Rust CLI：append-only journal + 投影）。
本仓库吃自己的狗粮：工作记忆在 `.tasktree/`，可用二进制在
`target/release/`（已在 PATH）。

会话开始先跑 `tasktree orient`；能力沿 `tasktree --help` →
子命令 `--help` → `explain <topic|CODE>` 逐阶发现。

工程纪律：

- `cargo build --release && cargo test --release` 全绿才算完。
- 参数与输出契约：`tasktree explain grammar`。新命令、新旗标、
  新 JSON 字段先读契约再动手——一致性 CI 会咬人。
- help 文本里的示例命令被 CI 真解析——改 help 必须保证示例可解析。
- JSON 输出是公开契约：规则见 src/output.rs 头注（字段只增不改不删）。
- 领到带 strand ID 的任务：`tasktree show --id <ID>` 拉工单全文，
  进展与结论记回同一条线。

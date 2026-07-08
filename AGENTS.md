# AGENTS.md

mnema 的源码仓库（Rust CLI：append-only journal + 投影）。
本仓库吃自己的狗粮：工作记忆在 `.mnema/`，可用二进制在
`target/release/`（已在 PATH）。

本文件是唯一源：codex 直读本文件，claude 经 CLAUDE.md 的
`@AGENTS.md` 引用读到同一份。

主会话（任务不带 strand ID）开始先跑 `mnema orient`；
能力沿 `mnema --help` → 子命令 `--help` →
`explain <topic|CODE>` 逐阶发现。

工程纪律：

- `cargo build --release && cargo test --release` 全绿才算完。
- 参数与输出契约：`mnema explain grammar`。新命令、新旗标、
  新 JSON 字段先读契约再动手——一致性 CI 会咬人。
- help 文本里的示例命令被 CI 真解析——改 help 必须保证示例可解析。
- JSON 输出是公开契约：规则见 src/output.rs 头注（字段只增不改不删）。

## 多 agent 协作协议

本节写给任何在本仓库启动的 agent——包括被另一路 agent 用
`claude -p` / `codex exec` 无头拉起的。角色不由启动方式决定，
由任务决定：

- 任务带 strand ID → 你是 worker，走工单纪律；
- 任务要求拆解可并行的活 → 你是协调者，走派发纪律。

### 工单纪律（任务带 strand ID）

- 入口是工单本身，不跑 orient（那是主会话的入口）：
  `mnema show --id <ID> --digest` 起步，要全文去掉 --digest；
  上游背景 `mnema depends --id <ID>`；主动检索 `search`。
- 进展与结论 `append` 回同一条线；收工
  `close --id <ID> --as done|failed`。
- entry 开头标明身份（谁派的哪一路），多路并发靠这个区分笔迹。
- 交付物全部落 strand；stdout 只放一句指针（"完事，见 strand <ID>"）。
- worker 交付物不因进程死活自动判 failed（进程死 ≠ 任务败）；换路做成的结果落新线（`mnema add --from <旧ID>`）或母线，不要 append 回已被另一路关掉的 worker 线。
- 禁 git commit——派你的一方审 diff 后自己提交。
- 默认不再往下派子代理，除非工单原文明确授权（防递归扇出）。

### 派发纪律（你要拆活）

- 先过判据：能并行摊开（多题审查、多文件扫描、双审交叉验证）才派；
  串行实现类（一次只能一路推进的改码）自己干，派了只多转述开销。
- 每路一条 strand（`mnema add`）。prompt 只需三样：strand ID、
  身份标识、任务专属指令——协议本身不必复述，被派方启动时会
  自动读到本文件。
- 入口命令（prompt 落文件，stdin 喂入）：
  - codex：`codex exec --full-auto -m gpt-5.5 -c model_reasoning_effort=high - < prompt.md`
  - claude：`claude -p --model sonnet --permission-mode bypassPermissions < prompt.md`
- 长任务后台跑、stdout 落日志；超时杀进程是调用方的责任
  （两个 CLI 都没有 timeout 旗标）。
- 模型分配：简单任务 sonnet、难题 codex 5.5 high、多用 codex
  （claude 无头路会撞 5h 会话限额中途阵亡）。
- 收工判读顺序：先看 strand 的 close 状态与 entries；退出码只说明
  进程死活，stdout 的自报成功不作数。
- worker 阵亡（非零退出/超时）→ strand 上的半途痕迹即接手点，
  换一路（不限厂商）`show --id <ID> --tail` 续。
- 双审交叉验证：同题两路独立跑，二审 prompt 禁止先读线上已有发现，
  协调者只裁决定性分歧。

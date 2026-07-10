# 关于

无头派发用的模型花名册（batch 标题树，多根）。读法：
`batch tree agent-roster.md` 看全貌；
`batch get agent-roster.md#grok-45/无头调用` 直接取某模型的启动命令；
`batch cat agent-roster.md#codex agent-roster.md#grok-45` 拼多条对比；
`batch paths agent-roster.md` 列全部地址（喂 fzf/脚本）。

通用纪律见根目录 **`AGENTS.md`**（主会话第一步 `mnema orient`；带 strand ID 的 worker 从 `mnema show --id <ID> --digest` 起步）。共性：prompt 落 **`scratch/<agent>/`**、后台跑、stdout/stderr 落同目录、收工以 strand 的 close 状态与 entries 判读（stdout 自报不作数）。这份花名册来自本仓实跑；单次面板只形成待复验画像，不冒充稳定人格。

**无头 worker 是异步协作者，不是同步子程序。** 派发成功后，协调者继续自己的主线工作，
不靠轮询进程或 strand 等它完成；到真正需要综合、合并或收口时，再一次性读取 worker
strand 的 close 状态、entries、测试证据与 worktree diff。`registered` 只表示工作线仍开，
不等同于外部进程正在运行；进程退出也不自动等于任务失败。

# codex

## 无头调用
PowerShell（普通本地 worker）：
```powershell
Get-Content -Raw scratch\codex\prompt.md |
  codex exec --ephemeral --ignore-user-config --ignore-rules `
    --sandbox workspace-write -m gpt-5.6-terra --json - `
    1> scratch\codex\stdout.jsonl 2> scratch\codex\stderr.log
```

已由外层容器或 benchmark token 隔离时，才把 `--sandbox workspace-write` 换成：
`--dangerously-bypass-approvals-and-sandbox`。这是避免 Windows 嵌套 sandbox 把
worker 降成只读，不是普通派工默认值。

- prompt/日志/摘要默认放 **`scratch/codex/`**（勿堆仓库根）。
- `codex exec` 是正式非交互入口；`--json` 在 stdout 输出 JSONL，工具事件可从
  `item.started` 读取，`turn.failed`/`error` 可识别 provider failure。
- `--full-auto` 已 deprecated；新脚本显式选择 `--sandbox workspace-write`。
- prompt 可作位置参数或从 stdin 读取；PowerShell 用上面的管道形式最稳。
- 工作目录用 `-C <dir>`，或先切 cwd；benchmark 每个 episode 使用 `--ephemeral`。

## gpt-5.6-terra

### 已观测画像
两任务同 seed baseline 闸中：i18next-cal 101/101（36 actions，约 10m55s），
pendulum-cal 85/85（16 actions，约 9m01s），两次均首次 episode 通过。
JS 任务中主动补了回归测试，改动较厚；Pendulum 能抓住 IANA timezone 名含 `/`
的隐藏边界。画像是谨慎、验证导向、愿意多读多测，速度不占优但完成校准较好。

### 适合
- benchmark 的 Codex anchor model；
- 隐藏边界多、正确性优先、需要自行补测试的任务；
- 长链调试、接手恢复、需要完整取证与 journal 纪律的工作；
- 复杂 Python/类型/API 兼容任务。

### 注意
- 在 i18next 对照中比 Luna 慢约 32%；
- 容易形成较大 diff 和更多自建测试，做极小补丁时需明确范围；
- action cap 强杀会丢失末 turn usage，不能用残缺 token 遥测评价其成本。

## gpt-5.6-sol

### 无头调用
将 Codex 通用命令中的模型换为 `-m gpt-5.6-sol`。

### 定位
GPT-5.6 当前三款 Codex 模型的智能水平排序为 **Sol > Terra > Luna**。Sol 是高难度
推理与综合的优先档；本仓尚缺与 Terra/Luna 同任务、同 seed 的完整量化面板，因此
这里只记录产品定位，不用未经验证的速度、diff 大小或完成校准冒充实测人格。

### 适合
- 高难设计、架构综合、跨模块根因分析；
- 需要同时审源码、运行证据与多路 agent 结论的裁决任务；
- 错误决策代价高、值得支付更高推理成本的工作。

### 注意
- 不应把 Sol 用在简单扫描和机械改字上浪费容量；
- 与 Terra/Luna 的稳定差异仍需同题盲测，智能排序不能自动推出速度或成本排序。

## gpt-5.6-luna

### 无头调用
将 Codex 通用命令中的模型换为 `-m gpt-5.6-luna`。

### 已观测画像
同一两任务闸中：i18next-cal 101/101（37 actions，约 7m28s），diff 更集中；
pendulum-cal 首次 83/85，空白接手后仍为 83/85，最终 36 actions、2 episodes、
budget exhausted。遗漏点是把 `Europe/Madrid` 内的 `/` 当成区间分隔符。
画像是落笔快、实现集中，但对隐藏组合边界与完成宣告的校准弱于 Terra。

### 适合
- 反馈回路快、可见测试密、失败后能快速重跑的 JS/局部实现任务；
- 原型、窄范围修补、需要较小 diff 的短任务；
- 作为模型性格/速度敏感性对照。

### 不宜直接承担
- 当前 benchmark 的唯一 Codex anchor；
- 隐藏边界密集且错误完成代价高的任务；
- 未经 baseline 能力闸就进入完整 A/B/C/D 因子。

Luna 不应被排除出模型画像层：先按 task 跑 baseline 能力闸，通过的
task-model cell 再进入接手机制矩阵。

## 坑
- 过度操心会自伤：写过「清理残留进程」的命令把自己 shell 也杀了 → 别让它做激进进程清理。
- 偶发网络 TLS 断连中途阵亡（换路续）。
- Go 测试/缓存偶发写不动默认 AppData 路径 → 可设 `TMP`/`TEMP`/`GOCACHE` 到可写目录（如 `D:\tmp`）。

# grok-45

## 无头调用
```powershell
grok --prompt-file scratch\grok\prompt.md -m grok-4.5 `
  --output-format streaming-json --always-approve `
  --no-subagents --no-memory --cwd <工作目录> --session-id <新UUID> `
  1> scratch\grok\stdout.jsonl 2> scratch\grok\stderr.log
```

- 二进制 `~/.grok/bin/grok.exe`（在 PATH；工具 shell 可能需绝对路径）。
- `--prompt-file <file>` 或 `-p "<prompt>"` 才进入单轮无头模式。
- 当前实机 Grok 0.2.93 不接受上游新文档中的 `--no-auto-update`，升级 CLI 后再复核。
- `streaming-json` stdout 当前只见 `thought/text/end`；工具计数仍需读取 session
  `events.jsonl` 的 `tool_started`，这是兼容层而非稳定公开契约。

## 性格与智能
Pendulum 三-episode 冷启动面板中，episode 1 被墙钟杀，episode 2/3 自然退出；
跨 episode 接住 mnema，留下两条 closed strands（5/6 entries），最终全量测试
1851 passed/5 skipped。已观察到设计直觉锐、实现利落、跟指令紧；journal 默认粒度
偏粗，明确要求中间节点后会改善。3D/空间能力强是仓库经验判断，尚未由本 benchmark
量化。

mnema 全命令黑盒体验补充（2026-07-10）：Grok 4.5 自行覆盖约 90 条
text/JSON、正常/错误/边界路径，准确抓到 `timeline --since-ts` 未来阈值反而
返回全量、非法时间未拒绝，以及 `link` 自环造成永久自阻；修复后补了 5 组回归，
release build 与 296+3 tests 全绿。它很适合从“实际调用会发生什么”出发做宽面
契约猎错，且会把确定问题直接收成小实现闭环；但首轮脚本曾因 CLI 形参用错产生
9 个假阴性，之后能主动清洗并重跑，说明黑盒结论仍应要求最小复现而非相信首报。

## 适合
- 要一版利落干净的实现或设计；
- 中断后的代码现场恢复与 mnema 冷启动；
- 空间、3D、结构布局类问题；
- 有充足额度时由单实现者完成实现、测试、build 闭环。
- CLI/API 的宽面黑盒体验、错误路径与机器契约猎错；
- 需要“发现一个真 bug → 定点修复 → 回归测试”快速闭环的任务。

## 坑
- 额度不足会中途 `usage limit`；长活先确认余额。
- 边界判断偶偏：会做优雅但范围略错的决定（如把 friction 按所在 strand 开闭过滤而漏报）→ 交付必须有人/遥测复核。
- 默认不做细粒度 journal 留痕，要在 prompt 里明确要求。

# composer-25

## 无头调用
```powershell
grok --prompt-file scratch\composer\prompt.md -m grok-composer-2.5-fast `
  --output-format streaming-json --always-approve `
  --no-subagents --no-memory --cwd <工作目录> --session-id <新UUID> `
  1> scratch\composer\stdout.jsonl 2> scratch\composer\stderr.log
```

Composer 2.5 使用同一个 Grok CLI，不是 Cursor CLI。

## 性格与智能
Pendulum 冷启动面板中首 episode 约 3m31s 自然退出，后两个接续 episode 均撞
10 分钟墙钟；跨 episode 使用 mnema，留下两条 closed strands（3/4 entries），最终
报告全量测试 1838 passed。小样本显示其首次实现启动快、记录短，但在“已完成现场的
重新核实并及时退出”上弱于 Grok 4.5；需要更多任务复验，不能据此下稳定人格结论。

mnema 死代码审计补充（2026-07-10）：Composer 2.5 把 32 条 release warning
分成测试可见性、迁移遗留、真实未使用 API、契约保留四类；没有机械删字段，最终
删除约 225 行、release build 零 warning、291+3 tests 全绿。它在局部收敛、删除型
重构和“哪些不能删”的判断上很稳，但过程约 29 分钟，strand 主要只留开工与交付
两端，协调者中途可见性明显弱于最终质量。

## 适合
- 快速首稿、原型和局部实现；
- 预算较短、验收明确、允许外部 grader 早停的任务；
- 作为 Grok 4.5 的同 CLI 不同模型对照。
- 死代码分类、迁移尾巴清理、局部减熵；
- 目标是缩小实现面且要求明确保留兼容字段的删除型重构。

## 坑
- 冷启动后可能长时间重复核实已完成现场，必须配 grader early-stop 和墙钟保险；
- journal 记录更短，若研究写侧行为需保留原样观察，不应靠强提示把差异抹平；
- 当前画像只来自一个三-episode Pendulum 面板。
- 长验证期间默认少写进度，不能把“strand 暂时没更新”误判成停工；仍需墙钟与
  构建日志作为外层观测，最终只认 close + tests。

# 混编心法

- 当前可用池只含 Grok 与 Codex 家族。
- **默认容量配比：Grok 家族 7 : Codex 家族 3。** 这是累计派发数量的推荐基线，不是逐任务机械轮询。十路独立任务可先按 Grok 4.5×4、Composer 2.5×3、Terra/Luna 合计×3 排布，再按任务适配调整。
- **能力路由优先于配比。** 高难综合与裁决优先 Sol；隐藏边界、复杂兼容、长链取证优先 Terra；快速 JS/局部实现可用 Luna；设计、空间、结构和完整实现优先 Grok 4.5；短反馈原型、明确 grader 可早停的任务优先 Composer 2.5。
- 重要设计仍做独立同解：至少一路 Grok 4.5 与一路 Sol/Terra；第二路在 Composer/Luna 中按任务类型选择。二审禁先读第一路结论，分歧交主会话裁决。
- 不为凑 7:3 把明显不适配的任务派给某模型；
- 快·准·稳分散：Luna/Composer 快，Terra 稳且边界意识强，Sol 承担最高难度综合，Grok 4.5 设计利落。没有单独最强，搭起来强。
- 脆点各异：Grok 依赖额度与 session-event 兼容层，Codex 偶发网络断且 Windows 嵌套 sandbox 易只读；长活启动前做最小探针，失败按 strand 接手点换路续。

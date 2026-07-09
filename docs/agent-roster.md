# 关于

无头派发用的模型花名册（batch 标题树，多根）。读法：
`batch tree docs/agent-roster.md` 看全貌；
`batch get docs/agent-roster.md#grok/无头调用` 直接取某模型的启动命令；
`batch cat docs/agent-roster.md#codex docs/agent-roster.md#grok` 拼多条对比；
`batch paths docs/agent-roster.md` 列全部地址（喂 fzf/脚本）。

通用派发纪律见 AGENTS.md「派发纪律」。共性：prompt 落文件、后台跑、stdout 落日志、收工以 strand 的 close 状态与 entries 判读（stdout 自报不作数）。这份花名册是本仓多轮实战攒的经验，据实更新。

# codex

## 无头调用
`codex exec --sandbox workspace-write -m gpt-5.5 -c model_reasoning_effort=high - < prompt.md`
- `--full-auto` 已 deprecated，用 `--sandbox workspace-write`。
- 无 `--prompt-file`，prompt 走 stdin（`- < file`）。
- workdir 由 cwd 决定；要在别处工作 `cd <dir> && codex exec ...`。

## 性格与智能
取证 + 经验主义：会亲手 dogfood 撞出 bug，findings 精确到 commit 哈希。默认就勤——不用提，自己一路 [progress] 留痕，journal 纪律三家最好。操作自觉（察觉并发 build 争用会自己隔离 target 目录绕开）。风格厚重、防御性、偏啰嗦。

## 适合
重活长活、要自己留痕自己兜底、取证类审查、协调者角色、串行实现改码。默认首选主力。

## 坑
- 本机 apply_patch / patch tool 坏 → 必须脚本化编辑（让它写 python 改文件）。
- 过度操心会自伤：写过「清理残留进程」的命令把自己 shell 也杀了 → 别让它做激进进程清理。
- 偶发网络 TLS 断连中途阵亡（换路续）。

# grok

## 无头调用
`grok --prompt-file <file> -m grok-4.5 --permission-mode bypassPermissions --cwd <工作目录>`
- 二进制 `~/.grok/bin/grok.exe`（在 PATH；工具 shell 可能需绝对路径）。
- 模型 id 是 `grok-4.5`。help 举例的 `grok-build` 是产品名、当模型会报 unknown model id；`grok models` 看可用。
- `--prompt-file <file>` 或 `-p "<prompt>"` 触发无头；`--cwd` 指工作目录。

## 性格与智能
设计直觉三家最锐、代码最简最干净、契约（JSON 字段只增不改）守得一丝不苟。跟指令跟得紧。纪律靠提——默认只记起终两头，一在 prompt 里要求「中间节点也记」就漂亮补上（decision 带 why、fixed fixes= 闭环都用）。

## 适合
要一版利落干净的实现/设计；单实现者完整闭环（有额度时实现+测试+build/验证全自己走完）。

## 坑
- 免费额度撑不住一个完整任务，会中途 "usage limit" 阵亡 → 长活需 SuperGrok 额度。
- 边界判断偶偏：会做优雅但范围略错的决定（如把 friction 按所在 strand 开闭过滤而漏报）→ 交付必须有人/遥测复核。
- 默认不做细粒度 journal 留痕，要在 prompt 里明确要求。

# claude

## 无头调用
`claude -p --model sonnet --permission-mode bypassPermissions < prompt.md`
- `--model sonnet`=Sonnet 5、`--model opus`=Opus 4.8。fable-5 一般经主会话/Agent 侧。
- prompt 走 stdin。

## 性格与智能
- opus-4.8：引经据典，把判断锚在原文/源码上（评审会逐条引 CORPUS 章节、核实源码行号）；结构化穷尽、偏长。
- sonnet-5：均衡快手，适合简单扫描与评审的一路。
- fable-5：面板里出过「范例应被发现、不该被播种」那种解题的巧劲。

## 适合
- opus：评审 / 裁决 / 把决定钉在原则与原文上。
- sonnet：简单并行扫描、双审的一路、便宜点缀。

## 坑
claude 无头会撞 5h 会话限额，长活中途阵亡 → 只作短活/点缀，长活交 codex 或 grok。

# 混编心法

- 独立同解才敢信，分歧处才要人裁 → 重要设计上异构面板（codex + 多个 claude 型号 + grok）独立评审，二审禁先读线上已有发现。
- 快·准·稳分散在三家：grok 快、codex 稳（自留痕自兜底）、opus 把关原则。没有单独最强，搭起来强。
- 脆点各异：claude 无头脆在 5h、grok 免费脆在额度、codex 偶发网络断 → 长活主力 codex；要设计/收尾利落插 grok（喂饱额度）；评审/裁决插 opus。
- 收尾恒定：不管派谁，交付都以 strand 的 [deliverable] + close 状态判读；主会话独立验 build/test 全绿再合并，不信自报。

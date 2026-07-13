# mnema 文档地图

[English](README.en.md) | 简体中文

每个规范事实只有一个主责文档。其他文档只链接，不复制整段规则；发生冲突时按下表主责归位，而不是保留两套说法。

| 文档 | 唯一职责 | 不负责 |
|---|---|---|
| [README](../README.md) | 产品入口、最短工作循环与常用命令 | 完整领域规范、内部模块设计 |
| [ARCHITECTURE](ARCHITECTURE.md) | 目标模块、边界、不变量和承重决策 | 命令教程、错误码清单、历史迁移步骤 |
| [CORPUS](CORPUS.md) | 规范领域语义：entry/strand、虚拟 Journal 根、三种关系、scope、生命周期、委派与 federation | 代码文件布局、厂商 agent 启动命令 |
| [DIAGNOSTICS](DIAGNOSTICS.md) | 错误、warning、notice、Doctor 与 code 注册表 | 任务语义判断、调度策略 |
| [TESTING](TESTING.md) | 测试原则、分层、不变量与隔离政策 | suite 实时清单 |
| [TEST-CATALOG](TEST-CATALOG.md) | 当前/计划 suite 注册、入口、lane、timeout 与证据 | 重述测试哲学 |
| [agent-roster](agent-roster.md) | 已实测的模型调用方式、适配与雷区 | mnema Core 语义和通用委派协议 |
| [MIGRATION-v2-to-v3](MIGRATION-v2-to-v3.md) | 当前 v2→v3 dry-run、激活与验证步骤 | 领域身份规则的重复说明 |
| [MIGRATION-v1-to-v2](MIGRATION-v1-to-v2.md) | 已退役 v1→v2 的历史操作记录 | 当前 v3 使用指南 |

仓库级 agent 协作纪律只维护在 [AGENTS.md](../AGENTS.md)，Claude 通过 `CLAUDE.md` 引用同一份。CLI 的现行语法与机器契约以 `mnema --help`、子命令 `--help`、`mnema explain <topic|CODE>` 为准；Markdown 不复制完整 help。

## 目标架构一句话

mnema 是多 agent 协作的语义拓扑基座：Journal 是不落盘的虚拟投影根，任意 strand 都可成为局部根；默认视野是当前根的向下闭包，parent/refs/depends-on 是不展开出口；Core 记录语义事实，不解释进程与调度事实。

## 维护规则

1. 新概念先确定主责文档，再从其他文档链接过去。
2. 被新设计替代的内容直接改写或删除，不保留“现行方案 + 旧方案”并列。
3. 历史迁移文档必须显式标记历史状态，不进入当前上手路径。
4. Help 示例由 CI 解析；JSON 字段遵循只增不改不删。
5. 测试新增先登记 `TEST-CATALOG.md`，原则变化才修改 `TESTING.md`。

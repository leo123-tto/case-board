# PR: 工作流智能层 — 四轨分流 · 四道门禁 · Agent集群

> 🤖 Generated with [Claude Code](https://claude.com/claude-code)
> Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>

## 概述

本 PR 为 CaseBoard 贡献一套完整的**律师人机协同工作流智能层**，核心理念是：不以死规则的代码形式将这些流程固化到 App 里，而是将经过实战验证的**规则文件、Agent 定义、Skill 路由矩阵、记忆管理协议**作为可独立加载的知识层，供 CaseBoard 的 AI 助手或外部 Agent 集群按需读取。

## 设计原则

不把全部规则静态塞进上下文，按**任务类型 + 执行阶段**动态组装。每项任务仅加载必需的规则子集，无关规则直接排除。对标 ACE 活页手册模式。

## 涵盖内容

| 模块 | 文件 | 内容 |
|---|---|---|
| **工作流架构** | `docs/workflow-architecture.md` | 四轨分流路由 + 加载三层机制 + Agent 行为四原则 + 通用底线 |
| **四道门禁** | `docs/four-gate-system.md` | Gate 1-4 完整管线：7 个生命周期钩子 + 全量提取铁律 + 三层矛盾分析 + P-F-C 三段论 + VIE 三层校验 + 受众四类适配 + 通用补充门禁 G1-G6 |
| **Agent 集群** | `docs/agent-cluster.md` | 7 个法律专用 Agent 定义 + 能力矩阵 + 依赖与并行规则 |
| **领域规则（民商事+刑事）** | `docs/domain-rules-overview.md` | 民商事（合同/侵权/公司/婚姻家事继承+请求权基础六层检查） + 刑事（定罪三阶层+量刑四步法+非法证据排除+四组辩护策略）+ 领域补充门禁 Y1-Y5 / Z1-Z6 |
| **Skill 路由矩阵** | `skills-index/skill-routing-matrix.md` | 80+ Skill 按门禁阶段+案件类型动态路由 + Gate 4 决策树 + 法律知识库速查 |
| **记忆与状态管理** | `docs/memory-state-protocol.md` | 四层记忆体系 + 断点续跑协议 + 原文存储纪律 + 时效性标注 + 自学习闭环 |
| **集成指南** | `docs/integration-guide.md` | 四阶段渐进式集成建议 + 文件系统映射 + 兼容性声明 |

## 与 CaseBoard 现有架构的契合

| CaseBoard 现有能力 | 本 PR 增强方向 |
|---|---|
| 11 档智能状态机 | 扩展为四道门禁生命周期钩子系统（7 个钩子 + 阶段转换检查） |
| LLM 全局抽取 | 对齐为 Gate 1 全量提取铁律 6 条 + 结构化提取 Schema |
| 案件分析报告 | 升级为 Gate 3 P-F-C 三段论链 + 四视角对立审查 + S×L 风险矩阵 |
| 元典查被执行人 | 纳入 Gate 2 外部主体核验统一协议 + 信用画像四级标注 |
| 保全续封/上诉期提醒 | 对齐 Gate 3 期限不可逆性检测 + docket-watcher Agent |
| 法律工具（5 个计算器） | 扩展为 80+ Skill 矩阵，按门禁阶段 + 案件类型动态路由 |
| 非诉模块 | 纳入轨道三尽调六阶段方法论 |

## 兼容性

- **CaseBoard 现有功能零影响**：所有新增文件均为独立文档，不修改现有源码
- **渐进式启用**：四个 Phase 可独立启用，不强制全量迁移
- **数据源不重复**：工作流文件独立存储，与 CaseBoard 的 SQLite 数据库并存

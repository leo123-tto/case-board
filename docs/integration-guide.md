# 与 CaseBoard 现有架构的集成建议

## CaseBoard 现有能力 & 本 PR 增强方向

| CaseBoard 现有能力 | 本 PR 增强方向 |
|---|---|
| 11 档智能状态机（接案→立案→仲裁→开庭→审理→调解→上诉→二审→再审→执行→结案） | 扩展为四道门禁生命周期钩子系统（7 个钩子 + 阶段转换检查），每个案件状态关联到具体门禁进度 |
| LLM 全局抽取（结构化 JSON 填表 + 案件分析报告 MD） | 对齐为 Gate 1 全量提取铁律 6 条 + 结构化提取 Schema（维度/内容摘要/来源/原文≥20字/备注） |
| 案件分析报告（MD → HTML / Word） | 升级为 Gate 3 P-F-C 三段论链 + 四视角对立审查 + S×L 风险矩阵，每个结论可追溯到三段论结构 |
| 元典查被执行人（14 端点 + LLM 风险提示报告） | 纳入 Gate 2「外部主体核验」统一协议 + 信用画像四级标注（正常/关注/⚠️风险/待核验） |
| 保全续封 / 上诉期到期提醒 | 对齐 Gate 3「期限不可逆性检测」+ docket-watcher Agent 自动扫描 |
| 法律工具（5 个计算器） | 扩展为 80+ Skill 矩阵，按门禁阶段 + 案件类型动态路由 |
| 非诉模块（7 类业务字段框架 + 7 问访谈引导） | 纳入轨道三尽调六阶段方法论（项目立项→指引加载→底稿摄入→事实查明→条线推进→交付输出） |

## 分阶段集成建议

### Phase 1：门禁管线可视化（最小可行集成）

将现有的 11 档状态机扩展为四道门禁时间轴视图：

```text
案件详情页新增「门禁进度」卡片：
┌────────────────────────────────────────────────┐
│ 门禁进度                                      │
│                                               │
│ Gate 1 提取  ████████████  ✅ 完成  12条事实   │
│ Gate 2 核验  ████████░░░░  🔄 进行中 3个矛盾   │
│ Gate 3 推理  ░░░░░░░░░░░░  ⏸️ 等待上游         │
│ Gate 4 交付  ░░░░░░░░░░░░  ⏸️ 等待上游         │
│                                               │
│ 钩子状态: after:gate1 ✅ | before:gate2 ✅     │
│ 回调次数: 0 | 遗留: 2项待核验                  │
└────────────────────────────────────────────────┘
```

**数据来源**：`cases/<案号>/STATUS.md` 中的钩子通过记录 + 审计日志

### Phase 2：Agent 集群调度面板

在现有「AI 助手」面板旁新增「Agent 集群」视图：

```text
┌─────────────────────────────────────────┐
│ Agent 集群                              │
│                                         │
│ extraction-agent    ✅ 完成  12:30      │
│ verification-agent  🔄 运行中 12:32     │
│ reasoning-agent     ⏸️ 排队             │
│ opposing-counsel    ⏸️ 排队             │
│ delivery-agent      ⏸️ 排队             │
│                                         │
│ 依赖链: extraction → verification       │
│ 并行: opposing-counsel | delivery       │
└─────────────────────────────────────────┘
```

### Phase 3：Skill 触发建议集成

在案件详情页根据当前门禁阶段 + 案件类型，主动弹出 Skill 调用建议：

```text
┌─────────────────────────────────────────┐
│ 💡 建议调用                              │
│                                         │
│ 🟡 强烈建议: evidence-list-writer       │
│    证据清单可生成法院偏好的非表格版本    │
│    [立即调用] [忽略]                     │
│                                         │
│ 🟢 可选: company-dispute-analysis       │
│    本案涉及公司纠纷，可核对公司法引用    │
│    [立即调用] [忽略]                     │
└─────────────────────────────────────────┘
```

**建议生成逻辑**：Gate 4 决策树遍历（见 `skills-index/skill-routing-matrix.md` §决策树）

### Phase 4：记忆注入 + 断点续跑

CaseBoard 启动时自动执行：

1. `docket-watcher` 扫描全部在办案件期限 → 更新首页「重要日期」widget
2. 读取上次会话 STATUS.md + 审计日志 → 首页显示「上次进度」卡片
3. 语义匹配相关历史经验 → 以「记忆注入块」形式显示在 AI 助手上下文

## 文件系统映射

CaseBoard 现有 `~/Library/Application Support/CaseBoard/` 与工作流文件的映射：

```
CaseBoard 数据目录（现有）
├── cases.db（SQLite）          ←── cases/<案号>/state.json（门禁状态快照）
├── case_<id>/                  ←── cases/<案号>/outputs/（四道门禁产物）
│   ├── materials/              ←── 原始材料（Gate 1 输入）
│   └── external/               ←── 元典 MCP 原始 JSON（Gate 2 输入）
└── settings.json               ←── .claude/settings.json（Agent 配置）

工作流规则目录（新增引用）
~/.claude/
├── CLAUDE.md                   ←── 全局指令 + 四轨分流路由
├── rules/                      ←── 门禁规则 + 领域规则
│   ├── 10_extraction.md        ←── Gate 1：全量提取铁律6条
│   ├── 20_verification.md      ←── Gate 2：三层矛盾+三类核验
│   ├── 30_reasoning.md         ←── Gate 3：P-F-C链+四视角+S×L矩阵
│   ├── 40_delivery.md          ←── Gate 4：VIE三层校验+十一项一致性+受众四类适配
│   ├── 51_civil_domain.md      ←── 民商事领域规则
│   ├── 52_criminal_domain.md   ←── 刑事领域规则
│   ├── 70_due_diligence.md     ←── 尽调六阶段
│   ├── 80_pricing_proposal.md  ←── 报价八阶段
│   └── CHANGELOG.md            ←── 规则变更日志
├── gates/                      ←── 交付门禁定义
├── checklists/                 ←── 自检清单（Logic Doctor/十一项一致性/律师确认等）
├── agents/                     ←── 7个Agent定义文件
├── references/                 ←── 440+参考文件（速查/类案/法条/模板等）
└── skills/                     ←── Skill 定义与存档
```

## 兼容性声明

- **CaseBoard 现有功能零影响**：所有新增文件均为独立目录，不修改 CaseBoard 现有源码
- **数据源不重复**：工作流规则引用 `cases/` 目录结构，与 CaseBoard 的 SQLite 数据库并存
- **渐进式启用**：每个 Phase 可独立启用，不强制全量迁移
- **门禁状态**：通过 `cases/<案号>/STATUS.md` 和审计日志暴露，CaseBoard 仅需读取 Markdown + JSON

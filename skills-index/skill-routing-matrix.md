# Skill 矩阵 · 按门禁阶段与案件类型的动态路由

## 技能分层

| 层级 | 含义 | 行为 |
|---|---|---|
| 🔴 强制调用 | 规则已内置，无需另行建议 | 自动调用 |
| 🟡 强烈建议 | 技能与交付物类型高度匹配，内部逻辑更专更深 | 主动识别，列出差异点，建议调用 |
| 🟢 可选增强 | 技能可补充领域知识或格式，但非必需 | 主动识别，简要说明价值 |

## 按门禁阶段的 Skill 路由

### Gate 1·提取阶段

| Skill | 触发场景 | 说明 |
|---|---|---|
| `legal-ocr` / `mineru-ocr` | PDF/图片 OCR 文字识别 | 扫描件→可提取文本 |
| `wechat-article-fetch` | 微信公众号文章抓取 | Playwright headless→Markdown |
| `legal-text-format` | 法律文本格式化 | 粘贴/抓取→规范Markdown，删除推广冗余 |
| `pdf` / `pdf-processor` / `pdf-organizer` | PDF 材料预处理 | 合并/拆分/解密/整理/扫描件预处理 |
| `img2pdf` | 截图/照片→标准化PDF | 证据整理场景 |
| `chronology-builder` | 时间线提取 | 从材料提取时间节点，构建法律大事记 |
| `funasr-transcribe` | 音视频转录 | 庭审录音/会议记录→文字 |

### Gate 2·核验阶段

| Skill | 触发场景 | 说明 |
|---|---|---|
| `contract-review` | 深度合同审查/条款效力分析 | 四层审查框架+结构化批注+风险分级。日常合同审查用 `contract-copilot` |
| `evidence-evaluation`（概念） | 证据三性+证明力 | 真实性/合法性/关联性审查 |
| `evidence-argument-chain`（概念） | 主张→要件→证据→证明力| 证据链缺口检测 |

### Gate 3·推理阶段

| Skill | 触发场景 | 说明 |
|---|---|---|
| `litigation-support-advisor` | 诉讼支持顾问 | Gate 3 主引擎+快速通道入口 |
| `litigation-analysis` | 判决书/庭审笔录深度分析 | 争议焦点识别+审理链条重建+上诉/再审决策 |
| `legal-case-analysis` | 案件综合分析与策略研判 | 风险评估+策略选项生成 |
| `anspruchsgrundlage-method` | 请求权基础六层检查 | 王泽鉴请求权基础分析法，民商事案件契约→侵权逐层检索 |
| `legal-visualization` | 法律关系图/流程图/时间轴图 | draw.io + PNG/SVG 导出 |
| `claim-chart` | 要件分析表/证据映射 | 要件→证据逐格对照+缺口清单 |
| `case-retrieval-report` | 类案检索报告 | 元典 MCP 检索+裁判文书摘要 |
| `company-dispute-analysis` | 公司法全体系知识库 | 新公司法266条+五部司法解释+法答网10个答疑 |
| `marriage-law-advisor` | 婚姻家庭法律咨询 | 民法典婚姻家庭编+离婚起诉状+Logic Doctor |

### Gate 4·交付阶段

| Skill | 触发场景 | 类型 | 
|---|---|---|
| `evidence-list-writer` | 证据清单/证据目录 | 🟡 强烈建议：支持七种诉讼地位+双模式证明对象+格式化.docx |
| `civil-appeal-writer` | 民事上诉状 | 🟡 强烈建议：按一审错误类型组织+改判/发回路径匹配 |
| `plaintiff-complaint-writer` | 民事起诉状 | 🟡 强烈建议：起诉状+证据清单联动+MCP核验规范性 |
| `cross-examination-writer` | 质证意见+对方证据≥5份 | 🟡 强烈建议：民事/行政+格式化.docx批量生成 |
| `officecli-docx` / `md2word` | .docx 格式输出 | 🔴 强制调用：Markdown→Word转换+排版校验 |
| `de-ai-polish` | C/D 类对外文件去 AI 化 | 🔴 强制：7类污染类型学+17类禁用模式 |
| `legal-proposal-generator` | 非法院文书交付物 | 🟢 可选：诉讼方案/咨询报告/服务建议书 |
| `legal-redline` | 合同红线对比 | 🟢 可选：带修订痕迹.docx+红线PDF |

## 独立轨道 Skill（不经过四道门禁）

| Skill | 轨道 | 说明 |
|---|---|---|
| `demand-draft` | 独立（律师函） | 七道门禁：保密过滤/自认风险/权利保留/和解谈判/保密弃权/语气姿态/事实准确性 |
| `contract-copilot` | 独立（合同起草+日常审查） | 分层分析与四步流程，输出可执行风险清单+起草骨架+修改建议 |
| `browser-skill` | 独立（浏览器自动化） | 登录态浏览器操作：页面快照/数据抓取/流程自动化 |
| `westock-data` | 独立（金融数据） | 上市公司财报/研报/公告/风险事件查询 |

## Skill 触发决策树（Gate 4 强制执行）

```text
Gate 4 交付物组装完成
  │
  ├─ 含可视化元素？ ──── 是 → 🔴 强制调用 legal-visualization
  ├─ 要件分析表/证据映射？ ─ 是 → 🔴 强制调用 claim-chart
  ├─ 大事记/时间线？ ──── 是 → 🔴 强制调用 chronology-builder
  ├─ .docx 格式？ ───── 是 → 🔴 强制调用 officecli-docx / md2word
  │
  ├─ 证据清单/证据目录？ ─ 是 → 🟡 强烈建议 evidence-list-writer
  ├─ [civil] 上诉状？ ──── 是 → 🟡 强烈建议 civil-appeal-writer
  ├─ [civil] 起诉状？ ──── 是 → 🟡 强烈建议 plaintiff-complaint-writer
  ├─ 质证意见+对方证据≥5份？ 是 → 🟡 强烈建议 cross-examination-writer
  │
  ├─ [civil]+公司纠纷？ ─ 是 → 🟢 可选 company-dispute-analysis
  ├─ [civil]+婚姻家事？ ─ 是 → 🟢 可选 marriage-law-advisor
  ├─ [civil]+Gate 3 未调请求权基础？ 是 → 🟢 可选 anspruchsgrundlage-method
  ├─ 非法院文书交付物？ ─ 是 → 🟢 可选 legal-proposal-generator
  └─ 其余情形 → 本体系模板自足，不另建议
```

## 法律知识库与速查

| 文件 | 适用场景 | 说明 |
|---|---|---|
| `references/civil-procedure-core.md` | Gate 1-4 各阶段 | 管辖/当事人/时效/审级/保全/送达/执行速查（~300行） |
| `references/evidence-rules-core.md` | Gate 2/4 | 举证责任/八种证据/三性审查/自认/电子数据/证明标准（~300行） |
| `references/enforcement-core.md` | Gate 4 执行相关 | 执行依据/财产调查/执行措施/失信限消/执行异议/迟延利息（~250行） |
| `references/construction-law/` | [civil]+建设工程纠纷 | 建工解释二（法释〔2026〕12号）全文+典型案例 |
| `references/company-law/` | [civil]+公司纠纷 | 新公司法全文+五部司法解释+入库案例 |
| `references/trial-defense-panrui/` | [criminal]+一审阶段 | 一审辩护实务操作流程+质证方法论 |
| `references/fadawang/`（38批） | [all] | 法答网精选答问 1-38 批 |

## 辅助支撑工具

| 工具 | 用途 |
|---|---|
| `mcp__yuandian-mcp-server__*` | 案例/法规/法条/企业信息 语义+关键词检索 |
| `mcp__wigolo__*` | 网页搜索/抓取/缓存/研究/结构化提取 |
| `mcp__officecli__officecli` | Office 文档创建/编辑/格式化/校验 |

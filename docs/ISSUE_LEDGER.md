# BUG / Issue 跟踪表

> 维护:zzf516988659-del
> 用途:跟踪发现 / 修复 / 上报的 BUG,避免遗漏
> 关联:每个 BUG 可能对应一个 PR(在 `PR_LEDGER.md` 里)

---

## 状态图例

| 状态 | 含义 |
|---|---|
| 🔴 new | 刚发现,待评估 |
| 🟠 triaged | 评估过,待修 |
| 🟡 in-progress | 修中 |
| 🔵 pr-submitted | PR 已提,等合并 |
| ✅ fixed | 合并 / 已修 |
| ⚪ wontfix | 暂不修(架构 / 优先级 / 不在范围) |
| 🟣 upstream-notified | 提交给 upstream 处理 |

---

## 优先级

| 级别 | 含义 | 响应时间 |
|---|---|---|
| P0 | 阻塞主流程 / 数据丢失 / 安全 | 24h |
| P1 | 主要功能不可用 | 3 天 |
| P2 | 体验问题 / 边界 case | 1 周 |
| P3 | 优化 / 改进 | 1 月+ |

---

## Open(待修 / 修中)

| ID | 级别 | 模块 | 现象 | 发现日 | 修复 PR | 状态 |
|---|---|---|---|---|---|---|
| — | — | — | (待登记) | — | — | — |

---

## Fixed(已修)

| ID | 级别 | 模块 | 现象 | 发现日 | 修复 PR | 合并日 | 备注 |
|---|---|---|---|---|---|---|---|
| BUG-001 | P1 | chat | VisualizeCase 阶段二 LLM 伪造用户已选择授权,导致工作台不写入却给 4 张视图建议 | 2026-07-13 | #35 (e2866ca) | 待合并 | Bug 1 of 3,连环 fix |
| BUG-002 | P1 | chat | VisualizeCase 阶段二正文复述 CaseGraph 结构,MiniMax-M3 token 爆炸,finish_reason=length | 2026-07-14 | #35 (f224f9c) | 待合并 | Bug 2 of 3 |
| BUG-003 | P1 | chat | 思考模型在 VisualizeCase 阶段二长 tool call 阶段被 reqwest.read_timeout=180s 误判踢掉,报"LLM 不可达" | 2026-07-14 | #35 (c1591df) | 待合并 | Bug 3 of 3 |

---

## Notified Upstream(已报给维护者)

| ID | 级别 | 模块 | 现象 | 上报日 | Upstream issue | 状态 |
|---|---|---|---|---|---|---|
| — | — | — | (待登记) | — | — | — |

---

## Won't Fix

| ID | 现象 | 原因 | 决策日 |
|---|---|---|---|
| — | — | — | — |

---

## 操作 SOP

**发现 BUG 后**:
1. 复现 + 记录(版本 / 步骤 / 现象 / 预期)
2. 判断等级(P0-P3)
3. 在本文档"Open"加一行
4. 决定:本地修 / 报给 upstream
5. 本地修:走 [`CONTRIBUTING_FLOW.md` § A](./CONTRIBUTING_FLOW.md)
6. 报给 upstream:用 `python scripts/contrib/bug-report-template.py > BUG.md` 生成模板,在 GitHub issues 提

**BUG 修复后**:
1. 跑 `python scripts/contrib/check-pr-status.py <PR号>` 确认合并
2. 从"Open"挪到"Fixed",填合并日
3. 关联 PR(在 `PR_LEDGER.md` 同步)

**BUG 报告给 upstream 后**:
1. 挪到"Notified Upstream"
2. 记录 upstream issue 号
3. 跟进维护者反馈

---

## 模板(发现 BUG 时)

```markdown
**现象**: [具体看到了什么]
**预期**: [应该看到什么]
**复现步骤**:
1. 打开 App 版本 v0.X.Y
2. 走到 ... 步骤
3. 触发 ... 操作
4. 看到 ... 现象

**环境**:
- 版本: v0.X.Y
- OS: macOS 14.5 / Windows 11
- 模型: MiniMax-M3 / DeepSeek v4-pro
- 复现频率: 100% / 偶发(概率约 X%)

**日志**:
[粘贴关键日志,脱敏后]

**截图/录像**:
[不附图,用文字描述]
```

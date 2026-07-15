# AGENTS.md — CaseBoard 贡献工作流

> AI 协作者(Claude Code / Codex / Mavis 等)与人类协作者共用的项目级指南。
> 接手 CaseBoard 贡献任务前,**先读这份文档再动手**。

---

## 1. 项目身份

| 字段 | 值 |
|---|---|
| 项目名 | 案件看板 · CaseBoard |
| 上游仓库 | `leo123-tto/case-board`(江苏漫修律师事务所 · 刘成律师) |
| 你的 fork | `zzf516988659-del/case-board` |
| 当前版本 | v0.4.9(迭代极快,基本 1-2 天一个版本) |
| 技术栈 | Tauri 2 + React 19 + Rust + SQLite |
| License | PolyForm Noncommercial 1.0.0(非商业免费,商业须授权) |

## 2. 角色与职责

| 角色 | 职责 | 工具入口 |
|---|---|---|
| **Maintainer(刘成律师)** | upstream 决策、合并 PR、发版 | GitHub Issues/PR |
| **Contributor(你 + Coder)** | BUG 修正、新需求开发、提 PR | `scripts/contrib/` |
| **Reviewer** | upstream 自审 / 其它贡献者 | GitHub PR |

**关键原则**:Contributor **永远不直接 push upstream**,必须走 fork → PR 流程。

## 3. 工作流入口(按场景)

| 我要… | 走这个流程 | 文档入口 |
|---|---|---|
| 修一个 BUG | BUG 修正流程 | [`docs/CONTRIBUTING_FLOW.md` § A](./docs/CONTRIBUTING_FLOW.md) |
| 开发一个需求 | 新需求开发流程 | [`docs/CONTRIBUTING_FLOW.md` § B](./docs/CONTRIBUTING_FLOW.md) |
| 提交一个 PR | PR 提交流程 | [`docs/CONTRIBUTING_FLOW.md` § C](./docs/CONTRIBUTING_FLOW.md) |
| 同步 upstream | 同步流程 | [`docs/UPSTREAM_SYNC.md`](./docs/UPSTREAM_SYNC.md) |
| 查询 PR 状态 | `scripts/contrib/check-pr-status.py` | — |

## 4. 目录地图

```
case-board/
├── AGENTS.md                         ← 你正在看
├── CONTRIBUTING.md                   ← upstream 官方贡献指南(必读)
├── README.md                         ← upstream 官方 README
├── docs/                             ← 贡献工作流文档(本文档库)
│   ├── CONTRIBUTING_FLOW.md              ★ 核心流程
│   ├── BRANCH_STRATEGY.md                ★ 分支策略
│   ├── COMMIT_CONVENTIONS.md             ★ commit 规范
│   ├── PRIVACY_IRON_RULE.md              ★ 隐私铁律
│   ├── UPSTREAM_SYNC.md                  ★ upstream 同步
│   ├── PR_LEDGER.md                      ★ PR 跟踪表
│   ├── ISSUE_LEDGER.md                   ★ BUG/Issue 跟踪表
│   └── FEATURE_BACKLOG.md                ★ 需求 backlog
├── scripts/
│   ├── release.sh                        (已有)upstream 打包脚本
│   └── contrib/                       ← 贡献工具集
│       ├── get-gcm-token.py              (Token 读取)
│       ├── create-pr.py                  (PR 创建)
│       ├── list-my-prs.py                (列出我的所有 PR)
│       ├── check-pr-status.py            (查询 PR 状态)
│       ├── sync-upstream.sh              (同步 upstream)
│       ├── bug-report-template.py        (BUG 报告模板)
│       └── feature-request-template.py   (需求模板)
└── (upstream 代码) src/ src-tauri/ ...
```

## 5. Coder 接任务时的速查清单

收到贡献任务后,先做这 5 件事,再开始写代码:

- [ ] **读** `docs/CONTRIBUTING_FLOW.md` 对应章节(选 A / B / C)
- [ ] **读** `docs/PRIVACY_IRON_RULE.md` 防误提交真实数据
- [ ] **拉最新** `upstream/main` 后建分支(`docs/BRANCH_STRATEGY.md`)
- [ ] **写代码** 时对照 `docs/COMMIT_CONVENTIONS.md` 规范
- [ ] **提 PR 前** 跑 `cargo fmt && cargo clippy && pnpm build` + 填 PR body

**绝对不做的事**:
- ❌ 把真实当事人/案件/案号/身份证 commit 进任何分支(包括本地)
- ❌ 直接 push `main`(必须走 PR 流程)
- ❌ 跳过 `pnpm tauri dev` 端到端验证就交 PR
- ❌ 改数据库 schema / 公共 API 不先开 issue 讨论
- ❌ 一次性堆 5 个不相关 commit 进同一个 PR

## 6. 隐私铁律(简版 · 完整版见 `docs/PRIVACY_IRON_RULE.md`)

**红线一条**:**永远不要 commit 真实当事人数据**。

- ❌ 案件名、当事人姓名、案号、身份证号、电话、地址
- ❌ 聊天记录截图、案件文书截图(哪怕脱敏也不放 PR)
- ❌ 测试 fixture 里塞真实或虚构案件数据
- ❌ LLM prompt 里出现真实案件片段
- ✅ 可以: 抽象成 `case-A` / `case-B` / 测试用纯占位符
- ✅ 可以: 描述 UI 行为但不附图

PR Reviewer 见真实数据必须**直接拒绝 PR**,不商量。

## 7. 状态更新约定

- **每个 PR 完成后** → 更新 `docs/PR_LEDGER.md` 一行
- **每个 BUG 修复后** → 更新 `docs/ISSUE_LEDGER.md` 状态
- **每个新需求** → 写入 `docs/FEATURE_BACKLOG.md`
- **每周一次** → 检查 `docs/PR_LEDGER.md` 中 open PR,跟进 reviewer

## 8. 紧急联系

- upstream 维护者:刘成律师(README 有微信二维码)
- 贡献者:你 + Coder 协作(本机: Mavis 调度)
- 安全漏洞:走 [SECURITY.md](./SECURITY.md),**不要公开发 issue**

---

**最后更新**:2026-07-15 · v1
**维护**:zzf516988659-del(对刘成律师 upstream 贡献)

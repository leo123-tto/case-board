# 贡献完整流程

> CaseBoard 贡献三件事:BUG 修正、新需求开发、提 PR。
> 本文档把三件事拆成可复用的流程,每步都有命令和检查点。

---

## A. BUG 修正流程

适用:在 App 使用 / 阅读源码 / 跑测试中发现 BUG。

### A1. 登记 BUG

打开 [`docs/ISSUE_LEDGER.md`](./ISSUE_LEDGER.md),按表格填一行:

```markdown
| BUG-001 | v0.4.9 | 案情可视化阶段二 LLM 伪造用户选择 | 复现:... | 待修 | 2026-07-15 |
```

**判断要不要立刻修 vs 提 issue 给维护者**:
- **本地修** — BUG 范围清晰 / 你有把握改 / 改动 ≤ 200 行
- **提 issue** — 涉及架构调整 / 不确定怎么改 / 改动 > 500 行

### A2. 拉最新 main

```bash
cd case-board
git fetch upstream
git checkout main
git merge upstream/main   # 同步到 upstream 最新
git push origin main     # 同步到自己的 fork(可选,看工作流)
```

### A3. 建分支

参考 [`docs/BRANCH_STRATEGY.md`](./BRANCH_STRATEGY.md),命名:`pr/fix/<scope>-<short-desc>`

```bash
git checkout -b pr/fix/visualize-fake-user-confirm
```

### A4. 复现 BUG(必须有)

写一个最小复现 case:
- 哪一步操作
- 看到什么
- 预期看到什么
- 日志 / 截图(无真实数据)

把这个写进 commit message,或者单独写进 `docs/ISSUE_LEDGER.md` 详情区。

### A5. 修代码

- 改完跑 `cargo fmt` / `cargo clippy --all-targets -- -D warnings`
- 前端改完跑 `pnpm exec tsc --noEmit && pnpm build`
- **必须** `pnpm tauri dev` 端到端验证一遍

### A6. 加测试

- BUG 修法如果能加 unit test,务必加(防回归)
- 测试命名:`bug_<area>_<short>`

### A7. 提交(看 COMMIT_CONVENTIONS.md)

```bash
git add <files>
git commit -m "fix(chat): 阶段二禁止 LLM 在正文复述 CaseGraph 结构"
```

### A8. Push + 建 PR

```bash
git push origin pr/fix/visualize-fake-user-confirm
python scripts/contrib/create-pr.py
```

PR body 用模板 `scripts/contrib/create-pr.py` 默认加载的文件,
或自己用 `scripts/contrib/bug-report-template.py` 生成。

### A9. 登记 PR

PR URL 拿到后,更新 [`docs/PR_LEDGER.md`](./PR_LEDGER.md) 一行。

### A10. 等 review + 跟进

- 上游 reviewer 可能提修改建议
- 改完用 `git commit --fixup` 或新 commit 追加
- 合并后更新 `docs/ISSUE_LEDGER.md` 状态 → 已修

---

## B. 新需求开发流程

适用:想给 CaseBoard 加个新功能(类似 PR #27 PR #28 那种)。

### B1. 先开 issue 讨论

**大改动必须先开 issue**,维护者(刘成律师)同意再写代码。
CONTRIBUTING.md 原话:
> **大改动请先开 issue 讨论再写代码**,避免做完合不进来。

### B2. 写设计稿(可选,看复杂度)

简单需求(≤ 200 行改动):直接在 PR body 里写设计。
复杂需求(> 500 行 / 涉及 schema / 跨模块):单独写 `docs/design/<feature>.md`。

设计稿要回答:
- 用户故事:谁用、解决什么问题
- API / 数据 schema 变化(如果有)
- 风险点 / 替代方案
- 测试策略

### B3. 登记到 backlog

[`docs/FEATURE_BACKLOG.md`](./FEATURE_BACKLOG.md) 加一行:

```markdown
| FEAT-005 | 高 | Windows 全局快捷键支持律师切换案件 | 设计中 | TBD | 2026-07-15 |
```

### B4-B10. 同 A2-A10

把"修 BUG"换成"加功能",流程一样。
唯一区别:commit type 用 `feat`,不是 `fix`。

---

## C. PR 提交流程(独立章节,适合单独复用)

### C1. 检查清单(发 PR 前)

```bash
# 1. 代码规范
cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings

# 2. 前端类型 + 构建
cd ..
pnpm exec tsc --noEmit
pnpm build

# 3. 端到端验证
pnpm tauri dev
# → 跑一遍你改的功能,确认没回归
```

### C2. PR body 模板

参照上游 `.github/PULL_REQUEST_TEMPLATE.md`,用 `scripts/contrib/create-pr.py` 自动加载。
至少包含:
- **背景**:为什么做
- **改动**:列文件 + 改动点
- **测试**:本地跑过的命令和结果
- **检查清单**:Conventional Commits / 无真实数据 / 不动 schema

### C3. 创建 PR

```bash
python scripts/contrib/get-gcm-token.py | python scripts/contrib/create-pr.py
```

或:
```bash
export GITHUB_TOKEN=$(python scripts/contrib/get-gcm-token.py)
python scripts/contrib/create-pr.py
unset GITHUB_TOKEN
```

### C4. 创建后

- 更新 [`docs/PR_LEDGER.md`](./PR_LEDGER.md) 加一行(URL / 状态)
- 等上游 reviewer 反馈
- 评论 / 改动 / 再 push

### C5. 合并后

- 同步 upstream(见 [`docs/UPSTREAM_SYNC.md`](./UPSTREAM_SYNC.md))
- 更新 `docs/ISSUE_LEDGER.md` 或 `docs/FEATURE_BACKLOG.md` 状态
- 关掉对应的本地分支:`git branch -d pr/fix/xxx`

---

## 工具脚本一览

| 工具 | 用途 | 用法 |
|---|---|---|
| `get-gcm-token.py` | 从 Windows Credential Manager 读 GitHub Token | `python get-gcm-token.py` |
| `create-pr.py` | 通过 GitHub API 创建 PR | `python create-pr.py` |
| `list-my-prs.py` | 列出我在 upstream 的所有 PR | `python list-my-prs.py` |
| `check-pr-status.py` | 查询指定 PR 的 CI/mergeable/评论 | `python check-pr-status.py 35` |
| `sync-upstream.sh` | 同步 upstream/main 到本地 | `bash sync-upstream.sh` |
| `bug-report-template.py` | 生成 BUG 报告模板 | `python bug-report-template.py > BUG.md` |
| `feature-request-template.py` | 生成需求模板 | `python feature-request-template.py > FEAT.md` |

---

## 复盘节奏(建议)

- **每天开工前**:`python check-pr-status.py <PR号>` 跟一下 open PR
- **每周一**:`python list-my-prs.py` 看所有 PR 总览,更新 PR_LEDGER
- **每月一次**:清点 FEATURE_BACKLOG,重排优先级

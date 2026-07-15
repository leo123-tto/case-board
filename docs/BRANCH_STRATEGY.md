# 分支策略

> CaseBoard 贡献分支管理。
> 上游约定:`main` 稳定 + `feat/xxx` / `fix/xxx` / `docs/xxx`。
> 本仓库 fork 后增加 `pr/` 前缀作为 PR 工作分支。

---

## 1. 分支类型

| 类型 | 命名格式 | 用途 | 存在周期 |
|---|---|---|---|
| `main` | `main` | 本地主线 | 长期 |
| `pr/feat/xxx` | `pr/feat/<scope>-<desc>` | 新需求 PR | 到合并后删 |
| `pr/fix/xxx` | `pr/fix/<scope>-<desc>` | BUG 修复 PR | 到合并后删 |
| `pr/docs/xxx` | `pr/docs/<scope>-<desc>` | 文档 PR | 到合并后删 |
| `pr/chore/xxx` | `pr/chore/<scope>-<desc>` | 杂项 PR | 到合并后删 |

> 注意:`<scope>` 跟 commit message 的 scope 保持一致。
> 比如 `chat` `home-companion` `db` `import` `wechat-qr` 等。

## 2. 创建分支

**总是基于最新的 main**:

```bash
# 1. 同步 upstream
git fetch upstream
git checkout main
git merge upstream/main

# 2. 切到新分支
git checkout -b pr/fix/visualize-fake-user-confirm
```

**命名规则**:
- 全小写
- 用 `-` 分隔单词
- `<desc>` 要简短有信息量(3-5 个单词)
- 例子:
  - ✅ `pr/fix/visualize-fake-user-confirm`
  - ✅ `pr/feat/windows-global-shortcut`
  - ✅ `pr/docs/agent-guide`
  - ❌ `pr/fix/fix-bug`
  - ❌ `pr/fix/very-long-descriptive-name-with-too-many-words`

## 3. 推送策略

```bash
git push origin pr/fix/xxx
```

- 只推自己的 fork(origin)
- **永远不直接 push upstream**
- 不要一次性 force push 改历史(会污染 PR 的 review thread)

## 4. 同步 upstream 到 PR 分支

当 `upstream/main` 有新 commit 而你的 PR 分支落后时:

```bash
# 在 PR 分支上
git fetch upstream
git rebase upstream/main

# 如果 rebase 冲突:
# 1. 解决冲突
# 2. git add <files>
# 3. git rebase --continue
# 4. git push --force-with-lease
```

**优先 rebase 而非 merge**(保持 PR commit 历史干净)。

## 5. 删除已合并分支

PR 合并后:

```bash
# 本地删
git branch -d pr/fix/visualize-fake-user-confirm

# 远端删
git push origin --delete pr/fix/visualize-fake-user-confirm
```

合并后保持 `pr/fix/*` 分支不超过 10 个,免得混乱。

## 6. 特殊情况

### 6.1 一个 PR 含多个 fix

可以!把它们做成多个 commit(每个 commit 一个 fix),不要合并。
PR 描述里要写清楚 commit 之间的依赖关系。
参考 PR #35(三个连环 bug,三个 commit)。

### 6.2 一个 fix 需要多分支试错

```bash
# 命名加 -v2 / -v3 后缀
pr/fix/foo-v1
pr/fix/foo-v2
pr/fix/foo-final  # 真正要推的
```

或者:
```bash
# 用 git worktree 隔离
git worktree add ../case-board-v2 pr/fix/foo
```

### 6.3 紧急 hotfix

跳过 issue 讨论直接修,但要在 PR body 里写明"hotfix,理由:xxx"。

### 6.4 撤销未提交的修改

```bash
# 撤销工作区修改
git checkout -- <file>

# 撤销已 add
git restore --staged <file>

# 撤销最近一次 commit(保留修改)
git reset --soft HEAD~1
```

## 7. 跟 main 的关系

```
upstream/main (稳定)
     ↓ fork
origin/main  (你 fork 的 main,跟随 upstream)
     ↓ checkout
pr/fix/xxx   (PR 分支)
     ↓ PR
upstream/main (合并)
```

> ⚠️ **不要把工具脚本 / 文档 / 配置 commit 到 PR 分支**。这些属于 fork 自己的 main,跟 PR 内容无关。
> PR 分支**只包含**与该 PR 相关的代码改动。

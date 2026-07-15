# 与 Upstream 同步策略

> CaseBoard 上游 `leo123-tto/case-board` 每天 1-2 个版本,迭代极快。
> Fork 必须定期同步,避免 PR 落后太多无法合并。

---

## 1. 同步频率

| 场景 | 频率 | 怎么同步 |
|---|---|---|
| 日常 | 每天开工前 | `bash scripts/contrib/sync-upstream.sh` |
| 大版本前 | upstream 发 v0.X.0 前 | 完整 rebase PR 分支 |
| 长时间不开 | 1 周以上 | 先 rebase 一次再继续 |
| PR 反馈冲突 | reviewer 提到时 | 立刻 rebase |

## 2. 同步脚本

```bash
# scripts/contrib/sync-upstream.sh
#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

# 拉 upstream 最新
git fetch upstream

# 当前在 main 的话,直接 fast-forward
if [ "$(git branch --show-current)" = "main" ]; then
    echo "[*] main 分支:fast-forward 到 upstream/main"
    git merge --ff-only upstream/main
    exit 0
fi

# 当前在 PR 分支的话,rebase 到 upstream/main
CURRENT=$(git branch --show-current)
if [[ "$CURRENT" == pr/* ]]; then
    echo "[*] PR 分支:rebase 到 upstream/main"
    git rebase upstream/main
    exit 0
fi

echo "[!] 不在 main 也不在 pr/* 分支,退出"
exit 1
```

## 3. 同步时机

### 3.1 开工前

```bash
bash scripts/contrib/sync-upstream.sh
git status
```

### 3.2 提 PR 前

确认 PR 分支的 base 跟 upstream/main 一致或只落后几个 commit:

```bash
# 查看 base 差距
git log --oneline upstream/main..HEAD

# 如果落后太多,rebase
git rebase upstream/main
```

### 3.3 PR 合并后

PR 被 upstream 合并后,本地的 PR 分支就过时了:

```bash
# 1. 同步 main 到 upstream
git checkout main
git merge --ff-only upstream/main

# 2. 删除本地 PR 分支
git branch -d pr/fix/xxx
git push origin --delete pr/fix/xxx
```

## 4. 冲突处理

### 4.1 rebase 冲突

```bash
git rebase upstream/main
# CONFLICT ...
git status  # 看哪些文件冲突
# 1. 打开冲突文件,解决冲突
# 2. git add <files>
# 3. git rebase --continue
# 4. git push --force-with-lease
```

### 4.2 哪些冲突最常见

- `Cargo.lock` 改了 → 重新跑 `cargo build` 让它重新生成
- 同一个文件的同一个函数 → 手动合并,优先保 PR 改动
- `CHANGELOG.md` 改了 → 不在 PR 里改它(upstream 维护者会统一合并)

### 4.3 rebase 还是 merge

- ✅ **PR 分支用 rebase**(保持 commit 历史线性)
- ❌ **main 分支用 merge --ff-only**(从 upstream fast-forward)

## 5. Fork 同步到 GitHub

GitHub 的 fork 默认不会自动从 upstream 同步。需要手动:

```bash
# 在 GitHub 网页:从 upstream/main 同步到 fork/main
# (Repo → Pull requests → New pull request → base: 自己 fork / compare: upstream/main)
# 或者直接:
git push origin main  # 把本地 main 推到 fork
```

## 6. 同步检查清单(每周一跑一次)

```bash
# 1. 看 upstream 状态
git fetch upstream
git log --oneline main..upstream/main | head -20
echo "落后 $(git rev-list --count main..upstream/main) 个 commit"

# 2. 看我的 PR 状态
python scripts/contrib/list-my-prs.py

# 3. 看每个 open PR 的状态
for pr in $(python scripts/contrib/list-my-prs.py --open-only); do
    python scripts/contrib/check-pr-status.py "$pr"
done
```

## 7. 同步失败的应急

如果 upstream 改动太大,导致 PR 几乎需要重写:

1. **关掉当前 PR**
2. **重新基于最新 upstream/main 建分支**
3. **把改动 cherry-pick 过来**
4. **开新 PR,引用旧 PR 号**

**不要** 在原 PR 上用 force push 一次性大改 —— 之前的 review 评论就全丢了。

## 8. 自动化(可选)

可以加 cron 每天自动同步:

```bash
# crontab -e
0 8 * * * cd /path/to/case-board && bash scripts/contrib/sync-upstream.sh >> ~/.caseboard-sync.log 2>&1
```

**注意**:CRON 在 Windows 上要换 PowerShell + Task Scheduler 实现。
本地 WSL 用户可以直接用 crontab。

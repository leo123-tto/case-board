#!/usr/bin/env bash
# sync-upstream.sh — 同步 upstream/main 到本地
#
# 用法:
#   bash sync-upstream.sh          # 自动判断(main fast-forward / PR 分支 rebase)
#
# 前置:
#   - 已配置 git remote: upstream -> leo123-tto/case-board
#   - 当前在 main 或 pr/* 分支

set -euo pipefail

# 切到仓库根目录(脚本在 scripts/contrib/)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

echo "[*] 仓库根: $REPO_ROOT"

# 1. 拉 upstream 最新
if ! git remote get-url upstream >/dev/null 2>&1; then
    echo "[!] 没配置 upstream remote,加一下:"
    echo "    git remote add upstream https://github.com/leo123-tto/case-board.git"
    exit 1
fi

echo "[*] 拉 upstream..."
git fetch upstream

CURRENT_BRANCH="$(git branch --show-current)"

if [ "$CURRENT_BRANCH" = "main" ]; then
    # main 分支:fast-forward
    BEHIND=$(git rev-list --count main..upstream/main)
    if [ "$BEHIND" = "0" ]; then
        echo "[*] main 已是最新,无操作"
        exit 0
    fi
    echo "[*] main 落后 upstream $BEHIND 个 commit,fast-forward"
    git merge --ff-only upstream/main
    echo "[OK] main 同步完成"

elif [[ "$CURRENT_BRANCH" == pr/* ]]; then
    # PR 分支:rebase
    BEHIND=$(git rev-list --count "$CURRENT_BRANCH"..upstream/main)
    if [ "$BEHIND" = "0" ]; then
        echo "[*] $CURRENT_BRANCH 已是最新,无操作"
        exit 0
    fi
    echo "[*] $CURRENT_BRANCH 落后 upstream $BEHIND 个 commit,rebase 中..."
    git rebase upstream/main
    echo "[OK] rebase 完成"
    echo "[*] 接下来: git push --force-with-lease"

else
    echo "[!] 当前分支 '$CURRENT_BRANCH' 既不是 main 也不是 pr/*"
    echo "    请先切换到 main 或 PR 分支"
    exit 1
fi

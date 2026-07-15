#!/usr/bin/env bash
# sync-upstream.sh �?同步 upstream/main 到本�?#
# 用法:
#   bash sync-upstream.sh          # 自动判断(main fast-forward / PR 分支 rebase)
#
# 前置:
#   - 已配�?git remote: upstream -> leo123-tto/case-board
#   - 当前�?main �?pr/* 分支
#
# 行为:
#   - �?main 分支:如果落后 upstream,fast-forward merge
#   - �?pr/* 分支:如果落后 upstream,rebase
#   - 其它分支:报错退�?#
# 强制 UTF-8 locale,避免 PowerShell 终端显示中文乱码
export LANG=C.UTF-8
export LC_ALL=C.UTF-8

set -euo pipefail

# 切到仓库根目�?脚本�?scripts/contrib/)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

echo "[*] repo root: $REPO_ROOT"

# 1. 检�?upstream remote 是否配置
if ! git remote get-url upstream >/dev/null 2>&1; then
    echo "[!] upstream remote not configured. Run:"
    echo "    git remote add upstream https://github.com/leo123-tto/case-board.git"
    exit 1
fi

echo "[*] fetching upstream..."
git fetch upstream

CURRENT_BRANCH="$(git branch --show-current)"

if [ "$CURRENT_BRANCH" = "main" ]; then
    # main 分支:fast-forward
    BEHIND=$(git rev-list --count main..upstream/main)
    if [ "$BEHIND" = "0" ]; then
        echo "[*] main is up-to-date, no action"
        exit 0
    fi
    echo "[*] main is $BEHIND commits behind upstream, fast-forwarding..."
    git merge --ff-only upstream/main
    echo "[OK] main synced"

elif [[ "$CURRENT_BRANCH" == pr/* ]]; then
    # PR 分支:rebase
    BEHIND=$(git rev-list --count "$CURRENT_BRANCH"..upstream/main)
    if [ "$BEHIND" = "0" ]; then
        echo "[*] $CURRENT_BRANCH is up-to-date, no action"
        exit 0
    fi
    echo "[*] $CURRENT_BRANCH is $BEHIND commits behind upstream, rebasing..."
    git rebase upstream/main
    echo "[OK] rebase done"
    echo "[*] next: git push --force-with-lease"

else
    echo "[!] current branch '$CURRENT_BRANCH' is neither main nor pr/*"
    echo "    please switch to main or a PR branch first"
    exit 1
fi

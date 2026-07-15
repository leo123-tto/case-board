#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""查询指定 PR 的状态(CI / mergeable / 评论 / 改动统计)。

用法:
  python check-pr-status.py 35                    # 查 PR #35
  python check-pr-status.py 35 --watch            # 每 30s 刷新(本地看 CI)
  python check-pr-status.py 35 --json             # JSON 输出
  python check-pr-status.py 35 --comments         # 列出所有评论

TOKEN:从环境变量 GITHUB_TOKEN / GH_TOKEN 读,或通过 GCM 自动读。
"""

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request

UPSTREAM_OWNER = "leo123-tto"
UPSTREAM_REPO = "case-board"


def get_token():
    for env_name in ("GH_TOKEN", "GITHUB_TOKEN"):
        v = os.environ.get(env_name)
        if v:
            return v
    try:
        import subprocess
        proc = subprocess.run(
            ["git", "credential", "fill"],
            input=b"protocol=https\nhost=github.com\n",
            capture_output=True,
            timeout=15,
        )
        for line in proc.stdout.decode("utf-8", errors="replace").splitlines():
            if line.startswith("password="):
                return line[len("password="):]
    except Exception:
        pass
    print("[X] 没找到 token,请设 GITHUB_TOKEN 环境变量", file=sys.stderr)
    sys.exit(1)


def api_get(path, token):
    url = f"https://api.github.com/repos/{UPSTREAM_OWNER}/{UPSTREAM_REPO}/{path}"
    req = urllib.request.Request(
        url,
        headers={
            "Authorization": f"token {token}",
            "Accept": "application/vnd.github+json",
            "User-Agent": "caseboard-pr-bot",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return json.loads(resp.read())
    except urllib.error.HTTPError as e:
        print(f"[X] HTTP {e.code}: {e.read().decode('utf-8', errors='replace')}", file=sys.stderr)
        sys.exit(1)


def fmt_status(pr, checks, comments):
    """格式化输出 PR 状态"""
    state = pr["state"]
    if pr.get("merged_at"):
        state = "merged"
    elif pr.get("closed_at"):
        state = "closed"

    ci_state = "none"
    ci_detail = []
    for c in checks.get("check_runs", []):
        ci_detail.append(f"  - {c['name']}: {c['conclusion'] or c['status']}")
    if "statuses" in checks and checks["statuses"]:
        ci_state = checks.get("state", "unknown")
        for s in checks["statuses"]:
            ci_detail.append(f"  - {s['context']}: {s['state']}")

    print(f"PR #{pr['number']} 状态")
    print(f"  标题:       {pr['title']}")
    print(f"  状态:       {state}")
    print(f"  draft:      {pr.get('draft', False)}")
    print(f"  mergeable:  {pr.get('mergeable')}")
    print(f"  创建:       {pr['created_at']}")
    print(f"  更新:       {pr['updated_at']}")
    if pr.get("merged_at"):
        print(f"  合并:       {pr['merged_at']}")
    if pr.get("closed_at"):
        print(f"  关闭:       {pr['closed_at']}")
    print(f"  作者:       {pr['user']['login']}")
    print(f"  base:       {pr['base']['ref']} @ {pr['base']['sha'][:7]}")
    print(f"  head:       {pr['head']['ref']} @ {pr['head']['sha'][:7]}")
    print(f"  改动:       +{pr.get('additions', 0)} / -{pr.get('deletions', 0)} ({pr.get('changed_files', 0)} files)")
    print(f"  评论:       {pr.get('comments', 0)} 通用, {pr.get('review_comments', 0)} review")
    print(f"  commits:    {pr.get('commits', 0)}")
    print(f"  CI 状态:    {ci_state}")
    if ci_detail:
        for line in ci_detail:
            print(line)
    else:
        print("    (暂无 CI checks)")


def main():
    parser = argparse.ArgumentParser(description="查询 PR 状态")
    parser.add_argument("pr_number", type=int, help="PR 号")
    parser.add_argument("--watch", action="store_true", help="每 30s 刷新")
    parser.add_argument("--json", action="store_true", help="JSON 输出")
    parser.add_argument("--comments", action="store_true", help="列出评论")
    args = parser.parse_args()

    token = get_token()

    while True:
        pr = api_get(f"pulls/{args.pr_number}", token)
        checks = api_get(f"commits/{pr['head']['sha']}/check-runs", token)
        comments = api_get(f"issues/{args.pr_number}/comments", token)

        if args.json:
            print(json.dumps({
                "pr": pr,
                "checks": checks,
                "comments": comments,
            }, ensure_ascii=False, indent=2))
            return

        if args.comments:
            print(f"PR #{pr['number']} 评论 ({len(comments)}):")
            for c in comments:
                print(f"  [{c['created_at'][:10]}] {c['user']['login']}:")
                print(f"    {c['body'][:200]}")
                print()
        else:
            fmt_status(pr, checks, comments)
            print(f"  URL:        {pr['html_url']}")

        if not args.watch:
            break
        print()
        print(f"[watch] 30s 后刷新 (Ctrl+C 退出)...")
        try:
            time.sleep(30)
        except KeyboardInterrupt:
            break


if __name__ == "__main__":
    main()

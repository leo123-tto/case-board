#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""列出我在 upstream (leo123-tto/case-board) 的所有 PR。

用法:
  python list-my-prs.py                # 全部
  python list-my-prs.py --open-only    # 只看 open
  python list-my-prs.py --merged       # 只看 merged
  python list-my-prs.py --stats        # 统计信息
  python list-my-prs.py --json         # JSON 输出(供其它脚本用)

TOKEN:从环境变量 GITHUB_TOKEN / GH_TOKEN 读,或通过 GCM 自动读。
"""

import argparse
import json
import os
import sys
import urllib.error
import urllib.request

UPSTREAM_OWNER = "leo123-tto"
UPSTREAM_REPO = "case-board"
MY_LOGIN = "zzf516988659-del"

API_URL = f"https://api.github.com/repos/{UPSTREAM_OWNER}/{UPSTREAM_REPO}/pulls"


def get_token():
    for env_name in ("GH_TOKEN", "GITHUB_TOKEN"):
        v = os.environ.get(env_name)
        if v:
            return v
    # 从 GCM 读
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
    except Exception as e:
        print(f"[!] GCM 读 token 失败: {e}", file=sys.stderr)
    print("[X] 没找到 token,请设 GITHUB_TOKEN 环境变量", file=sys.stderr)
    sys.exit(1)


def list_my_prs(token, state="all", author=None):
    """分页拉取所有 PR(默认只看作者为我)"""
    all_prs = []
    page = 1
    while True:
        url = f"{API_URL}?state={state}&per_page=100&page={page}"
        if author:
            url += f"&author={author}"
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
                prs = json.loads(resp.read())
        except urllib.error.HTTPError as e:
            print(f"[X] HTTP {e.code}: {e.read().decode('utf-8', errors='replace')}", file=sys.stderr)
            sys.exit(1)
        if not prs:
            break
        all_prs.extend(prs)
        if len(prs) < 100:
            break
        page += 1
    return all_prs


def fmt_pr_row(pr):
    return f"#{pr['number']:>3} | {pr['state']:<6} | {pr['created_at'][:10]} | {pr['title'][:60]}"


def main():
    parser = argparse.ArgumentParser(description="列出我在 upstream 的 PR")
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--open-only", action="store_true", help="只看 open")
    group.add_argument("--merged", action="store_true", help="只看已 merge 的")
    parser.add_argument("--stats", action="store_true", help="输出统计")
    parser.add_argument("--json", action="store_true", help="JSON 输出")
    parser.add_argument("--all-authors", action="store_true", help="看所有作者的 PR(不仅限我)")
    args = parser.parse_args()

    token = get_token()

    if args.merged:
        state = "closed"  # merged 在 closed 里
    elif args.open_only:
        state = "open"
    else:
        state = "all"

    author = None if args.all_authors else MY_LOGIN
    prs = list_my_prs(token, state=state, author=author)

    # 过滤 merged(closed 包含 closed-not-merged + merged)
    if args.merged:
        prs = [p for p in prs if p.get("merged_at")]

    if args.json:
        # 精简字段
        out = [
            {
                "number": p["number"],
                "title": p["title"],
                "state": p["state"],
                "merged": bool(p.get("merged_at")),
                "created_at": p["created_at"],
                "merged_at": p.get("merged_at"),
                "html_url": p["html_url"],
                "head": p["head"]["ref"],
                "additions": p.get("additions", 0),
                "deletions": p.get("deletions", 0),
            }
            for p in prs
        ]
        print(json.dumps(out, ensure_ascii=False, indent=2))
        return

    if args.stats:
        total = len(prs)
        open_n = sum(1 for p in prs if p["state"] == "open")
        merged_n = sum(1 for p in prs if p.get("merged_at"))
        closed_n = sum(1 for p in prs if p["state"] == "closed" and not p.get("merged_at"))
        print(f"  总数:   {total}")
        print(f"  open:   {open_n}")
        print(f"  merged: {merged_n}")
        print(f"  closed: {closed_n}")
        return

    # 默认:表格输出
    print(f"PR 列表 (state={state}, author={author or 'all'}):")
    print(f"  {'#':>3}  {'状态':<6}  {'日期':<10}  标题")
    print(f"  {'-'*3}  {'-'*6}  {'-'*10}  {'-'*60}")
    for pr in prs:
        state_label = pr["state"]
        if pr.get("merged_at"):
            state_label = "merged"
        print(f"  #{pr['number']:>3}  {state_label:<6}  {pr['created_at'][:10]}  {pr['title'][:60]}")


if __name__ == "__main__":
    main()

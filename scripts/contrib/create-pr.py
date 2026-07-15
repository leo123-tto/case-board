#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""跨 fork PR 创建脚本(基于 GitHub REST API)

通过 GitHub API 创建 PR: fork (zzf516988659-del) → upstream (leo123-tto)
gh CLI 在本机损坏,直接走 api.github.com + urllib

用法:
  # 方式 1:环境变量传 token
  GITHUB_TOKEN=$(python get-gcm-token.py) python create-pr.py

  # 方式 2:直接传参
  python create-pr.py ghp_xxxxx

  # 方式 3:从 ~/.git-credentials 读
  python create-pr.py

TOKEN 查找顺序:
  1. 命令行参数 argv[1]
  2. 环境变量 GH_TOKEN / GITHUB_TOKEN
  3. ~/.git-credentials 文件
"""

import json
import os
import sys
import urllib.request
import urllib.error

UPSTREAM_OWNER = "leo123-tto"
UPSTREAM_REPO = "case-board"
FORK_OWNER = "zzf516988659-del"
HEAD_BRANCH = f"{FORK_OWNER}:pr/fix/visualize-fake-user-confirm"
BASE_BRANCH = "main"
TITLE = "fix(chat): VisualizeCase 三连环 bug — 反伪造 + 反叙述 + 思考模型 idle 缩放"
BODY_FILE = "pr-body-visualize-fake-user-confirm.md"
PR_URL = f"https://api.github.com/repos/{UPSTREAM_OWNER}/{UPSTREAM_REPO}/pulls"


def get_token(argv):
    if len(argv) > 1 and argv[1]:
        return argv[1]
    # 尝试从 ~/.git-credentials 读(Linux/macOS 上 GCM 可能写到 ~/.git-credentials)
    cred_path = os.path.expanduser("~/.git-credentials")
    if os.path.exists(cred_path):
        try:
            with open(cred_path, "r", encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if "github.com" in line and "@" in line:
                        # 格式:https://USER:TOKEN@github.com
                        after_at = line.split("@", 1)[0]
                        token = after_at.rsplit(":", 1)[-1]
                        if token:
                            print(f"[*] 从 ~/.git-credentials 读到 token ({len(token)} chars)")
                            return token
        except Exception as e:
            print(f"[!] 读 ~/.git-credentials 失败: {e}")
    # 尝试环境变量
    for env_name in ("GH_TOKEN", "GITHUB_TOKEN"):
        v = os.environ.get(env_name)
        if v:
            print(f"[*] 从 env {env_name} 读到 token ({len(v)} chars)")
            return v
    print("[X] 没找到 token。请通过参数或环境变量提供。")
    sys.exit(1)


def load_body():
    if not os.path.exists(BODY_FILE):
        print(f"[X] 找不到 {BODY_FILE},请把 PR body 草稿放在当前目录")
        sys.exit(1)
    with open(BODY_FILE, "r", encoding="utf-8") as f:
        return f.read()


def create_pr(token, title, body):
    payload = {
        "title": title,
        "head": HEAD_BRANCH,
        "base": BASE_BRANCH,
        "body": body,
        "maintainer_can_modify": True,
        "draft": False,
    }
    data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    req = urllib.request.Request(
        PR_URL,
        data=data,
        method="POST",
        headers={
            "Authorization": f"token {token}",
            "User-Agent": "caseboard-pr-bot",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
            "Content-Type": "application/json; charset=utf-8",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            body_bytes = resp.read()
            status = resp.status
    except urllib.error.HTTPError as e:
        body_bytes = e.read()
        status = e.code
    print(f"[*] HTTP {status}")
    if status in (201, 200):
        result = json.loads(body_bytes)
        return result
    else:
        print(f"[!] 响应体: {body_bytes.decode('utf-8', errors='replace')}")
        return None


def main():
    token = get_token(sys.argv)
    print(f"[*] 目标: {UPSTREAM_OWNER}/{UPSTREAM_REPO} <- {HEAD_BRANCH}")
    print(f"[*] 标题: {TITLE}")
    body = load_body()
    print(f"[*] 描述长度: {len(body)} 字符")
    result = create_pr(token, TITLE, body)
    if result:
        print()
        print("[OK] PR created!")
        print(f"    URL:  {result.get('html_url')}")
        print(f"    NUM:  #{result.get('number')}")
        print(f"    STATE: {result.get('state')}")
        print(f"    MERGEABLE: {result.get('mergeable')}")
    else:
        print()
        print("[X] PR creation failed, see error above.")
        sys.exit(2)


if __name__ == "__main__":
    main()

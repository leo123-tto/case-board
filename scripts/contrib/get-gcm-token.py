#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Get GCM (Git Credential Manager) OAuth token for github.com.

Strategy: use GCM via a subprocess with input piped to it. GCM will read the
token from the Windows Credential Manager and print it to stdout.

Usage:
  python get-gcm-token.py              # 输出 token 到 stdout
  eval $(python get-gcm-token.py | head -1 | awk '{print "GITHUB_TOKEN=" $0}')

或直接设置环境变量:
  GITHUB_TOKEN=$(python get-gcm-token.py) python create-pr.py
"""
import subprocess
import sys

# Format: protocol://host (no trailing slash)
cred_input = "protocol=https\nhost=github.com\n"

try:
    # git credential fill reads from stdin the protocol+host, prints username+password to stdout
    proc = subprocess.run(
        ["git", "credential", "fill"],
        input=cred_input.encode("utf-8"),
        capture_output=True,
        timeout=15,
    )
    stdout = proc.stdout.decode("utf-8", errors="replace").strip()
    stderr = proc.stderr.decode("utf-8", errors="replace").strip()
    if proc.returncode != 0:
        print(f"[X] git credential fill 失败: {stderr}", file=sys.stderr)
        sys.exit(1)
    if not stdout:
        print(f"[X] git credential fill 无输出。stderr: {stderr}", file=sys.stderr)
        sys.exit(2)

    # Parse
    user = None
    password = None
    for line in stdout.splitlines():
        if line.startswith("username="):
            user = line[len("username="):]
        elif line.startswith("password="):
            password = line[len("password="):]
    if not password:
        print("[X] 没找到 password 字段", file=sys.stderr)
        sys.exit(3)

    # 输出到 stderr 的诊断信息(不污染 stdout)
    print(f"[*] username: {user}", file=sys.stderr)
    print(f"[*] password length: {len(password)} chars, prefix: {password[:4]}", file=sys.stderr)

    # token 单独输出到 stdout 供 create-pr.py 使用
    print(password)
except subprocess.TimeoutExpired:
    print("[X] git credential fill 超时 (GCM 可能需要交互)", file=sys.stderr)
    sys.exit(4)
except FileNotFoundError:
    print("[X] 找不到 git 命令", file=sys.stderr)
    sys.exit(5)

"""进度上报：stdout JSON Lines。

每行一个 JSON 对象，字段对齐法穿 court_filing_helpers 的 progress 事件：
    phase / stage / level / message / ts + 任意扩展字段
phase ∈ {system, login, http, playwright, captcha}

【重要】stdout 只写本模块的 JSONL；playwright/ddddocr 等第三方库日志由 cli.py
统一配置 logging StreamHandler → stderr，避免 Rust 侧 JSON 解析失败。
"""

import json
import sys
import time

# 标准退出码
EXIT_SUCCESS = 0
EXIT_FAILURE = 1
EXIT_ARG_ERROR = 2


def _now() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%S")


def emit(
    phase: str,
    stage: str,
    message: str = "",
    *,
    level: str = "info",
    **extra: object,
) -> None:
    """向 stdout 写一行 JSON 进度事件并立即 flush。"""
    event: dict[str, object] = {
        "phase": phase,
        "stage": stage,
        "level": level,
        "message": message,
        "ts": _now(),
    }
    event.update(extra)
    line = json.dumps(event, ensure_ascii=False)
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


def emit_result(success: bool, message: str, **extra: object) -> None:
    """收尾：写一条带 result 的 system 事件。"""
    emit("system", "cli.done", message, level="info" if success else "error",
         result={"success": success, "message": message, **extra})

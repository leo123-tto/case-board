"""Playwright 浏览器封装（原生 sync_playwright，不依赖 cloakbrowser）。

法穿 create_browser(anti_detection=False) 的等价轻量实现。
"""

import logging
from contextlib import contextmanager
from typing import Any, Iterator

logger = logging.getLogger("court_filing_cli")


@contextmanager
def create_browser(headless: bool = False) -> Iterator[tuple[Any, Any]]:
    """启动 Chromium，yield (page, context)。用完自动关闭。

    Args:
        headless: 是否无头。立案默认 False（肉眼可见，便于人工兜底验证码看现场）。
    """
    from playwright.sync_api import sync_playwright

    launch_args = [
        "--disable-blink-features=AutomationControlled",
        "--no-sandbox",
    ]
    pw = sync_playwright().start()
    browser = None
    context = None
    try:
        browser = pw.chromium.launch(headless=headless, args=launch_args)
        context = browser.new_context(
            user_agent=(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
                "AppleWebKit/537.36 (KHTML, like Gecko) "
                "Chrome/124.0.0.0 Safari/537.36"
            ),
            viewport={"width": 1440, "height": 900},
            locale="zh-CN",
        )
        page = context.new_page()
        yield page, context
    finally:
        try:
            if context is not None:
                context.close()
        except Exception:
            pass
        try:
            if browser is not None:
                browser.close()
        except Exception:
            pass
        pw.stop()

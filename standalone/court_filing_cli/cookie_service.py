"""Cookie 持久化服务（纯 pathlib，不依赖 Django/apps.core.utils.path）。

修正了源 apps/.../core/cookie_service.py 的 bug：
原 save() 未接收 context.cookies() 返回值、写入空 payload，导致 Cookie 实际未保存。
"""

import json
import logging
from pathlib import Path
from typing import Any

logger = logging.getLogger("court_filing_cli")


class CookieService:
    """浏览器上下文 Cookie 的 JSON 文件持久化。

    storage_path 可在构造时传，也可在 load/save 时覆盖。
    文件格式: {"cookies": [ <playwright Cookie 对象>, ... ]}
    """

    def __init__(self, storage_path: str | None = None) -> None:
        self.storage_path = storage_path

    def load(self, context: Any, storage_path: str | None = None) -> bool:
        """加载 Cookie 到浏览器上下文，返回是否成功加载到有效 Cookie。"""
        path = storage_path or self.storage_path
        if not path:
            return False
        p = Path(path)
        if not p.exists():
            return False
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as e:
            logger.warning("读取 Cookie 文件失败 %s: %s", path, e)
            return False
        cookies = data.get("cookies") if isinstance(data, dict) else None
        if not cookies:
            return False
        try:
            context.add_cookies(cookies)
        except Exception as e:
            logger.warning("注入 Cookie 失败 %s: %s", path, e)
            return False
        logger.info("已加载 Cookie: %s (%d 条)", path, len(cookies))
        return True

    def save(self, context: Any, storage_path: str | None = None) -> str:
        """保存浏览器上下文的 Cookie，返回存储路径。"""
        path = storage_path or self.storage_path
        if not path:
            raise ValueError("storage_path is required")
        p = Path(path)
        p.parent.mkdir(parents=True, exist_ok=True)
        cookies = context.cookies()  # 修正：接收返回值
        payload = {"cookies": cookies}
        p.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
        logger.info("Cookie 已保存: %s (%d 条)", path, len(cookies))
        return str(p)

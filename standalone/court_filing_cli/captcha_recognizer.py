"""验证码识别器（纯 Python，不依赖 Django）。

从 apps/automation/services/scraper/core/captcha_recognizer.py 抽取：
- CaptchaRecognizer（ABC）
- DdddocrRecognizer（ddddocr 自动识别）
人工兜底识别器(captcha_manual.ManualCaptchaRecognizer)单独实现，走文件握手。
"""

import logging
from abc import ABC, abstractmethod
from typing import Any, cast

logger = logging.getLogger("court_filing_cli")


class CaptchaRecognizer(ABC):
    """验证码识别器抽象接口。

    实现者应：识别失败返回 None（不抛异常），记录错误日志。
    """

    @abstractmethod
    def recognize(self, image_bytes: bytes) -> str | None:
        """从图片字节流识别验证码，失败返回 None。"""
        pass

    @abstractmethod
    def recognize_from_element(self, page: Any, selector: str) -> str | None:
        """从页面元素截图识别验证码，失败返回 None。"""
        pass


class DdddocrRecognizer(CaptchaRecognizer):
    """使用 ddddocr 库的验证码识别器。"""

    def __init__(self, show_ad: bool = False):
        try:
            import ddddocr

            self.ocr = ddddocr.DdddOcr(show_ad=show_ad)
            logger.info("DdddocrRecognizer 初始化成功")
        except ImportError as e:
            raise ImportError("ddddocr 未安装，请运行: pip install ddddocr") from e

    def recognize(self, image_bytes: bytes) -> str | None:
        if not image_bytes:
            logger.warning("验证码图片字节流为空")
            return None
        try:
            result = self.ocr.classification(image_bytes)
            cleaned = result.strip().replace(" ", "")
            logger.info("验证码识别成功: %s", cleaned)
            return cast(str | None, cleaned)
        except Exception as e:
            logger.error("验证码识别失败: %s", e, exc_info=True)
            return None

    def recognize_from_element(self, page: Any, selector: str) -> str | None:
        try:
            element = page.locator(selector)
            element.wait_for(state="visible", timeout=5000)
            image_bytes = element.screenshot()
            return self.recognize(image_bytes)
        except Exception as e:
            logger.error("从页面元素获取验证码失败 (selector: %s): %s", selector, e, exc_info=True)
            return None

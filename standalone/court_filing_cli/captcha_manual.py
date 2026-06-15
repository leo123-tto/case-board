"""验证码人工兜底 + 自动降级（CLI 核心设计）。

ManualCaptchaRecognizer:
  文件握手 + stdout 事件双通道。
  "等答案"发生在 CLI 进程内轮询，Rust 侧不阻塞。

AutoDegradingRecognizer:
  先 ddddocr 自动识别，失败 N 次后降级到 ManualCaptchaRecognizer。

协议（多轮安全）：
  轮次 N：
  CLI 写 output_dir/captcha_pending.json
    → stdout emit {"phase":"captcha","stage":"captcha.required", ...}
    → 轮询 output_dir/captcha_answer.json（间隔 1.5s，超时 300s）
    → 校验 task_id+round 匹配 → 返回答案
"""

import base64
import json
import logging
import time
import uuid
from pathlib import Path
from typing import Any

from court_filing_cli.captcha_recognizer import CaptchaRecognizer, DdddocrRecognizer
from court_filing_cli.progress import emit

logger = logging.getLogger("court_filing_cli.captcha_manual")

_CAPTCHA_PENDING = "captcha_pending.json"
_CAPTCHA_ANSWER = "captcha_answer.json"


class ManualCaptchaRecognizer(CaptchaRecognizer):
    """文件握手式人工验证码识别器。

    工作流：
    1. 写 captcha_pending.json（含 base64 图片 + task_id + round）
    2. stdout emit captcha.required 事件
    3. 轮询 captcha_answer.json，校验 task_id+round 匹配
    4. 返回答案（或超时返回 None）
    """

    def __init__(
        self,
        output_dir: str,
        task_id: str,
        timeout: int = 300,
        poll_interval: float = 1.5,
    ) -> None:
        """
        Args:
            output_dir: 验证码文件输出目录（captcha_pending.json / captcha_answer.json）
            task_id: 本次 CLI 运行的唯一标识（用于匹配 answer）
            timeout: 等超时秒数
            poll_interval: 轮询间隔秒数
        """
        self.output_dir = Path(output_dir)
        self.task_id = task_id
        self.timeout = timeout
        self.poll_interval = poll_interval
        self._round = 0

    def recognize(self, image_bytes: bytes) -> str | None:
        if not image_bytes:
            logger.warning("图片字节流为空")
            return None

        self._round += 1
        round_num = self._round

        try:
            # 1. 保存验证码图片
            self.output_dir.mkdir(parents=True, exist_ok=True)
            image_path = self.output_dir / f"captcha_{round_num}.png"
            image_path.write_bytes(image_bytes)

            # 2. 写 captcha_pending.json
            image_b64 = base64.b64encode(image_bytes).decode("ascii")
            pending = {
                "round": round_num,
                "task_id": self.task_id,
                "image_base64": f"data:image/png;base64,{image_b64}",
                "image_path": str(image_path),
                "created_ts": time.strftime("%Y-%m-%dT%H:%M:%S"),
                "timeout_sec": self.timeout,
            }
            pending_path = self.output_dir / _CAPTCHA_PENDING
            pending_path.write_text(json.dumps(pending, ensure_ascii=False, indent=2), encoding="utf-8")
            logger.info("验证码待输入: round=%d, task_id=%s, path=%s", round_num, self.task_id, image_path)

            # 3. stdout emit captcha.required
            emit(
                "captcha", "captcha.required",
                f"等待人工输入验证码（第 {round_num} 轮）",
                round=round_num,
                task_id=self.task_id,
                image_base64=pending["image_base64"],
                image_path=str(image_path),
                timeout_sec=self.timeout,
            )

            # 4. 轮询 captcha_answer.json
            answer_path = self.output_dir / _CAPTCHA_ANSWER
            deadline = time.time() + self.timeout
            while time.time() < deadline:
                time.sleep(self.poll_interval)

                if not answer_path.exists():
                    continue

                try:
                    raw = json.loads(answer_path.read_text(encoding="utf-8"))
                except (json.JSONDecodeError, OSError) as e:
                    logger.warning("读取 answer 文件失败: %s", e)
                    continue

                # 校验 task_id + round 匹配
                if raw.get("task_id") != self.task_id or raw.get("round") != round_num:
                    logger.debug("answer 不匹配: task_id=%s round=%s (期望 %s/%d)",
                                 raw.get("task_id"), raw.get("round"), self.task_id, round_num)
                    continue

                # 匹配成功，读取答案并清理
                answer = str(raw.get("answer", "")).strip().replace(" ", "")
                try:
                    answer_path.unlink(missing_ok=True)
                except OSError:
                    pass
                try:
                    pending_path.unlink(missing_ok=True)
                except OSError:
                    pass

                if answer:
                    logger.info("收到人工验证码: round=%d, answer=%s", round_num, answer)
                    emit("captcha", "captcha.answered", f"验证码已输入（第 {round_num} 轮）",
                         round=round_num, task_id=self.task_id)
                    return answer
                else:
                    logger.warning("answer 为空，继续等待")

            # 5. 超时
            logger.warning("人工验证码等待超时: round=%d, timeout=%ss", round_num, self.timeout)
            emit("captcha", "captcha.timeout", f"验证码等待超时（第 {round_num} 轮）",
                 level="warning", round=round_num, task_id=self.task_id)
            return None

        except Exception as e:
            logger.error("人工验证码异常: %s", e, exc_info=True)
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


class AutoDegradingRecognizer(CaptchaRecognizer):
    """自动→人工降级验证码识别器。

    工作流（配合 court_zxfw._try_captcha_login 的重试循环）：
    - 前 max_auto_attempts 次：用 DdddocrRecognizer，返回 None 表示未识别（让 court_zxfw 重试）
    - 第 max_auto_attempts+1 次起：降级到 ManualCaptchaRecognizer

    max_captcha_retries 应设大于 max_auto_attempts（如 retries=10, auto_attempts=3），
    让 court_zxfw 的循环有机会走到降级阶段。
    """

    def __init__(
        self,
        output_dir: str,
        task_id: str | None = None,
        max_auto_attempts: int = 3,
        timeout: int = 300,
    ) -> None:
        """
        Args:
            output_dir: 验证码文件输出目录
            task_id: 本次 CLI 运行的唯一标识，None 则自动生成
            max_auto_attempts: 自动识别最大尝试次数，超过后降级人工
            timeout: 人工兜底超时秒数
        """
        self._auto = DdddocrRecognizer()
        self._manual = ManualCaptchaRecognizer(
            output_dir=output_dir,
            task_id=task_id or uuid.uuid4().hex[:12],
            timeout=timeout,
        )
        self._max_auto = max_auto_attempts
        self._auto_failures = 0

    @property
    def _degraded(self) -> bool:
        return self._auto_failures >= self._max_auto

    def recognize(self, image_bytes: bytes) -> str | None:
        if not image_bytes:
            return None

        if not self._degraded:
            # 自动识别阶段
            result = self._auto.recognize(image_bytes)
            if result:
                self._auto_failures = 0  # 成功则重置计数
                return result
            self._auto_failures += 1
            logger.info("ddddocr 自动识别失败 (%d/%d)，刷新验证码重试",
                        self._auto_failures, self._max_auto)
            if self._degraded:
                logger.info("已达到自动重试上限，降级到人工兜底")
                emit("captcha", "captcha.degrading",
                     "ddddocr 自动识别失败，降级到人工输入",
                     level="warning", auto_failures=self._auto_failures)
            return None  # 返回 None 让 court_zxfw 刷新验证码重试

        # 人工兜底阶段
        return self._manual.recognize(image_bytes)

    def recognize_from_element(self, page: Any, selector: str) -> str | None:
        if not self._degraded:
            result = self._auto.recognize_from_element(page, selector)
            if result:
                self._auto_failures = 0
                return result
            self._auto_failures += 1
            logger.info("ddddocr 自动识别失败 (%d/%d)", self._auto_failures, self._max_auto)
            if self._degraded:
                emit("captcha", "captcha.degrading",
                     "ddddocr 自动识别失败，降级到人工输入",
                     level="warning", auto_failures=self._auto_failures)
            return None
        return self._manual.recognize_from_element(page, selector)

"""立案编排层（替代法穿 court_filing_helpers._run_filing）。

串联：浏览器启动 → 登录 → CourtZxfwFilingService.file_case/file_execution。
不依赖 Django ORM，case_data 从外部 JSON 传入。
"""

import logging
import time
from typing import Any

from court_filing_cli.progress import emit
from court_filing_cli.schemas import CaseData

logger = logging.getLogger("court_filing_cli.runner")


def _make_progress_reporter() -> Any:
    """创建 _progress_reporter 回调，转发到 stdout JSONL。

    法穿 CourtZxfwFilingService._report_progress 读 case_data["_progress_reporter"]，
    这里注入一个写 stdout JSONL 的函数。
    """
    def reporter(payload: dict[str, Any]) -> None:
        emit(
            phase=payload.get("phase", "system"),
            stage=payload.get("stage", ""),
            message=payload.get("message", ""),
            level=payload.get("level", "info"),
            detail=payload.get("detail", ""),
        )
    return reporter


def _build_case_data_dict(
    case_data: CaseData,
    materials: dict[str, list[tuple[str, str]]],
) -> dict[str, Any]:
    """从 CaseData + materials 构建法穿 case_data dict。"""
    d = case_data.to_dict()
    d["materials"] = materials
    d["_progress_reporter"] = _make_progress_reporter()
    d["filing_engine"] = "playwright"  # 强制 Playwright，跳过 HTTP 链路
    return d


def _build_captcha_recognizer(
    captcha_mode: str,
    output_dir: str,
) -> Any:
    """根据 captcha_mode 构建验证码识别器。"""
    if captcha_mode == "manual":
        import uuid
        from court_filing_cli.captcha_manual import ManualCaptchaRecognizer
        return ManualCaptchaRecognizer(output_dir=output_dir, task_id=uuid.uuid4().hex[:12])
    else:
        # auto：ddddocr 自动 → 失败 3 次降级人工
        from court_filing_cli.captcha_manual import AutoDegradingRecognizer
        return AutoDegradingRecognizer(output_dir=output_dir, max_auto_attempts=3)


def run_filing(
    case_data: CaseData,
    materials: dict[str, list[tuple[str, str]]],
    account: str,
    password: str,
    output_dir: str,
    cookie_dir: str | None = None,
    headless: bool = False,
    save_screenshot: bool = False,
    captcha_mode: str = "auto",
) -> dict[str, Any]:
    """立案主流程：登录 → 立案 6 步/5 步 → 到预览页。

    Args:
        case_data: 案件数据
        materials: 材料槽位映射 {slot: [(path, filename)]}
        account: 一张网账号
        password: 一张网密码
        output_dir: 输出目录（截图等）
        cookie_dir: Cookie 存储目录
        headless: 是否无头模式
        save_screenshot: 是否保存截图（注意：立案截图依赖 Django，CLI 版默认 False）

    Returns:
        {"success": bool, "message": str, "url": str}
    """
    from court_filing_cli.browser import create_browser
    from court_filing_cli.cookie_service import CookieService
    from court_filing_cli.sites.court_zxfw import CourtZxfwService
    from court_filing_cli.sites.court_zxfw_filing.service import CourtZxfwFilingService

    # ── 准备 Cookie ──
    cookie_service = None
    if cookie_dir:
        import os
        safe_account = account.replace("@", "_at_").replace("/", "_")
        cookie_path = os.path.join(cookie_dir, f"court_zxfw_{safe_account}.json")
        cookie_service = CookieService(storage_path=cookie_path)

    # ── 组装 case_data dict ──
    case_data_dict = _build_case_data_dict(case_data, materials)

    timing: dict[str, float] = {"overall_start": time.monotonic()}

    try:
        with create_browser(headless=headless) as (page, context):
            # ── 登录 ──
            captcha_recognizer = _build_captcha_recognizer(captcha_mode, output_dir)
            login_service = CourtZxfwService(
                page=page,
                context=context,
                cookie_service=cookie_service,
                captcha_recognizer=captcha_recognizer,
                debug_dir=output_dir if save_screenshot else None,
            )

            # auto 模式下 max_captcha_retries 设大，给 ddddocr→人工降级机会
            max_retries = 10 if captcha_mode == "auto" else 3

            emit("login", "login.start", "正在登录一张网...", captcha_mode=captcha_mode)
            login_result = login_service.login(
                account=account,
                password=password,
                max_captcha_retries=max_retries,
                save_debug=save_screenshot,
            )

            if not login_result.get("success"):
                msg = login_result.get("message", "登录失败")
                emit("login", "login.failed", msg, level="error")
                timing["login_end"] = time.monotonic()
                return {"success": False, "message": f"登录失败: {msg}", "timing": timing}

            emit("login", "login.success", login_result.get("message", "登录成功"))
            timing["login_end"] = time.monotonic()

            # ── 立案 ──
            filing_service = CourtZxfwFilingService(page=page, save_debug=False)

            if case_data.filing_type == "execution":
                emit("system", "filing.start", "开始执行申请执行立案流程")
                result = filing_service.file_execution(case_data_dict)
            else:
                emit("system", "filing.start", "开始执行民事一审立案流程")
                result = filing_service.file_case(case_data_dict)

            timing["overall_end"] = time.monotonic()

            if result.get("success"):
                emit("system", "filing.success", result.get("message", "立案完成"))
                result["timing"] = timing
                return result
            else:
                msg = result.get("message", "立案失败")
                emit("system", "filing.failed", msg, level="error")
                result["timing"] = timing
                return result

    except Exception as e:
        timing["overall_end"] = time.monotonic()
        logger.error("立案流程异常: %s", e, exc_info=True)
        emit("system", "cli.error", f"立案流程异常: {e}", level="error", traceback=str(e))
        return {"success": False, "message": f"立案流程异常: {e}", "timing": timing}

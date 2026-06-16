#!/usr/bin/env python3
"""Parse and download zxfw.court.gov.cn court delivery documents."""

from __future__ import annotations

import argparse
import json
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


LIST_API = "https://zxfw.court.gov.cn/yzw/yzw-zxfw-sdfw/api/v1/sdfw/getWsListBySdbhNew"
HEADERS = {
    "Content-Type": "application/json",
    "Origin": "https://zxfw.court.gov.cn",
    "Referer": "https://zxfw.court.gov.cn/zxfw/",
    "User-Agent": (
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
        "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36"
    ),
}


def normalize_case_no(value: str) -> str:
    return "".join({"（": "(", "）": ")"}.get(ch, ch) for ch in value if not ch.isspace())


def parse_sms(text: str) -> dict[str, Any]:
    court_match = re.search(r"【([^【】]*?法院)】", text)
    case_match = re.search(
        r"[（(]\s*\d{4}\s*[）)]\s*[\u4e00-\u9fa5]{1,3}\s*\d{2,6}\s*[\u4e00-\u9fa5]{1,4}\s*\d+\s*号",
        text,
    )
    link = extract_link(text)
    case_no = case_match.group(0).strip() if case_match else None
    return {
        "court": court_match.group(1) if court_match else None,
        "case_no": case_no,
        "normalized_case_no": normalize_case_no(case_no) if case_no else None,
        "has_link": link is not None,
        "link": link,
    }


def extract_link(text: str) -> dict[str, str] | None:
    if "zxfw.court.gov.cn" not in text:
        return None

    def grab(key: str) -> str | None:
        match = re.search(rf"{key}=([0-9A-Za-z]+)", text)
        return match.group(1) if match else None

    sdbh = grab("sdbh")
    qdbh = grab("qdbh")
    sdsin = grab("sdsin")
    if not (sdbh and qdbh and sdsin):
        return None
    return {"sdbh": sdbh, "qdbh": qdbh, "sdsin": sdsin}


def fetch_doc_list(link: dict[str, str]) -> list[dict[str, Any]]:
    payload = json.dumps(link, ensure_ascii=False).encode("utf-8")
    req = urllib.request.Request(LIST_API, data=payload, headers=HEADERS, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=40) as resp:
            body = resp.read().decode("utf-8", errors="replace")
            status = resp.status
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"一张网返回 HTTP {exc.code}: {truncate(body, 300)}") from exc
    except urllib.error.URLError as exc:
        raise RuntimeError(f"请求一张网失败: {exc}") from exc

    if status < 200 or status >= 300:
        raise RuntimeError(f"一张网返回 HTTP {status}: {truncate(body, 300)}")
    try:
        parsed = json.loads(body)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"一张网响应非 JSON: {exc} · {truncate(body, 300)}") from exc

    if parsed.get("code") != 200:
        raise RuntimeError(f"一张网业务错误(code={parsed.get('code')}): {parsed.get('msg', '未知')}")
    data = parsed.get("data")
    if data is None:
        return []
    if not isinstance(data, list):
        raise RuntimeError("一张网文书列表 data 不是数组")
    return data


def preview(text: str) -> dict[str, Any]:
    parsed = parse_sms(text)
    link = parsed.get("link")
    if not link:
        parsed["docs"] = []
        parsed["note"] = (
            "没识别到「人民法院在线服务/一张网」(zxfw.court.gov.cn)送达链接。"
            "目前只支持一张网;其它平台暂不支持。"
        )
        return parsed

    docs = fetch_doc_list(link)
    parsed["docs"] = [
        {
            "name": doc.get("c_wsmc") or "",
            "ext": doc.get("c_wjgs") or "pdf",
            "court": doc.get("c_fymc"),
            "has_download_url": bool(doc.get("wjlj")),
        }
        for doc in docs
    ]
    if not parsed.get("court"):
        parsed["court"] = next((doc.get("court") for doc in parsed["docs"] if doc.get("court")), None)
    return parsed


def download(text: str, out_dir: Path) -> dict[str, Any]:
    parsed = parse_sms(text)
    link = parsed.get("link")
    if not link:
        raise RuntimeError("没识别到 zxfw.court.gov.cn 一张网链接参数")
    out_dir.mkdir(parents=True, exist_ok=True)
    docs = fetch_doc_list(link)
    downloaded: list[str] = []
    skipped: list[str] = []
    for doc in docs:
        name = doc.get("c_wsmc") or "court_document"
        ext = doc.get("c_wjgs") or "pdf"
        url = doc.get("wjlj") or ""
        if not url:
            skipped.append(f"{name}(缺少下载地址 wjlj)")
            continue
        dest = unique_path(out_dir, sanitize_filename(name), sanitize_ext(ext))
        try:
            download_url(url, dest)
            downloaded.append(str(dest))
        except Exception as exc:  # noqa: BLE001 - report per-document failures.
            skipped.append(f"{name}({exc})")
    return {"parsed": parsed, "downloaded": downloaded, "skipped": skipped}


def download_url(url: str, dest: Path) -> None:
    req = urllib.request.Request(url, headers={"User-Agent": HEADERS["User-Agent"]}, method="GET")
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            if resp.status < 200 or resp.status >= 300:
                raise RuntimeError(f"下载文书 HTTP {resp.status}")
            dest.write_bytes(resp.read())
    except urllib.error.HTTPError as exc:
        raise RuntimeError(f"下载文书 HTTP {exc.code}") from exc
    except urllib.error.URLError as exc:
        raise RuntimeError(f"下载文书失败: {exc}") from exc


def sanitize_filename(value: str) -> str:
    cleaned = re.sub(r"""[/\\:*?"<>|\n\r\t]""", "_", value).strip()
    return (cleaned or "court_document")[:80]


def sanitize_ext(value: str) -> str:
    cleaned = re.sub(r"[^0-9A-Za-z]", "", value).strip(".")
    return cleaned or "pdf"


def unique_path(folder: Path, base: str, ext: str) -> Path:
    first = folder / f"{base}.{ext}"
    if not first.exists():
        return first
    for n in range(2, 1000):
        candidate = folder / f"{base} ({n}).{ext}"
        if not candidate.exists():
            return candidate
    return first


def truncate(value: str, limit: int) -> str:
    return value[:limit]


def read_sms(args: argparse.Namespace) -> str:
    if args.sms is not None:
        return args.sms
    if args.sms_file is not None:
        return Path(args.sms_file).read_text(encoding="utf-8")
    return sys.stdin.read()


def emit(value: Any) -> None:
    print(json.dumps(value, ensure_ascii=False, indent=2))


def main() -> int:
    parser = argparse.ArgumentParser(description="Handle zxfw.court.gov.cn court delivery SMS.")
    subparsers = parser.add_subparsers(dest="command", required=True)

    for command in ("parse", "preview", "download"):
        sub = subparsers.add_parser(command)
        sub.add_argument("--sms", help="Full SMS text")
        sub.add_argument("--sms-file", help="Path to a UTF-8 text file containing the SMS")
        if command == "download":
            sub.add_argument("--out-dir", required=True, help="Destination folder for downloaded documents")

    args = parser.parse_args()
    try:
        text = read_sms(args)
        if args.command == "parse":
            emit(parse_sms(text))
        elif args.command == "preview":
            emit(preview(text))
        elif args.command == "download":
            emit(download(text, Path(args.out_dir).expanduser()))
        return 0
    except Exception as exc:  # noqa: BLE001 - CLI should return actionable JSON errors.
        emit({"error": str(exc)})
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

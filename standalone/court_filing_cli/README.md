# 法院「一张网」在线立案 CLI

从 [法穿(FachuanHybridSystem)](../../) 立案核心抽取的独立版本，不依赖 Django/法穿后端。
供案件看板(CaseBoard)通过 `subprocess` 调用。

## 架构

```
CaseBoard (Tauri, Rust+TS)
   └─ spawn python3 -m court_filing_cli
         ├─ court_zxfw.py   ← 登录一张网 (Cookie→Playwright+ddddocr)
         ├─ runner.py        ← 立案编排 (M2)
         └─ stdout JSONL → Rust BufReader 逐行读 → emit Tauri 事件
```

## 安装

```bash
# 1. 创建 venv
cd standalone/court_filing_cli
python3 -m venv .venv
source .venv/bin/activate

# 2. 安装依赖
pip install -e ".[dev]"

# 3. 安装 Chromium（首次必装）
playwright install chromium
```

## 使用

### 仅登录测试（M1）

```bash
python -m court_filing_cli \
  --login-only \
  --account 13800138000 \
  --password "你的密码" \
  --output-dir /tmp/court_filing_test \
  --cookie-dir /tmp/court_filing_test/cookies
```

输出：stdout 一行 JSONL 进度事件（`phase=login, stage=login.success`），
Cookie 保存到 `cookie_dir/court_zxfw_13800138000.json`。

### 立案（M2）

```bash
python -m court_filing_cli \
  --account 13800138000 \
  --password "你的密码" \
  --filing-type civil \
  --case-data /path/to/case_data.json \
  --materials /path/to/materials.json \
  --output-dir /tmp/court_filing/job1 \
  --cookie-dir /tmp/court_filing/cookies \
  --save-screenshot
```

**注意**：自动化只到「预览页」，**不自动提交**。最终提交需人工在浏览器中核对后手动操作。

## 输出格式（stdout JSONL）

每行一个 JSON 对象，字段：

| 字段 | 说明 |
|------|------|
| `phase` | `system` / `login` / `http` / `playwright` / `captcha` |
| `stage` | 阶段标识（如 `login.success`, `playwright.step.select_court`） |
| `level` | `info` / `warning` / `error` |
| `message` | 人类可读信息 |
| `ts` | 时间戳 `YYYY-MM-DDTHH:MM:SS` |
| `result` | 仅最后一条（`cli.done`），含 `{success, message, ...}` |

第三方库日志（playwright/ddddocr）**只输出到 stderr**，stdout 纯 JSONL。

## 依赖

| 包 | 用途 |
|----|------|
| `playwright` | 浏览器自动化（登录+立案） |
| `ddddocr` | 验证码自动识别 |
| `httpx` | HTTP 客户端（未来 HTTP 逆向登录启用时用） |

**不需要**：django, ninja, cloakbrowser, gmssl（可选）。

## 验证码处理

- `--captcha-mode auto`（默认）：ddddocr 自动识别，失败 3 次降级人工兜底
- `--captcha-mode manual`：直接人工兜底
- 人工兜底：CLI 写 `output_dir/captcha_pending.json`，stdout emit `captcha.required` 事件，
  轮询等待 `output_dir/captcha_answer.json`

## 文件结构

```
court_filing_cli/
├── __init__.py          # 包标识 + 版本
├── __main__.py          # python -m 入口
├── cli.py               # argparse + 主流程
├── cookie_service.py    # Cookie 持久化（纯 pathlib）
├── captcha_recognizer.py # DdddocrRecognizer（自动识别）
├── progress.py          # stdout JSONL emit
├── browser.py           # sync_playwright 封装
├── schemas.py           # CaseData/Party/Agent 结构定义
├── sites/
│   ├── __init__.py
│   └── court_zxfw.py   # 登录核心（从法穿抽取，小改解耦 Django）
└── README.md
```

## 版本

v0.1.0 — M1 阶段（登录验证）。M2（立案6步）、M3（人工验证码）待实现。

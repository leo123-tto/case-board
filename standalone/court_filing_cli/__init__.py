"""法院「一张网」在线立案 CLI（独立运行，不依赖 Django）。

从法穿(FachuanHybridSystem)立案核心抽取，供案件看板(CaseBoard)通过 subprocess 调用。
- 进度通过 stdout JSON Lines 上报（Rust 侧 BufReader 逐行解析）
- 验证码人工兜底通过文件握手（captcha_pending.json / captcha_answer.json）
- 自动化只到「预览页」，不自动提交（人工核对后手动提交）

用法:
    python -m court_filing_cli --account X --password Y \\
        --filing-type civil --case-data case.json --materials mats.json \\
        --output-dir /tmp/job1
"""

__version__ = "0.1.0"

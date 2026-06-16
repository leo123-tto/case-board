---
name: court-zxfw-delivery
description: Parse and handle Chinese court delivery SMS messages from "人民法院在线服务/法院一张网" at zxfw.court.gov.cn. Use when a user pastes a court SMS or asks to extract qdbh/sdbh/sdsin parameters, preview delivered documents, download service PDFs, archive one-network court documents into a case folder, or reproduce the 案件看板 "法院短信处理/一张网送达" workflow.
---

# 法院一张网送达

## Core Workflow

Use this skill for "人民法院在线服务 / 法院一张网" delivery links from `zxfw.court.gov.cn`.

1. Parse the full SMS text first. Extract:
   - court name from the first `【...法院】`
   - case number matching full-width or half-width parentheses
   - one-network link parameters: `sdbh`, `qdbh`, `sdsin`
2. Preview before downloading. Call the one-network list API with only `sdbh`, `qdbh`, and `sdsin`; show the document names and court before any file writes.
3. Download only after the user has identified the target folder or case. Download URLs (`wjlj`) are time-limited, so fetch a fresh document list immediately before downloading.
4. Report real failures verbatim enough to act on them. Common failures are expired SMS links, missing `zxfw.court.gov.cn`, non-JSON responses, business errors where `code != 200`, or document rows missing `wjlj`.

Do not use this skill for provincial platforms such as 江苏微解纷 or other court portals that require login, browser automation, or credentials. This skill only covers the pure one-network API path.

## Script

Use `scripts/zxfw_delivery.py` for deterministic parsing, preview, and downloads.

```bash
python3 /Users/Apple/.codex/skills/court-zxfw-delivery/scripts/zxfw_delivery.py parse --sms-file sms.txt
python3 /Users/Apple/.codex/skills/court-zxfw-delivery/scripts/zxfw_delivery.py preview --sms-file sms.txt
python3 /Users/Apple/.codex/skills/court-zxfw-delivery/scripts/zxfw_delivery.py download --sms-file sms.txt --out-dir "/path/to/case/source_folder"
```

Input options:

- `--sms "..."` reads SMS text from an argument.
- `--sms-file path` reads SMS text from a file.
- With neither option, the script reads SMS text from stdin.

The script prints JSON. `parse` has no network access. `preview` calls the list API but does not write files. `download` re-fetches the list and writes PDFs or other returned formats into `--out-dir` using sanitized unique filenames.

## Case System Integration

When reproducing the original 案件看板 behavior, follow the reference implementation notes in `references/case-dashboard-integration.md`.

The important integration boundary is:

- The one-network logic only parses SMS, lists documents, and downloads files.
- The host case system decides which case to archive into, updates its database, and triggers OCR/extraction.

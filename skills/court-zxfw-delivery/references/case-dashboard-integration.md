# 案件看板接入要点

Use these notes when recreating or modifying the original 案件看板 "法院短信处理" feature.

## Boundaries

- Keep pure one-network logic separate from case-system orchestration.
- Pure logic: parse SMS, normalize case number, fetch document list, download a document URL.
- Host system orchestration: case matching, folder selection, database sync, OCR/extraction, UI state.

## Matching

1. Normalize case numbers before comparing: remove whitespace and convert `（` / `）` to `(` / `)`.
2. Match against the current case number first.
3. If the app has instance/stage rows, match all stage case numbers as a fallback.
4. If case-number matching fails, match party names from plaintiffs, defendants, and party contacts against the SMS text.
5. Exclude demo/sample cases from party-name matching.
6. Treat party-name matching as a candidate, not a final decision; ask the user to confirm the target case before download.

## Download and Archive

- Do not persist `wjlj` as a durable link. It is a time-limited pre-signed download URL.
- Re-fetch the document list after the user confirms the target case, then download immediately.
- Sanitize file names by replacing `/ \ : * ? " < > |` and line breaks with `_`.
- Avoid overwriting existing files; append ` (2)`, ` (3)`, etc.
- After files land in the case source folder, let the host system run its normal scan/sync/extraction path.

## User-Facing Failure Messages

- Missing one-network link: "没识别到「人民法院在线服务/一张网」(zxfw.court.gov.cn)送达链接。目前只支持一张网;其它平台暂不支持自动下载。"
- Empty document list: "一张网未返回任何文书(链接可能已失效,请重新粘贴最新短信)。"
- No automatic case match: "没自动匹配到在办案件,请手动选择要归档到哪个案件。"

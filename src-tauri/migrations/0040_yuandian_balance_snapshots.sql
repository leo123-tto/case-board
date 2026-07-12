-- 2026-07-12 · 元典 MCP 官方余额快照与本机积分账对账
--
-- 余额工具不在公开的 36 个付费业务 API 目录内，由 yuandian-law MCP server
-- 额外暴露为 yuandian_get_user_balance。每次用户刷新时保存一条快照，用相邻
-- 快照的余额减少量与 CaseBoard 本机累计积分增量对比。

CREATE TABLE IF NOT EXISTS yuandian_balance_snapshots (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    key_fingerprint          TEXT NOT NULL,
    point_balance            INTEGER NOT NULL,
    count_balance            INTEGER NOT NULL DEFAULT 0,
    local_credits_total      INTEGER NOT NULL DEFAULT 0,
    local_api_calls_total    INTEGER NOT NULL DEFAULT 0,
    fetched_at               TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_yuandian_balance_key_id
    ON yuandian_balance_snapshots(key_fingerprint, id DESC);

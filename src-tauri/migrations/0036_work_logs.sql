-- ============================================================================
-- 0028: 工作记录时间轴 · work_logs 表
--
-- 记录律师为案件付出的具体劳动（打电话/写文书/会见/阅卷等）。
-- log_time 存录入时完整 ISO 时间戳，content 存工作内容。
-- 按 log_time DESC, id DESC 排序，确保最新记录在最上方。
-- ============================================================================

CREATE TABLE IF NOT EXISTS work_logs (
    id           TEXT PRIMARY KEY,
    case_id      TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    log_time     TEXT NOT NULL,   -- ISO 8601 完整时间戳(精确到毫秒)
    content      TEXT NOT NULL,   -- 工作内容描述
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_work_logs_case ON work_logs(case_id, log_time DESC);

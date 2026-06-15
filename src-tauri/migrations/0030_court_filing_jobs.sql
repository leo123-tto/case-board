-- 2026-06-15 · 法院一张网在线立案任务表(court_filing_jobs)
-- 记录在线立案任务状态、输出目录、进度和验证码等待状态
CREATE TABLE IF NOT EXISTS court_filing_jobs (
    id              TEXT PRIMARY KEY,
    case_id         TEXT NOT NULL REFERENCES cases(id),
    filing_type     TEXT NOT NULL DEFAULT 'civil',         -- civil / execution
    court_name      TEXT NOT NULL DEFAULT '',
    cookie_account  TEXT,                                    -- 一张网账号(脱敏展示用)
    status          TEXT NOT NULL DEFAULT 'pending',        -- pending / running / waiting_captcha / completed / failed / cancelled
    output_dir      TEXT,
    preview_url     TEXT,                                    -- 成功时的预览页 URL
    progress_json   TEXT,                                    -- 最近一条 progress 事件 JSON(前端实时展示)
    captcha_active  INTEGER NOT NULL DEFAULT 0,             -- 0/1 是否正在等验证码
    error           TEXT,
    timing_json     TEXT,                                    -- {"login_ms":...,"playwright_ms":...,"overall_ms":...}
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_court_filing_jobs_case_id ON court_filing_jobs(case_id);

-- 2026-07-04 · AI 作业与分析陈旧标记基础设施
--
-- 目标:
--   1. 给后续统一 AI job 队列留出本地状态表,先不改变现有同步/后台执行策略。
--   2. 用 documents.extracted_text_hash 追踪“抽取正文”本身的变化。
--   3. 用 cases.analysis_* 标记案件画像/报告是否落后于当前材料集。

CREATE TABLE IF NOT EXISTS ai_jobs (
    id                 TEXT PRIMARY KEY NOT NULL,
    case_id            TEXT,
    kind               TEXT NOT NULL,
    status             TEXT NOT NULL DEFAULT 'queued',
    phase              TEXT,
    progress           REAL NOT NULL DEFAULT 0,
    input_signature    TEXT,
    output_refs_json   TEXT,
    error_sanitized    TEXT,
    provider           TEXT,
    cost_json          TEXT,
    cancellable        INTEGER NOT NULL DEFAULT 0,
    created_at         TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at         TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at        TEXT,
    FOREIGN KEY (case_id) REFERENCES cases(id) ON DELETE CASCADE
);

CREATE INDEX idx_ai_jobs_case_created ON ai_jobs(case_id, created_at DESC);
CREATE INDEX idx_ai_jobs_status_created ON ai_jobs(status, created_at DESC);

ALTER TABLE documents ADD COLUMN extracted_text_hash TEXT;

ALTER TABLE cases ADD COLUMN analysis_input_signature TEXT;
ALTER TABLE cases ADD COLUMN analysis_stale INTEGER NOT NULL DEFAULT 0;
ALTER TABLE cases ADD COLUMN analysis_stale_reason TEXT;

CREATE INDEX idx_cases_analysis_stale ON cases(analysis_stale);

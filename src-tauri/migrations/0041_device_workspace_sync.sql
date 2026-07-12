-- 个人多设备工作区同步（LAN；macOS / Windows 对等）· 2026-07-12
--
-- 只同步 CaseBoard 自己生成或编辑的 Markdown 工作产物；不传案件原文件、OCR 抽取缓存、
-- API key 或整库。远端绝对路径不入协议，接收端一律落到自己的 app_data_dir。

ALTER TABLE cases ADD COLUMN sync_key TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_cases_sync_key
    ON cases(sync_key) WHERE sync_key IS NOT NULL;

-- 每份工作产物的本机同步头。正文仍是 app data 下的 .md 文件。
CREATE TABLE IF NOT EXISTS device_sync_artifacts (
    artifact_id       TEXT PRIMARY KEY,
    case_sync_key     TEXT NOT NULL,
    content_hash      TEXT NOT NULL,
    parent_hash       TEXT,
    revision          INTEGER NOT NULL DEFAULT 1,
    origin_device_id  TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    deleted_at        TEXT
);

CREATE INDEX IF NOT EXISTS idx_device_sync_artifacts_case
    ON device_sync_artifacts(case_sync_key);

-- 尚未在本机匹配到案件的工作产物先安全落入收件箱；以后匹配到案号/案件名再自动归案。
CREATE TABLE IF NOT EXISTS device_sync_inbox (
    artifact_id    TEXT PRIMARY KEY,
    case_sync_key  TEXT NOT NULL,
    case_name      TEXT NOT NULL,
    case_no        TEXT,
    packet_json    TEXT NOT NULL,
    local_path     TEXT NOT NULL,
    received_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS device_sync_state (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

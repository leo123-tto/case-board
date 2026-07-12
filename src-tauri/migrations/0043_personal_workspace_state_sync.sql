-- 个人空间全状态同步 · 2026-07-12
--
-- 0041 只跟踪工作区 Markdown。本迁移新增逻辑业务记录账本和设备名册：
-- - 账本只保存哈希/版本/墓碑，不保存设置值、API Key 或案件正文；
-- - 真正载荷只存在于端到端加密的同步消息中；
-- - 不复制 live SQLite，不写入任何对端绝对路径。

CREATE TABLE IF NOT EXISTS device_sync_records (
    entity_type      TEXT NOT NULL,
    record_id        TEXT NOT NULL,
    content_hash     TEXT NOT NULL,
    parent_hash      TEXT,
    revision         INTEGER NOT NULL DEFAULT 1,
    origin_device_id TEXT NOT NULL,
    updated_at       TEXT NOT NULL,
    deleted_at       TEXT,
    PRIMARY KEY (entity_type, record_id)
);

CREATE INDEX IF NOT EXISTS idx_device_sync_records_active
    ON device_sync_records(entity_type, deleted_at);

CREATE TABLE IF NOT EXISTS device_sync_devices (
    device_id     TEXT PRIMARY KEY,
    device_name   TEXT NOT NULL,
    platform      TEXT NOT NULL,
    first_seen_at TEXT NOT NULL,
    last_seen_at  TEXT NOT NULL,
    revoked_at    TEXT
);

-- 副设备源文件只向主力设备单向归集。本表是本机传输账，不进入业务记录同步。
CREATE TABLE IF NOT EXISTS device_sync_source_files (
    document_id            TEXT PRIMARY KEY,
    local_path             TEXT NOT NULL,
    fingerprint            TEXT NOT NULL,
    content_hash           TEXT NOT NULL,
    size_bytes             INTEGER NOT NULL,
    uploaded_to_primary_at TEXT,
    last_error             TEXT
);

-- 主力设备的断点接收状态。临时文件只在本机 AppData，完成校验后才移入案件源目录。
CREATE TABLE IF NOT EXISTS device_sync_source_inbox (
    document_id      TEXT PRIMARY KEY,
    case_sync_key    TEXT NOT NULL,
    filename         TEXT NOT NULL,
    relative_path    TEXT NOT NULL,
    content_hash     TEXT NOT NULL,
    total_size       INTEGER NOT NULL,
    origin_device_id TEXT NOT NULL,
    origin_name      TEXT NOT NULL,
    temp_path        TEXT NOT NULL,
    received_bytes   INTEGER NOT NULL DEFAULT 0,
    updated_at       TEXT NOT NULL
);

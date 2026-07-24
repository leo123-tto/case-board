-- 2026-07-23 · AI 工作区本机导出偏好。
-- 绝对路径只属于当前设备，不进入个人空间逻辑同步。

CREATE TABLE ai_workspace_local_preferences (
    workspace_id          TEXT PRIMARY KEY,
    preferred_export_dir TEXT NOT NULL CHECK (length(trim(preferred_export_dir)) > 0),
    updated_at            TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (workspace_id) REFERENCES ai_workspaces(id) ON DELETE CASCADE
);

CREATE TABLE ai_workspace_document_exports (
    document_id TEXT NOT NULL,
    format      TEXT NOT NULL CHECK (format IN ('docx', 'html')),
    export_path TEXT NOT NULL CHECK (length(trim(export_path)) > 0),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (document_id, format),
    FOREIGN KEY (document_id) REFERENCES ai_workspace_documents(id) ON DELETE CASCADE
);

-- 2026-07-15 · 非诉 AI 事务工作区。
-- 独立于 cases/documents/chat_messages；原始材料只记录路径，派生文本和文稿由 AppData 管理。

CREATE TABLE ai_workspaces (
    id                   TEXT PRIMARY KEY,
    title                TEXT NOT NULL CHECK (length(trim(title)) > 0),
    description          TEXT,
    is_favorite          INTEGER NOT NULL DEFAULT 0 CHECK (is_favorite IN (0, 1)),
    last_opened_at       TEXT,
    last_document_id     TEXT,
    last_conversation_id TEXT,
    created_at           TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at           TEXT NOT NULL DEFAULT (datetime('now')),
    archived_at          TEXT
);

CREATE INDEX idx_ai_workspaces_active_updated
    ON ai_workspaces(archived_at, updated_at DESC);
CREATE INDEX idx_ai_workspaces_recent
    ON ai_workspaces(archived_at, last_opened_at DESC);

CREATE TABLE ai_workspace_documents (
    id                     TEXT PRIMARY KEY,
    workspace_id           TEXT NOT NULL,
    kind                   TEXT NOT NULL CHECK (kind IN ('source', 'artifact')),
    title                  TEXT NOT NULL CHECK (length(trim(title)) > 0),
    filename               TEXT NOT NULL CHECK (length(trim(filename)) > 0),
    mime_type              TEXT,
    size_bytes             INTEGER,
    source_path            TEXT,
    normalized_source_path TEXT,
    content_path           TEXT,
    extracted_text_path    TEXT,
    extraction_status      TEXT NOT NULL DEFAULT 'queued'
        CHECK (extraction_status IN ('queued', 'processing', 'ready', 'review', 'failed', 'missing', 'not_required')),
    last_error             TEXT,
    missing                INTEGER NOT NULL DEFAULT 0 CHECK (missing IN (0, 1)),
    quality_status         TEXT,
    working_copy_revision  INTEGER NOT NULL DEFAULT 0 CHECK (working_copy_revision >= 0),
    working_copy_hash      TEXT,
    latest_version_no      INTEGER NOT NULL DEFAULT 0 CHECK (latest_version_no >= 0),
    created_at             TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at             TEXT NOT NULL DEFAULT (datetime('now')),
    archived_at            TEXT,
    FOREIGN KEY (workspace_id) REFERENCES ai_workspaces(id) ON DELETE CASCADE,
    CHECK (
        (kind = 'source' AND source_path IS NOT NULL AND normalized_source_path IS NOT NULL)
        OR
        (kind = 'artifact' AND content_path IS NOT NULL)
    )
);

CREATE UNIQUE INDEX idx_ai_workspace_source_path_active
    ON ai_workspace_documents(workspace_id, normalized_source_path)
    WHERE kind = 'source' AND archived_at IS NULL;
CREATE INDEX idx_ai_workspace_documents_active
    ON ai_workspace_documents(workspace_id, kind, archived_at, updated_at DESC);

CREATE TABLE ai_workspace_document_chunks (
    id            TEXT PRIMARY KEY,
    document_id   TEXT NOT NULL,
    ordinal       INTEGER NOT NULL CHECK (ordinal >= 0),
    page_no       INTEGER,
    section_label TEXT,
    content       TEXT NOT NULL,
    content_hash  TEXT NOT NULL,
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (document_id) REFERENCES ai_workspace_documents(id) ON DELETE CASCADE,
    UNIQUE (document_id, ordinal)
);

CREATE INDEX idx_ai_workspace_chunks_document_page
    ON ai_workspace_document_chunks(document_id, page_no, ordinal);

CREATE TABLE ai_workspace_document_versions (
    id                   TEXT PRIMARY KEY,
    document_id          TEXT NOT NULL,
    version_no           INTEGER NOT NULL CHECK (version_no > 0),
    content_md           TEXT NOT NULL,
    created_by           TEXT NOT NULL CHECK (created_by IN ('user', 'ai', 'system')),
    trigger              TEXT NOT NULL,
    change_summary       TEXT NOT NULL DEFAULT '',
    source_snapshot_json TEXT NOT NULL DEFAULT '[]',
    message_id           TEXT,
    created_at           TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (document_id) REFERENCES ai_workspace_documents(id) ON DELETE CASCADE,
    UNIQUE (document_id, version_no)
);

CREATE INDEX idx_ai_workspace_versions_document
    ON ai_workspace_document_versions(document_id, version_no DESC);

CREATE TABLE ai_workspace_conversations (
    id              TEXT PRIMARY KEY,
    workspace_id    TEXT NOT NULL,
    title           TEXT NOT NULL CHECK (length(trim(title)) > 0),
    title_is_manual INTEGER NOT NULL DEFAULT 0 CHECK (title_is_manual IN (0, 1)),
    last_message_at TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now')),
    archived_at     TEXT,
    FOREIGN KEY (workspace_id) REFERENCES ai_workspaces(id) ON DELETE CASCADE
);

CREATE INDEX idx_ai_workspace_conversations_active
    ON ai_workspace_conversations(workspace_id, archived_at, last_message_at DESC, updated_at DESC);

CREATE TABLE ai_workspace_messages (
    id                         TEXT PRIMARY KEY,
    conversation_id            TEXT NOT NULL,
    role                       TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system')),
    content                    TEXT NOT NULL DEFAULT '',
    status                     TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'streaming', 'completed', 'incomplete', 'failed', 'cancelled')),
    attached_document_ids_json TEXT NOT NULL DEFAULT '[]',
    citations_json             TEXT NOT NULL DEFAULT '[]',
    artifact_document_id       TEXT,
    model                      TEXT,
    prompt_tokens              INTEGER,
    completion_tokens          INTEGER,
    latency_ms                 INTEGER,
    error_short                TEXT,
    task_id                    TEXT,
    created_at                 TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at                 TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (conversation_id) REFERENCES ai_workspace_conversations(id) ON DELETE CASCADE,
    FOREIGN KEY (artifact_document_id) REFERENCES ai_workspace_documents(id) ON DELETE SET NULL
);

CREATE INDEX idx_ai_workspace_messages_conversation
    ON ai_workspace_messages(conversation_id, created_at, id);

CREATE TABLE ai_workspace_tasks (
    id                   TEXT PRIMARY KEY,
    workspace_id         TEXT NOT NULL,
    conversation_id      TEXT NOT NULL,
    assistant_message_id TEXT NOT NULL,
    status               TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'streaming', 'completed', 'incomplete', 'failed', 'cancelled')),
    input_json           TEXT NOT NULL DEFAULT '{}',
    tool_calls_json      TEXT NOT NULL DEFAULT '[]',
    error_short          TEXT,
    created_at           TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at           TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at          TEXT,
    FOREIGN KEY (workspace_id) REFERENCES ai_workspaces(id) ON DELETE CASCADE,
    FOREIGN KEY (conversation_id) REFERENCES ai_workspace_conversations(id) ON DELETE CASCADE,
    FOREIGN KEY (assistant_message_id) REFERENCES ai_workspace_messages(id) ON DELETE CASCADE
);

CREATE INDEX idx_ai_workspace_tasks_scope
    ON ai_workspace_tasks(workspace_id, conversation_id, status, updated_at DESC);

CREATE TABLE ai_workspace_document_proposals (
    id                   TEXT PRIMARY KEY,
    workspace_id         TEXT NOT NULL,
    document_id          TEXT NOT NULL,
    conversation_id      TEXT,
    message_id           TEXT,
    base_revision        INTEGER NOT NULL CHECK (base_revision >= 0),
    base_content_hash    TEXT NOT NULL,
    proposed_markdown    TEXT NOT NULL,
    summary              TEXT NOT NULL DEFAULT '',
    source_snapshot_json TEXT NOT NULL DEFAULT '[]',
    status               TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'accepted', 'rejected', 'superseded')),
    resolved_markdown    TEXT,
    created_at           TEXT NOT NULL DEFAULT (datetime('now')),
    resolved_at          TEXT,
    FOREIGN KEY (workspace_id) REFERENCES ai_workspaces(id) ON DELETE CASCADE,
    FOREIGN KEY (document_id) REFERENCES ai_workspace_documents(id) ON DELETE CASCADE,
    FOREIGN KEY (conversation_id) REFERENCES ai_workspace_conversations(id) ON DELETE SET NULL,
    FOREIGN KEY (message_id) REFERENCES ai_workspace_messages(id) ON DELETE SET NULL
);

CREATE INDEX idx_ai_workspace_proposals_document
    ON ai_workspace_document_proposals(workspace_id, document_id, status, created_at DESC);

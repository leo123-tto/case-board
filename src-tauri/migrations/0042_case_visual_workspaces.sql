-- AI 案情可视化工作台：当前工作区、完整修订快照、AI 更新提案。
-- 只保存应用内部结构和来源引用，不复制、不修改案件原文件。

CREATE TABLE case_visual_workspaces (
    id                    TEXT PRIMARY KEY NOT NULL,
    case_id               TEXT NOT NULL UNIQUE,
    schema_version        INTEGER NOT NULL CHECK (schema_version = 1),
    graph_json            TEXT NOT NULL CHECK (json_valid(graph_json)),
    layout_json           TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(layout_json)),
    revision              INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    source_fingerprint    TEXT,
    created_by_message_id TEXT,
    created_at            TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at            TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (case_id) REFERENCES cases(id) ON DELETE CASCADE
);

CREATE INDEX idx_case_visual_workspaces_updated
    ON case_visual_workspaces(updated_at DESC);

CREATE TABLE case_visual_revisions (
    id             TEXT PRIMARY KEY NOT NULL,
    workspace_id   TEXT NOT NULL,
    revision       INTEGER NOT NULL CHECK (revision >= 1),
    base_revision  INTEGER NOT NULL CHECK (base_revision >= 0),
    graph_json     TEXT NOT NULL CHECK (json_valid(graph_json)),
    layout_json    TEXT NOT NULL CHECK (json_valid(layout_json)),
    source         TEXT NOT NULL CHECK (source IN ('ai_initial', 'ai_merge', 'user_edit', 'restore')),
    summary        TEXT NOT NULL DEFAULT '',
    is_layout_only INTEGER NOT NULL DEFAULT 0 CHECK (is_layout_only IN (0, 1)),
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (workspace_id) REFERENCES case_visual_workspaces(id) ON DELETE CASCADE,
    UNIQUE (workspace_id, revision)
);

CREATE INDEX idx_case_visual_revisions_workspace
    ON case_visual_revisions(workspace_id, revision DESC);

CREATE TABLE case_visual_proposals (
    id            TEXT PRIMARY KEY NOT NULL,
    workspace_id  TEXT NOT NULL,
    base_revision INTEGER NOT NULL CHECK (base_revision >= 1),
    patch_json    TEXT NOT NULL CHECK (json_valid(patch_json)),
    summary_json  TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(summary_json)),
    status        TEXT NOT NULL DEFAULT 'pending'
                  CHECK (status IN ('pending', 'accepted', 'rejected', 'stale')),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (workspace_id) REFERENCES case_visual_workspaces(id) ON DELETE CASCADE
);

CREATE INDEX idx_case_visual_proposals_workspace_status
    ON case_visual_proposals(workspace_id, status, created_at DESC);

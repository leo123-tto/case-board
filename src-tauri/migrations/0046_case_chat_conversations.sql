-- 2026-07-19 · 案件 AI 助手多会话。
-- 旧聊天记录按案件归入一个确定性的“历史对话”，不删除任何消息或任务。

CREATE TABLE case_chat_conversations (
    id              TEXT PRIMARY KEY,
    case_id         TEXT NOT NULL,
    title           TEXT NOT NULL CHECK (length(trim(title)) > 0),
    title_is_manual INTEGER NOT NULL DEFAULT 0 CHECK (title_is_manual IN (0, 1)),
    last_message_at TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now')),
    archived_at     TEXT,
    FOREIGN KEY (case_id) REFERENCES cases(id) ON DELETE CASCADE
);

CREATE INDEX idx_case_chat_conversations_active
    ON case_chat_conversations(case_id, archived_at, last_message_at DESC, updated_at DESC);

ALTER TABLE cases ADD COLUMN last_chat_conversation_id TEXT;
ALTER TABLE chat_messages ADD COLUMN conversation_id TEXT;
ALTER TABLE chat_tasks ADD COLUMN conversation_id TEXT;

INSERT INTO case_chat_conversations
    (id, case_id, title, title_is_manual, last_message_at, created_at, updated_at)
SELECT
    'legacy-' || c.id,
    c.id,
    '历史对话',
    1,
    COALESCE(
        (SELECT MAX(m.created_at) FROM chat_messages m WHERE m.case_id = c.id),
        (SELECT MAX(t.started_at) FROM chat_tasks t WHERE t.case_id = c.id)
    ),
    COALESCE(
        (SELECT MIN(m.created_at) FROM chat_messages m WHERE m.case_id = c.id),
        (SELECT MIN(t.started_at) FROM chat_tasks t WHERE t.case_id = c.id),
        datetime('now')
    ),
    datetime('now')
FROM cases c
WHERE EXISTS (SELECT 1 FROM chat_messages m WHERE m.case_id = c.id)
   OR EXISTS (SELECT 1 FROM chat_tasks t WHERE t.case_id = c.id);

UPDATE chat_messages
SET conversation_id = 'legacy-' || case_id
WHERE conversation_id IS NULL;

UPDATE chat_tasks
SET conversation_id = COALESCE(
    (SELECT m.conversation_id FROM chat_messages m WHERE m.id = chat_tasks.message_id),
    'legacy-' || case_id
)
WHERE conversation_id IS NULL;

UPDATE cases
SET last_chat_conversation_id = 'legacy-' || id
WHERE EXISTS (
    SELECT 1 FROM case_chat_conversations c
    WHERE c.id = 'legacy-' || cases.id
);

CREATE INDEX idx_chat_messages_conversation
    ON chat_messages(case_id, conversation_id, created_at, id);
CREATE INDEX idx_chat_tasks_conversation
    ON chat_tasks(case_id, conversation_id, started_at DESC);

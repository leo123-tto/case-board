-- 2026-07-24 · AI 工作区 Word 导出模板偏好。
-- 只属于本机导出记录；不进入设备同步。

ALTER TABLE ai_workspace_document_exports
    ADD COLUMN word_template TEXT
    CHECK (
        word_template IS NULL
        OR (
            format = 'docx'
            AND word_template IN ('editor', 'legal_filing')
        )
    );

UPDATE ai_workspace_document_exports
SET word_template = 'editor'
WHERE format = 'docx' AND word_template IS NULL;

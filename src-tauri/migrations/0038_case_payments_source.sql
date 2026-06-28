-- 2026-06-28 · AI 识别还款保留源文件指针
--
-- 仅存原始文件的只读引用,用于执行详情里核对 AI 自动入账是否准确。
-- 不复制、不移动、不修改用户案件文件。

ALTER TABLE case_payments ADD COLUMN source_document_id TEXT;
ALTER TABLE case_payments ADD COLUMN source_path TEXT;
ALTER TABLE case_payments ADD COLUMN source_filename TEXT;

CREATE INDEX idx_case_payments_source_doc ON case_payments(source_document_id);

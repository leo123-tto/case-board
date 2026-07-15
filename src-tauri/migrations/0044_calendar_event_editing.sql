-- 2026-07-15 · 首页日程支持编辑标题、日期和备注。
-- 已有 calendar_events / case_todos 保留原记录，只追加可空备注列。
ALTER TABLE calendar_events ADD COLUMN note TEXT;
ALTER TABLE case_todos ADD COLUMN note TEXT;

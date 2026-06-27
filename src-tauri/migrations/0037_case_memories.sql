-- 案件 AI 助手记忆:律师确认后的本案长期上下文。
-- 只存 CaseBoard 元数据,不触碰用户原始案件文件。

CREATE TABLE IF NOT EXISTS case_memories (
  id TEXT PRIMARY KEY,
  case_id TEXT NOT NULL,
  content TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT 'manual',
  status TEXT NOT NULL DEFAULT 'active',
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  disabled_at TEXT,
  FOREIGN KEY(case_id) REFERENCES cases(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_case_memories_case_status
  ON case_memories(case_id, status, disabled_at, updated_at);

CREATE TABLE IF NOT EXISTS global_memories (
  id TEXT PRIMARY KEY,
  content TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT 'manual',
  status TEXT NOT NULL DEFAULT 'active',
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  disabled_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_global_memories_status
  ON global_memories(status, disabled_at, updated_at);

CREATE TABLE IF NOT EXISTS memory_events (
  id TEXT PRIMARY KEY,
  case_id TEXT,
  event_type TEXT NOT NULL DEFAULT 'chat_turn',
  user_message_id TEXT,
  assistant_message_id TEXT,
  user_text TEXT NOT NULL,
  assistant_text TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT 'case_chat',
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  FOREIGN KEY(case_id) REFERENCES cases(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS memory_candidates (
  id TEXT PRIMARY KEY,
  event_id TEXT NOT NULL,
  case_id TEXT,
  scope TEXT NOT NULL,
  content TEXT NOT NULL,
  trigger TEXT NOT NULL,
  confidence REAL NOT NULL DEFAULT 0,
  status TEXT NOT NULL DEFAULT 'pending',
  source TEXT NOT NULL DEFAULT 'heuristic',
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  decided_at TEXT,
  decision_reason TEXT,
  FOREIGN KEY(event_id) REFERENCES memory_events(id) ON DELETE CASCADE,
  FOREIGN KEY(case_id) REFERENCES cases(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS memory_evidence (
  id TEXT PRIMARY KEY,
  candidate_id TEXT NOT NULL,
  evidence_type TEXT NOT NULL,
  ref_id TEXT,
  quote TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  FOREIGN KEY(candidate_id) REFERENCES memory_candidates(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_memory_events_case_created
  ON memory_events(case_id, created_at);

CREATE INDEX IF NOT EXISTS idx_memory_candidates_case_status
  ON memory_candidates(case_id, status, updated_at);

CREATE INDEX IF NOT EXISTS idx_memory_candidates_scope_status
  ON memory_candidates(scope, status, updated_at);

CREATE INDEX IF NOT EXISTS idx_memory_evidence_candidate
  ON memory_evidence(candidate_id);

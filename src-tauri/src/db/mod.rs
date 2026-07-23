//! 数据库连接池与 schema migrations。
//!
//! V0.1 用 SQLite + sqlx。数据库文件落在 macOS 标准 app data 目录:
//!   `~/Library/Application Support/CaseBoard/caseboard.db`
//!
//! 启动流程:
//!   1. 拿到 app data dir(`directories` crate 跨平台)
//!   2. 确保目录存在(首次启动)
//!   3. 创建 SqlitePool(`?mode=rwc` 不存在自动建)
//!   4. 跑 migrations(`sqlx::migrate!`)
//!
//! 测试模式可以传 `sqlite::memory:` 跑内存库,不污染本机文件系统。

use std::path::PathBuf;

use directories::ProjectDirs;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

pub mod ai_jobs;
pub mod ai_workspace_chat;
pub mod ai_workspace_documents;
pub mod ai_workspace_proposals;
pub mod ai_workspaces;
pub mod bookmarks;
pub mod calendar_events;
pub mod case_chat_conversations;
pub mod case_instances;
pub mod case_logs;
pub mod case_memories;
pub mod case_visuals;
pub mod cases;
pub mod chat;
pub mod chat_tasks;
pub mod closing_materials;
pub mod contract_drafts;
pub mod contract_preferences;
pub mod court_filing;
pub mod credits;
pub mod document_tags;
pub mod documents;
pub mod lawyer_insights;
pub mod lawyer_profiles;
pub mod metrics;
pub mod payments;
pub mod seed;
pub mod todos;

/// `directories` 用的标识——macOS 上这会拼成 `~/Library/Application Support/CaseBoard/`
const APP_QUALIFIER: &str = "";
const APP_ORG: &str = "";
const APP_NAME: &str = "CaseBoard";

/// 拿到当前操作系统下 CaseBoard 的数据目录路径。
///
/// macOS: `~/Library/Application Support/CaseBoard/`
/// Linux: `~/.local/share/CaseBoard/`
/// Windows: `%APPDATA%\CaseBoard\data\`
pub fn app_data_dir() -> Result<PathBuf, DbError> {
    let proj =
        ProjectDirs::from(APP_QUALIFIER, APP_ORG, APP_NAME).ok_or(DbError::HomeDirNotFound)?;
    Ok(proj.data_dir().to_path_buf())
}

/// 默认数据库文件路径(`<app_data_dir>/caseboard.db`)。
pub fn default_db_path() -> Result<PathBuf, DbError> {
    Ok(app_data_dir()?.join("caseboard.db"))
}

/// 初始化连接池:确保目录存在、连接、跑 migrations。
///
/// `db_path` 可以是真实路径(`PathBuf::from("...caseboard.db")`)或者特殊串:
///   - `:memory:` —— 内存库,测试用
pub async fn init_pool(db_path: &str) -> Result<SqlitePool, DbError> {
    // 如果不是内存库,先确保父目录存在
    if db_path != ":memory:" {
        let path = PathBuf::from(db_path);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| DbError::Io(e.to_string()))?;
        }
    }

    let is_memory = db_path == ":memory:";

    let mut options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .foreign_keys(true);

    // 文件库走 WAL(并发友好),内存库不能用 WAL
    if !is_memory {
        options = options.journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
    }

    // 内存库每个连接是独立的 SQLite 实例 → 必须只用 1 个连接,否则
    // migration 跑完表只在那一个连接里,其他连接看不到
    let max_connections = if is_memory { 1 } else { 5 };

    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await
        .map_err(|e| DbError::Connect(e.to_string()))?;

    // 2026-06-15:跑迁移前先对齐 _sqlx_migrations 校验值,根治「migration N ... has been modified」
    // 启动崩溃。病根 = 不同发布分支对**同一批已发布迁移**做了注释与示例文字改动，
    // SQL 一字未改但 SHA-384 变了 → 老用户 DB 里
    // 存的旧校验值对不上新二进制内嵌值 → sqlx 启动中止(release 是 panic=abort,直接闪退)。
    // 详见 docs/反馈问题排查-2026-06-15.md。
    reconcile_migration_checksums(&pool).await?;

    // 2026-06-18(整合外部 PR #13 @zzf516988659-del):容忍「DB 里已 applied 但本二进制 resolved
    // 里没有」的迁移行(sqlx 0.8 默认遇此 panic)。病根 = 跨 fork/跨仓发布节奏漂移:用户先装了某
    // fork binary(内嵌更多迁移、apply 过)、再装主仓 binary(内嵌较少)→ 启动报「migration N
    // previously applied but missing」直接闪退。已 applied 的不会重跑,schema 不受影响。
    // 配合上面的 reconcile_migration_checksums,是跨仓发布漂移的最后一道兜底。
    sqlx::migrate!("./migrations")
        .set_ignore_missing(true)
        .run(&pool)
        .await
        .map_err(|e| DbError::Migrate(e.to_string()))?;

    // 少数装过 0045/0046 中间构建的数据库已经登记了迁移版本，但实际 schema 没落全。
    // SQLx 会把这些版本视为已应用而不再执行，因此必须按真实 schema 条件修复。
    repair_case_chat_conversation_schema(&pool).await?;

    Ok(pool)
}

async fn table_has_column(pool: &SqlitePool, table: &str, column: &str) -> Result<bool, DbError> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2")
            .bind(table)
            .bind(column)
            .fetch_one(pool)
            .await
            .map_err(|e| DbError::Migrate(e.to_string()))?;
    Ok(count > 0)
}

async fn add_column_if_missing(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    ddl: &str,
) -> Result<bool, DbError> {
    if table_has_column(pool, table, column).await? {
        return Ok(false);
    }
    sqlx::query(ddl)
        .execute(pool)
        .await
        .map_err(|e| DbError::Migrate(e.to_string()))?;
    Ok(true)
}

/// 修复“0046 已登记但案件多会话 schema 未实际落下”的中间构建数据库。
///
/// SQLite 不支持可移植的 `ADD COLUMN IF NOT EXISTS`，所以这里先检查真实 schema，
/// 再逐列补齐。每一步都可安全重入；若进程在中途退出，下次启动会继续补剩余部分。
async fn repair_case_chat_conversation_schema(pool: &SqlitePool) -> Result<(), DbError> {
    sqlx::raw_sql(
        r#"CREATE TABLE IF NOT EXISTS case_chat_conversations (
             id TEXT PRIMARY KEY,
             case_id TEXT NOT NULL,
             title TEXT NOT NULL CHECK (length(trim(title)) > 0),
             title_is_manual INTEGER NOT NULL DEFAULT 0 CHECK (title_is_manual IN (0, 1)),
             last_message_at TEXT,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now')),
             archived_at TEXT,
             FOREIGN KEY (case_id) REFERENCES cases(id) ON DELETE CASCADE
           );"#,
    )
    .execute(pool)
    .await
    .map_err(|e| DbError::Migrate(e.to_string()))?;

    let mut repaired = false;
    repaired |= add_column_if_missing(
        pool,
        "cases",
        "last_chat_conversation_id",
        "ALTER TABLE cases ADD COLUMN last_chat_conversation_id TEXT",
    )
    .await?;
    repaired |= add_column_if_missing(
        pool,
        "chat_messages",
        "conversation_id",
        "ALTER TABLE chat_messages ADD COLUMN conversation_id TEXT",
    )
    .await?;
    repaired |= add_column_if_missing(
        pool,
        "chat_tasks",
        "conversation_id",
        "ALTER TABLE chat_tasks ADD COLUMN conversation_id TEXT",
    )
    .await?;

    sqlx::raw_sql(
        r#"CREATE INDEX IF NOT EXISTS idx_case_chat_conversations_active
             ON case_chat_conversations(case_id, archived_at, last_message_at DESC, updated_at DESC);
           INSERT OR IGNORE INTO case_chat_conversations
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
           WHERE EXISTS (
                   SELECT 1 FROM chat_messages m
                   WHERE m.case_id = c.id AND m.conversation_id IS NULL
                 )
              OR EXISTS (
                   SELECT 1 FROM chat_tasks t
                   WHERE t.case_id = c.id AND t.conversation_id IS NULL
                 );
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
           WHERE last_chat_conversation_id IS NULL
             AND EXISTS (
               SELECT 1 FROM case_chat_conversations c
               WHERE c.id = 'legacy-' || cases.id
             );
           CREATE INDEX IF NOT EXISTS idx_chat_messages_conversation
             ON chat_messages(case_id, conversation_id, created_at, id);
           CREATE INDEX IF NOT EXISTS idx_chat_tasks_conversation
             ON chat_tasks(case_id, conversation_id, started_at DESC);"#,
    )
    .execute(pool)
    .await
    .map_err(|e| DbError::Migrate(e.to_string()))?;

    if repaired {
        crate::dlog!("[db] 已按真实 schema 补齐案件多会话迁移 0046");
    }
    Ok(())
}

/// 把已存在的 `_sqlx_migrations.checksum` 对齐到本二进制内嵌的迁移校验值。
///
/// 仅当该表已存在(= 非全新库,跑过至少一次迁移)时才动;逐条只在校验值**不同**时更新并 dlog。
/// SQL 一字未改(只是注释/项目名漂移),已应用的迁移 sqlx 本就不会重跑 —— 对齐校验值不改变任何
/// 已执行的 SQL、不动数据,只是消掉「文件被动过」这道与双轨发布天然冲突的 tripwire。
async fn reconcile_migration_checksums(pool: &SqlitePool) -> Result<(), DbError> {
    // 只允许对齐 0046 及以前已经确认存在「私仓/公开版注释漂移」的历史迁移。
    // 若 0046 来自中间构建且真实 schema 不完整，迁移完成后的条件修复会补齐。
    // 新迁移若 checksum 不同必须让 sqlx fail-loud，避免再次出现“记录已应用、schema 未落下”。
    const LAST_KNOWN_COMMENT_DRIFT_VERSION: i64 = 46;

    // 全新库还没这张表 → 无需对齐(后续 migrate 会正常建表并全量应用)。
    let table_exists: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| DbError::Migrate(e.to_string()))?;
    if table_exists.is_none() {
        return Ok(());
    }

    for m in sqlx::migrate!("./migrations").iter() {
        if m.version > LAST_KNOWN_COMMENT_DRIFT_VERSION {
            continue;
        }
        let embedded: &[u8] = &m.checksum;
        let stored: Option<(Vec<u8>,)> =
            sqlx::query_as("SELECT checksum FROM _sqlx_migrations WHERE version = ?1")
                .bind(m.version)
                .fetch_optional(pool)
                .await
                .map_err(|e| DbError::Migrate(e.to_string()))?;
        if let Some((stored,)) = stored {
            if stored.as_slice() != embedded {
                sqlx::query("UPDATE _sqlx_migrations SET checksum = ?1 WHERE version = ?2")
                    .bind(embedded)
                    .bind(m.version)
                    .execute(pool)
                    .await
                    .map_err(|e| DbError::Migrate(e.to_string()))?;
                crate::dlog!(
                    "[db] 迁移 {} 校验值与内嵌不一致,已对齐(注释漂移,SQL 未变)",
                    m.version
                );
            }
        }
    }
    Ok(())
}

/// 数据库相关错误。映射到前端友好的字符串。
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("找不到用户主目录")]
    HomeDirNotFound,
    #[error("IO 错误: {0}")]
    Io(String),
    #[error("数据库连接失败: {0}")]
    Connect(String),
    #[error("数据库迁移失败: {0}")]
    Migrate(String),
}

impl serde::Serialize for DbError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

// ============================================================================
// 测试
// ============================================================================

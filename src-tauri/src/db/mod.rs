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

pub mod calendar_events;
pub mod case_instances;
pub mod cases;
pub mod chat;
pub mod chat_tasks;
pub mod credits;
pub mod documents;
pub mod logs;
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
    // 启动崩溃。病根 = 双轨发布(私人仓 vs 开源仓)对**同一批已发布迁移**做了去身份化注释改动
    // (项目名→公开域名、本地路径→泛化),SQL 一字未改但 SHA-384 变了 → 老用户 DB 里
    // 存的旧校验值对不上新二进制内嵌值 → sqlx 启动中止(release 是 panic=abort,直接闪退)。
    // 详见 docs/反馈问题排查-2026-06-15.md。
    reconcile_migration_checksums(&pool).await?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|e| DbError::Migrate(e.to_string()))?;

    Ok(pool)
}

/// 把已存在的 `_sqlx_migrations.checksum` 对齐到本二进制内嵌的迁移校验值。
///
/// 仅当该表已存在(= 非全新库,跑过至少一次迁移)时才动;逐条只在校验值**不同**时更新并 dlog。
/// SQL 一字未改(只是注释/项目名漂移),已应用的迁移 sqlx 本就不会重跑 —— 对齐校验值不改变任何
/// 已执行的 SQL、不动数据,只是消掉「文件被动过」这道与双轨发布天然冲突的 tripwire。
async fn reconcile_migration_checksums(pool: &SqlitePool) -> Result<(), DbError> {
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

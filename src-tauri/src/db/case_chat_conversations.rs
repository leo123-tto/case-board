use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq, Eq)]
pub struct CaseChatConversation {
    pub id: String,
    pub case_id: String,
    pub title: String,
    pub title_is_manual: i64,
    pub last_message_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

pub async fn list_conversations(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<Vec<CaseChatConversation>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM case_chat_conversations \
         WHERE case_id = ? AND archived_at IS NULL \
         ORDER BY COALESCE(last_message_at, updated_at) DESC, created_at DESC, id DESC",
    )
    .bind(case_id)
    .fetch_all(pool)
    .await
}

pub async fn get_conversation(
    pool: &SqlitePool,
    case_id: &str,
    conversation_id: &str,
) -> Result<Option<CaseChatConversation>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM case_chat_conversations \
         WHERE id = ? AND case_id = ? AND archived_at IS NULL",
    )
    .bind(conversation_id)
    .bind(case_id)
    .fetch_optional(pool)
    .await
}

pub async fn create_conversation(
    pool: &SqlitePool,
    case_id: &str,
    title: Option<&str>,
) -> Result<CaseChatConversation, sqlx::Error> {
    let title = title.map(str::trim).filter(|value| !value.is_empty());
    let id = uuid::Uuid::new_v4().to_string();
    let mut tx = pool.begin().await?;
    let case_exists: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM cases WHERE id = ?")
        .bind(case_id)
        .fetch_optional(&mut *tx)
        .await?;
    if case_exists.is_none() {
        return Err(sqlx::Error::RowNotFound);
    }
    sqlx::query(
        "INSERT INTO case_chat_conversations (id, case_id, title, title_is_manual) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(case_id)
    .bind(title.unwrap_or("新对话"))
    .bind(i64::from(title.is_some()))
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE cases SET last_chat_conversation_id = ? WHERE id = ?")
        .bind(&id)
        .bind(case_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    get_conversation(pool, case_id, &id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn ensure_conversation(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<CaseChatConversation, sqlx::Error> {
    let selected: Option<String> =
        sqlx::query_scalar("SELECT last_chat_conversation_id FROM cases WHERE id = ?")
            .bind(case_id)
            .fetch_optional(pool)
            .await?
            .flatten();
    if let Some(id) = selected {
        if let Some(conversation) = get_conversation(pool, case_id, &id).await? {
            return Ok(conversation);
        }
    }
    if let Some(conversation) = list_conversations(pool, case_id).await?.into_iter().next() {
        select_conversation(pool, case_id, &conversation.id).await?;
        return Ok(conversation);
    }
    create_conversation(pool, case_id, None).await
}

pub async fn select_conversation(
    pool: &SqlitePool,
    case_id: &str,
    conversation_id: &str,
) -> Result<(), sqlx::Error> {
    let result = sqlx::query(
        "UPDATE cases SET last_chat_conversation_id = ? \
         WHERE id = ? AND EXISTS ( \
           SELECT 1 FROM case_chat_conversations c \
           WHERE c.id = ? AND c.case_id = cases.id AND c.archived_at IS NULL \
         )",
    )
    .bind(conversation_id)
    .bind(case_id)
    .bind(conversation_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

pub async fn rename_conversation(
    pool: &SqlitePool,
    case_id: &str,
    conversation_id: &str,
    title: &str,
) -> Result<CaseChatConversation, sqlx::Error> {
    let title = title.trim();
    if title.is_empty() {
        return Err(sqlx::Error::Protocol("对话名称不能为空".into()));
    }
    let result = sqlx::query(
        "UPDATE case_chat_conversations SET title = ?, title_is_manual = 1, \
         updated_at = datetime('now') \
         WHERE id = ? AND case_id = ? AND archived_at IS NULL",
    )
    .bind(title)
    .bind(conversation_id)
    .bind(case_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    get_conversation(pool, case_id, conversation_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn touch_after_user_message(
    pool: &SqlitePool,
    case_id: &str,
    conversation_id: &str,
    user_message: &str,
) -> Result<(), sqlx::Error> {
    let title: String = user_message.trim().chars().take(28).collect();
    let title = if title.is_empty() {
        "新对话"
    } else {
        &title
    };
    let result = sqlx::query(
        "UPDATE case_chat_conversations SET \
           title = CASE WHEN title_is_manual = 0 AND title = '新对话' THEN ? ELSE title END, \
           last_message_at = datetime('now'), updated_at = datetime('now') \
         WHERE id = ? AND case_id = ? AND archived_at IS NULL",
    )
    .bind(title)
    .bind(conversation_id)
    .bind(case_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    select_conversation(pool, case_id, conversation_id).await
}

pub async fn archive_conversation(
    pool: &SqlitePool,
    case_id: &str,
    conversation_id: &str,
) -> Result<CaseChatConversation, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let result = sqlx::query(
        "UPDATE case_chat_conversations SET archived_at = datetime('now'), \
         updated_at = datetime('now') \
         WHERE id = ? AND case_id = ? AND archived_at IS NULL",
    )
    .bind(conversation_id)
    .bind(case_id)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }

    let next: Option<CaseChatConversation> = sqlx::query_as(
        "SELECT * FROM case_chat_conversations \
         WHERE case_id = ? AND archived_at IS NULL \
         ORDER BY COALESCE(last_message_at, updated_at) DESC, created_at DESC LIMIT 1",
    )
    .bind(case_id)
    .fetch_optional(&mut *tx)
    .await?;
    let selected = if let Some(next) = next {
        next
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO case_chat_conversations (id, case_id, title) VALUES (?, ?, '新对话')",
        )
        .bind(&id)
        .bind(case_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query_as("SELECT * FROM case_chat_conversations WHERE id = ?")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?
    };
    sqlx::query("UPDATE cases SET last_chat_conversation_id = ? WHERE id = ?")
        .bind(&selected.id)
        .bind(case_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(selected)
}

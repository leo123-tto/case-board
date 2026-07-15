//! 独立日历日程(2026-06-14 · calendar_events 表)
//!
//! 作者反馈:日历某天右键「添加日程」,自由写一条提醒/事件,可删除。
//! **不绑定案件**(纯个人提醒,像普通 To-Do),只挂在某个日期上 —— 区别于绑案件的 case_todos。
//! 首页日程日历显示 = 案件自动抽取 + 带日期的待办 + 这里的独立日程。

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CalendarEvent {
    pub id: String,
    pub date: String, // "YYYY-MM-DD"
    pub title: String,
    pub created_at: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewCalendarEvent {
    pub date: String,
    pub title: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateCalendarEvent {
    pub date: String,
    pub title: String,
    pub note: Option<String>,
}

pub async fn add(pool: &SqlitePool, e: NewCalendarEvent) -> Result<CalendarEvent, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO calendar_events (id, date, title) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(e.date.trim())
        .bind(e.title.trim())
        .execute(pool)
        .await?;

    sqlx::query_as::<_, CalendarEvent>("SELECT * FROM calendar_events WHERE id = ?")
        .bind(&id)
        .fetch_one(pool)
        .await
}

/// 列出全部独立日程(按日期升序,首页日历用)。
pub async fn list_all(pool: &SqlitePool) -> Result<Vec<CalendarEvent>, sqlx::Error> {
    sqlx::query_as::<_, CalendarEvent>(
        "SELECT * FROM calendar_events ORDER BY date ASC, created_at ASC",
    )
    .fetch_all(pool)
    .await
}

pub async fn update(
    pool: &SqlitePool,
    id: &str,
    upd: UpdateCalendarEvent,
) -> Result<CalendarEvent, sqlx::Error> {
    let date = upd.date.trim();
    if NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
        return Err(sqlx::Error::Protocol(
            "日程日期不是有效的 YYYY-MM-DD".into(),
        ));
    }
    let title = upd.title.trim();
    if title.is_empty() {
        return Err(sqlx::Error::Protocol("日程名称不能为空".into()));
    }
    let note = upd.note.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let result =
        sqlx::query("UPDATE calendar_events SET date = ?, title = ?, note = ? WHERE id = ?")
            .bind(date)
            .bind(title)
            .bind(note)
            .bind(id)
            .execute(pool)
            .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    sqlx::query_as::<_, CalendarEvent>("SELECT * FROM calendar_events WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<u64, sqlx::Error> {
    let r = sqlx::query("DELETE FROM calendar_events WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

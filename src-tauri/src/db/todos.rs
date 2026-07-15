//! 案件待办清单(2026-06-13 · case_todos 表)
//!
//! 反馈人「胡彬律师」:每个案件一个手动输入的待办清单,案件详情页增删 + 打钩完成,
//! 首页"待办汇总"跨案件显示未完成项。仿滴答清单 —— 打钩 = done=1(默认隐藏,**不删行**)。

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Todo {
    pub id: String,
    pub case_id: String,
    pub title: String,
    pub done: i64, // 0=未完成 1=已完成
    pub done_at: Option<String>,
    /// 2026-06-14:可选"重要日期"(ISO "YYYY-MM-DD")。Some → 该待办汇入首页日程日历。
    pub due_date: Option<String>,
    pub note: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewTodo {
    pub case_id: String,
    pub title: String,
    #[serde(default)]
    pub due_date: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateTodo {
    pub title: Option<String>,
    pub done: Option<i64>,
    /// Some("YYYY-MM-DD") 改日期;Some("") 清空日期;None 不动。
    #[serde(default)]
    pub due_date: Option<String>,
    /// Some(text) 改备注；Some("") 清空；None 不动。
    #[serde(default)]
    pub note: Option<String>,
}

/// 跨案件未完成待办(首页汇总用)—— 扁平结构带 case_name,
/// 不用 (Todo, String) tuple(sqlx tuple 按序号读、Todo 按列名读,二者不兼容)。
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct OpenTodoRow {
    pub id: String,
    pub case_id: String,
    pub case_name: String,
    pub title: String,
    pub due_date: Option<String>,
    pub note: Option<String>,
    pub created_at: String,
}

pub async fn add(pool: &SqlitePool, t: NewTodo) -> Result<Todo, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    // 空字符串日期归一成 NULL(前端没填 / 清空都送空串)。
    let due = t.due_date.as_deref().filter(|s| !s.trim().is_empty());
    sqlx::query("INSERT INTO case_todos (id, case_id, title, due_date) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(&t.case_id)
        .bind(&t.title)
        .bind(due)
        .execute(pool)
        .await?;

    sqlx::query_as::<_, Todo>("SELECT * FROM case_todos WHERE id = ?")
        .bind(&id)
        .fetch_one(pool)
        .await
}

/// 列出某案件全部待办(未完成在前,各自按创建倒序)。
pub async fn list_by_case(pool: &SqlitePool, case_id: &str) -> Result<Vec<Todo>, sqlx::Error> {
    sqlx::query_as::<_, Todo>(
        "SELECT * FROM case_todos WHERE case_id = ? ORDER BY done ASC, created_at DESC",
    )
    .bind(case_id)
    .fetch_all(pool)
    .await
}

/// 跨案件所有未完成待办(首页汇总),按案件分组、组内创建倒序。
pub async fn list_open(pool: &SqlitePool) -> Result<Vec<OpenTodoRow>, sqlx::Error> {
    sqlx::query_as::<_, OpenTodoRow>(
        "SELECT t.id, t.case_id, c.name AS case_name, t.title, t.due_date, t.note, t.created_at \
         FROM case_todos t JOIN cases c ON t.case_id = c.id \
         WHERE t.done = 0 \
         ORDER BY c.name ASC, t.created_at DESC",
    )
    .fetch_all(pool)
    .await
}

/// 更新待办:改标题 / 打钩或取消打钩。done=1 时写 done_at,取消时清空。
pub async fn update(pool: &SqlitePool, id: &str, upd: &UpdateTodo) -> Result<u64, sqlx::Error> {
    // due_date:None=不动;""=清空(NULL);"YYYY-MM-DD"=设置。用 CASE 区分三态。
    let due_changed = upd.due_date.is_some();
    let due_val = upd.due_date.as_deref().filter(|s| !s.trim().is_empty());
    let note_changed = upd.note.is_some();
    let note_val = upd.note.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let r = sqlx::query(
        "UPDATE case_todos SET \
         title = COALESCE(?, title), \
         done = COALESCE(?, done), \
         done_at = CASE \
             WHEN ? = 1 THEN datetime('now') \
             WHEN ? = 0 THEN NULL \
             ELSE done_at END, \
         due_date = CASE WHEN ? THEN ? ELSE due_date END, \
         note = CASE WHEN ? THEN ? ELSE note END, \
         updated_at = datetime('now') \
         WHERE id = ?",
    )
    .bind(&upd.title)
    .bind(upd.done)
    .bind(upd.done)
    .bind(upd.done)
    .bind(due_changed)
    .bind(due_val)
    .bind(note_changed)
    .bind(note_val)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(r.rows_affected())
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<u64, sqlx::Error> {
    let r = sqlx::query("DELETE FROM case_todos WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}

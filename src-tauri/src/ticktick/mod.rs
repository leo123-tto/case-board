//! 滴答清单(dida365 / TickTick)双向同步 —— 编排层 + dispatch。
//!
//! **公开功能**(开源版可用)。鉴权走用户在滴答设置里生成的「API 口令」(dida365 →
//! 账户与安全 → API 口令,dp_ 前缀的个人访问令牌,免费用户也可用),直接当 Bearer
//! token 打 `/open/v1/` 接口 —— 不内置任何作者凭证,免注册开发者应用、免 OAuth 授权、
//! 免回调服务器、免 token 刷新。前端经单一命令 `ticktick_call`(action + payload)进来。
//!
//! 「我的待办镜像」:维护一份滴答某清单(默认收件箱)的本地镜像,完整双向、带完成状态,
//! 在设置页管理、首页展示;**不碰** 案件待办(case_todos)。**cutoff**:首次同步建基线,
//! 只拉取之后新建的远端任务,手机历史积压不会一锅端。
//! 存储:`<app_data_dir>/ticktick_sync.json`(不进 git)；个人空间启用时作为加密记录同步，
//! 后台自动拉取只由主力设备执行，副设备仍可手动同步。

pub mod client;
pub mod state;

use serde::Serialize;
use serde_json::Value;
use state::{now_ms, MirrorItem, TickTickState, TickTickTokens};
use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;
use tauri::AppHandle;
use tokio::sync::Mutex;

/// 全局串行化:自动同步后台任务、各 dispatch 调用都读改写同一个 JSON 文件,加锁防竞争。
pub(crate) fn state_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// 前端唯一入口:所有滴答操作经此命令按 action 分发。
#[tauri::command]
pub async fn ticktick_call(
    app: tauri::AppHandle,
    action: String,
    payload: Value,
) -> Result<Value, String> {
    dispatch(&app, &action, payload).await
}

/// 按 action 分发。
pub async fn dispatch(app: &AppHandle, action: &str, payload: Value) -> Result<Value, String> {
    match action {
        "connectToken" => connect_token(app, payload).await,
        "status" => status(app).await,
        "listProjects" => list_projects(app).await,
        "setProject" => set_project(app, payload).await,
        "clearProject" => clear_project(app).await,
        "setAutoSync" => set_auto_sync(app, payload).await,
        "syncNow" => sync_now(app).await,
        "listItems" => list_items(app).await,
        "addItem" => add_item(app, payload).await,
        "toggleItem" => toggle_item(app, payload).await,
        "deleteItem" => delete_item(app, payload).await,
        "disconnect" => disconnect(app).await,
        other => Err(format!("未知 action:{other}")),
    }
}

/// 启动时挂后台自动同步:每 60s 一次(连接 + 开了自动同步 + 选了清单才真同步)。
/// 公开函数,从 `lib.rs` setup 直接调用。
/// 用 `tauri::async_runtime::spawn`(不是 `tokio::spawn`)—— setup 阶段 tokio reactor 还没就绪。
pub fn spawn_auto_sync(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            auto_sync_tick(&app).await;
        }
    });
}

/// 切回 App(窗口 focus)时触发一次同步。公开函数,从 `lib.rs` 窗口事件直接调用。
pub fn sync_on_focus(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        auto_sync_tick(&app).await;
    });
}

/// 自动同步一拍:仅在已连接 + 开了自动同步 + 选了清单时跑,失败静默(错误进 last_error)。
async fn auto_sync_tick(app: &AppHandle) {
    if !crate::device_sync::is_automation_owner() {
        return;
    }
    let go = {
        let _g = state_lock().lock().await;
        match state::load(app) {
            Ok(st) => st.connected() && st.config.auto_sync && st.config.project_id.is_some(),
            Err(_) => false,
        }
    };
    if go {
        let _ = sync_now(app).await;
    }
}

async fn set_auto_sync(app: &AppHandle, payload: Value) -> Result<Value, String> {
    let on = payload.get("on").and_then(|v| v.as_bool()).ok_or("缺 on")?;
    let _g = state_lock().lock().await;
    let mut st = state::load(app)?;
    st.config.auto_sync = on;
    state::save(app, &st)?;
    Ok(serde_json::json!({ "ok": true }))
}

fn ps(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusOut {
    connected: bool,
    project_id: Option<String>,
    project_name: Option<String>,
    cutoff_ms: i64,
    last_sync_ms: i64,
    item_count: usize,
    auto_sync: bool,
    last_error: Option<String>,
}

async fn status(app: &AppHandle) -> Result<Value, String> {
    let _g = state_lock().lock().await;
    let st = state::load(app)?;
    let out = StatusOut {
        connected: st.connected(),
        project_id: st.config.project_id.clone(),
        project_name: st.config.project_name.clone(),
        cutoff_ms: st.sync_enabled_at_ms,
        last_sync_ms: st.last_sync_ms,
        item_count: st.items.iter().filter(|i| !i.deleted).count(),
        auto_sync: st.config.auto_sync,
        last_error: st.last_error.clone(),
    };
    serde_json::to_value(out).map_err(|e| e.to_string())
}

/// 用「API 口令」连接(dida365 设置 → 账户与安全 → API 口令,dp_ 前缀个人访问令牌)。
/// 同步校验:用口令只读拉清单列表,通过(200)才落库;失败透传真错、不存任何东西。
/// 校验通过后:口令存进 access_token(不过期、无 refresh),记 cutoff,探测收件箱当默认清单。
async fn connect_token(app: &AppHandle, payload: Value) -> Result<Value, String> {
    let token = ps(&payload, "token").unwrap_or_default().trim().to_string();
    if token.is_empty() {
        return Err("请先粘贴 API 口令".to_string());
    }
    let server = ps(&payload, "server").unwrap_or_else(|| "dida365".to_string());
    let api_base = if server == "ticktick" {
        "https://api.ticktick.com"
    } else {
        "https://api.dida365.com"
    };

    // 先校验口令:用它只读拉清单列表,200 才算有效。失败不落库,透传真错(见已知坑 8)。
    let probe_cfg = state::TickTickConfig {
        api_base: api_base.to_string(),
        ..Default::default()
    };
    client::list_projects(&probe_cfg, &token)
        .await
        .map_err(|e| format!("API 口令无效或无权限(验证失败):{e}"))?;

    // 校验通过 → 落库。
    let _g = state_lock().lock().await;
    let mut st = state::load(app)?;
    st.config.api_base = api_base.to_string();
    st.tokens = TickTickTokens {
        access_token: Some(token.clone()),
        refresh_token: None,
        expires_at_ms: 0,
    };
    if st.sync_enabled_at_ms == 0 {
        st.sync_enabled_at_ms = now_ms(); // cutoff:连接的时间点
    }
    st.last_error = None;
    // 默认同步目标 = 收件箱(最常用;用户可在界面切换)。探测失败则留空,走选清单界面。
    if st.config.project_id.is_none() {
        if let Ok(inbox) = client::discover_inbox_id(&st.config, &token).await {
            st.config.project_id = Some(inbox);
            st.config.project_name = Some("📥 收件箱".to_string());
        }
    }
    state::save(app, &st)?;
    Ok(serde_json::json!({ "ok": true }))
}

async fn disconnect(app: &AppHandle) -> Result<Value, String> {
    let _g = state_lock().lock().await;
    let mut st = state::load(app)?;
    st.tokens = Default::default();
    st.sync_enabled_at_ms = 0;
    st.baseline_captured = false;
    st.baseline_ids.clear();
    st.last_sync_ms = 0;
    st.last_error = None;
    st.items.clear();
    st.config.project_id = None;
    st.config.project_name = None;
    state::save(app, &st)?;
    Ok(serde_json::json!({ "ok": true }))
}

async fn set_project(app: &AppHandle, payload: Value) -> Result<Value, String> {
    let raw_pid = ps(&payload, "projectId").ok_or("缺 projectId")?;
    let pname = ps(&payload, "projectName");
    let _g = state_lock().lock().await;
    let mut st = state::load(app)?;

    // 「收件箱」是合成项:滴答接口不返回它,选中时探测出真实 inbox id 存下。
    let (resolved_id, resolved_name) = if raw_pid == "__inbox__" {
        let token = ensure_token(&st)?;
        let inbox = client::discover_inbox_id(&st.config, &token).await?;
        (inbox, Some("📥 收件箱".to_string()))
    } else {
        (raw_pid, pname)
    };

    // 换了清单 → 清空旧镜像与基线(新清单重新建基线)。
    if st.config.project_id.as_deref() != Some(resolved_id.as_str()) {
        st.items.clear();
        st.baseline_captured = false;
        st.baseline_ids.clear();
        st.last_sync_ms = 0;
    }
    st.config.project_id = Some(resolved_id);
    st.config.project_name = resolved_name;
    state::save(app, &st)?;
    Ok(serde_json::json!({ "ok": true }))
}

/// 回到「选清单」界面(不断开登录),用于换同步目标清单。
async fn clear_project(app: &AppHandle) -> Result<Value, String> {
    let _g = state_lock().lock().await;
    let mut st = state::load(app)?;
    st.config.project_id = None;
    st.config.project_name = None;
    state::save(app, &st)?;
    Ok(serde_json::json!({ "ok": true }))
}

async fn list_projects(app: &AppHandle) -> Result<Value, String> {
    let _g = state_lock().lock().await;
    let st = state::load(app)?;
    let token = ensure_token(&st)?;
    let projects = client::list_projects(&st.config, &token).await?;
    // 合成「收件箱」置顶(滴答 /project 不返回它,但它是最常用的快速收集箱)。
    let mut out: Vec<Value> =
        vec![serde_json::json!({ "id": "__inbox__", "name": "📥 收件箱(Inbox)" })];
    out.extend(
        projects
            .into_iter()
            .map(|p| serde_json::json!({ "id": p.id, "name": p.name })),
    );
    Ok(Value::Array(out))
}

async fn list_items(app: &AppHandle) -> Result<Value, String> {
    let _g = state_lock().lock().await;
    let st = state::load(app)?;
    let mut items: Vec<&MirrorItem> = st.items.iter().filter(|i| !i.deleted).collect();
    items.sort_by(|a, b| {
        a.done
            .cmp(&b.done)
            .then(b.updated_at_ms.cmp(&a.updated_at_ms))
    });
    serde_json::to_value(items).map_err(|e| e.to_string())
}

async fn add_item(app: &AppHandle, payload: Value) -> Result<Value, String> {
    let title = ps(&payload, "title").unwrap_or_default();
    if title.trim().is_empty() {
        return Err("待办内容不能为空".to_string());
    }
    // 不填日期 → 默认「今天」(本地时区)。滴答「今天」视图只显示有日期的任务,无日期的
    // 只躺在收件箱里;而手机 App 默认看的是「今天」→ 会看不到看板加的待办。默认今天后,
    // 加的待办直接出现在手机「今天」页(过了今天未完成会变逾期、仍留在「今天」,不丢)。
    let due = ps(&payload, "due")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| Some(chrono::Local::now().format("%Y-%m-%d").to_string()));
    let now = now_ms();
    let item = MirrorItem {
        id: uuid::Uuid::new_v4().to_string(),
        ticktick_id: None,
        title: title.trim().to_string(),
        done: false,
        due,
        created_at_ms: now,
        updated_at_ms: now,
        deleted: false,
        dirty: true,
    };
    let _g = state_lock().lock().await;
    let mut st = state::load(app)?;
    st.items.push(item.clone());
    state::save(app, &st)?;
    serde_json::to_value(item).map_err(|e| e.to_string())
}

async fn toggle_item(app: &AppHandle, payload: Value) -> Result<Value, String> {
    let id = ps(&payload, "id").ok_or("缺 id")?;
    let done = payload
        .get("done")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let _g = state_lock().lock().await;
    let mut st = state::load(app)?;
    let item = st
        .items
        .iter_mut()
        .find(|i| i.id == id)
        .ok_or("待办不存在")?;
    item.done = done;
    item.updated_at_ms = now_ms();
    item.dirty = true;
    state::save(app, &st)?;
    Ok(serde_json::json!({ "ok": true }))
}

async fn delete_item(app: &AppHandle, payload: Value) -> Result<Value, String> {
    let id = ps(&payload, "id").ok_or("缺 id")?;
    let _g = state_lock().lock().await;
    let mut st = state::load(app)?;
    if let Some(item) = st.items.iter_mut().find(|i| i.id == id) {
        if item.ticktick_id.is_some() {
            // 已同步过 → 标墓碑,下次同步连远端一起删。
            item.deleted = true;
            item.dirty = true;
            item.updated_at_ms = now_ms();
        } else {
            // 从没推送过 → 直接移除。
            st.items.retain(|i| i.id != id);
        }
        state::save(app, &st)?;
    }
    Ok(serde_json::json!({ "ok": true }))
}

/// 取可用 token。API 口令长期有效、不过期、无刷新,直接返回;
/// 若服务端返回 401(口令被用户在滴答端删除/失效),由各 API 调用透传真错引导重连。
fn ensure_token(st: &TickTickState) -> Result<String, String> {
    st.tokens
        .access_token
        .clone()
        .ok_or_else(|| "未连接滴答清单".to_string())
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct SyncReport {
    pulled: usize,
    pushed: usize,
    completed_remote: usize,
    deleted_remote: usize,
    errors: Vec<String>,
}

/// 双向同步一次。整段持锁(防并发同步);失败逐条收集进 report.errors,透传真错。
async fn sync_now(app: &AppHandle) -> Result<Value, String> {
    let _g = state_lock().lock().await;
    let mut st = state::load(app)?;
    if !st.connected() {
        return Err("尚未连接滴答清单".to_string());
    }
    let pid = st.config.project_id.clone().ok_or("请先选择要同步的清单")?;
    let token = ensure_token(&st)?;
    let cfg = st.config.clone();
    let first_sync = !st.baseline_captured;

    let mut report = SyncReport::default();

    // 同步开始前已有远端 id 的本地项(用于"完成检测",排除本轮新建的)。
    let prior_synced: HashSet<String> = st
        .items
        .iter()
        .filter_map(|i| i.ticktick_id.clone())
        .collect();

    // 拉远端未完成任务(完成的拉不到 —— 见 client.rs 说明)。
    let remote = client::project_data(&cfg, &token, &pid).await?;
    let remote_ids: HashSet<String> = remote.iter().map(|t| t.id.clone()).collect();

    // ---------- PUSH:本地 → 远端 ----------
    let mut to_remove: Vec<String> = Vec::new();
    for i in 0..st.items.len() {
        let (id, ticktick_id, title, done, due, deleted, dirty) = {
            let it = &st.items[i];
            (
                it.id.clone(),
                it.ticktick_id.clone(),
                it.title.clone(),
                it.done,
                it.due.clone(),
                it.deleted,
                it.dirty,
            )
        };

        if deleted {
            if let Some(tid) = &ticktick_id {
                match client::delete_task(&cfg, &token, &pid, tid).await {
                    Ok(()) => {
                        report.deleted_remote += 1;
                        to_remove.push(id);
                    }
                    Err(e) => report.errors.push(e),
                }
            } else {
                to_remove.push(id);
            }
            continue;
        }

        match ticktick_id {
            None => match client::create_task(&cfg, &token, &pid, &title, due.as_deref()).await {
                Ok(rt) => {
                    let it = &mut st.items[i];
                    it.ticktick_id = Some(rt.id);
                    it.dirty = false;
                    report.pushed += 1;
                }
                Err(e) => report.errors.push(e),
            },
            Some(tid) if dirty => {
                let r = if done {
                    client::complete_task(&cfg, &token, &pid, &tid).await
                } else {
                    client::update_task(&cfg, &token, &tid, &pid, &title, false, due.as_deref())
                        .await
                };
                match r {
                    Ok(()) => {
                        st.items[i].dirty = false;
                        report.pushed += 1;
                    }
                    Err(e) => report.errors.push(e),
                }
            }
            Some(_) => {}
        }
    }
    if !to_remove.is_empty() {
        st.items.retain(|i| !to_remove.contains(&i.id));
    }

    // ---------- PULL:远端 → 本地 ----------
    for rt in &remote {
        if let Some(idx) = st
            .items
            .iter()
            .position(|i| i.ticktick_id.as_deref() == Some(rt.id.as_str()) && !i.deleted)
        {
            // 已知项:远端更新且本地无未推送改动 → 采纳远端(最新者胜)。
            if !st.items[idx].dirty {
                let rt_ms = rt
                    .modified_time
                    .as_deref()
                    .and_then(state::parse_iso_ms)
                    .unwrap_or(0);
                if rt_ms == 0 || rt_ms >= st.items[idx].updated_at_ms {
                    let it = &mut st.items[idx];
                    it.title = rt.title.clone();
                    it.due = rt.due_date.clone().or_else(|| rt.start_date.clone());
                    it.done = rt.status == 2;
                }
            }
        } else {
            // 新远端项:首次同步只建基线、一律不拉(挡住手机历史积压);
            // 之后只拉「不在基线」的(= 连接之后才在手机上新建的)。用 id 集合判断,
            // 不依赖 modifiedTime(滴答不保证返回,用时间戳会静默拉不进任何任务)。
            if !first_sync && !st.baseline_ids.contains(&rt.id) {
                let now = now_ms();
                st.items.push(MirrorItem {
                    id: uuid::Uuid::new_v4().to_string(),
                    ticktick_id: Some(rt.id.clone()),
                    title: rt.title.clone(),
                    done: rt.status == 2,
                    due: rt.due_date.clone().or_else(|| rt.start_date.clone()),
                    created_at_ms: now,
                    updated_at_ms: now,
                    deleted: false,
                    dirty: false,
                });
                report.pulled += 1;
            }
        }
    }

    // 首次同步:把当下远端已有任务记为基线(历史积压,永不拉)。
    if first_sync {
        st.baseline_ids = remote_ids.iter().cloned().collect();
        st.baseline_captured = true;
    }

    // ---------- 完成检测:曾同步、现在从未完成列表消失 → 多半被手机端勾完成 ----------
    for it in st.items.iter_mut() {
        if it.deleted || it.done || it.dirty {
            continue;
        }
        if let Some(tid) = &it.ticktick_id {
            if prior_synced.contains(tid) && !remote_ids.contains(tid) {
                it.done = true;
                it.updated_at_ms = now_ms();
                report.completed_remote += 1;
            }
        }
    }

    st.last_sync_ms = now_ms();
    st.last_error = if report.errors.is_empty() {
        None
    } else {
        Some(report.errors.join("; "))
    };
    state::save(app, &st)?;
    serde_json::to_value(report).map_err(|e| e.to_string())
}

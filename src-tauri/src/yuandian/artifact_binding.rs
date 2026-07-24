//! 执行查询派生产物与生成时委托人快照的无迁移绑定。
//!
//! 绑定只保存报告文件名和 raw 文件名；绝对路径继续留在 cases 专用列，避免把本机
//! AppData 路径带入个人空间同步。切换精确委托人时，调用方必须原子清除本节点。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::db::case_representation::{self, CaseRepresentation};

pub(crate) const OVERRIDE_KEY: &str = "execution_artifacts";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ExecutionArtifactOwner {
    pub representation: Option<CaseRepresentation>,
}

impl ExecutionArtifactOwner {
    fn is_exact(&self) -> bool {
        self.representation
            .as_ref()
            .is_some_and(|representation| !representation.parties.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct P1Artifacts {
    pub risk_report_path: String,
    pub dig_hints_path: String,
    pub raw_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct P1ArtifactBinding {
    risk_report_file: String,
    dig_hints_file: String,
    raw_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ExecutionArtifactBinding {
    version: u8,
    owner: ExecutionArtifactOwner,
    p1: P1ArtifactBinding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deep_report_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    full_report_file: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct FullReportInputs {
    pub risk_report_path: PathBuf,
    pub deep_report_path: PathBuf,
}

pub(crate) async fn current_owner(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<ExecutionArtifactOwner, String> {
    let row: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT agg_our_side, user_overrides_json FROM cases WHERE id = ?")
            .bind(case_id)
            .fetch_optional(pool)
            .await
            .map_err(|error| format!("读取案件委托人产物归属失败: {error}"))?;
    let Some((agg_our_side, user_overrides_json)) = row else {
        return Err(format!("案件不存在:{case_id}"));
    };
    owner_from_case_state(agg_our_side.as_deref(), user_overrides_json.as_deref())
}

pub(crate) async fn publish_p1(
    pool: &SqlitePool,
    case_id: &str,
    artifacts: P1Artifacts,
    generated_at: &str,
) -> Result<(), String> {
    let risk_report_file = checked_file_name(&artifacts.risk_report_path)?;
    let dig_hints_file = checked_file_name(&artifacts.dig_hints_path)?;
    let raw_files = artifacts
        .raw_files
        .iter()
        .map(|file| checked_file_name(file))
        .collect::<Result<Vec<_>, _>>()?;

    let mut tx = pool
        .begin()
        .await
        .map_err(|error| format!("开始发布 P1 产物失败: {error}"))?;
    let row: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT agg_our_side, user_overrides_json FROM cases WHERE id = ?")
            .bind(case_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|error| format!("读取案件 P1 产物状态失败: {error}"))?;
    let Some((agg_our_side, user_overrides_json)) = row else {
        return Err(format!("案件不存在:{case_id}"));
    };
    let owner = owner_from_case_state(agg_our_side.as_deref(), user_overrides_json.as_deref())?;
    let mut overrides = parse_overrides(user_overrides_json.as_deref())?;
    let root = overrides
        .as_object_mut()
        .ok_or_else(|| "user_overrides_json 不是对象，拒绝发布 P1 产物".to_string())?;
    let binding = ExecutionArtifactBinding {
        version: 1,
        owner,
        p1: P1ArtifactBinding {
            risk_report_file,
            dig_hints_file,
            raw_files,
        },
        deep_report_file: None,
        full_report_file: None,
    };
    root.insert(
        OVERRIDE_KEY.to_string(),
        serde_json::to_value(binding)
            .map_err(|error| format!("序列化执行产物绑定失败: {error}"))?,
    );
    let next_overrides = serde_json::to_string(&overrides)
        .map_err(|error| format!("序列化 user_overrides_json 失败: {error}"))?;

    sqlx::query(
        "UPDATE cases SET user_overrides_json = ?, \
         risk_assessment_path = ?, risk_assessment_at = ?, \
         deep_dive_report_path = NULL, deep_dive_at = NULL, \
         full_report_path = NULL, full_report_at = NULL \
         WHERE id = ?",
    )
    .bind(next_overrides)
    .bind(&artifacts.risk_report_path)
    .bind(generated_at)
    .bind(case_id)
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("发布 P1 产物失败: {error}"))?;
    tx.commit()
        .await
        .map_err(|error| format!("提交 P1 产物失败: {error}"))
}

/// 精确委托人必须命中同一 owner 的绑定；历史粗立场案件可继续走旧目录扫描兼容路径。
pub(crate) async fn load_p1_for_current_owner(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<Option<P1Artifacts>, String> {
    let (current_owner, binding) = load_binding_state(pool, case_id).await?;
    let Some(binding) = binding else {
        return if current_owner.is_exact() {
            Err(stale_owner_error())
        } else {
            Ok(None)
        };
    };
    ensure_current_owner(&current_owner, &binding)?;
    let reports_dir = super::reports_dir_for_case(case_id)?;
    Ok(Some(P1Artifacts {
        risk_report_path: reports_dir
            .join(checked_file_name(&binding.p1.risk_report_file)?)
            .to_string_lossy()
            .to_string(),
        dig_hints_path: reports_dir
            .join(checked_file_name(&binding.p1.dig_hints_file)?)
            .to_string_lossy()
            .to_string(),
        raw_files: binding
            .p1
            .raw_files
            .iter()
            .map(|file| checked_file_name(file))
            .collect::<Result<Vec<_>, String>>()?,
    }))
}

pub(crate) async fn publish_deep(
    pool: &SqlitePool,
    case_id: &str,
    report_path: &str,
    generated_at: &str,
) -> Result<(), String> {
    let report_file = checked_file_name(report_path)?;
    mutate_bound_artifacts(
        pool,
        case_id,
        |binding| {
            binding.deep_report_file = Some(report_file);
            binding.full_report_file = None;
        },
        "UPDATE cases SET user_overrides_json = ?, deep_dive_report_path = ?, deep_dive_at = ?, \
         full_report_path = NULL, full_report_at = NULL WHERE id = ?",
        report_path,
        generated_at,
    )
    .await
}

pub(crate) async fn load_full_inputs_for_current_owner(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<Option<FullReportInputs>, String> {
    let (current_owner, binding) = load_binding_state(pool, case_id).await?;
    let Some(binding) = binding else {
        return if current_owner.is_exact() {
            Err(stale_owner_error())
        } else {
            Ok(None)
        };
    };
    ensure_current_owner(&current_owner, &binding)?;
    let deep_report_file = binding
        .deep_report_file
        .as_deref()
        .ok_or_else(|| "当前委托人尚未生成深挖报告，请先重新深挖".to_string())?;
    let reports_dir = super::reports_dir_for_case(case_id)?;
    Ok(Some(FullReportInputs {
        risk_report_path: reports_dir.join(checked_file_name(&binding.p1.risk_report_file)?),
        deep_report_path: reports_dir.join(checked_file_name(deep_report_file)?),
    }))
}

pub(crate) async fn publish_full(
    pool: &SqlitePool,
    case_id: &str,
    report_path: &str,
    generated_at: &str,
) -> Result<(), String> {
    let report_file = checked_file_name(report_path)?;
    mutate_bound_artifacts(
        pool,
        case_id,
        |binding| binding.full_report_file = Some(report_file),
        "UPDATE cases SET user_overrides_json = ?, full_report_path = ?, full_report_at = ? \
         WHERE id = ?",
        report_path,
        generated_at,
    )
    .await
}

async fn mutate_bound_artifacts(
    pool: &SqlitePool,
    case_id: &str,
    mutate: impl FnOnce(&mut ExecutionArtifactBinding),
    update_sql: &str,
    report_path: &str,
    generated_at: &str,
) -> Result<(), String> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| format!("开始更新执行产物绑定失败: {error}"))?;
    let row: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT agg_our_side, user_overrides_json FROM cases WHERE id = ?")
            .bind(case_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|error| format!("读取执行产物绑定失败: {error}"))?;
    let Some((agg_our_side, user_overrides_json)) = row else {
        return Err(format!("案件不存在:{case_id}"));
    };
    let current_owner =
        owner_from_case_state(agg_our_side.as_deref(), user_overrides_json.as_deref())?;
    let mut overrides = parse_overrides(user_overrides_json.as_deref())?;
    let root = overrides
        .as_object_mut()
        .ok_or_else(|| "user_overrides_json 不是对象，拒绝更新执行产物".to_string())?;
    let value = root
        .get(OVERRIDE_KEY)
        .cloned()
        .ok_or_else(stale_owner_error)?;
    let mut binding = parse_binding(value)?;
    ensure_current_owner(&current_owner, &binding)?;
    mutate(&mut binding);
    root.insert(
        OVERRIDE_KEY.to_string(),
        serde_json::to_value(binding)
            .map_err(|error| format!("序列化执行产物绑定失败: {error}"))?,
    );
    let next_overrides = serde_json::to_string(&overrides)
        .map_err(|error| format!("序列化 user_overrides_json 失败: {error}"))?;

    sqlx::query(update_sql)
        .bind(next_overrides)
        .bind(report_path)
        .bind(generated_at)
        .bind(case_id)
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("更新执行产物绑定失败: {error}"))?;
    tx.commit()
        .await
        .map_err(|error| format!("提交执行产物绑定失败: {error}"))
}

async fn load_binding_state(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<(ExecutionArtifactOwner, Option<ExecutionArtifactBinding>), String> {
    let row: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT agg_our_side, user_overrides_json FROM cases WHERE id = ?")
            .bind(case_id)
            .fetch_optional(pool)
            .await
            .map_err(|error| format!("读取执行产物绑定失败: {error}"))?;
    let Some((agg_our_side, user_overrides_json)) = row else {
        return Err(format!("案件不存在:{case_id}"));
    };
    let owner = owner_from_case_state(agg_our_side.as_deref(), user_overrides_json.as_deref())?;
    let overrides = parse_overrides(user_overrides_json.as_deref())?;
    let binding = overrides
        .as_object()
        .and_then(|root| root.get(OVERRIDE_KEY))
        .cloned()
        .map(parse_binding)
        .transpose()?;
    Ok((owner, binding))
}

fn parse_binding(value: serde_json::Value) -> Result<ExecutionArtifactBinding, String> {
    let binding: ExecutionArtifactBinding =
        serde_json::from_value(value).map_err(|error| format!("执行产物绑定已损坏: {error}"))?;
    if binding.version != 1 {
        return Err(format!("执行产物绑定版本无效: {}", binding.version));
    }
    Ok(binding)
}

fn owner_from_case_state(
    agg_our_side: Option<&str>,
    user_overrides_json: Option<&str>,
) -> Result<ExecutionArtifactOwner, String> {
    let mut representation =
        case_representation::effective_representation(user_overrides_json, agg_our_side)
            .map_err(|error| format!("案件精确委托人状态无效: {error}"))?;
    if let Some(representation) = &mut representation {
        representation
            .parties
            .sort_by(|left, right| (&left.name, &left.role).cmp(&(&right.name, &right.role)));
    }
    Ok(ExecutionArtifactOwner { representation })
}

fn ensure_current_owner(
    current_owner: &ExecutionArtifactOwner,
    binding: &ExecutionArtifactBinding,
) -> Result<(), String> {
    if binding.owner != *current_owner {
        return Err(stale_owner_error());
    }
    Ok(())
}

fn stale_owner_error() -> String {
    "已有执行查询产物不属于当前委托人，请重新查询被执行人".to_string()
}

fn parse_overrides(raw: Option<&str>) -> Result<serde_json::Value, String> {
    match raw.map(str::trim).filter(|raw| !raw.is_empty()) {
        Some(raw) => serde_json::from_str(raw)
            .map_err(|error| format!("user_overrides_json 已损坏: {error}")),
        None => Ok(serde_json::json!({})),
    }
}

fn checked_file_name(path: &str) -> Result<String, String> {
    let path = Path::new(path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "执行产物文件名无效".to_string())?;
    if Path::new(file_name) != path && path.components().count() == 1 {
        return Err("执行产物文件名无效".to_string());
    }
    Ok(file_name.to_string())
}

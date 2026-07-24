//! 精确委托人的领域模型与原子写入。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::cases;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepresentationInput {
    pub side: String,
    pub party_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaseRepresentation {
    pub version: u8,
    pub side: String,
    pub parties: Vec<RepresentedParty>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepresentedParty {
    pub name: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepresentationValidationError(String);

impl std::fmt::Display for RepresentationValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for RepresentationValidationError {}

/// 精确委托人优先；旧数据尚未选择具体当事人时，退回既有的我方阵营判断。
pub fn effective_representation(
    user_overrides_json: Option<&str>,
    agg_our_side: Option<&str>,
) -> Result<Option<CaseRepresentation>, RepresentationValidationError> {
    let Some(raw) = user_overrides_json else {
        return Ok(legacy_representation(agg_our_side, user_overrides_json));
    };
    let overrides: serde_json::Value = serde_json::from_str(raw).map_err(|error| {
        RepresentationValidationError(format!("user_overrides_json 已损坏: {error}"))
    })?;
    let root = overrides
        .as_object()
        .ok_or_else(|| RepresentationValidationError("user_overrides_json 不是对象".to_string()))?;
    let Some(value) = root.get("representation") else {
        return Ok(legacy_representation(agg_our_side, user_overrides_json));
    };
    let representation =
        serde_json::from_value::<CaseRepresentation>(value.clone()).map_err(|error| {
            RepresentationValidationError(format!("representation 格式无效: {error}"))
        })?;
    validate_representation(&representation)?;
    validate_override_side(root, &representation.side)?;
    Ok(Some(representation))
}

fn legacy_representation(
    agg_our_side: Option<&str>,
    user_overrides_json: Option<&str>,
) -> Option<CaseRepresentation> {
    cases::effective_our_side(agg_our_side, user_overrides_json).map(|side| CaseRepresentation {
        version: 1,
        side,
        parties: Vec::new(),
    })
}

/// 验证并原子保存一个案件的精确委托人选择。
///
/// 聚合名单、已有用户覆盖和陈旧标记在同一个 SQLite 事务中读取和写入；任一验证或
/// JSON 解析失败均回滚，避免覆盖无关的人工编辑。
pub async fn update_case_representation(
    pool: &SqlitePool,
    case_id: &str,
    input: RepresentationInput,
) -> Result<CaseRepresentation, sqlx::Error> {
    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

    let result = async {
        let (agg_plaintiffs, agg_defendants, agg_third_parties, current_overrides): (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT agg_plaintiffs, agg_defendants, agg_third_parties, user_overrides_json \
             FROM cases WHERE id = ?",
        )
        .bind(case_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(sqlx::Error::RowNotFound)?;

        let (side, source_names_json, role) = match input.side.trim() {
            "原告方" => ("原告方", agg_plaintiffs.as_deref(), "原告"),
            "被告方" => ("被告方", agg_defendants.as_deref(), "被告"),
            "第三人" => ("第三人", agg_third_parties.as_deref(), "第三人"),
            _ => {
                return Err(sqlx::Error::Protocol(
                    "委托阵营必须是原告方、被告方或第三人".into(),
                ))
            }
        };

        let party_names = normalized_party_names(input.party_names)?;
        let source_names = parse_aggregate_party_names(source_names_json)?;
        for party_name in &party_names {
            if !source_names.contains(party_name) {
                return Err(sqlx::Error::Protocol(format!(
                    "当事人“{}”不属于{}",
                    party_name, side
                )));
            }
        }

        let representation = CaseRepresentation {
            version: 1,
            side: side.to_string(),
            parties: party_names
                .into_iter()
                .map(|name| RepresentedParty {
                    name,
                    role: role.to_string(),
                })
                .collect(),
        };
        validate_representation(&representation).map_err(representation_validation_sql_error)?;

        // 已有的精确委托人一旦损坏，必须先显式修复，不能由这次提交静默覆盖。
        let persisted = effective_representation(current_overrides.as_deref(), None)
            .map_err(representation_validation_sql_error)?;
        if let Some(persisted) = &persisted {
            validate_persisted_representation_semantics(
                persisted,
                agg_plaintiffs.as_deref(),
                agg_defendants.as_deref(),
                agg_third_parties.as_deref(),
            )
            .map_err(representation_validation_sql_error)?;
        }

        let mut overrides = parse_overrides(current_overrides.as_deref())?;
        let root = overrides.as_object_mut().ok_or_else(|| {
            sqlx::Error::Protocol("user_overrides_json 不是对象，拒绝覆盖".into())
        })?;
        let fields_value = root
            .entry("fields")
            .or_insert_with(|| serde_json::json!({}));
        let fields = fields_value.as_object_mut().ok_or_else(|| {
            sqlx::Error::Protocol("user_overrides_json.fields 不是对象，拒绝覆盖".into())
        })?;
        fields.insert(
            "agg_our_side".to_string(),
            serde_json::Value::String(representation.side.clone()),
        );
        root.insert(
            "representation".to_string(),
            serde_json::to_value(&representation)
                .map_err(|error| sqlx::Error::Protocol(format!("序列化精确委托人失败: {error}")))?,
        );
        let representation_changed = persisted.as_ref() != Some(&representation);
        if representation_changed {
            // 产物文件保留，但所有入口和可验证绑定必须与委托人切换在同一事务失效。
            root.remove(crate::yuandian::artifact_binding::OVERRIDE_KEY);
        }
        let next_overrides = serde_json::to_string(&overrides).map_err(|error| {
            sqlx::Error::Protocol(format!("序列化 user_overrides_json 失败: {error}"))
        })?;

        if representation_changed {
            sqlx::query(
                "UPDATE cases SET user_overrides_json = ?, analysis_stale = 1, \
                 analysis_stale_reason = 'represented_parties_changed', \
                 risk_assessment_path = NULL, risk_assessment_at = NULL, \
                 deep_dive_report_path = NULL, deep_dive_at = NULL, \
                 full_report_path = NULL, full_report_at = NULL, \
                 updated_at = datetime('now') WHERE id = ?",
            )
            .bind(next_overrides)
            .bind(case_id)
            .execute(&mut *conn)
            .await?;
        } else {
            sqlx::query(
                "UPDATE cases SET user_overrides_json = ?, analysis_stale = 1, \
                 analysis_stale_reason = 'represented_parties_changed', \
                 updated_at = datetime('now') WHERE id = ?",
            )
            .bind(next_overrides)
            .bind(case_id)
            .execute(&mut *conn)
            .await?;
        }

        Ok::<_, sqlx::Error>(representation)
    }
    .await;

    match result {
        Ok(representation) => {
            sqlx::query("COMMIT").execute(&mut *conn).await?;
            Ok(representation)
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            Err(error)
        }
    }
}

fn validate_representation(
    representation: &CaseRepresentation,
) -> Result<(), RepresentationValidationError> {
    if representation.version != 1 {
        return Err(RepresentationValidationError(format!(
            "representation.version 必须为 1，实际为 {}",
            representation.version
        )));
    }
    let expected_role = representation_role_for_side(&representation.side)?;
    if representation.parties.is_empty() {
        return Err(RepresentationValidationError(
            "representation.parties 不能为空".to_string(),
        ));
    }

    let mut names = HashSet::new();
    for party in &representation.parties {
        if party.name != party.name.trim() {
            return Err(RepresentationValidationError(
                "representation.party.name 必须为非空规范姓名".to_string(),
            ));
        }
        let name = party.name.trim();
        if name.is_empty() {
            return Err(RepresentationValidationError(
                "representation.party.name 不能为空".to_string(),
            ));
        }
        if !names.insert(name) {
            return Err(RepresentationValidationError(format!(
                "representation.party.name 重复: {name}"
            )));
        }
        if party.role != expected_role {
            return Err(RepresentationValidationError(format!(
                "representation.party.role 必须为{expected_role}"
            )));
        }
    }
    Ok(())
}

fn representation_role_for_side(side: &str) -> Result<&'static str, RepresentationValidationError> {
    match side {
        "原告方" => Ok("原告"),
        "被告方" => Ok("被告"),
        "第三人" => Ok("第三人"),
        _ => Err(RepresentationValidationError(
            "representation.side 必须是原告方、被告方或第三人".to_string(),
        )),
    }
}

fn validate_override_side(
    root: &serde_json::Map<String, serde_json::Value>,
    representation_side: &str,
) -> Result<(), RepresentationValidationError> {
    let Some(fields) = root.get("fields") else {
        return Ok(());
    };
    let fields = fields.as_object().ok_or_else(|| {
        RepresentationValidationError("user_overrides_json.fields 不是对象".to_string())
    })?;
    let Some(our_side) = fields.get("agg_our_side") else {
        return Ok(());
    };
    if our_side.is_null() {
        return Ok(());
    }
    let our_side = our_side.as_str().ok_or_else(|| {
        RepresentationValidationError("fields.agg_our_side 必须是字符串或 null".to_string())
    })?;
    if !our_side.trim().is_empty() && our_side.trim() != representation_side {
        return Err(RepresentationValidationError(format!(
            "fields.agg_our_side 与 representation.side 冲突: {} != {}",
            our_side.trim(),
            representation_side
        )));
    }
    Ok(())
}

fn representation_validation_sql_error(error: RepresentationValidationError) -> sqlx::Error {
    sqlx::Error::Protocol(error.to_string())
}

fn validate_persisted_representation_semantics(
    representation: &CaseRepresentation,
    agg_plaintiffs: Option<&str>,
    agg_defendants: Option<&str>,
    agg_third_parties: Option<&str>,
) -> Result<(), RepresentationValidationError> {
    let aggregate = match representation.side.as_str() {
        "原告方" => agg_plaintiffs,
        "被告方" => agg_defendants,
        "第三人" => agg_third_parties,
        // `effective_representation` 已完成结构校验；保留此分支防止未来调用跳过校验。
        _ => {
            return Err(RepresentationValidationError(
                "已保存 representation.side 无效".to_string(),
            ))
        }
    };
    let aggregate_names = parse_aggregate_party_names(aggregate).map_err(|error| {
        RepresentationValidationError(format!("无法校验已保存 representation: {error}"))
    })?;

    for party in &representation.parties {
        if party.name != party.name.trim() {
            return Err(RepresentationValidationError(format!(
                "已保存 representation.party.name 未规范化: {}",
                party.name
            )));
        }
        if !aggregate_names.contains(&party.name) {
            return Err(RepresentationValidationError(format!(
                "已保存 representation.party.name 不属于{}: {}",
                representation.side, party.name
            )));
        }
    }
    Ok(())
}

fn normalized_party_names(party_names: Vec<String>) -> Result<Vec<String>, sqlx::Error> {
    if party_names.is_empty() {
        return Err(sqlx::Error::Protocol("必须至少选择一名委托当事人".into()));
    }

    let mut seen = HashSet::new();
    party_names
        .into_iter()
        .map(|name| {
            let name = name.trim().to_string();
            if name.is_empty() {
                return Err(sqlx::Error::Protocol("委托当事人姓名不能为空".into()));
            }
            if !seen.insert(name.clone()) {
                return Err(sqlx::Error::Protocol(format!("委托当事人“{}”重复", name)));
            }
            Ok(name)
        })
        .collect()
}

fn parse_aggregate_party_names(raw: Option<&str>) -> Result<HashSet<String>, sqlx::Error> {
    let raw = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| sqlx::Error::Protocol("所选阵营没有可验证的聚合当事人名单".into()))?;
    let names = serde_json::from_str::<Vec<String>>(raw)
        .map_err(|error| sqlx::Error::Protocol(format!("聚合当事人名单格式无效: {error}")))?;
    Ok(names
        .into_iter()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect())
}

fn parse_overrides(raw: Option<&str>) -> Result<serde_json::Value, sqlx::Error> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        Some(raw) => serde_json::from_str(raw).map_err(|error| {
            sqlx::Error::Protocol(format!("user_overrides_json 已损坏，拒绝覆盖: {error}"))
        }),
        None => Ok(serde_json::json!({})),
    }
}

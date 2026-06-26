use serde::Deserialize;
use sqlx::SqlitePool;
use std::path::Path;
use uuid::Uuid;

use super::cases::{effective_our_side, Case};
use super::documents::Document;

#[derive(Debug, Clone, Deserialize)]
struct Party {
    name: Option<String>,
    role: Option<String>,
    address: Option<String>,
    phone: Option<String>,
    is_our_side: Option<bool>,
    legal_representative: Option<String>,
    representative: Option<String>,
    contact: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct KeyDate {
    date: Option<String>,
    event: Option<String>,
    event_type: Option<String>,
    note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct FeeItem {
    item: Option<String>,
    amount: Option<f64>,
    note: Option<String>,
}

pub async fn generate(pool: &SqlitePool, case_id: &str) -> Result<Document, String> {
    let base = crate::db::app_data_dir().map_err(|e| e.to_string())?;
    generate_at_base(pool, &base, case_id).await
}

async fn generate_at_base(
    pool: &SqlitePool,
    base: &Path,
    case_id: &str,
) -> Result<Document, String> {
    let case = super::cases::get_case(pool, case_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("案件不存在:{case_id}"))?;
    let body = build_markdown(&case);

    let doc_id = Uuid::new_v4().to_string();
    let dir = base
        .join("extracts")
        .join(case_id)
        .join("closing_materials");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建结案材料目录失败:{e}"))?;
    let filename = format!(
        "结案归档材料要素_{}_{}.md",
        chrono::Local::now().format("%Y%m%d_%H%M%S"),
        &doc_id[..8]
    );
    let path = dir.join(&filename);
    std::fs::write(&path, &body).map_err(|e| format!("写入结案材料失败:{e}"))?;

    let path_text = path.to_string_lossy().into_owned();
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let result = sqlx::query(
        "INSERT INTO documents \
         (id, case_id, source_path, filename, stage, category, is_ai_artifact, \
          mime_type, size_bytes, modified_at, extraction_status, \
          extracted_text_path, source, created_at) \
         VALUES (?, ?, ?, ?, NULL, '结案材料', 1, 'text/markdown', ?, ?, 'done', ?, \
          'closing_materials', ?)",
    )
    .bind(&doc_id)
    .bind(case_id)
    .bind(&path_text)
    .bind(&filename)
    .bind(body.len() as i64)
    .bind(&now)
    .bind(&path_text)
    .bind(&now)
    .execute(pool)
    .await;

    if let Err(error) = result {
        let _ = std::fs::remove_file(&path);
        return Err(format!("登记结案材料失败:{error}"));
    }

    sqlx::query_as::<_, Document>("SELECT * FROM documents WHERE id = ?")
        .bind(&doc_id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())
}

fn build_markdown(case: &Case) -> String {
    let parties = parse_parties(case.agg_party_contacts.as_deref());
    let dates = parse_dates(case.agg_key_dates.as_deref());
    let fees = parse_fees(case.agg_fees.as_deref());
    let our_side = effective_our_side(
        case.agg_our_side.as_deref(),
        case.user_overrides_json.as_deref(),
    );
    let case_no = first_non_empty([case.agg_case_no.as_deref(), case.case_no.as_deref()]);
    let cause = first_non_empty([case.agg_cause.as_deref(), case.cause.as_deref()]);
    let court = first_non_empty([case.agg_court.as_deref(), case.court.as_deref()]);
    let client_parties = party_names(&parties, true)
        .or_else(|| role_party_names(&parties, our_side.as_deref()))
        .or_else(|| parse_name_list(case.agg_plaintiffs.as_deref()))
        .unwrap_or_else(pending);
    let opponent_parties = party_names(&parties, false)
        .or_else(|| parse_name_list(case.agg_defendants.as_deref()))
        .unwrap_or_else(pending);
    let claim_amount = case
        .agg_claim_amount
        .map(format_money)
        .unwrap_or_else(pending);
    let case_summary = case
        .case_summary
        .as_deref()
        .and_then(non_empty)
        .unwrap_or("待补");
    let resolution = case
        .agg_resolution
        .as_deref()
        .and_then(non_empty)
        .unwrap_or("待补");
    let intake_date = date_for(&dates, &["委托", "收案", "接案"])
        .or_else(|| case.agg_filed_at.as_deref().and_then(non_empty))
        .unwrap_or("待补");
    let filed_date = date_for(&dates, &["起诉", "立案"]).unwrap_or("待补");
    let review_date = date_for(&dates, &["阅卷"]).unwrap_or("待补");
    let meeting_date = date_for(&dates, &["会见", "谈话", "沟通"]).unwrap_or("待补");
    let hearing_date = date_for(&dates, &["开庭", "庭审"]).unwrap_or("待补");
    let closing_date = date_for(&dates, &["结案", "调解", "判决", "裁定"]).unwrap_or("待补");
    let entrusted_matter = entrusted_matter(cause, case_summary);
    let fee_payable = fee_amount_for(&fees, &["应收", "应收费"]).unwrap_or_else(pending);
    let fee_received = fee_amount_for(&fees, &["实收", "已收"]).unwrap_or_else(pending);
    let fee_advance = fee_amount_for(&fees, &["预收", "预收费"]).unwrap_or_else(pending);
    let fee_note = fee_notes(&fees).unwrap_or_else(pending);
    let first_client = first_party(&parties, true);
    let first_opponent = first_party(&parties, false);

    let mut out = String::new();
    out.push_str("# 结案归档材料要素\n\n");
    out.push_str("> 本文根据 CaseBoard 当前案件画像自动整理,用于线下归档表格复制粘贴。`待补` 字段表示看板尚未可靠识别,请人工补录或复核。\n\n");

    out.push_str("## 一、案件受理登记表\n\n");
    push_fact(&mut out, "收案时间", intake_date);
    push_fact(&mut out, "委托的法律事务名称", &entrusted_matter);
    push_fact(&mut out, "案情简介", case_summary);
    push_fact(&mut out, "委托方当事人", &client_parties);
    push_fact(
        &mut out,
        "委托方法定代表人/联系人",
        party_contact_line(first_client),
    );
    push_fact(
        &mut out,
        "委托方地址",
        party_field(first_client, |p| p.address.as_deref()),
    );
    push_fact(
        &mut out,
        "委托方电话",
        party_field(first_client, |p| p.phone.as_deref()),
    );
    push_fact(&mut out, "对方当事人", &opponent_parties);
    push_fact(
        &mut out,
        "对方法定代表人/联系人",
        party_contact_line(first_opponent),
    );
    push_fact(
        &mut out,
        "对方地址",
        party_field(first_opponent, |p| p.address.as_deref()),
    );
    push_fact(
        &mut out,
        "对方电话",
        party_field(first_opponent, |p| p.phone.as_deref()),
    );
    push_fact(&mut out, "案由", cause.unwrap_or("待补"));
    push_fact(&mut out, "诉讼标的", &claim_amount);
    push_fact(&mut out, "目的要求", resolution);
    push_fact(&mut out, "受案机构/审判机关", court.unwrap_or("待补"));
    push_fact(
        &mut out,
        "应收费/预收费",
        &format!("{fee_payable} / {fee_advance}"),
    );
    push_fact(&mut out, "费用备注", &fee_note);
    push_fact(&mut out, "律师意见/备注", "待补");
    out.push('\n');

    out.push_str("## 二、办案小结\n\n");
    push_fact(
        &mut out,
        "当事人",
        &format!("委托方:{client_parties}; 相对方:{opponent_parties}"),
    );
    push_fact(&mut out, "案由", cause.unwrap_or("待补"));
    push_fact(&mut out, "收案日期", intake_date);
    push_fact(&mut out, "起诉日期", filed_date);
    push_fact(&mut out, "阅卷日期", review_date);
    push_fact(&mut out, "初次会见/谈话日期", meeting_date);
    push_fact(&mut out, "首次开庭日期", hearing_date);
    push_fact(&mut out, "简要案情和工作概况", case_summary);
    push_fact(&mut out, "处理结果", resolution);
    push_fact(&mut out, "律师办案体会", "待补");
    push_fact(&mut out, "保管期限", "待补");
    push_fact(&mut out, "事务所主任审查意见", "待补");
    push_fact(&mut out, "结案日期", closing_date);
    out.push('\n');

    out.push_str("## 三、结案卷宗呈批表\n\n");
    push_fact(&mut out, "案件编号", case_no.unwrap_or("待补"));
    push_fact(&mut out, "委托日期", intake_date);
    push_fact(&mut out, "结案日期", closing_date);
    push_fact(&mut out, "案件名称", &case.name);
    push_fact(&mut out, "委托人", &client_parties);
    push_fact(&mut out, "委托事项", &entrusted_matter);
    push_fact(&mut out, "利益相对方", &opponent_parties);
    push_fact(&mut out, "主办律师", "待补");
    push_fact(&mut out, "协办律师(助理)", "待补");
    push_fact(&mut out, "应收律师费", &fee_payable);
    push_fact(&mut out, "收费方式", &fee_note);
    push_fact(&mut out, "实收律师费", &fee_received);
    push_fact(&mut out, "发票日期", "待补");
    push_fact(&mut out, "案件结果", resolution);
    push_fact(&mut out, "备注", "待补");
    push_fact(&mut out, "交卷人/收卷人", "待补 / 待补");
    out.push('\n');

    out.push_str("## 四、可复制文本块\n\n");
    out.push_str("### 案情简介\n\n");
    out.push_str(case_summary);
    out.push_str("\n\n### 简要案情和工作概况\n\n");
    out.push_str(case_summary);
    if !dates.is_empty() {
        out.push_str("\n\n已识别办案节点:\n");
        for item in &dates {
            out.push_str(&format!(
                "- {} {}{}\n",
                item.date.as_deref().unwrap_or("日期待补"),
                date_label(item).unwrap_or("办案节点"),
                item.note
                    .as_deref()
                    .and_then(non_empty)
                    .map(|v| format!(": {v}"))
                    .unwrap_or_default()
            ));
        }
    }
    out.push_str("\n### 处理结果/案件结果\n\n");
    out.push_str(resolution);
    out.push('\n');
    out
}

fn parse_parties(raw: Option<&str>) -> Vec<Party> {
    raw.and_then(|s| serde_json::from_str::<Vec<Party>>(s).ok())
        .unwrap_or_default()
}

fn parse_dates(raw: Option<&str>) -> Vec<KeyDate> {
    let mut dates = raw
        .and_then(|s| serde_json::from_str::<Vec<KeyDate>>(s).ok())
        .unwrap_or_default();
    dates.sort_by(|a, b| {
        a.date
            .as_deref()
            .unwrap_or("")
            .cmp(b.date.as_deref().unwrap_or(""))
    });
    dates
}

fn parse_fees(raw: Option<&str>) -> Vec<FeeItem> {
    raw.and_then(|s| serde_json::from_str::<Vec<FeeItem>>(s).ok())
        .unwrap_or_default()
}

fn parse_name_list(raw: Option<&str>) -> Option<String> {
    let value = raw.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())?;
    let names = value
        .as_array()?
        .iter()
        .filter_map(|item| {
            item.as_str()
                .or_else(|| item.get("name").and_then(|v| v.as_str()))
                .and_then(non_empty)
        })
        .collect::<Vec<_>>();
    (!names.is_empty()).then(|| names.join("、"))
}

fn party_names(parties: &[Party], is_our_side: bool) -> Option<String> {
    let names = parties
        .iter()
        .filter(|party| party.is_our_side == Some(is_our_side))
        .filter_map(|party| party.name.as_deref().and_then(non_empty))
        .collect::<Vec<_>>();
    (!names.is_empty()).then(|| names.join("、"))
}

fn role_party_names(parties: &[Party], our_side: Option<&str>) -> Option<String> {
    let side = our_side?;
    let names = parties
        .iter()
        .filter(|party| {
            party.role.as_deref().is_some_and(|role| {
                side.contains(role) || role.contains(side.trim_end_matches('方'))
            })
        })
        .filter_map(|party| party.name.as_deref().and_then(non_empty))
        .collect::<Vec<_>>();
    (!names.is_empty()).then(|| names.join("、"))
}

fn first_party(parties: &[Party], is_our_side: bool) -> Option<&Party> {
    parties
        .iter()
        .find(|party| party.is_our_side == Some(is_our_side))
}

fn party_contact_line(party: Option<&Party>) -> &str {
    party
        .and_then(|p| {
            p.legal_representative
                .as_deref()
                .or(p.representative.as_deref())
                .or(p.contact.as_deref())
                .and_then(non_empty)
        })
        .unwrap_or("待补")
}

fn party_field<'a>(party: Option<&'a Party>, f: impl Fn(&'a Party) -> Option<&'a str>) -> &'a str {
    party.and_then(f).and_then(non_empty).unwrap_or("待补")
}

fn date_for<'a>(dates: &'a [KeyDate], keywords: &[&str]) -> Option<&'a str> {
    dates.iter().find_map(|item| {
        let label = date_label(item).unwrap_or("");
        keywords
            .iter()
            .any(|keyword| label.contains(keyword))
            .then(|| item.date.as_deref().and_then(non_empty))
            .flatten()
    })
}

fn date_label(item: &KeyDate) -> Option<&str> {
    item.event_type
        .as_deref()
        .or(item.event.as_deref())
        .and_then(non_empty)
}

fn fee_amount_for(fees: &[FeeItem], keywords: &[&str]) -> Option<String> {
    fees.iter().find_map(|item| {
        let label = item.item.as_deref().unwrap_or("");
        keywords
            .iter()
            .any(|keyword| label.contains(keyword))
            .then_some(item.amount)
            .flatten()
            .map(format_money)
    })
}

fn fee_notes(fees: &[FeeItem]) -> Option<String> {
    let parts = fees
        .iter()
        .filter_map(|fee| {
            let item = fee.item.as_deref().and_then(non_empty)?;
            let amount = fee.amount.map(format_money);
            let note = fee.note.as_deref().and_then(non_empty);
            Some(match (amount, note) {
                (Some(amount), Some(note)) => format!("{item}:{amount}({note})"),
                (Some(amount), None) => format!("{item}:{amount}"),
                (None, Some(note)) => format!("{item}:{note}"),
                (None, None) => item.to_string(),
            })
        })
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join("; "))
}

fn entrusted_matter(cause: Option<&str>, summary: &str) -> String {
    match cause {
        Some(cause) if cause != "待补" => cause.to_string(),
        _ => summary.chars().take(40).collect(),
    }
}

fn first_non_empty<const N: usize>(values: [Option<&str>; N]) -> Option<&str> {
    values
        .into_iter()
        .find_map(|value| value.and_then(non_empty))
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn pending() -> String {
    "待补".to_string()
}

fn push_fact(out: &mut String, label: &str, value: &str) {
    out.push_str(&format!("- **{}**: {}\n", label, value.trim()));
}

fn format_money(value: f64) -> String {
    if value >= 10_000_000.0 {
        format!("{:.2} 亿元", value / 100_000_000.0)
    } else if value >= 10_000.0 {
        format!("{:.2} 万元", value / 10_000.0)
    } else {
        format!("{:.2} 元", value)
    }
}

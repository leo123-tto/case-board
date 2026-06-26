//! 办案画像统计。
//!
//! 只基于本机 `cases` 表的已结构化字段做聚合,不读取、不移动、不修改案件原文件。

use std::collections::BTreeMap;

use serde::Serialize;

use super::cases::{effective_our_side, Case};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct InsightBucket {
    pub label: String,
    pub count: usize,
    pub ratio: f64,
    pub amount_total: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LawyerInsightsReport {
    pub total_cases: usize,
    pub active_cases: usize,
    pub closed_cases: usize,
    pub analyzed_cases: usize,
    pub amount_cases: usize,
    pub total_claim_amount: f64,
    pub average_claim_amount: Option<f64>,
    pub top_causes: Vec<InsightBucket>,
    pub top_courts: Vec<InsightBucket>,
    pub our_side_mix: Vec<InsightBucket>,
    pub stage_mix: Vec<InsightBucket>,
    pub strengths: Vec<String>,
    pub data_gaps: Vec<String>,
    pub next_questions: Vec<String>,
    pub markdown: String,
}

pub fn build_lawyer_insights_report(cases: &[Case]) -> LawyerInsightsReport {
    let total_cases = cases.len();
    let closed_cases = cases.iter().filter(|case| is_closed_case(case)).count();
    let active_cases = total_cases.saturating_sub(closed_cases);
    let analyzed_cases = cases
        .iter()
        .filter(|case| {
            case.case_report_path
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
        })
        .count();

    let amounts: Vec<f64> = cases
        .iter()
        .filter_map(|case| case.agg_claim_amount)
        .filter(|amount| amount.is_finite() && *amount > 0.0)
        .collect();
    let amount_cases = amounts.len();
    let total_claim_amount = amounts.iter().sum::<f64>();
    let average_claim_amount = if amount_cases > 0 {
        Some(round_money(total_claim_amount / amount_cases as f64))
    } else {
        None
    };

    let top_causes = bucketize(cases, total_cases, |case| {
        text_or_unknown(
            case.agg_cause.as_deref().or(case.cause.as_deref()),
            "未识别案由",
        )
    });
    let top_courts = bucketize(cases, total_cases, |case| {
        text_or_unknown(
            case.agg_court.as_deref().or(case.court.as_deref()),
            "未识别法院",
        )
    });
    let our_side_mix = bucketize(cases, total_cases, |case| {
        effective_our_side(
            case.agg_our_side.as_deref(),
            case.user_overrides_json.as_deref(),
        )
        .unwrap_or_else(|| "未识别立场".to_string())
    });
    let stage_mix = bucketize(cases, total_cases, stage_label);

    let mut strengths = build_strengths(
        total_cases,
        &top_causes,
        &top_courts,
        &our_side_mix,
        average_claim_amount,
    );
    let data_gaps = build_data_gaps(
        total_cases,
        amount_cases,
        &top_causes,
        &top_courts,
        &our_side_mix,
    );
    let next_questions = build_next_questions(&top_causes, &top_courts);

    if strengths.is_empty() {
        strengths
            .push("当前数据较分散,更适合作为案件盘点底稿,暂不强行下办案特长结论。".to_string());
    }

    let mut report = LawyerInsightsReport {
        total_cases,
        active_cases,
        closed_cases,
        analyzed_cases,
        amount_cases,
        total_claim_amount: round_money(total_claim_amount),
        average_claim_amount,
        top_causes,
        top_courts,
        our_side_mix,
        stage_mix,
        strengths,
        data_gaps,
        next_questions,
        markdown: String::new(),
    };
    report.markdown = build_markdown(&report);
    report
}

fn bucketize<F>(cases: &[Case], total_cases: usize, label_of: F) -> Vec<InsightBucket>
where
    F: Fn(&Case) -> String,
{
    let mut map: BTreeMap<String, (usize, f64)> = BTreeMap::new();
    for case in cases {
        let label = label_of(case);
        let amount = case
            .agg_claim_amount
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(0.0);
        let entry = map.entry(label).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += amount;
    }

    let mut buckets: Vec<InsightBucket> = map
        .into_iter()
        .map(|(label, (count, amount_total))| InsightBucket {
            label,
            count,
            ratio: if total_cases == 0 {
                0.0
            } else {
                round_ratio(count as f64 / total_cases as f64)
            },
            amount_total: round_money(amount_total),
        })
        .collect();
    buckets.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| {
                b.amount_total
                    .partial_cmp(&a.amount_total)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.label.cmp(&b.label))
    });
    buckets
}

fn text_or_unknown(value: Option<&str>, fallback: &str) -> String {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn stage_label(case: &Case) -> String {
    case.workflow_status
        .as_deref()
        .or(case.stage.as_deref())
        .or(case.agg_status_text.as_deref())
        .or(Some(case.case_status.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("未识别阶段")
        .to_string()
}

fn is_closed_case(case: &Case) -> bool {
    let fields = [
        Some(case.case_status.as_str()),
        case.workflow_status.as_deref(),
        case.agg_status_text.as_deref(),
        case.agg_resolution.as_deref(),
    ];
    fields.into_iter().flatten().any(|text| {
        let t = text.trim();
        t.contains("已结案")
            || t.contains("已归档")
            || t.contains("结案")
            || t.contains("调解结案")
            || t.contains("判决生效")
    })
}

fn build_strengths(
    total_cases: usize,
    top_causes: &[InsightBucket],
    top_courts: &[InsightBucket],
    our_side_mix: &[InsightBucket],
    average_claim_amount: Option<f64>,
) -> Vec<String> {
    if total_cases == 0 {
        return vec!["还没有足够案件数据,暂时不能形成办案画像。".to_string()];
    }

    let mut strengths = Vec::new();
    if let Some(top) = top_causes.first() {
        if !top.label.starts_with("未识别") && top.ratio >= 0.30 {
            strengths.push(format!(
                "案件明显集中在{}方向,已形成可复盘的同类案件经验样本({}件,{:.0}%)。",
                top.label,
                top.count,
                top.ratio * 100.0
            ));
        }
    }
    if let Some(top) = top_courts.first() {
        if !top.label.starts_with("未识别") && top.ratio >= 0.30 {
            strengths.push(format!(
                "承办法院/区域相对集中在{},便于沉淀当地裁判尺度、流程节点和沟通经验({}件,{:.0}%)。",
                top.label,
                top.count,
                top.ratio * 100.0
            ));
        }
    }
    if let Some(top) = our_side_mix.first() {
        if !top.label.starts_with("未识别") && top.ratio >= 0.50 {
            strengths.push(format!(
                "代理立场以{}为主,可围绕该立场继续沉淀证据组织和攻防模板({}件,{:.0}%)。",
                top.label,
                top.count,
                top.ratio * 100.0
            ));
        }
    }
    if let Some(avg) = average_claim_amount {
        strengths.push(format!(
            "已录入标的金额案件的平均标的为{},可作为案件价值分层和精力分配的基础指标。",
            format_money(avg)
        ));
    }
    strengths
}

fn build_data_gaps(
    total_cases: usize,
    amount_cases: usize,
    top_causes: &[InsightBucket],
    top_courts: &[InsightBucket],
    our_side_mix: &[InsightBucket],
) -> Vec<String> {
    let mut gaps = Vec::new();
    if total_cases == 0 {
        gaps.push("暂无案件数据。".to_string());
        return gaps;
    }
    if amount_cases < total_cases {
        gaps.push(format!(
            "标的金额仅覆盖 {}/{} 件案件,金额画像会偏向已抽取/已填写样本。",
            amount_cases, total_cases
        ));
    }
    if top_causes
        .first()
        .is_none_or(|bucket| bucket.label.starts_with("未识别"))
    {
        gaps.push("案由识别不足,建议先对案件跑一次全局分析或手动补正案由。".to_string());
    }
    if top_courts
        .first()
        .is_none_or(|bucket| bucket.label.starts_with("未识别"))
    {
        gaps.push("法院/地域字段不足,暂不能形成稳定地域画像。".to_string());
    }
    if our_side_mix
        .first()
        .is_none_or(|bucket| bucket.label.starts_with("未识别"))
    {
        gaps.push("我方代理立场不足,会影响“擅长原告/被告/执行应对”等判断。".to_string());
    }
    gaps.push(
        "当前看板尚未稳定采集案源渠道、律师费收入、胜诉率和客户复购,这些指标不能直接推断。"
            .to_string(),
    );
    gaps
}

fn build_next_questions(top_causes: &[InsightBucket], top_courts: &[InsightBucket]) -> Vec<String> {
    let cause = top_causes
        .iter()
        .find(|bucket| !bucket.label.starts_with("未识别"))
        .map(|bucket| bucket.label.as_str())
        .unwrap_or("高频案由");
    let court = top_courts
        .iter()
        .find(|bucket| !bucket.label.starts_with("未识别"))
        .map(|bucket| bucket.label.as_str())
        .unwrap_or("主要法院/区域");

    vec![
        format!("把{}案件单独导出,复盘常见争点、证据缺口和胜败因素。", cause),
        format!("围绕{}案件,整理当地流程习惯、平均周期和裁判口径。", court),
        "按客户/对手方名称二次分组,识别长期客户、重复交易对手和潜在交叉销售机会。".to_string(),
        "把已结案案件单独导出给 AI 深挖,形成可复用的办案经验和报价/排期参考。".to_string(),
    ]
}

fn build_markdown(report: &LawyerInsightsReport) -> String {
    let mut md = String::new();
    md.push_str("# 办案画像报告\n\n");
    md.push_str("> 说明:本报告仅基于 CaseBoard 本机案件结构化字段生成,不读取或修改案件原文件;未采集字段不会被推断。\n\n");
    if report.total_cases == 0 {
        md.push_str("暂无案件数据。\n");
        return md;
    }

    md.push_str("## 一、核心概览\n\n");
    md.push_str("| 指标 | 数值 |\n| --- | ---: |\n");
    md.push_str(&format!("| 案件总数 | {} |\n", report.total_cases));
    md.push_str(&format!("| 在办案件 | {} |\n", report.active_cases));
    md.push_str(&format!("| 已结案/归档 | {} |\n", report.closed_cases));
    md.push_str(&format!("| 已生成案件报告 | {} |\n", report.analyzed_cases));
    md.push_str(&format!(
        "| 已识别标的金额 | {} 件 / {} |\n",
        report.amount_cases, report.total_cases
    ));
    md.push_str(&format!(
        "| 标的金额合计 | {} |\n",
        format_money(report.total_claim_amount)
    ));
    md.push_str(&format!(
        "| 平均标的金额 | {} |\n\n",
        report
            .average_claim_amount
            .map(format_money)
            .unwrap_or_else(|| "暂无".to_string())
    ));

    push_bucket_section(&mut md, "二、高频案由", &report.top_causes);
    push_bucket_section(&mut md, "三、主要法院/地域", &report.top_courts);
    push_bucket_section(&mut md, "四、我方代理立场", &report.our_side_mix);
    push_bucket_section(&mut md, "五、案件阶段/状态", &report.stage_mix);

    push_list_section(&mut md, "六、画像判断", &report.strengths);
    push_list_section(&mut md, "七、数据缺口", &report.data_gaps);
    push_list_section(&mut md, "八、下一步可深挖的问题", &report.next_questions);
    md
}

fn push_bucket_section(md: &mut String, title: &str, buckets: &[InsightBucket]) {
    md.push_str(&format!("## {}\n\n", title));
    md.push_str(
        "| 排名 | 项目 | 件数 | 占比 | 标的金额合计 |\n| ---: | --- | ---: | ---: | ---: |\n",
    );
    for (idx, bucket) in buckets.iter().take(8).enumerate() {
        md.push_str(&format!(
            "| {} | {} | {} | {:.0}% | {} |\n",
            idx + 1,
            bucket.label,
            bucket.count,
            bucket.ratio * 100.0,
            format_money(bucket.amount_total)
        ));
    }
    md.push('\n');
}

fn push_list_section(md: &mut String, title: &str, items: &[String]) {
    md.push_str(&format!("## {}\n\n", title));
    for item in items {
        md.push_str(&format!("- {}\n", item));
    }
    md.push('\n');
}

fn round_ratio(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn round_money(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn format_money(v: f64) -> String {
    if v >= 10_000_000.0 {
        format!("{:.2} 亿元", v / 100_000_000.0)
    } else if v >= 10_000.0 {
        format!("{:.2} 万元", v / 10_000.0)
    } else {
        format!("{:.2} 元", v)
    }
}

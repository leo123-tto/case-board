//! 审查意见书生成(2026-06-17 · 合同审查 P1)。
//!
//! 审查结果 → Markdown「审查意见书」→ 复用 `docx_filing::build_report_docx_bytes`(原生 OOXML,
//! 仿宋正文 / 黑体标题 / 表格 / 零外部依赖)出 Word。MD 也回前端做预览。

use crate::contract_review::analyze::ContractReviewResult;

/// 审查意见书的标题。
pub fn opinion_title(contract_name: &str) -> String {
    let n = contract_name.trim();
    if n.is_empty() {
        "合同审查意见书".to_string()
    } else {
        format!("《{}》审查意见书", n)
    }
}

/// 把审查结果拼成审查意见书 Markdown(正文;标题由 `build_report_docx_bytes` 引擎另加)。
///
/// `stance_label` / `strictness_label` 是给人看的中文(如「乙方」「常规」)。
/// `skipped`:P3 redline 阶段无法落入 Word 的降级项说明(P1 传空 slice)。
pub fn build_opinion_md(
    result: &ContractReviewResult,
    stance_label: &str,
    strictness_label: &str,
    skipped: &[String],
) -> String {
    let mut md = String::new();

    // 抬头元信息
    if !result.contract_type.trim().is_empty() {
        md.push_str(&format!("**合同类型:**{}\n\n", result.contract_type.trim()));
    }
    md.push_str(&format!("**审查立场:**{}\n\n", stance_label));
    md.push_str(&format!("**审查口径:**{}\n\n", strictness_label));
    let generated = chrono::Local::now().format("%Y-%m-%d").to_string();
    md.push_str(&format!("**出具日期:**{}\n\n", generated));
    md.push_str("---\n\n");

    // 一、综合审查意见
    md.push_str("## 一、综合审查意见\n\n");
    let summary = result.conclusion.summary.trim();
    if !summary.is_empty() {
        md.push_str(summary);
        md.push_str("\n\n");
    }
    let verdict = result.conclusion.verdict.trim();
    if !verdict.is_empty() {
        md.push_str(&format!("**审查结论:{}**\n\n", verdict));
    }
    let pre: Vec<&String> = result
        .conclusion
        .preconditions
        .iter()
        .filter(|s| !s.trim().is_empty())
        .collect();
    if !pre.is_empty() {
        md.push_str("**签署前先决事项:**\n\n");
        for (i, p) in pre.iter().enumerate() {
            md.push_str(&format!("{}. {}\n", i + 1, p.trim()));
        }
        md.push('\n');
    }

    // 二、风险统计
    let sorted = result.sorted_risks();
    let (mut p0, mut p1, mut p2) = (0u32, 0u32, 0u32);
    for r in &sorted {
        match r.norm_level() {
            "P0" => p0 += 1,
            "P1" => p1 += 1,
            _ => p2 += 1,
        }
    }
    md.push_str("## 二、风险概览\n\n");
    md.push_str(&format!(
        "本次共识别 **{}** 项风险:P0(优先处理){} 项、P1(建议修改){} 项、P2(优化项){} 项。\n\n",
        sorted.len(),
        p0,
        p1,
        p2
    ));

    // 三、详细审查意见(逐项)
    md.push_str("## 三、详细审查意见\n\n");
    if sorted.is_empty() {
        md.push_str("未识别到明显风险点。\n\n");
    }
    for (i, r) in sorted.iter().enumerate() {
        md.push_str(&format!(
            "### {}. [{}] {}\n\n",
            i + 1,
            r.norm_level(),
            r.title.trim()
        ));
        push_kv(&mut md, "条款位置", &r.clause_ref);
        push_kv(&mut md, "风险后果", &r.consequence);
        push_kv(&mut md, "原条款", &r.anchor_text);
        push_kv(&mut md, "整改建议", &r.suggestion);
        push_kv(&mut md, "推荐措辞", &r.recommended_text);
        push_kv(&mut md, "法律依据", &r.basis);
        push_kv(
            &mut md,
            "处理方式",
            if r.wants_revise() {
                "已在修订版正文直接修改并批注"
            } else {
                "以批注形式提示"
            },
        );
        md.push('\n');
    }

    // 四、未落入正文的事项(P3 降级项)
    let skipped: Vec<&String> = skipped.iter().filter(|s| !s.trim().is_empty()).collect();
    if !skipped.is_empty() {
        md.push_str("## 四、未在 Word 中落痕、仅在本意见书提示的事项\n\n");
        md.push_str("以下事项因定位限制(如位于表格 / 文本框,或原文无法精确匹配)未在修订版正文落痕,请在本意见书中查阅:\n\n");
        for (i, s) in skipped.iter().enumerate() {
            md.push_str(&format!("{}. {}\n", i + 1, s.trim()));
        }
        md.push('\n');
    }

    // 声明
    md.push_str("---\n\n");
    md.push_str("> **声明:**本意见书基于所提供合同文本及现有信息出具,仅供内部决策参考,不构成对交易最终法律判断;标注「待核实」「未提及/待补充」处需另行核实。最终签署决策由委托方作出。\n\n");
    md.push_str("> 本意见书由 CaseBoard 合同审查辅助生成,审查方法论参考杨卫薪律师 contract-copilot(CC BY-NC),具体审查内容由本系统自建。\n");

    md
}

fn push_kv(md: &mut String, k: &str, v: &str) {
    let v = v.trim();
    if v.is_empty() {
        return;
    }
    md.push_str(&format!("- **{}:**{}\n", k, v));
}

/// 审查意见书 → Word(原生 OOXML)。复用 base 档报告引擎。
pub fn build_opinion_docx(
    result: &ContractReviewResult,
    contract_name: &str,
    stance_label: &str,
    strictness_label: &str,
    skipped: &[String],
) -> Result<Vec<u8>, String> {
    let title = opinion_title(contract_name);
    let md = build_opinion_md(result, stance_label, strictness_label, skipped);
    crate::docx_filing::build_report_docx_bytes(&title, &md, None)
}

//! 2026-05-25 V0.1.7 · 完整报告。
//!
//! 把「风险报告」(查被执行人结果)+「深挖报告」两份 MD 一起喂 DeepSeek,
//! 出第三份「完整报告」,统一画像 + 风险 + 财产线索 + 抗辩 + 行动清单。
//!
//! 触发:用户在执行详情页点「查看完整报告」时,若 cases.full_report_path
//! 为 NULL 则调本命令,否则直接弹现有的 MD。

use serde::Serialize;
use sqlx::SqlitePool;

use crate::llm::LlmConfig;
use crate::yuandian::reports_dir_for_case;

const SYSTEM_PROMPT: &str = r###"你是律师执行追踪助理 LLM。

我提供两份报告作为输入:
- **风险报告**:基于元典 API 拉到的工商 / 失信 / 限消 / 历史诉讼 / 股权 / 不动产等基础信息,给出的初步风险评估
- **深挖报告**:基于风险报告的 dig_hints 进一步追查的关联公司 / 案号 / 第三方主体 / 历史股东等深度信息

请综合这两份报告,出一份「完整报告」(MD 格式,直接返回 MD 内容,不要包 ```)。结构:

# 完整执行追踪报告

## 一、案件核心结论
2-3 句话讲清:
- 被执行人是什么主体(自然人 / 公司 / 个体)
- 当前还款能力初判(强 / 中 / 弱 / 不明)
- 是否值得加大执行投入(给明确建议)

## 二、被执行人画像
合并两份报告里所有关于被执行人的信息:
- 主体基本面(经营状态 / 历史诉讼 / 失信限消)
- 财产线索(股权 / 不动产 / 账户 / 银行流水 / 应收账款)
- 关联主体(配偶 / 历史股东 / 关联公司 / 同地址企业)

## 三、可执行的财产线索
采用**四通保全详图**汇总,先给一个保全优先级矩阵,再按四条通道展开:
- 通道一 · 直接锁定:可直接冻结/查封的公开线索,如对外投资股权、股权冻结/出质、担保、明确资产。
- 通道二 · 调查追踪:需调查令、法院调取或实地核实的线索,如地址、历史变更、土地/在建工程、知识产权、经营现金流。
- 通道三 · 债权拦截:可向第三方付款方或其他法院设卡的债权/收益,如招投标应收、胜诉债权、经营合同收益。
- 通道四 · 关联穿透:股东出资、分支机构、实控人/关联企业、历史股东或同地址主体。

每条线索统一写「发现 → 评估 → 操作」,必须给:
- 发现:具体线索是什么(如"某某公司 30% 股权")、来自风险报告 or 深挖报告的哪段。
- 评估:可执行性强弱、预计价值区间、竞合/质押/前序冻结风险。
- 操作:协助执行通知 / 调查令 / 现场勘验 / 轮候冻结 / 追加或变更被执行人等下一步。

## 四、⚠️ 拒执风险线索(2026-05-25 V0.1.9 新增)
**仅在两份输入报告里能找到时间证据时才写本节;找不到就明确写"未发现立案后变更"**

corpus 顶部「案件元信息」段会给立案日。请扫两份报告,只列**立案日之后**发生的:
- 股东变更 / 股权转让(可疑:转移财产)
- 注册资本减少 / 抽逃出资
- 法定代表人变更
- 新增对外投资(可疑:转移现金到新主体)
- 注销 / 经营异常
- 任何与本案债务存在金额规模相当的资产处分

按时间倒序列表,每条给:
- **时间**(从原报告里摘) / **事项** / **数据来源**(哪个端点) / **风险类型**(转移财产 / 抽逃出资 / 隔离关联 / 其他)
- **风险等级初判**:🔴 高 / 🟠 中 / 🟡 低
- **法律后果可能性**(可能构成《刑法》313 条拒执罪 / 《合同法》74 条转移财产撤销之诉 / 最高院 2019/11 司法解释追加股东 / 等)

**只列事实 + 判断,不给"建议追加 / 撤销 / 报案"等具体行动建议** — 那是律师的判断范畴,工具只做事实呈现。

## 五、对抗风险与对方可能抗辩
列出对方可能采取的抗辩 / 隐匿行为:
1. 主体不适格抗辩
2. 财产转移痕迹(过户 / 增资 / 减资 / 股权变动时间点)
3. 关联公司隔离(实控人嵌套)
4. 历史诉讼当事人不同(同名异主体)
等

## 六、自然人主体说明(如果有)
对每个自然人被执行人,固定输出:

> ⚠️ **自然人 {姓名}**(身份证 {如有}):元典法律开放平台**未提供**按身份证查询自然人涉诉 / 失信 / 执行信息的接口。
> 请律师自行通过:裁判文书网 / 中国执行信息公开网 / 信用中国 核查。

**不要硬编**任何自然人执行 / 失信 / 文书记录 — 没有数据来源。

---

# 铁律

1. **不能编造**:只能基于两份输入材料,无原始数据支撑的不写
2. **不要重复**:风险报告已有的不要原文照搬,做提炼整合
3. **中文专业**:法律 / 工商 / 财务术语准确,避免口语
4. **直接给报告**:不要"以下是综合分析"等元话术,直接 # 标题开始
5. **金额用规范表达**:如 "30 万元" / "1,500 万元"
6. **拒执线索一节只列事实 + 判断,不给行动建议** — 工具只做原始数据整理 + 基础判断
7. 报告控制在 2000-4000 字
"###;

#[derive(Serialize)]
pub struct FullReportResult {
    pub case_id: String,
    pub report_path: Option<String>,
    pub generated_at: String,
    pub elapsed_ms: u128,
    pub error: Option<String>,
}

pub async fn run_full_report(
    pool: &SqlitePool,
    case_id: &str,
    llm_config: &LlmConfig,
) -> FullReportResult {
    let start = std::time::Instant::now();
    let err = |msg: String| FullReportResult {
        case_id: case_id.to_string(),
        report_path: None,
        generated_at: String::new(),
        elapsed_ms: start.elapsed().as_millis(),
        error: Some(msg),
    };

    // 1) 读 cases 拿两份前置报告的路径
    let row: (Option<String>, Option<String>) = match sqlx::query_as(
        "SELECT risk_assessment_path, deep_dive_report_path FROM cases WHERE id = ?",
    )
    .bind(case_id)
    .fetch_one(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => return err(format!("查 cases 失败:{}", e)),
    };

    let risk_path = match row.0 {
        Some(p) if !p.is_empty() => p,
        _ => return err("尚未生成风险报告(请先点「查被执行人」)".into()),
    };
    let dig_path = match row.1 {
        Some(p) if !p.is_empty() => p,
        _ => return err("尚未生成深挖报告(请先点「深挖」)".into()),
    };

    // 2) 读两份 MD
    let risk_md = match std::fs::read_to_string(&risk_path) {
        Ok(s) => s,
        Err(e) => return err(format!("读风险报告失败:{}", e)),
    };
    let dig_md = match std::fs::read_to_string(&dig_path) {
        Ok(s) => s,
        Err(e) => return err(format!("读深挖报告失败:{}", e)),
    };

    // 2026-05-25 V0.1.9 加 · 案件元信息(含立案日,prompt 用作拒执 cutoff)
    let case_meta = super::fetch_case_meta_md(pool, case_id).await;
    let corpus = format!(
        "{}\n========== 风险报告 ==========\n{}\n\n========== 深挖报告 ==========\n{}",
        case_meta, risk_md, dig_md
    );

    // 3) 调 LLM(纯 MD 输出,不要 JSON 格式约束;Phase 2:按执行立场包 system prompt)
    let stance = super::orchestrator::exec_stance_for_case(pool, case_id).await;
    let system = stance.wrap_system(SYSTEM_PROMPT);
    let content = match super::call_llm(
        llm_config,
        &system,
        &corpus,
        super::LlmCallOpts {
            max_tokens: 8192,
            temperature: 0.1,
            timeout_mult: 3,
            json_object: false,
        },
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return err(e),
    };

    // 剥 markdown fence 防御一下(model 偶尔会包 ```markdown ... ```,B12 统一到 yuandian::strip_md_fence)
    let content = super::strip_md_fence(&content);

    // 4) 落盘
    let reports_dir = match reports_dir_for_case(case_id) {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    if let Err(e) = std::fs::create_dir_all(&reports_dir) {
        return err(format!("建 reports 目录失败:{}", e));
    }
    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let report_path = reports_dir.join(format!("full_{}.md", ts));
    if let Err(e) = std::fs::write(&report_path, &content) {
        return err(format!("写完整报告 MD 失败:{}", e));
    }
    let report_path_str = report_path.to_string_lossy().to_string();

    // 5) 写 cases
    let now = chrono::Utc::now().to_rfc3339();
    if let Err(e) =
        sqlx::query("UPDATE cases SET full_report_path = ?, full_report_at = ? WHERE id = ?")
            .bind(&report_path_str)
            .bind(&now)
            .bind(case_id)
            .execute(pool)
            .await
    {
        crate::dlog!("[full_report] 写 cases 失败:{}", e);
    }

    FullReportResult {
        case_id: case_id.to_string(),
        report_path: Some(report_path_str),
        generated_at: now,
        elapsed_ms: start.elapsed().as_millis(),
        error: None,
    }
}

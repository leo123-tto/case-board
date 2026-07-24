//! 元典原始 JSON → DeepSeek 风险评估报告(2026-05-24 k · P1.2)。
//!
//! 输入:Orchestrator 跑完后的 raw JSON 文件路径列表(每个文件是一个 endpoint 的响应)
//! 流程:
//!   1. 读所有 JSON,拼成结构化 corpus(按 subject 分组,每 subject 下列出各 endpoint 数据)
//!   2. 喂 DeepSeek 单次 LLM call → 输出 `{ report_md, dig_hints }` JSON
//!      - report_md:风险提示报告 MD(参考股权转让案件那个 yuandian_深查 格式)
//!      - dig_hints:深挖建议列表(关联公司 / 案号 / 主体名 + 为什么深挖)
//!   3. 报告 MD 落 `reports/risk_<case_id>_<ts>.md`,dig_hints JSON 落 `reports/dig_hints_<case_id>_<ts>.json`
//!   4. 写 cases.risk_assessment_path + risk_assessment_at

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::llm::LlmConfig;
use crate::yuandian::reports_dir_for_case;

#[derive(Debug, Clone, Deserialize)]
pub struct AssessmentOutput {
    pub report_md: String,
    /// 深挖建议:列出值得 P2 深挖的目标(关联公司名 / 案号 / 自然人姓名 + reason)
    pub dig_hints: Vec<DigHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigHint {
    /// "enterprise" | "case" | "person"
    pub kind: String,
    pub target: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssessmentReport {
    pub case_id: String,
    pub report_path: Option<String>,
    pub dig_hints_path: Option<String>,
    pub dig_hints: Vec<DigHint>,
    pub raw_count: usize,
    pub corpus_chars: usize,
    pub elapsed_ms: u128,
    pub error: Option<String>,
}

const SYSTEM_PROMPT: &str = r###"你是资深律师 + 商业调查分析师,精通从企业 / 司法公开数据中提取财产线索、判断执行优先级。

我会给你**同一个执行案件**对各个被执行人(自然人 / 企业)从元典法律开放平台查到的**所有原始 JSON 数据**(按 subject 分组,每 subject 下列出十几个 endpoint 的响应)。

请你**通读所有数据**,综合判断后输出一个 JSON 对象,包含两部分:

1. `report_md` — **风险提示报告**(中文 Markdown),帮律师 5 分钟内看完关键发现
2. `dig_hints` — **深挖建议列表**(JSON 数组),列出值得继续深挖的目标

# 输出 JSON 格式

{
  "report_md": "## 摘要\n...完整 Markdown...",
  "dig_hints": [
    {"kind":"enterprise","target":"无锡XX科技有限公司","reason":"被执行人 50% 出资,新发现的关联公司,需查其名下财产"},
    {"kind":"case","target":"(2025)苏0213执5108号","reason":"被执行人新执行案件,需查标的金额 + 法院"},
    {"kind":"person","target":"张三","reason":"被执行人配偶,可能存在共同财产线索"}
  ]
}

# report_md 结构(用 ## 二级标题,推荐顺序)

财产线索部分采用**四通保全详图**组织,不要按数据类型平铺。四条通道固定为:
- 通道一 · 直接锁定:已有公开数据支撑、可直接申请法院冻结/查封的线索,如对外投资股权、股权冻结/出质、动产/土地抵押、明确账户或司法拍卖线索。
- 通道二 · 调查追踪:存在方向但需调查令、法院调取或现场核实的线索,如曾用名、地址/设备库存、土地与在建工程、高价值知识产权、资质与经营现金流。
- 通道三 · 债权拦截:可向第三方付款方或其他法院设卡的到期债权/预期收益,如招投标项目应收账款、胜诉债权、经营合同收益。
- 通道四 · 关联穿透:通过股东出资责任、分支机构财产、实控人/关联企业、历史关联方扩大执行范围的线索。

每条财产线索必须统一写成「发现 → 评估 → 操作」:
- 发现:原始 JSON 里具体出现了什么,写明端点/主体/金额/时间。
- 评估:可执行性、剩余价值、前序冻结/质押/竞合风险。
- 操作:调查令、协助执行通知、轮候冻结、追加/变更被执行人等下一步动作;仅给执行动作建议,不替律师下最终策略判断。

## 摘要
1-3 句话总结:有几个被执行人 / 找到几条关键风险 / 最值得关注的财产线索

## 主体 <name>(<企业|自然人>)
对每个被执行人独立一节,内容包括(仅写有数据的部分):

### 关键画像
- (企业)法代 / 股东构成 / 注册资本 / 经营状态 / 登记机关
- (自然人)身份证号 / 户籍 / 涉诉文书数

### 被执行案件全景
表格:立案日期 / 案号 / 标的金额 / 法院 / 状态。**特别标注本案 vs 其他案件的关系**(本案是最大单笔?其他案件是否在执行中?)

### 失信 / 限消
是否在失信被执行人名单 / 限制高消费;具体执行法院 + 行为情形

### 财产线索 ⭐
按四通保全详图写成四个小节:通道一 · 直接锁定 / 通道二 · 调查追踪 / 通道三 · 债权拦截 / 通道四 · 关联穿透。每条线索都用「发现 → 评估 → 操作」三段;没有数据的通道也明确写"未发现公开记录",不要留白。

### 涉诉 / 公告
- 新增法院文书(本案以外的诉讼)
- 法院公告 / 开庭公告(对方有新债权人?)
- 工商变更(关键时间点是否与本案时间耦合?)

### 行政与合规风险
经营异常 / 严重违法 / 行政处罚

### 数据干净度
失信 0 / 行政处罚 0 / 股权冻结 0 / 担保 0 等等 — **如果对方各项数据干净,要明确说"暂未发现 XX",别留白**

## ⚠️ 拒执风险线索(2026-05-25 V0.1.9 新增,仅在能找到时间证据时写)

corpus 顶部「案件元信息」段会给立案日。请扫所有 raw JSON,只列**立案日之后**发生的:
- 股东变更 / 股权转让(可疑:转移财产)
- 注册资本减少 / 抽逃出资
- 法定代表人变更
- 新增对外投资(可疑:转移现金到新主体)
- 注销 / 经营异常
- 任何与本案债务规模相当的资产处分

按时间倒序列表,每条给:
- **时间** / **事项** / **数据来源**(哪个端点) / **风险类型**(转移财产 / 抽逃出资 / 隔离关联 / 其他)
- **风险等级初判**:🔴 高 / 🟠 中 / 🟡 低
- **法律后果可能性**(可能构成《刑法》313 条拒执罪 / 《合同法》74 条转移财产撤销之诉 / 最高院 2019/11 司法解释追加股东 / 等)

**只列事实 + 判断,不给"建议追加股东 / 撤销之诉 / 报案"等具体行动建议** — 那是律师的判断范畴,工具只做事实呈现。

如果立案日为"未抽到"或者所有变更都在立案日之前,本节直接写"未发现立案后疑似拒执变更"。

## 数据来源
列出本次查询用到的元典端点 + 查询时间

# dig_hints 选目标的逻辑

**宁全勿精,挖一切线索 — 无上限**。原则:"宁可全而不要精简,因为不知道哪一点最后是有效信息"。
深挖费用很低,但任何一点线索成为执行突破口的价值都很大。

写到 dig_hints 数组的项,**只要满足下面任一条件就加入,不要主动精简**:
1. **关联公司**:被执行人持股 / 控股的子公司 / 母公司 / 全资 / 参股(所有有股权关系的都列)
2. **新发现案号**:被执行人有的其他执行案件 / 涉诉案件(都列)
3. **第三方主体**:出现在工商变更 / 担保 / 出质 / 法院公告 里的所有关键自然人或企业(包括质权人 / 担保人 / 共同被告 / 共同被执行人)
4. **配偶 / 关联自然人**:同住地址 / 同案件出现的人 / 历史股东
5. **同名异主体**:文书检索时出现的同名异身份证人(记下方便后续排除)
6. **历史股东 / 离职法代**:工商变更里出现过的、跟本案时间点耦合的退出者
7. **关联地址主体**:跟被执行人同注册地址的其他企业

**没有上限**。每条 reason 一句话讲清"为什么值得深挖"。dig_hints 全量列表会落盘到本地 MD,所有线索都保留,不会丢。

# 自然人主体处理(2026-05-25 V0.1.9 新增)

对每个自然人被执行人,corpus 里会有一个 `<姓名>_placeholder.md` 文件(不是 JSON,是占位说明)。
固定输出格式:

> ⚠️ **自然人 {姓名}**(身份证 {如有,从 OCR 数据来}):元典法律开放平台**未提供**按身份证查询自然人涉诉 / 失信 / 执行信息的接口。
> 请律师自行通过:裁判文书网 https://wenshu.court.gov.cn/ / 中国执行信息公开网 http://zxgk.court.gov.cn/ / 信用中国 https://www.creditchina.gov.cn/ 核查。

**不要硬编**任何自然人执行 / 失信 / 文书记录 — 没有数据来源。

dig_hints 里**不要**针对自然人产生 `kind:"person"` 的深挖(元典不支持);
如果自然人持股 / 任职某企业,改成 `kind:"enterprise"` 列那个企业。

# 铁律

1. 不能编造数据;原始 JSON 没出现的事实不能写
2. 数据干净也要明确写"未发现 X"(透明)
3. 中文,专业,简洁;**不要写"根据您提供的资料"等元话术**,直接给报告
4. JSON 字符串里的换行用 \n,不要真实换行
5. report_md 一定要包含「⚠️ 拒执风险线索」这一节(没有发现也要明确写"未发现立案后疑似变更")
6. **拒执线索只列事实 + 判断,不给行动建议** — 工具只做原始数据整理 + 基础判断
"###;

pub async fn run_assessment(
    pool: &SqlitePool,
    case_id: &str,
    llm_config: &LlmConfig,
    raw_files: &[String],
) -> AssessmentReport {
    let start = std::time::Instant::now();
    if let Err(error) = super::artifact_binding::current_owner(pool, case_id).await {
        return error_report(case_id, &error, start);
    }

    let raw_dir = match raw_dir_for_case(case_id) {
        Ok(p) => p,
        Err(e) => return error_report(case_id, &e, start),
    };

    // 2026-05-25 V0.1.9 加 · 案件元信息(立案日 → 拒执 cutoff)
    let mut corpus = super::fetch_case_meta_md(pool, case_id).await;
    let mut read_ok = 0;
    for fname in raw_files {
        let path = raw_dir.join(fname);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                crate::dlog!("[risk] 读 {} 失败:{}", path.display(), e);
                continue;
            }
        };
        // 按文件名分段(<subject>_<endpoint>.json 或 <subject>_placeholder.md)
        corpus.push_str(&format!("\n========== {} ==========\n", fname));
        corpus.push_str(&content);
        corpus.push('\n');
        read_ok += 1;
    }

    if read_ok == 0 {
        return error_report(case_id, "无可读 raw JSON 文件", start);
    }

    let corpus_chars = corpus.chars().count();
    crate::dlog!(
        "[risk] case={} 读 {} 个 raw 文件,corpus {} chars",
        case_id,
        read_ok,
        corpus_chars
    );

    // Phase 2:按执行立场包 system prompt(债务人模式前置防御总纲)
    let stance = super::orchestrator::exec_stance_for_case(pool, case_id).await;
    let system = stance.wrap_system(SYSTEM_PROMPT);

    // 调 DeepSeek
    let raw = match super::call_llm(
        llm_config,
        &system,
        &corpus,
        super::LlmCallOpts {
            max_tokens: 12288,
            temperature: 0.0,
            timeout_mult: 3,
            json_object: true,
        },
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return error_report(case_id, &e, start),
    };

    let cleaned = super::strip_md_fence(&raw);
    let parsed: AssessmentOutput = match serde_json::from_str(&cleaned) {
        Ok(v) => v,
        Err(e) => {
            return error_report(
                case_id,
                &format!("LLM 输出 JSON 解析失败:{}\n---\n{}", e, cleaned),
                start,
            )
        }
    };

    // 落盘
    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let reports_dir = match reports_dir_for_case(case_id) {
        Ok(p) => p,
        Err(e) => return error_report(case_id, &e, start),
    };
    if let Err(e) = std::fs::create_dir_all(&reports_dir) {
        return error_report(case_id, &format!("建 reports 目录失败:{}", e), start);
    }
    let report_path = reports_dir.join(format!("risk_{}.md", ts));
    let dig_path = reports_dir.join(format!("dig_hints_{}.json", ts));
    let dig_md_path = reports_dir.join(format!("dig_hints_{}.md", ts));

    if let Err(e) = std::fs::write(&report_path, &parsed.report_md) {
        crate::dlog!("[risk] 写 report MD 失败:{}", e);
    }
    if let Err(e) = std::fs::write(
        &dig_path,
        serde_json::to_string_pretty(&parsed.dig_hints).unwrap_or_else(|_| "[]".into()),
    ) {
        crate::dlog!("[risk] 写 dig hints JSON 失败:{}", e);
    }
    // 同步落一个可读 MD 版(2026-05-25 作者要求:全量保留 + 方便人工查阅)
    if let Err(e) = std::fs::write(&dig_md_path, render_dig_hints_md(&parsed.dig_hints, &ts)) {
        crate::dlog!("[risk] 写 dig hints MD 失败:{}", e);
    }

    let report_path_str = report_path.to_string_lossy().to_string();
    let dig_path_str = dig_path.to_string_lossy().to_string();

    // 原子发布 P1 路径、raw 清单和生成时委托人快照；新 P1 同时使旧 P2/full 失效。
    let now = chrono::Utc::now().to_rfc3339();
    if let Err(error) = super::artifact_binding::publish_p1(
        pool,
        case_id,
        super::artifact_binding::P1Artifacts {
            risk_report_path: report_path_str.clone(),
            dig_hints_path: dig_path_str.clone(),
            raw_files: raw_files.to_vec(),
        },
        &now,
    )
    .await
    {
        return error_report(
            case_id,
            &format!("绑定 P1 产物到当前委托人失败:{error}"),
            start,
        );
    }

    AssessmentReport {
        case_id: case_id.to_string(),
        report_path: Some(report_path_str),
        dig_hints_path: Some(dig_path_str),
        dig_hints: parsed.dig_hints,
        raw_count: read_ok,
        corpus_chars,
        elapsed_ms: start.elapsed().as_millis(),
        error: None,
    }
}

fn raw_dir_for_case(case_id: &str) -> Result<PathBuf, String> {
    let base = crate::db::app_data_dir().map_err(|e| format!("无法定位 app data dir: {}", e))?;
    Ok(base.join("external").join(case_id).join("yuandian_raw"))
}

/// 把 dig_hints 数组渲染成人类友好的 MD(分类按 kind 分节,reason 缩进显示)
fn render_dig_hints_md(hints: &[DigHint], ts: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("# 深挖建议清单 · {}\n\n", ts));
    s.push_str(&format!(
        "共 **{}** 条线索 — 「🔬 深挖」按钮会全部跑一遍\n\n",
        hints.len()
    ));
    s.push_str("---\n\n");

    // 按 kind 分组
    let mut by_kind: std::collections::BTreeMap<&str, Vec<&DigHint>> = Default::default();
    for h in hints {
        by_kind.entry(h.kind.as_str()).or_default().push(h);
    }

    for (kind, group) in &by_kind {
        let label = match *kind {
            "enterprise" => "🏢 关联公司 / 主体企业",
            "case" => "⚖️ 新发现案号",
            "person" => "👤 关联自然人",
            other => other,
        };
        s.push_str(&format!("## {}({} 条)\n\n", label, group.len()));
        for (i, h) in group.iter().enumerate() {
            s.push_str(&format!("{}. **{}**\n", i + 1, h.target));
            s.push_str(&format!("   - 深挖理由:{}\n\n", h.reason));
        }
    }

    s.push_str("---\n\n");
    s.push_str("> 数据来源:元典法律开放平台原始 JSON → DeepSeek 风险评估时生成\n");
    s.push_str("> 同目录 `dig_hints_<ts>.json` 是机器可读版,深挖时由后端读取\n");
    s
}

fn error_report(case_id: &str, err: &str, start: std::time::Instant) -> AssessmentReport {
    crate::dlog!("[risk] case={} 失败:{}", case_id, err);
    AssessmentReport {
        case_id: case_id.to_string(),
        report_path: None,
        dig_hints_path: None,
        dig_hints: vec![],
        raw_count: 0,
        corpus_chars: 0,
        elapsed_ms: start.elapsed().as_millis(),
        error: Some(err.to_string()),
    }
}

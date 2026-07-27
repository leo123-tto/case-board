//! P2 深挖:按 P1 LLM 给的 dig_hints 拉关联公司 / 案号 / 第三方主体 →
//! 出深查报告(参考股权转让案件 yuandian_深查 格式)。
//!
//! 输入:case_id + 之前 P1 生成的 dig_hints.json 路径
//! 流程:
//!
//! 1. 读 dig_hints(类型 enterprise / case / person)
//! 2. 对每个 hint 分别处理:enterprise 走 search → id → aggregation + executions +
//!    executed + out_invest + frozen_equity + pledge + writ_list;case 走 search_qwal 按案号
//!    (ah=) 拿详情;person 走 search_ptal + qwal 按姓名
//! 3. 原始 JSON 落 external/<case_id>/yuandian_deepdive/<target>_<endpoint>.json
//! 4. P1 + P2 全部 JSON 拼一起喂 DeepSeek → 出深查报告 MD
//! 5. 落 reports/deepdive_<ts>.md + 写 cases.deep_dive_report_path

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::llm::LlmConfig;
use crate::yuandian::risk_assessment::DigHint;
use crate::yuandian::{self, file_name, reports_dir_for_case, save_json, EntityId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepDiveReport {
    pub case_id: String,
    pub hints_used: usize,
    pub raw_count: usize,
    pub corpus_chars: usize,
    pub report_path: Option<String>,
    pub elapsed_ms: u128,
    pub error: Option<String>,
}

/// 单次 P2 深挖
pub async fn run_deep_dive(
    pool: &SqlitePool,
    case_id: &str,
    api_key: &crate::yuandian::YuandianCredentialSource,
    llm_config: &LlmConfig,
) -> Result<DeepDiveReport, String> {
    // 保持到全部 hints/原始文件/元典/LLM 工作结束，禁止查询中变更精确委托人。
    let _query_guard = super::orchestrator::acquire_execution_query_read_guard(case_id).await;
    // 必须先于 hints 目录读取、原始数据落盘、元典与 LLM 调用完成精确委托人守卫。
    super::orchestrator::exact_representation_for_external_query(pool, case_id).await?;
    let bound_p1 = super::artifact_binding::load_p1_for_current_owner(pool, case_id).await?;
    Ok(run_deep_dive_inner(pool, case_id, api_key, llm_config, bound_p1).await)
}

async fn run_deep_dive_inner(
    pool: &SqlitePool,
    case_id: &str,
    api_key: &crate::yuandian::YuandianCredentialSource,
    llm_config: &LlmConfig,
    bound_p1: Option<super::artifact_binding::P1Artifacts>,
) -> DeepDiveReport {
    let start = std::time::Instant::now();

    // 1. 读最新 dig_hints.json
    let hints_dir = match reports_dir_for_case(case_id) {
        Ok(p) => p,
        Err(e) => return error_report(case_id, &e, start, 0, 0, 0),
    };
    let hints = match bound_p1.as_ref() {
        Some(p1) => load_hints_at_path(std::path::Path::new(&p1.dig_hints_path)),
        None => load_latest_hints(&hints_dir),
    };
    let hints = match hints {
        Ok(h) => h,
        Err(e) => return error_report(case_id, &e, start, 0, 0, 0),
    };
    if hints.is_empty() {
        return error_report(
            case_id,
            "没找到深挖建议(请先点「🔍 查被执行人」生成 P1 报告 + 深挖建议)",
            start,
            0,
            0,
            0,
        );
    }

    // 2. 准备深挖输出目录
    let deep_dir = match deep_raw_dir_for_case(case_id) {
        Ok(p) => p,
        Err(e) => return error_report(case_id, &e, start, hints.len(), 0, 0),
    };
    if let Err(e) = std::fs::create_dir_all(&deep_dir) {
        return error_report(
            case_id,
            &format!("建目录失败:{}", e),
            start,
            hints.len(),
            0,
            0,
        );
    }

    let mut raw_files = Vec::new();
    let hints_used = hints.len();

    for hint in &hints {
        crate::dlog!(
            "[deepdive] case={} hint kind={} target={}",
            case_id,
            hint.kind,
            hint.target
        );
        match hint.kind.as_str() {
            "enterprise" => {
                if let Err(e) =
                    dig_enterprise(api_key, &hint.target, &deep_dir, &mut raw_files).await
                {
                    crate::dlog!("[deepdive] {} (enterprise) 失败:{}", hint.target, e);
                }
            }
            "case" => {
                if let Err(e) = dig_case(api_key, &hint.target, &deep_dir, &mut raw_files).await {
                    crate::dlog!("[deepdive] {} (case) 失败:{}", hint.target, e);
                }
            }
            "person" => {
                if let Err(e) = dig_person(api_key, &hint.target, &deep_dir, &mut raw_files).await {
                    crate::dlog!("[deepdive] {} (person) 失败:{}", hint.target, e);
                }
            }
            _ => crate::dlog!("[deepdive] 未知 kind: {}", hint.kind),
        }
    }

    // D2-1:深挖(P2)也记元典积分 —— 同 orchestrator 口径。每个落盘 .json = 一次计费端点调用
    // (aggregation 5、其余 1)。修"执行模块调元典不记账、月度统计漏算"。
    {
        let year_month = crate::db::credits::current_year_month();
        for f in &raw_files {
            let c = crate::db::credits::credits_for_raw_file(f);
            if c > 0 {
                let _ = crate::db::credits::record_yuandian_call(pool, &year_month, c).await;
            }
        }
    }

    if raw_files.is_empty() {
        return error_report(case_id, "所有深挖目标都没拿到数据", start, hints_used, 0, 0);
    }

    // 3. 拼 corpus(案件元信息 + P1 raw + P2 raw)
    // 2026-05-25 V0.1.9 加 · 案件元信息顶部(立案日 → 拒执 cutoff)
    let mut corpus = super::fetch_case_meta_md(pool, case_id).await;
    corpus.push_str("\n# P1 基础查询 (元典对被执行人的查询结果)\n");
    if let Ok(p1_dir) = raw_dir_for_case(case_id) {
        if let Some(p1) = &bound_p1 {
            for name in &p1.raw_files {
                if let Ok(content) = std::fs::read_to_string(p1_dir.join(name)) {
                    corpus.push_str(&format!("\n========== P1 / {} ==========\n", name));
                    corpus.push_str(&content);
                    corpus.push('\n');
                }
            }
        } else if let Ok(entries) = std::fs::read_dir(&p1_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if let Ok(content) = std::fs::read_to_string(entry.path()) {
                        corpus.push_str(&format!("\n========== P1 / {} ==========\n", name));
                        corpus.push_str(&content);
                        corpus.push('\n');
                    }
                }
            }
        }
    }
    corpus.push_str("\n# P2 深挖查询 (按 LLM 给的 dig_hints 拉关联公司/案号/第三方)\n");
    for fname in &raw_files {
        let path = deep_dir.join(fname);
        if let Ok(content) = std::fs::read_to_string(&path) {
            corpus.push_str(&format!("\n========== P2 / {} ==========\n", fname));
            corpus.push_str(&content);
            corpus.push('\n');
        }
    }

    let corpus_chars = corpus.chars().count();
    crate::dlog!(
        "[deepdive] case={} hints={} raw={} corpus={} chars",
        case_id,
        hints_used,
        raw_files.len(),
        corpus_chars
    );

    // 4. 拼 hints 摘要喂 LLM
    let hint_summary = hints
        .iter()
        .map(|h| format!("- [{}] {} — {}", h.kind, h.target, h.reason))
        .collect::<Vec<_>>()
        .join("\n");

    let user_msg = format!(
        "# LLM 之前给的深挖建议\n\n{}\n\n# 所有元典原始数据(P1 + P2)\n\n{}",
        hint_summary, corpus
    );

    // 5. 调 DeepSeek 出深查报告(Phase 2:按执行立场包 system prompt)
    let stance = super::orchestrator::exec_stance_for_case(pool, case_id).await;
    let system = stance.wrap_system(SYSTEM_PROMPT_DEEPDIVE);
    let raw = match super::call_llm(
        llm_config,
        &system,
        &user_msg,
        super::LlmCallOpts {
            max_tokens: 16384,
            temperature: 0.0,
            timeout_mult: 4,
            json_object: false,
        },
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            return error_report(
                case_id,
                &e,
                start,
                hints_used,
                raw_files.len(),
                corpus_chars,
            )
        }
    };

    // 6. 报告 MD 落盘(剥 markdown fence 防御:model 偶尔把整篇报告裹进 ```markdown ... ```,B12 统一)
    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let report_path = hints_dir.join(format!("deepdive_{}.md", ts));
    let content = super::strip_md_fence(&raw);
    if let Err(e) = std::fs::write(&report_path, &content) {
        return error_report(
            case_id,
            &format!("写报告 MD 失败:{}", e),
            start,
            hints_used,
            raw_files.len(),
            corpus_chars,
        );
    }
    let report_path_str = report_path.to_string_lossy().to_string();

    // 7. 新版 P1 原子推进同一 owner 的 P2 绑定；历史粗立场 P1 保留旧路径兼容。
    let now = chrono::Utc::now().to_rfc3339();
    let publish_result = if bound_p1.is_some() {
        super::artifact_binding::publish_deep(pool, case_id, &report_path_str, &now).await
    } else {
        sqlx::query("UPDATE cases SET deep_dive_report_path = ?, deep_dive_at = ? WHERE id = ?")
            .bind(&report_path_str)
            .bind(&now)
            .bind(case_id)
            .execute(pool)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    };
    if let Err(error) = publish_result {
        return error_report(
            case_id,
            &format!("绑定深挖产物到当前委托人失败:{error}"),
            start,
            hints_used,
            raw_files.len(),
            corpus_chars,
        );
    }

    DeepDiveReport {
        case_id: case_id.to_string(),
        hints_used,
        raw_count: raw_files.len(),
        corpus_chars,
        report_path: Some(report_path_str),
        elapsed_ms: start.elapsed().as_millis(),
        error: None,
    }
}

const SYSTEM_PROMPT_DEEPDIVE: &str = r###"你是资深律师 + 商业调查分析师。我已经拿到一个执行案件的:
1. P1 基础查询(对被执行人本身的元典数据)
2. P2 深挖查询(按之前给的 dig_hints 拉的关联公司 / 案号 / 第三方主体的元典数据)

请你**综合所有数据**写一份**深查报告**(中文 Markdown),帮律师 5 分钟看完关键发现 + 拿到可执行的下一步。

# 报告结构(用 ## 二级标题,顺序固定)

财产线索采用**四通保全详图**组织,不要按数据类型平铺。四条通道固定为:
- 通道一 · 直接锁定:可直接冻结/查封的公开线索,如对外投资股权、股权冻结/出质、担保、明确资产。
- 通道二 · 调查追踪:需调查令、法院调取或实地核实的线索,如地址、历史变更、土地/在建工程、知识产权、经营现金流。
- 通道三 · 债权拦截:可向第三方付款方或其他法院设卡的债权/收益,如招投标应收、胜诉债权、经营合同收益。
- 通道四 · 关联穿透:股东出资、分支机构、实控人/关联企业、历史股东或同地址主体。

每条财产线索统一写「发现 → 评估 → 操作」:
- 发现:只摘 P1/P2 原始数据中出现的主体、金额、时间、端点。
- 评估:可执行性、前序冻结/质押/竞合、关联程度和剩余价值。
- 操作:调查令、协助执行通知、轮候冻结、追加/变更被执行人等动作建议;不替律师下最终策略判断。

## 摘要
一段话(2-4 句)总结这次深挖最有价值的 3 个发现 + 是否值得继续追。

## 主体 1:<被执行人名字>(<企业|自然人>)
对每个原本就在 P1 出现的被执行人独立一节。内容(只写有数据的部分):

### 关键画像
- (企业)法代 / 股东 / 注册资本 / 经营状态 / 注册地址
- (自然人)身份证号 / 户籍 / 文书命中数

### 被执行案件全景
表格(立案日期 / 案号 / 标的金额 / 法院 / 状态);**标注本案 vs 其他案件**(本案是不是最大单笔?);**合计在执标的**

### 涉诉新发现
- 财产保全 / 诉前保全(新债权人?)
- 新立案文书 / 法院公告

### 工商变更
重点列出**与本案时间高度耦合**的股东变更 / 法代变更 / 注册资本变更(警惕抽逃出资 / 恶意转让)

### 财产线索 ⭐
**这是核心,要重点写**:
按四通保全详图写四个小节:通道一 · 直接锁定 / 通道二 · 调查追踪 / 通道三 · 债权拦截 / 通道四 · 关联穿透。每条线索都用「发现 → 评估 → 操作」三段;没有数据的通道也明确写"未发现公开记录"。

### 失信 / 限消 / 行政处罚
逐项写。**数据干净也明确说"未发现 XX"**

## 主体 N(关联公司,P2 新发现)
对每个 P2 dig_hints 里的 enterprise 类型独立一节(同上格式),**重点写**:
- 关键股东 / 法代是否跟原被执行人重叠
- 该公司有哪些被执行 / 失信 / 文书记录
- 是否存在可执行财产
- **跟原被执行人的关联程度**(同股东 / 同地址 / 同法代 → 高风险关联;否则 → 信息参考)

## 新发现案号 / 文书
对每个 P2 dig_hints 里的 case 类型独立列出:案号 / 标的 / 法院 / 当事人 / 摘要

## ⚠️ 拒执风险线索(2026-05-25 V0.1.9 新增)

corpus 顶部「案件元信息」段会给立案日。请扫 P1 + P2 全部数据,只列**立案日之后**发生的:
- 股东变更 / 股权转让(可疑:转移财产)
- 注册资本减少 / 抽逃出资
- 法定代表人变更
- 新增对外投资(可疑:转移现金到新主体)
- 注销 / 经营异常
- 与本案债务规模相当的资产处分

按时间倒序列表,每条给:
- **时间** / **事项** / **主体**(被执行人本身 / P2 关联公司) / **数据来源**(哪个端点) / **风险类型**(转移财产 / 抽逃出资 / 隔离关联)
- **风险等级初判**:🔴 高 / 🟠 中 / 🟡 低
- **法律后果可能性**(可能构成《刑法》313 条拒执罪 / 《合同法》74 条转移财产撤销之诉 / 最高院 2019/11 司法解释追加股东 / 等)

**只列事实 + 判断,不给"建议追加 / 撤销 / 报案"等具体行动建议** — 那是律师的判断范畴,工具只做事实呈现。

如果立案日为"未抽到"或者所有变更都在立案日之前,本节直接写"未发现立案后疑似拒执变更"。

## 自然人主体说明(如果 dig_hints 涉及自然人,固定输出)

> ⚠️ **自然人 {姓名}**(身份证 {如有}):元典法律开放平台**未提供**按身份证查询自然人涉诉信息的接口。
> 请律师自行通过:裁判文书网 / 中国执行信息公开网 / 信用中国 核查。

**不要硬编**任何自然人执行 / 失信 / 文书记录。

## 数据来源
列出本次深挖用到的元典端点 + dig_hints 来源 + 查询时间

# 铁律

1. **不能编造数据;原始 JSON 没出现的事实绝对不能写。** P1 + P2 全部数据都给你了,只从里面摘
2. 数据干净也要明确写"未发现 X"
3. 中文,专业,简洁
4. 直接给报告,不要"根据您提供的资料"等元话术
5. **⚠️ 拒执风险线索 这一节必须有**(没发现也要明确写"未发现立案后疑似变更")
6. **拒执线索只列事实 + 判断,不给行动建议** — 工具只做原始数据整理 + 基础判断
7. 输出**纯 Markdown 文本**,不要 JSON,不要 ```markdown``` 围栏
"###;

/* ============ 深挖单项实现 ============ */

async fn dig_enterprise(
    api_key: &crate::yuandian::YuandianCredentialSource,
    name: &str,
    base_dir: &std::path::Path,
    files: &mut Vec<String>,
) -> Result<(), String> {
    // search → id
    let search = yuandian::enterprise_search(api_key, name)
        .await
        .map_err(|e| format!("search:{}", e))?;
    save_json(base_dir, name, "search", &search)?;
    files.push(file_name(name, "search"));

    let candidates = search
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let id = candidates
        .iter()
        .find(|c| {
            c.get("企业名称")
                .and_then(|v| v.as_str())
                .map(|n| n == name)
                .unwrap_or(false)
        })
        .or_else(|| candidates.first())
        .and_then(|c| c.get("id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let Some(id) = id else {
        return Err("元典没找到该关联公司".into());
    };
    let entity = EntityId::Id(id);

    macro_rules! call {
        ($ep:literal, $fn:expr) => {{
            match $fn.await {
                Ok(v) => {
                    save_json(base_dir, name, $ep, &v)?;
                    files.push(file_name(name, $ep));
                }
                Err(e) => crate::dlog!("[deepdive] {} {}: {}", name, $ep, e),
            }
        }};
    }
    // P2 深挖关联公司:只跑核心 7 个端点(P1 已经做过类似 14 个,这里精简成最关键的)
    call!(
        "aggregation",
        yuandian::enterprise_aggregation_summary(api_key, &entity)
    );
    call!(
        "executions",
        yuandian::enterprise_executions(api_key, &entity, 1)
    );
    call!(
        "executed_person",
        yuandian::enterprise_executed_person(api_key, &entity, 1)
    );
    call!(
        "out_invest",
        yuandian::enterprise_out_invest(api_key, &entity, 1)
    );
    call!(
        "frozen_equity",
        yuandian::enterprise_frozen_equity(api_key, &entity, 1)
    );
    call!("pledge", yuandian::enterprise_pledge(api_key, &entity, 1));
    call!(
        "writ_list",
        yuandian::enterprise_writ_list(api_key, &entity, 1)
    );
    Ok(())
}

async fn dig_case(
    api_key: &crate::yuandian::YuandianCredentialSource,
    case_no: &str,
    base_dir: &std::path::Path,
    files: &mut Vec<String>,
) -> Result<(), String> {
    let v = yuandian::search_qwal(api_key, case_no, 5)
        .await
        .map_err(|e| format!("qwal_search by 案号:{}", e))?;
    save_json(base_dir, case_no, "qwal_by_ah", &v)?;
    files.push(file_name(case_no, "qwal_by_ah"));
    let v2 = yuandian::search_ptal(api_key, case_no, 5)
        .await
        .map_err(|e| format!("ptal_search by 案号:{}", e))?;
    save_json(base_dir, case_no, "ptal_by_ah", &v2)?;
    files.push(file_name(case_no, "ptal_by_ah"));
    Ok(())
}

async fn dig_person(
    api_key: &crate::yuandian::YuandianCredentialSource,
    name: &str,
    base_dir: &std::path::Path,
    files: &mut Vec<String>,
) -> Result<(), String> {
    let v = yuandian::search_ptal(api_key, name, 10)
        .await
        .map_err(|e| format!("ptal_search:{}", e))?;
    save_json(base_dir, name, "ptal_search", &v)?;
    files.push(file_name(name, "ptal_search"));
    let v2 = yuandian::search_qwal(api_key, name, 5)
        .await
        .map_err(|e| format!("qwal_search:{}", e))?;
    save_json(base_dir, name, "qwal_search", &v2)?;
    files.push(file_name(name, "qwal_search"));
    Ok(())
}

/* ============ 工具 ============ */

fn raw_dir_for_case(case_id: &str) -> Result<PathBuf, String> {
    let base = crate::db::app_data_dir().map_err(|e| format!("无法定位 app data dir: {}", e))?;
    Ok(base.join("external").join(case_id).join("yuandian_raw"))
}

fn deep_raw_dir_for_case(case_id: &str) -> Result<PathBuf, String> {
    let base = crate::db::app_data_dir().map_err(|e| format!("无法定位 app data dir: {}", e))?;
    Ok(base
        .join("external")
        .join(case_id)
        .join("yuandian_deepdive"))
}

/// 找 reports/ 目录下最新的 dig_hints_*.json 加载
fn load_latest_hints(reports_dir: &std::path::Path) -> Result<Vec<DigHint>, String> {
    if !reports_dir.exists() {
        return Err("还没生成过 P1 风险报告(没找到 dig_hints)".into());
    }
    let entries = std::fs::read_dir(reports_dir).map_err(|e| format!("读 reports/ 失败:{}", e))?;
    let mut latest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("dig_hints_") || !name.ends_with(".json") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(mtime) = meta.modified() {
                if latest.as_ref().is_none_or(|(t, _)| mtime > *t) {
                    latest = Some((mtime, entry.path()));
                }
            }
        }
    }
    let (_, path) = latest.ok_or_else(|| "reports/ 没找到 dig_hints 文件".to_string())?;
    load_hints_at_path(&path)
}

fn load_hints_at_path(path: &std::path::Path) -> Result<Vec<DigHint>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("读 {} 失败:{}", path.display(), e))?;
    serde_json::from_str::<Vec<DigHint>>(&text)
        .map_err(|e| format!("dig_hints JSON 解析失败:{}", e))
}

fn error_report(
    case_id: &str,
    err: &str,
    start: std::time::Instant,
    hints_used: usize,
    raw_count: usize,
    corpus_chars: usize,
) -> DeepDiveReport {
    crate::dlog!("[deepdive] case={} 失败:{}", case_id, err);
    DeepDiveReport {
        case_id: case_id.to_string(),
        hints_used,
        raw_count,
        corpus_chars,
        report_path: None,
        elapsed_ms: start.elapsed().as_millis(),
        error: Some(err.to_string()),
    }
}

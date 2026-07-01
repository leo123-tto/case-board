//! 案件全局抽取(2026-05-24 h · 替代旧 aggregator 规则方案)。
//!
//! 思路:把案件所有文档的 extract MD 拼起来,**一次喂给 DeepSeek**(1M 上下文容易装下),
//! 让 LLM 同时输出两个东西:
//!
//!   call A:**JSON 表格**(填 cases.agg_* 字段)→ 写入数据库
//!   call B:**完整案件分析报告 MD** → 落盘到 reports/<case_id>.md
//!
//! 两次调用,不是单次双输出 — 单次输出大 JSON 嵌套长 MD 容易转义/截断,分开干净可靠。
//!
//! 替代了:
//!   - `db/aggregator.rs` 一大堆规则(去污 / 去重 / 反诉过滤 / 优先级排序)→ 全交给 LLM
//!   - 逐文档 `llm::extract_case_fields_with_hint` + 后聚合 → 一次全局抽
//!
//! 保留:
//!   - documents.extraction_status / extracted_text_path(OCR 落盘还在,作为本 module 输入)
//!   - 增量缓存(documents.cache_key)— mtime + size 没变就不重 OCR

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::llm::{LlmConfig, LlmError};

/// LLM 全局抽出的"填表"结果(对齐 cases.agg_* 字段)。
///
/// 所有字段都是 Option,LLM 没看到信息时 null。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalExtractTable {
    pub case_no: Option<String>,
    pub court: Option<String>,
    pub cause: Option<String>,
    pub filed_at: Option<String>, // YYYY-MM-DD
    pub claim_amount: Option<f64>,
    pub workflow_status: Option<String>, // 8 档之一:接案/立案中/.../已结案
    pub plaintiffs: Vec<String>,
    pub defendants: Vec<String>,
    pub third_parties: Vec<String>,
    pub judges: Vec<String>,
    pub party_contacts: Vec<PartyContact>,
    pub court_contacts: Vec<CourtContact>,
    pub key_dates: Vec<KeyDate>,
    pub fees: Vec<FeeItem>,
    pub resolution: Option<String>,
    pub status_text: Option<String>,
    pub summary: Option<String>,
    /// 2026-06-13:我方代理立场(原告方/被告方/第三人/反诉混合/null),从 is_our_side=true 当事人推断。
    /// 驱动报告侧重 + AI 助手立场。律师已确认时通过 corpus 前缀回喂当输入(见 extract_combined)。
    #[serde(default)]
    pub our_side: Option<String>,
    /// 2026-06-11 审级模型:各审级实例([仲裁]→一审→二审→[再审]),每审级一条。
    /// 顶层 case_no/court/judges 填最新审级,全量明细在这里。
    #[serde(default)]
    pub instances: Vec<InstanceExtract>,
    /// 2026-06-11:从转账截图/汇款凭证抽的对方实际还款(落 case_payments,标 [AI识别])。
    #[serde(default)]
    pub repayments: Vec<RepaymentExtract>,
}

/// 单个审级(对齐 case_instances 表)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InstanceExtract {
    pub level: Option<String>, // 仲裁 / 一审 / 二审 / 再审
    pub case_no: Option<String>,
    pub authority: Option<String>,           // 承办机关全称
    pub authority_type: Option<String>,      // 法院 / 仲裁委 / 其他
    pub handlers: Vec<CourtContact>,         // 该审级承办人(法官/仲裁员/书记员)
    pub party_roles: Vec<InstancePartyRole>, // 该审级当事人称谓
    pub filed_at: Option<String>,
    pub result: Option<String>,
    pub note: Option<String>, // 发回重审/管辖异议等边缘场景说明
}

/// 某审级里一个当事人的称谓(二审=上诉人/被上诉人,note 收"原审被告"等文书自带对应关系)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InstancePartyRole {
    pub name: Option<String>,
    pub role: Option<String>,
    pub is_our_side: Option<bool>,
    pub note: Option<String>,
}

/// 对方实际还款一笔(银行转账截图/汇款凭证/执行笔录里识别)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RepaymentExtract {
    pub amount: Option<f64>,     // 元
    pub paid_at: Option<String>, // YYYY-MM-DD
    pub payer: Option<String>,
    pub note: Option<String>,
    /// 与 corpus 里的文档标题完全一致的文件名,用于回连源文件核对。
    pub source_filename: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PartyContact {
    /// 姓名(2026-05-26 V0.1.12:改 Option,合同里只有机构名无具体联系人时 LLM 合理返回 null)
    pub name: Option<String>,
    pub role: Option<String>, // 原告 / 被告 / 委托代理人 / 第三人 / ...(主诉讼地位)
    pub id_no: Option<String>,
    pub address: Option<String>,
    pub phone: Option<String>,
    pub is_our_side: Option<bool>,
    /// 2026-05-26 V0.1.12:同人跨文档其它身份("文档类型:角色"),如 ["委托合同:委托人", "执行申请:申请人"]
    /// 主身份(role)取最权威诉讼文书,这里收"程序角色"避免重复 entry
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CourtContact {
    /// 2026-05-26 V0.1.12:改 Option,合议庭某人只知职务无名时 LLM 合理返回 null
    pub name: Option<String>,
    pub role: Option<String>, // 审判员 / 法官助理 / 书记员
    pub phone: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyDate {
    /// 2026-05-26 V0.1.12:改 Option — 委托合同"签订日期"等落款未填的常 case,LLM 会返回 null
    pub date: Option<String>, // YYYY-MM-DD
    pub event: String,
    pub note: Option<String>,
    /// 2026-05-24 k-9:有"到期"概念的事件(保全 / 续封 / 上诉期 / 还款期 等)的失效日期。
    /// LLM 应用知识自动算:银行存款/资金 1 年、车辆/动产 2 年、不动产/股权 3 年;续封同期;判决书上诉期 15 天。
    /// 没"到期"概念(立案 / 开庭 / 调解结案等)填 null。
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeeItem {
    pub item: String,
    pub amount: Option<f64>,
    pub note: Option<String>,
}

/// 单个文档输入(给 LLM 看的)。
pub struct DocInput {
    pub filename: String,
    pub category: Option<String>,
    pub stage: Option<String>,
    pub text_md: String,
}

/// === 单次 Prompt 同时输出表格 + 报告 ===
///
/// 2026-05-24 i · 合并 call:用一个 prompt + JSON output 同时输出 `table` 和 `report_md`。
/// 优点:
///   - 省一半 input tokens(corpus 只传一次)
///   - 保证 table 和 report 基于同一份"思考",不会两次 call 表述不一致
///   - 简化编排
const SYSTEM_PROMPT_COMBINED: &str = r###"你是资深律师助理,精通法律案件信息抽取与分析。我会给你**同一个案件的所有文档全文**(按文档分段)。

请你**通读所有文档后,一次性输出一个 JSON 对象**,包含两部分:

1. `table`:结构化案件画像(供数据库填表)
2. `report_md`:案件分析报告(供律师阅读的完整 Markdown 文本)

# 输出格式(严格 JSON,无 markdown 围栏,无解释,直接给 JSON 对象):

{
  "table": {
    "case_no": "当前(最新)审级的案号或null",
    "court": "当前(最新)审级的承办机关全称(法院或仲裁委)或null",
    "cause": "案由或null",
    "filed_at": "YYYY-MM-DD或null",
    "claim_amount": 数字或null,
    "workflow_status": "11 档之一或null",
    "plaintiffs": ["..."],
    "defendants": ["..."],
    "third_parties": ["..."],
    "judges": ["..."],
    "party_contacts": [{"name":"姓名或null","role":"主诉讼地位或null","id_no":null,"address":null,"phone":null,"is_our_side":true,"aliases":["委托合同:委托人","执行申请:申请人"]}],
    "court_contacts": [{"name":"姓名或null","role":"职务或null","phone":null}],
    "key_dates": [{"date":"YYYY-MM-DD或null","event":"事件类型","note":null,"expires_at":"YYYY-MM-DD或null"}],
    "fees": [{"item":"费用项目","amount":数字或null,"note":null}],
    "resolution": "调解 / 判决 / 执行结果(自由文本,200 字内)",
    "status_text": "用一句话描述当前状态(如 2026-05 调解结案,首期款已付;若未来才开庭则写 将于 2026-06-22 开庭)",
    "summary": "案件一句话概括(50 字内)",
    "our_side": "我方代理立场:原告方/被告方/第三人/反诉混合/null",
    "instances": [{"level":"一审","case_no":"该审级案号或null","authority":"该审级承办机关全称或null","authority_type":"法院或仲裁委或其他","handlers":[{"name":"姓名或null","role":"审判员/仲裁员/书记员","phone":null}],"party_roles":[{"name":"张三","role":"该审级称谓(原告/被告/上诉人/被上诉人/申请人/被申请人)","is_our_side":true,"note":"原审被告 等文书自带对应关系或null"}],"filed_at":"YYYY-MM-DD或null","result":"该审级结果或null","note":"发回重审等边缘情况说明或null"}],
    "repayments": [{"amount":100000,"paid_at":"YYYY-MM-DD或null","payer":"付款人或null","note":"来源说明(如 银行转账截图)或null","source_filename":"该笔付款所在文档的文件名或null"}]
  },
  "report_md": "## 案件概况\n...完整 Markdown 报告..."
}

# table 字段铁律

1. 跨文档关联:同一信息出现在多份文档时,以最权威来源为准(判决书 > 调解书 > 受理通知 > 起诉状 > 笔录 > 申请书)
2. 反诉过滤:有反诉文档(标题或正文明确"反诉")时,反诉视角的原被告**不要混进原诉**(plaintiffs/defendants 只填原诉视角)
3. is_our_side(我方=true / 对方=false / 不确定=null),综合下列信号**交叉判断、避免冲突**:
   - 委托代理合同 / 授权委托书:这类材料一般只针对我方当事人(对方的委托材料通常不在我方卷里),**委托书指向的当事人/受托代理的对象**往往就是我方 → 倾向 is_our_side=true。注意代理合同常是"委托方与律所"签订,**须结合委托书具体指向哪位当事人**,不要仅凭"有这份文档"就下结论。
   - 起诉状/上诉状首部的原告/上诉人、答辩状首部的被告/被上诉人 → 该方倾向 is_our_side=true
   - 多个信号**交叉印证**;信号相互冲突或不足时不要硬判,交给 3b 收敛或填 null
3b. **our_side(我方代理立场,关键字段)**:从 is_our_side=true 的当事人**主诉讼地位**归纳出案件级阵营:
   - 我方是原告 / 申请人 / 申请执行人 / 上诉人(原审原告)→ `"原告方"`
   - 我方是被告 / 被申请人 / 被执行人 / 被上诉人(原审被告)→ `"被告方"`
   - 我方是第三人 → `"第三人"`
   - 既代理本诉原告又涉反诉被告等多重身份 → `"反诉混合"`
   - 信号不足或相互冲突、判不出我方是谁 → null(绝不瞎猜,让律师补)
   - ⚠️ 看「我方」实体阵营(攻方/守方),**不要**被审级称谓带偏(一审被告→二审"上诉人",实体上仍是 `"被告方"`)
   - 若 corpus 开头出现【律师已确认:我方代理立场=XXX】,**以该值为准**,据此回填 is_our_side 并撰写报告侧重,不得改判
4. 日期统一 YYYY-MM-DD,金额数字(元)不要"万元"
5. workflow_status 严格从 11 档选一个:接案 / 立案中 / 仲裁中 / 待开庭 / 审理中 / 已调解 / 上诉期 / 二审中 / 再审中 / 执行中 / 已结案
5a. 语料开头会给你 `【当前日期=YYYY-MM-DD】`。`workflow_status` 和 `status_text` 必须结合当前日期判断先后:
   - 未来日期的开庭 / 还款期 / 上诉期,写成 `待开庭` / `将于 YYYY-MM-DD 开庭` / `上诉期至 YYYY-MM-DD`,**绝不能**写成 `已开庭` / `已过期`
   - 只有出现了该节点之后的文书(如庭审笔录 / 判决书 / 调解书 / 执行文书)或正文明确写明该节点已发生,才能写 `已开庭` / `审理中` / `已结案`
   - 仅看到传票 / 开庭通知且日期晚于当前日期时,`workflow_status` 一律优先 `待开庭`
6. key_dates 只列办案过程节点(立案/开庭/调解/判决/上诉/二审开庭/二审判决/执行立案/申请保全/续封/还款期),不要 LPR/违约金计算/数字大写等噪音
6a. 对 `开庭` / `二审开庭` 节点:若文书里出现应到时间、庭审时间、法庭号、开庭地点,`note` 必须优先写成便于首页直接展示的短信息(例:`13:30 第6法庭`、`14:30-16:00 第6法庭 钱潮路369号801室`),不要只写泛泛的"开庭"或留空
6b. 对 `执行立案` 节点:若执行通知书、执行案件受理通知、短信或工作记录里出现执行案号(如 `(2026)苏02执123号`、`执恢`、`执保`),`note` 必须写入完整执行案号。审判案号和执行案号是两个编号,不得把审判案号当执行案号。
7. key_dates.expires_at(有"到期"概念的事件填,无则 null):
   - **保全 / 续封**:银行存款 / 账户资金 = date + 1 年;车辆 / 动产 = date + 2 年;不动产 / 股权 / 其他财产权 = date + 3 年(从 note 或上下文判断保全标的类型)
     ⚠️ 续封/续冻提醒的期限依据只能来自法院出具的保全裁定书、协助执行通知书、查封/冻结/扣押通知或回执、续封/续冻裁定,以及律师工作记录中明确记载的法院查封/冻结情况。**我方提交的财产保全申请书/申请材料不是期限依据**,只能说明我方申请了什么,不得因此生成带 expires_at 的保全 key_dates 或 preservations。
     首页提醒主要显示到期日、案件、法院、承办法官、法院联系电话;到期日写 expires_at,法院/法官/电话写 court、judges、court_contacts。note 可写"保全到期提醒"或留空,不要写标的金额、申请理由、当事人地址、法院地址、执行过程长段说明
     同一裁定同时写银行账户/车辆/不动产等多种期限时,必须拆成多条 key_dates / preservations,分别计算到期日;不要合并成一个最长期限
   - **解封 / 解除查封 / 解除冻结 / 解除保全**:写 key_dates 事件"解封",date=解除裁定日期,expires_at=null;这是后续提醒撤销信号,不要再生成保全到期提醒
   - **判决书上诉期**:date + 15 天(民事一审)
   - **裁定书上诉期**:date + 10 天
   - **调解书 / 一审终审 / 二审判决**:无上诉期,填 null
   - **还款期**(调解约定分期付款):每一期都单独列一条,expires_at = date 本身
   - **立案 / 开庭 / 调解 / 判决书签发**:无到期,填 null
7. 不知道就填 null,绝不编造
8. **留空是合法答案**:任何字段不确定 / 文档中缺失 / 矛盾无法判断时,**填 null**(数组类字段填 [])。**不要硬填空字符串 ""**,不要硬编造 — 律师拿到 null 会自己补,拿到伪造数据会判错
9. **party_contacts 同人合并铁律**(2026-05-26 V0.1.12 新加):
   - 同一姓名(去空格/标点)在多份文档出现的,**合并成 1 个 entry**,绝不重复
   - `role` 取**最权威诉讼地位**(判决书 > 起诉状/答辩状 > 申请书),即原告/被告/第三人
   - `aliases` 收"程序角色"(委托人 / 申请人 / 被申请人 / 反诉原告 等),格式 `["<文档类型>:<角色>"]`
     例:张三在委托合同是委托人、起诉状是原告、执行申请是申请人,
     输出 → `{name:"张三", role:"原告", aliases:["委托合同:委托人","执行申请:申请人"], ...}`
   - 同身份多次出现(如多份起诉状都是原告)只取一条主身份,不重复进 aliases
   - 只有姓名,无机构,无任何身份关联的不要进 party_contacts(避免噪音)
10. **instances 审级铁律**(2026-06-11 新加):
   - 一个纠纷的审判程序生命线 = [仲裁]→一审→二审→[再审],**每个审级单独一条 instance**;只有一个审级时也要输出那一条
   - level 严格 4 选 1:仲裁 / 一审 / 二审 / 再审
   - 从案号特征辅助判断:`民初/初字`=一审,`民终/终字`=二审,`民再/民申/再字`=再审,`仲案/劳人仲/仲裁字`=仲裁;**`执`字号是执行程序不是审级,不进 instances**
   - authority_type 判断:名称含"人民法院"=法院;含"仲裁委员会 / 劳动人事争议仲裁委员会 / 劳动争议仲裁委员会 / 仲裁院 / 国际仲裁中心"=仲裁委;其余=其他
   - party_roles 填**该审级文书首部的称谓原文**(一审=原告/被告,二审=上诉人/被上诉人,仲裁=申请人/被申请人,再审=再审申请人/被申请人),note 收文书自带的对应关系(如"原审被告"),**不要自行推断身份反转**
   - 顶层 case_no / court / judges / party_contacts 填**最新审级**(再审>二审>一审>仲裁)的值;劳动争议先仲裁后诉讼的,法院审级为最新
   - 执行案号不进 instances,但必须进入 key_dates 的 `执行立案.note`,供执行页展示和法院短信归案匹配
   - 同审级有发回重审等特殊情形时,note 写明
11. **repayments 还款铁律**(2026-06-11 新加):
   - 从**银行转账截图 / 汇款凭证 / 微信支付宝转账记录 / 执行笔录**中抽**对方实际付款给我方**的记录,每笔一条
   - 必须有明确金额(元);日期能确定填 YYYY-MM-DD,无法确定填 null;payer 填付款人姓名
   - 我方支出(诉讼费 / 保全费 / 律师费)**不是还款**,不要进 repayments
   - 调解书/和解协议里**约定的**分期还款计划是计划不是实际付款,不进 repayments(那是 key_dates 的还款期)
   - note 一句话说明来源(如"银行转账截图")
   - source_filename 必须填写该笔付款所在的文档文件名,且必须与上方文档分段标题里的文件名完全一致;若无法确定是哪一份,填 null,不要猜

# report_md 结构(用 ## 二级标题,顺序固定)

⭐ **全文立场铁律**:整份报告从 **our_side(我方代理立场)** 视角写,服务我方:
  - our_side=`原告方`:重心放「我方诉请的请求权基础是否扎实、我方举证是否到位、如何预判并击破对方抗辩」。
  - our_side=`被告方`:重心放「对方诉请有哪些缺陷/法律障碍、我方有哪些抗辩与反驳、举证责任如何分配对我方有利、对方举证薄弱点」,**不要替对方论证其请求成立**。
  - our_side=`第三人`/`反诉混合`:点明我方独立利益与攻防重点。
  - our_side=null(未识别):保持中立陈述,并在「注意事项」首条提示「⚠️ 未能识别我方代理立场,请在案件详情页确认后重新分析,报告才能按立场给侧重」。

## 案件概况
一句话定性 + 当前阶段 + **我方代理立场(原告方/被告方/...)**。

## 当事人与代理
- 原告 / 被告 / 第三人(列基本身份信息),**标明哪一方是我方**
- 我方代理身份(如有委托合同)
- 对方信息掌握程度

## 时间线
按时间倒序列出**办案过程节点**,每条标注日期 + 一句话说明。

## 争议焦点与请求(按我方立场写)
> 方法:把每个争议焦点对应到具体**请求权基础**(主要规范),点明该请求权核心构成要件里哪些本案已满足、哪些才是真正争点;**比例原则——无争议的要件一句带过,只对核心争点展开**,不堆砌冗余论证。
- our_side=原告方:我方诉请(金额/标的)+ 请求权基础 + 主要事实证据 + 对方可能的答辩及我方应对
- our_side=被告方:对方诉请要点 + **我方核心抗辩理由** + 对方请求/举证的缺陷 + 反诉(如有)
- our_side=第三人/反诉混合/未知:客观列争议焦点 + 我方利益所在

## 程序进展与结果
- 当前状态
- 已生效法律文书核心结论
- 履行情况

## 关键日期提醒
未来 / 近期需要关注的截止时间(开庭 / 还款期 / 上诉期 / 续封等)。

## 承办机关联系
承办法官 / 仲裁员 / 书记员 / 法官助理 + 电话 + 机关地址(多审级时按审级分列,最新在前)。

## 收费与费用
案件受理费 / 财产保全费 / 律师代理费 / 谁负担。

## 注意事项
律师需要特别关注的点(履行风险 / 文件缺失 / 后续程序等)。

# report_md 铁律

1. 只从给定文档抽信息,不编造;不知道的标"(不详)"
2. 跨文档冲突时以最权威来源为准,并标注"来源 XX 文档"
3. 反诉情况单独说明,不混进原诉
4. 中文 / 专业 / 简洁,不要"根据您提供的文档"之类元话术
5. JSON 字符串里的换行用 \n(而不是真的换行),其他符合 JSON 标准
"###;

#[derive(Debug, Clone, Deserialize)]
pub struct CombinedExtractResult {
    pub table: GlobalExtractTable,
    pub report_md: String,
}

/// 拼接所有文档为一个 LLM 输入。
pub fn build_corpus(docs: &[DocInput]) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "本案件共 {} 份文档,以下逐份列出。\n\n",
        docs.len()
    ));
    for (i, d) in docs.iter().enumerate() {
        s.push_str(&format!(
            "\n========== 文档 {}/{}: {} | 分类: {} | 阶段: {} ==========\n\n",
            i + 1,
            docs.len(),
            d.filename,
            d.category.as_deref().unwrap_or("—"),
            d.stage.as_deref().unwrap_or("—"),
        ));
        s.push_str(d.text_md.trim());
        s.push('\n');
    }
    s
}

/// 单次 LLM call 同时输出表格 + 报告(2026-05-24 i)。
///
/// 返回 `CombinedExtractResult { table, report_md }`。
/// 设 timeout 比单次 LLM call 长(报告 output 长,有时要 30-60 秒)。
pub async fn extract_combined(
    config: &LlmConfig,
    corpus: &str,
    confirmed_our_side: Option<&str>,
) -> Result<CombinedExtractResult, LlmError> {
    // 律师在详情页确认/纠正过的立场 → 作为输入回喂(prompt 铁律 3b 会以此为准),
    // 修复"LLM 推断站反、用户纠正后报告不变"。只回喂用户确认值,不回喂 LLM 自己上次的猜测(避免错值固化)。
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut user_content = format!("【当前日期={}】\n", today);
    if let Some(side) = confirmed_our_side.map(str::trim).filter(|s| !s.is_empty()) {
        user_content.push_str(&format!("【律师已确认:我方代理立场={}】\n", side));
    }
    user_content.push('\n');
    user_content.push_str(corpus);
    // 2026-06-15:MiniMax 自有协议(/chatcompletion_v2)不支持 response_format:json_object(实测 2013 报错),
    // 且 M 系列恒思考、思考占 output token → 不发 response_format、把 max_tokens 抬高、温度禁 0.0。
    let is_minimax = config.endpoint.contains("chatcompletion_v2");
    let mut body = serde_json::json!({
        "model": config.model,
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT_COMBINED},
            {"role": "user", "content": user_content},
        ],
        // 报告 + 表格 大约 5-10K tokens output;MiniMax 还要叠思考 token,抬到 32K(M3 支持)。
        "max_tokens": if is_minimax { 32768 } else { 12288 },
        "temperature": config.temperature,
        "stream": false,
    });
    if !is_minimax {
        body["response_format"] = serde_json::json!({"type": "json_object"});
    }

    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.timeout_secs * 3))
        .build()
        .map_err(|e| LlmError::Network(e.to_string()))?
        .post(&config.endpoint)
        .json(&body);

    if let Some(key) = &config.api_key {
        req = req.bearer_auth(key);
    }

    let response = req
        .send()
        .await
        .map_err(|e| LlmError::Network(e.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(LlmError::HttpStatus(status.as_u16(), text));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| LlmError::ResponseFormat(e.to_string()))?;

    // 整合外部 PR #14 @zzf516988659-del:多字段 fallback —— MiniMax M 系列恒思考,
    // finish_reason=length 时 message.content 截断为空,但 reasoning_content 还有内容;
    // 旧代码只查 message.content 一字段 → 「无 content」误报。
    // (PR 原带 dlog! 打印整份 raw response + dlog 写盘:因含案件内容、可能流入反馈 MD/落盘,
    //  违反隐私铁律「反馈不传案件内容」,本仓未采纳那两块;只取多字段 fallback + 富错误信息。)
    let first_choice = json.get("choices").and_then(|c| c.get(0));
    let first_message = first_choice.and_then(|c| c.get("message"));
    // 优先 content(OpenAI 标准 / M 系列正常);空串视为无效 → reasoning_content 兜底 → 顶层 text。
    let content_opt: Option<&str> = first_message
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            first_message
                .and_then(|m| m.get("reasoning_content"))
                .and_then(|c| c.as_str())
                .filter(|s| !s.trim().is_empty())
        })
        .or_else(|| json.get("text").and_then(|c| c.as_str()));
    let content = content_opt.ok_or_else(|| {
        // 错误信息只带 finish_reason + token 数(无案件内容),让用户区分是 max_tokens 撞墙还是模型问题。
        let finish_reason = first_choice
            .and_then(|c| c.get("finish_reason"))
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let usage = json.get("usage");
        let completion = usage
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let reasoning = usage
            .and_then(|u| u.get("completion_tokens_details"))
            .and_then(|d| d.get("reasoning_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        LlmError::ResponseFormat(format!(
            "无 choices[0].message.content (finish_reason={}, completion_tokens={}, reasoning_tokens={})",
            finish_reason, completion, reasoning
        ))
    })?;

    // MiniMax M 系列可能把 <think> 块塞进 content 开头 → 用更鲁棒的剥离(剥 think + 取首{到末})。
    let cleaned = if is_minimax {
        crate::llm::extract_json_from_content(content)
    } else {
        strip_markdown_fence(content)
    };
    serde_json::from_str::<CombinedExtractResult>(&cleaned)
        .map_err(|e| LlmError::ContentJson(format!("{}\n---原始---\n{}", e, cleaned)))
}

/// 项目1:从已结案/判决案件的信息 + 分析报告,提炼一张「办案经验卡片」(Markdown),
/// 供日后同类案件 `search_local_kb` 检索复用。不走 JSON,直接让模型输出规范 Markdown。
pub async fn distill_experience(
    config: &LlmConfig,
    case_brief: &str,
    report_md: &str,
) -> Result<String, LlmError> {
    let user_content = format!("【案件信息】\n{case_brief}\n\n【案件分析报告】\n{report_md}");
    let body = serde_json::json!({
        "model": config.model,
        "messages": [
            {"role": "system", "content": EXPERIENCE_DISTILL_PROMPT},
            {"role": "user", "content": user_content},
        ],
        "max_tokens": 4096,
        "temperature": 0.2,
        "stream": false,
    });

    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.timeout_secs * 2))
        .build()
        .map_err(|e| LlmError::Network(e.to_string()))?
        .post(&config.endpoint)
        .json(&body);
    if let Some(key) = &config.api_key {
        req = req.bearer_auth(key);
    }
    let response = req
        .send()
        .await
        .map_err(|e| LlmError::Network(e.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(LlmError::HttpStatus(status.as_u16(), text));
    }
    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| LlmError::ResponseFormat(e.to_string()))?;
    let content = json
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| LlmError::ResponseFormat("无 choices[0].message.content".into()))?;
    Ok(strip_markdown_fence(content).trim().to_string())
}

const EXPERIENCE_DISTILL_PROMPT: &str = r#"你是资深诉讼律师的办案经验整理助手。给你一个**已结案/已判决**案件的信息与分析报告,提炼一张「办案经验卡片」,供日后办理**同类案件**时检索复用。

用 Markdown 输出,严格按以下结构(不要多余说明、不要代码围栏):

# 办案经验 · <案由> · <一句话案件标识>

- **案件**:<案号> / <法院> / 我方<原告方或被告方> / <调解或判决或执行结果>

## 争议焦点
- 逐条列本案核心争议问题

## 裁判规则
- 法院/仲裁对此类问题的裁判标准、口径(从本案结果与说理中提炼)

## 法条适用
- 精确到 <法规名> 第 X 条 + 要点(**只写材料里确有依据的,不得编造法条/案号**)

## 办案心得
- 律师视角实务经验:证据怎么组织、对方抗辩怎么破、同类案下次注意什么

要求:基于给定材料,不编造;每部分 3-5 条精炼;面向「复用」写,让下次遇到同类案能直接借鉴。"#;

/// 去掉 LLM 输出可能带的 ```json ``` 围栏(JSON output mode 不应该有,但保险)。
fn strip_markdown_fence(s: &str) -> String {
    let trimmed = s.trim();
    if let Some(stripped) = trimmed.strip_prefix("```json") {
        return stripped
            .trim_start()
            .trim_end_matches("```")
            .trim()
            .to_string();
    }
    if let Some(stripped) = trimmed.strip_prefix("```") {
        return stripped
            .trim_start()
            .trim_end_matches("```")
            .trim()
            .to_string();
    }
    trimmed.to_string()
}

/// 报告 MD 落盘路径:`~/Library/Application Support/CaseBoard/reports/<case_id>.md`
pub fn report_path_for_case(case_id: &str) -> Result<PathBuf, String> {
    let base = crate::db::app_data_dir().map_err(|e| format!("无法定位 app data dir: {}", e))?;
    let dir = base.join("reports");
    std::fs::create_dir_all(&dir).map_err(|e| format!("建 reports 目录失败: {}", e))?;
    Ok(dir.join(format!("{}.md", case_id)))
}

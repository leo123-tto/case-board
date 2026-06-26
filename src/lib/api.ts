/**
 * 集中封装所有 Tauri 后端命令调用。
 *
 * 把 `invoke<T>("xxx", { ... })` 这种 IPC 调用收敛到一处,好处:
 *   - 类型安全:命令参数 + 返回值都有 TS 类型
 *   - 重构友好:Rust 端改命令名,只用改这一个文件
 *   - 易于 mock 测试(以后)
 */

import { invoke } from "@tauri-apps/api/core";
import type {
  CaseLog,
  ElementDocumentType,
  ElementDraft,
  SavedElementDocument,
} from "./types";

import type {
  Case,
  CaseInstance,
  CaseWithDocs,
  CourtFilingJob,
  CourtFilingEnvReport,
  Document,
  ExtractedFields,
  FeishuCalendarEvent,
  LawyerProfile,
  NewCaseInstance,
  ImportPlan,
  ImportResult,
  ScannedDoc,
  Settings,
  UpdateInfo,
  VerifyResult,
} from "./types";

/* ------------------------------------------------------------------ */
/* 扫描 / 导入                                                        */
/* ------------------------------------------------------------------ */

/** 纯扫描,不入库。给"先看看"用。 */
export function scanCaseFolder(path: string): Promise<ScannedDoc[]> {
  return invoke<ScannedDoc[]>("scan_case_folder", { path });
}

/** 导入文件夹:扫描 + upsert 案件 + 替换文档列表。是 V0.1 的主路径。 */
export function importCaseFolder(path: string): Promise<ImportResult> {
  return invoke<ImportResult>("import_case_folder", { path });
}

/** 多案件检测:对文件夹做拆分预案(只读)。multi=false 时按单案导入即可。 */
export function planImportFolder(path: string): Promise<ImportPlan> {
  return invoke<ImportPlan>("plan_import_folder", { path });
}

/** 按确认后的拆分预案批量建案。`root` = 被拖入的上层文件夹(用于替换旧的整体单案)。 */
export function commitImportFolder(
  root: string,
  cases: { dir: string; name: string }[],
  sharedDirs: string[],
): Promise<ImportResult[]> {
  return invoke<ImportResult[]>("commit_import_folder", {
    root,
    cases,
    sharedDirs,
  });
}

/* ------------------------------------------------------------------ */
/* 案件读写                                                            */
/* ------------------------------------------------------------------ */

/** 列出所有已导入案件,按 updated_at 倒序。 */
export function listCases(): Promise<Case[]> {
  return invoke<Case[]>("list_cases");
}

/** 取案件详情 + 该案件所有文档。 */
export function getCaseWithDocs(id: string): Promise<CaseWithDocs> {
  return invoke<CaseWithDocs>("get_case_with_docs", { id });
}

export interface InsightBucket {
  label: string;
  count: number;
  ratio: number;
  amount_total: number;
}

export interface LawyerInsightsReport {
  total_cases: number;
  active_cases: number;
  closed_cases: number;
  analyzed_cases: number;
  amount_cases: number;
  total_claim_amount: number;
  average_claim_amount: number | null;
  top_causes: InsightBucket[];
  top_courts: InsightBucket[];
  our_side_mix: InsightBucket[];
  stage_mix: InsightBucket[];
  strengths: string[];
  data_gaps: string[];
  next_questions: string[];
  markdown: string;
}

/** 基于本机案件数据生成办案画像。只读统计,不上传。 */
export function getLawyerInsights(): Promise<LawyerInsightsReport> {
  return invoke<LawyerInsightsReport>("get_lawyer_insights");
}

/** 删除一个案件(级联删除关联文档)。不动原始文件夹。 */
export function deleteCase(id: string): Promise<void> {
  return invoke<void>("delete_case", { id });
}

/* ------------------------------------------------------------------ */
/* 文件读取                                                            */
/* ------------------------------------------------------------------ */

/** 读一个文本文件(.md/.html/.txt)的全文。仅限 5MB 以内。 */
export function readTextFile(path: string): Promise<string> {
  return invoke<string>("read_text_file", { path });
}

/**
 * 抽 .docx / .doc / .rtf / .odt 的纯文本,用于在 App 内即时预览 Word 文档(不启动 Word)。
 * .docx 走跨平台原生解析;.doc/.rtf/.odt 在 macOS 用 textutil 即时预览,其他平台暂不支持预览
 *(导入案件时这类文档由 MinerU 云端解析入库,内容照常进 AI 上下文)。
 */
export function extractDocText(path: string): Promise<string> {
  return invoke<string>("extract_doc_text", { path });
}

/**
 * 把一段诉讼文书纯文本喂给本机 LLM(llama.cpp + MiniCPM-V 4.6),
 * 抽出 7 个结构化字段。耗时通常 3-8 秒。
 */
export function extractFieldsFromText(text: string): Promise<ExtractedFields> {
  return invoke<ExtractedFields>("extract_fields_from_text", { text });
}

/** 用系统默认应用打开一个文件(PDF→Preview, docx→Word, 图片→Preview)。 */
export function openInDefaultApp(path: string): Promise<void> {
  return invoke<void>("open_in_default_app", { path });
}

/** 用系统默认浏览器打开 URL(Settings 里 token 申请链接、外链等)。2026-05-24 k */
export function openUrl(url: string): Promise<void> {
  return invoke<void>("open_url", { url });
}

/** 在 Finder 中显示该路径(选中并打开父目录)。 */
export function revealInFinder(path: string): Promise<void> {
  return invoke<void>("reveal_in_finder", { path });
}

/* ---- 源文件看板 Phase 3:文档标记 ---- */

/** 列出某案件全部文档的标记(重要/忽略 + 原告/被告/第三人)。 */
export function listDocumentTags(caseId: string): Promise<import("./types").DocumentTag[]> {
  return invoke("list_document_tags", { caseId });
}

/** 设文档重要度(单值):value="重要"|"忽略" 或 null(清空)。documentIds 多个=整批。 */
export function setDocumentImportance(
  documentIds: string[],
  value: string | null,
): Promise<void> {
  return invoke("set_document_importance", { documentIds, value });
}

/** 切换文档当事人侧(可多值):value=原告|被告|第三人,enabled=加/删。documentIds 多个=整批。 */
export function setDocumentPartySide(
  documentIds: string[],
  value: string,
  enabled: boolean,
): Promise<void> {
  return invoke("set_document_party_side", { documentIds, value, enabled });
}

/** 人工设文档分类(单值,六选一;value=null 清空)。 */
export function setDocumentCategory(
  documentId: string,
  value: string | null,
): Promise<void> {
  return invoke("set_document_category", { documentId, value });
}

/** 人工设证据倾向(单值):value=有利|不利|中性 或 null(清空)。documentIds 多个=整批。 */
export function setDocumentEvidenceAttitude(
  documentIds: string[],
  value: string | null,
): Promise<void> {
  return invoke("set_document_evidence_attitude", { documentIds, value });
}

/** 人工设提交阶段(单值):value 为固定阶段之一或 null(清空)。documentIds 多个=整批。 */
export function setDocumentSubmissionStage(
  documentIds: string[],
  value: string | null,
): Promise<void> {
  return invoke("set_document_submission_stage", { documentIds, value });
}

/** 🪄 AI 自动整理:一次 LLM 调用判整案材料的 重要度+归类+显示名,写 ai_suggest。返回写入数。 */
export function aiOrganizeCase(caseId: string): Promise<number> {
  return invoke("ai_organize_case", { caseId });
}

/** 人工设文档板内显示名(右键重命名);name=null/空 → 清回原文件名。纯元数据,不动原件。 */
export function setDocumentDisplayName(
  documentId: string,
  name: string | null,
): Promise<void> {
  return invoke("set_document_display_name", { documentId, name });
}

/** 在某文档已抽取文本里按页搜索关键词,返回命中页 + 摘要(前端点一下跳页)。 */
export function searchInDocument(
  documentId: string,
  query: string,
): Promise<import("./types").SearchHit[]> {
  return invoke("search_in_document", { documentId, query });
}

/** 列某文档的 PDF 页码书签(按页升序)。 */
export function listDocumentBookmarks(
  documentId: string,
): Promise<import("./types").Bookmark[]> {
  return invoke("list_document_bookmarks", { documentId });
}

/** 加一个 PDF 页码书签(page 1-based,label 可空)。返回新书签。 */
export function addDocumentBookmark(
  documentId: string,
  page: number,
  label: string | null,
): Promise<import("./types").Bookmark> {
  return invoke("add_document_bookmark", { documentId, page, label });
}

/** 删一个 PDF 页码书签。 */
export function deleteDocumentBookmark(id: string): Promise<void> {
  return invoke("delete_document_bookmark", { id });
}

/**
 * 2026-06-19:把案件源文件夹加进 asset 协议 scope(运行期、按案件授权),
 * 让源文件查看器能用流式 `asset://` 协议在 iframe 里原生渲染该案 PDF。
 * **打开查看器前必须 await 本调用**,否则 iframe 首次请求会 403(scope 未就绪)。
 */
export function allowCaseAssets(folder: string): Promise<void> {
  return invoke<void>("allow_case_assets", { folder });
}

/* ------------------------------------------------------------------ */
/* 用户设置                                                            */
/* ------------------------------------------------------------------ */

/** 2026-05-25 V0.1.6:在线验证 MinerU API token。 */
export function verifyMinerUKey(token: string): Promise<VerifyResult> {
  return invoke<VerifyResult>("verify_mineru_key", { token });
}

/** 2026-06-12:在线验证 PaddleOCR VL(AI Studio)访问令牌。 */
export function verifyPaddleVlKey(token: string): Promise<VerifyResult> {
  return invoke<VerifyResult>("verify_paddle_vl_key", { token });
}

/** 2026-05-25 V0.1.6:onboarding 完成时调一次,如果案件表为空就 seed 示例案件。 */
export function seedDemoCaseIfEmpty(): Promise<boolean> {
  return invoke<boolean>("seed_demo_case_if_empty");
}

/** 2026-05-25 V0.1.6:在线验证 DeepSeek API key。 */
export function verifyDeepSeekKey(
  apiKey: string,
  endpoint?: string,
): Promise<VerifyResult> {
  return invoke<VerifyResult>("verify_deepseek_key", {
    apiKey,
    endpoint: endpoint || null,
  });
}

/** 2026-06-15:在线验证 MiniMax API key(走 /v1/models 鉴权)。 */
export function verifyMiniMaxKey(
  apiKey: string,
  endpoint?: string,
): Promise<VerifyResult> {
  return invoke<VerifyResult>("verify_minimax_key", {
    apiKey,
    endpoint: endpoint || null,
  });
}

/** 2026-06-16:在线验证通用 OpenAI 兼容后端(GLM/MiMo/自定义)—— 一发最小 chat 请求验 key+地址+模型。 */
export function verifyOpenAICompatKey(
  apiKey: string,
  endpoint: string,
  model: string,
): Promise<VerifyResult> {
  return invoke<VerifyResult>("verify_openai_compat_key", {
    apiKey,
    endpoint,
    model,
  });
}

/** 2026-05-25 V0.1.8:在线验证元典(open.chineselaw.com)API key。
 *  消耗 1 次企业搜索配额(用 name=test top_k=1 探测,代价最小)。*/
export function verifyYuandianKey(apiKey: string): Promise<VerifyResult> {
  return invoke<VerifyResult>("verify_yuandian_key", { apiKey });
}

/** 2026-05-25 V0.1.8:检测远程最新版本(分发站点的 version.json)。
 *  失败时 has_update=false + error 字段填上原因,前端可静默忽略。*/
export function checkForUpdate(): Promise<UpdateInfo> {
  return invoke<UpdateInfo>("check_for_update");
}

/** 2026-05-25 V0.1.8:拿当前 App 版本(Cargo.toml CARGO_PKG_VERSION)。 */
export function appVersion(): Promise<string> {
  return invoke<string>("app_version");
}

/** 读取用户设置(endpoint 字段有 sensible 默认值,api_key 不会有默认值)。 */
export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

/** 写入用户设置(全量覆盖)。 */
export function saveSettings(payload: Settings): Promise<void> {
  return invoke<void>("save_settings", { payload });
}

/** 智能粘贴:把平台接入文档复制来的配置文本解析成 MCP server 列表(本地解析,不联网)。 */
export function parseMcpPaste(text: string): Promise<import("./types").ParsedMcpPaste> {
  return invoke("parse_mcp_paste", { text });
}

/** MCP 连接测试:真连一次(握手 + 列工具)。失败 reject 真实原因(401/403 等)。 */
export function testMcpServer(
  config: import("./types").McpServerConfig
): Promise<import("./types").McpTestReport> {
  return invoke("test_mcp_server", { config });
}

/* ------------------------------------------------------------------ */
/* 团队版 Phase 1(LAN 接力同步)                                      */
/* ------------------------------------------------------------------ */

export function teamStatus(): Promise<import("./types").TeamStatus> {
  return invoke("team_status");
}

export function teamCreate(
  teamName: string,
  myName: string
): Promise<import("./types").TeamStatus> {
  return invoke("team_create", { teamName, myName });
}

/** 扫描局域网内可加入的团队(约 3 秒)。 */
export function teamDiscover(): Promise<import("./types").DiscoveredTeam[]> {
  return invoke("team_discover");
}

export function teamJoin(
  teamId: string,
  code: string,
  myName: string
): Promise<import("./types").TeamStatus> {
  return invoke("team_join", { teamId, code, myName });
}

export function teamLeave(): Promise<void> {
  return invoke("team_leave");
}

export function teamKick(memberId: string): Promise<import("./types").TeamRoster> {
  return invoke("team_kick", { memberId });
}

/** 团队长配置成员权限:view=null 表示全队可见;edit=可编辑哪些成员。 */
export function teamSetPermissions(
  memberId: string,
  view: string[] | null,
  edit: string[]
): Promise<import("./types").TeamRoster> {
  return invoke("team_set_permissions", { memberId, view, edit });
}

export function teamRefreshCode(): Promise<string> {
  return invoke("team_refresh_code");
}

export function teamSyncNow(): Promise<import("./types").TeamSyncReport> {
  return invoke("team_sync_now");
}

export function teamView(): Promise<import("./types").TeamView> {
  return invoke("team_view");
}

/** 提交对队友案件的编辑(需编辑权;接力转交,所有人应用后生效)。field: workflow_status | note。 */
export function teamSubmitEdit(
  targetMemberId: string,
  caseId: string,
  caseName: string,
  field: string,
  value: string
): Promise<void> {
  return invoke("team_submit_edit", { targetMemberId, caseId, caseName, field, value });
}

/** 案件所有人撤销一条已生效的队友改动。 */
export function teamRevertEdit(editId: string): Promise<void> {
  return invoke("team_revert_edit", { editId });
}

/** 检测本机模型 + llama-server 状态(给 onboarding/Settings 用)。 */
export function detectLocalReadiness(
  modelDir?: string | null
): Promise<import("./types").LocalReadiness> {
  return invoke("detect_local_readiness", { modelDir: modelDir ?? null });
}

/** 主动让 App 后台启动 llama-server(等到就绪才返回,可能要 10-30s)。 */
export function ensureLocalReady(): Promise<void> {
  return invoke("ensure_local_ready");
}

/* ------------------------------------------------------------------ */
/* 案件聚合维护                                                        */
/* ------------------------------------------------------------------ */

/**
 * `reaggregate_all_cases` 命令的返回汇报(对应 Rust `ReaggregateReport`)。
 * 用于显示"刷新了 N 个案件,X 个成功,Y 个失败"。
 */
export interface ReaggregateReport {
  total: number;
  succeeded: number;
  failed: number;
  /** [(case_id, 错误消息), ...] */
  failures: [string, string][];
}

/**
 * 对所有案件重跑一次 **LLM 全局抽**(2026-05-24 h)。
 *
 * 用途:升级 prompt / schema 后,刷新所有案件的 cases.agg_* + 案件分析报告。
 * 串行跑,每个案件 ~10-30 秒,**会调 LLM**(不像旧 aggregator)。
 */
export function reaggregateAllCases(): Promise<ReaggregateReport> {
  return invoke<ReaggregateReport>("reaggregate_all_cases");
}

/** 单个案件全局抽报告(2026-05-24 i)。 */
export interface GlobalExtractReport {
  case_id: string;
  docs_included: number;
  table_ok: boolean;
  report_ok: boolean;
  report_path: string | null;
  elapsed_ms: number;
  error: string | null;
}

/**
 * 单个案件主动触发 LLM 全局抽。用户点「📖 案件报告」按钮时,
 * 若案件还没报告(case_report_path 为 null),前端就 await 这个 → 完成后弹报告。
 */
export function globalExtractCase(caseId: string): Promise<GlobalExtractReport> {
  return invoke<GlobalExtractReport>("global_extract_case", { caseId });
}

/**
 * 项目1:把(通常已结案/判决的)案件提炼成「办案经验卡片」写入本地知识库,
 * 返回写入文件的绝对路径。卡片落 raw/cases-experience/,search_local_kb 整库可检索复用。
 */
export function distillCaseExperience(caseId: string): Promise<string> {
  return invoke<string>("distill_case_experience", { caseId });
}

/* ------------------------------------------------------------------ */
/* 报告导出(2026-05-24 j)                                              */
/* ------------------------------------------------------------------ */

/** 导出案件报告为 HTML(陶土红 × 羊皮纸风格,单文件内嵌 CSS)。返回实际写入路径。 */
export function exportReportHtml(caseId: string, savePath: string): Promise<string> {
  return invoke<string>("export_report_html", { caseId, savePath });
}

/** 导出案件报告为 Word .docx(走 macOS textutil)。返回实际写入路径。 */
export function exportReportDocx(caseId: string, savePath: string): Promise<string> {
  return invoke<string>("export_report_docx", { caseId, savePath });
}

/** 导出办案画像 Markdown。返回实际写入路径。 */
export function exportLawyerInsightsMarkdown(savePath: string): Promise<string> {
  return invoke<string>("export_lawyer_insights_markdown", { savePath });
}

/** 2026-05-25 V0.1.7 · 通用 MD → HTML 导出(任意 MD 路径 + 标题)。 */
export function exportMdHtml(
  mdPath: string,
  title: string,
  savePath: string,
): Promise<string> {
  return invoke<string>("export_md_html", { mdPath, title, savePath });
}

/** 2026-05-25 V0.1.7 · 通用 MD → Word 导出。 */
export function exportMdDocx(
  mdPath: string,
  title: string,
  savePath: string,
): Promise<string> {
  return invoke<string>("export_md_docx", { mdPath, title, savePath });
}

/**
 * 2026-05-31 V0.3 M1 · 把 save_artifact 生成的文书导出为 **Word(法律格式)**。
 * 走原生 OOXML(方正小标宋标题/黑体小标题/仿宋正文/两端对齐/首行缩进2字),
 * 复刻 quote.law 样本排版,区别于 exportMdDocx 的通用 textutil 路径。`docId` = documents 行 id。
 */
export function exportFilingDocx(docId: string, savePath: string): Promise<string> {
  return invoke<string>("export_filing_docx", { docId, savePath });
}

/** save_editor_doc 返回:被写回的 document id(本批永远 = 传入 doc_id,原地覆盖)。 */
export interface EditorSaveResult {
  doc_id: string;
}

/**
 * 2026-05-31 V0.3 D1+D2 · Milkdown 编辑器保存:把编辑后的正文写回该文书的 .md 文件。
 *
 * 后端按 doc_id 查 category(=doc_type)→ 重建 filing 注释头 → 原地覆盖 source_path →
 * 更新 size_bytes/modified_at。前端只传 (title, 正文 body),**不传注释头**(后端重建)。
 * 对 AI 写的材料(source='chat' 分析产物 / 'chat_artifact' 起草文书)有效(后端校验 is_ai_artifact)。
 */
export function saveEditorDoc(
  docId: string,
  title: string,
  contentMd: string,
): Promise<EditorSaveResult> {
  return invoke<EditorSaveResult>("save_editor_doc", { docId, title, contentMd });
}

/* ------------------------------------------------------------------ */
/* 元典查被执行人(2026-05-24 k · P1)                                   */
/* ------------------------------------------------------------------ */

export interface YuandianSubject {
  name: string;
  kind: "person" | "enterprise";
  id_no: string | null;
}

export interface OrchestratorReport {
  case_id: string;
  subjects: YuandianSubject[];
  raw_files: string[];
  elapsed_ms: number;
  failures: [string, string][];
}

export interface DigHint {
  kind: string;
  target: string;
  reason: string;
}

export interface AssessmentReport {
  case_id: string;
  report_path: string | null;
  dig_hints_path: string | null;
  dig_hints: DigHint[];
  raw_count: number;
  corpus_chars: number;
  elapsed_ms: number;
  error: string | null;
}

export interface YuandianP1Response {
  orchestrator: OrchestratorReport;
  assessment: AssessmentReport;
}

/**
 * 主动触发元典 P1 查询:
 *   1. 从 LLM 抽出的 party_contacts 拿被执行人
 *   2. 跑元典聚合查询(聚合优先,约 5-8 端点)→ 原始 JSON 落 external/<case_id>/yuandian_raw/
 *   3. 喂 DeepSeek 写风险提示报告 + 深挖建议
 *   4. 写 cases.risk_assessment_path
 */
export function yuandianBasicQuery(caseId: string): Promise<YuandianP1Response> {
  return invoke<YuandianP1Response>("yuandian_basic_query", { caseId });
}

/** P2 深挖结果(2026-05-24 k-9)。 */
export interface DeepDiveReport {
  case_id: string;
  hints_used: number;
  raw_count: number;
  corpus_chars: number;
  report_path: string | null;
  elapsed_ms: number;
  error: string | null;
}

/**
 * 主动触发 P2 深挖:用 P1 LLM 给的 dig_hints 拉关联公司 / 案号 / 第三方主体,
 * 出深查报告(参考股权转让案件 yuandian_深查 格式)。
 *
 * 前置:必须先跑 P1(yuandianBasicQuery)生成 dig_hints,否则报错。
 * 时间:60-180 秒(取决于 hints 数量)。
 */
export function yuandianDeepDive(caseId: string): Promise<DeepDiveReport> {
  return invoke<DeepDiveReport>("yuandian_deep_dive", { caseId });
}

/** 2026-05-25 V0.1.7 · 完整报告(合并风险报告 + 深挖报告 → 第三份)结果。 */
export interface FullReportResult {
  case_id: string;
  report_path: string | null;
  generated_at: string;
  elapsed_ms: number;
  error: string | null;
}

/** 合并完整报告:前置必须有 risk_assessment_path + deep_dive_report_path。 */
export function yuandianFullReport(caseId: string): Promise<FullReportResult> {
  return invoke<FullReportResult>("yuandian_full_report", { caseId });
}

/* ------------------------------------------------------------------ */
/* 还款记录(2026-05-25 · case_payments)                              */
/* ------------------------------------------------------------------ */

export interface Payment {
  id: string;
  case_id: string;
  amount: number;
  paid_at: string;
  note: string | null;
  created_at: string;
}

export interface NewPayment {
  case_id: string;
  amount: number;
  paid_at: string;
  note: string | null;
}

export function addPayment(p: NewPayment): Promise<Payment> {
  return invoke<Payment>("add_payment", { new: p });
}

export function listPayments(caseId: string): Promise<Payment[]> {
  return invoke<Payment[]>("list_payments", { caseId });
}

export function deletePayment(id: string): Promise<number> {
  return invoke<number>("delete_payment", { id });
}

/* ------------------------------------------------------------------ */
/* 待办清单(2026-06-13 · case_todos · 胡彬律师反馈)                   */
/* ------------------------------------------------------------------ */

export interface Todo {
  id: string;
  case_id: string;
  title: string;
  done: number; // 0=未完成 1=已完成
  done_at: string | null;
  /** 2026-06-14:可选"重要日期"(ISO "YYYY-MM-DD");有则汇入首页日程日历 */
  due_date: string | null;
  created_at: string;
  updated_at: string;
}

/** 跨案件未完成待办(首页汇总)— 扁平结构带 case_name */
export interface OpenTodoRow {
  id: string;
  case_id: string;
  case_name: string;
  title: string;
  due_date: string | null;
  created_at: string;
}

export function addTodo(t: {
  case_id: string;
  title: string;
  due_date?: string | null;
}): Promise<Todo> {
  return invoke<Todo>("add_todo", { new: t });
}

export function listTodos(caseId: string): Promise<Todo[]> {
  return invoke<Todo[]>("list_todos", { caseId });
}

export function listOpenTodos(): Promise<OpenTodoRow[]> {
  return invoke<OpenTodoRow[]>("list_open_todos", {});
}

export function updateTodo(
  id: string,
  upd: { title?: string; done?: number; due_date?: string | null },
): Promise<number> {
  return invoke<number>("update_todo", { id, upd });
}

export function deleteTodo(id: string): Promise<number> {
  return invoke<number>("delete_todo", { id });
}

/* ------------------------------------------------------------------ */
/* 独立日历日程(2026-06-14 · calendar_events · 不绑案件,日历右键添加)  */
/* ------------------------------------------------------------------ */

export interface CalendarEvent {
  id: string;
  date: string; // "YYYY-MM-DD"
  title: string;
  created_at: string;
}

export function addCalendarEvent(e: {
  date: string;
  title: string;
}): Promise<CalendarEvent> {
  return invoke<CalendarEvent>("add_calendar_event", { new: e });
}

export function listCalendarEvents(): Promise<CalendarEvent[]> {
  return invoke<CalendarEvent[]>("list_calendar_events", {});
}

export function deleteCalendarEvent(id: string): Promise<number> {
  return invoke<number>("delete_calendar_event", { id });
}

/* ------------------------------------------------------------------ */
/* 飞书日历(2026-06-17 · 整合外部贡献 PR #9 · 复用本机 lark-cli 登录态)   */
/* ------------------------------------------------------------------ */

/** 拉取飞书日历事件(未启用返回空数组)。start/end 为 "YYYY-MM-DD"。 */
export function fetchFeishuCalendar(
  start: string,
  end: string,
): Promise<FeishuCalendarEvent[]> {
  return invoke<FeishuCalendarEvent[]>("fetch_feishu_calendar", { start, end });
}

/** 按飞书日历事件标题反查本地案件目录(需配案件池表);未配/未命中返回 null。 */
export function findFeishuCasePath(eventSummary: string): Promise<string | null> {
  return invoke<string | null>("find_feishu_case_path", { eventSummary });
}

/** 发送一条不含案件信息的飞书机器人测试消息。 */
export function testFeishuWebhook(url: string): Promise<void> {
  return invoke<void>("test_feishu_webhook", { url });
}

/* ------------------------------------------------------------------ */
/* 法院一张网在线立案(2026-06-17 · 整合外部贡献 PR #8)                   */
/* ------------------------------------------------------------------ */

export function startCourtFiling(
  caseId: string,
  filingType: "civil" | "execution",
  agentIds: string[],
  originalCaseNumber?: string,
  materialFolder?: string,
): Promise<CourtFilingJob> {
  return invoke<CourtFilingJob>("start_court_filing", {
    caseId,
    filingType,
    agentIds,
    originalCaseNumber,
    materialFolder,
  });
}

export function listCourtFilingJobs(caseId: string): Promise<CourtFilingJob[]> {
  return invoke<CourtFilingJob[]>("list_court_filing_jobs", { caseId });
}

export function getCourtFilingJob(id: string): Promise<CourtFilingJob | null> {
  return invoke<CourtFilingJob | null>("get_court_filing_job", { id });
}

/** 检测在线立案运行环境。 */
export function courtFilingEnvCheck(): Promise<CourtFilingEnvReport> {
  return invoke<CourtFilingEnvReport>("court_filing_env_check");
}

/** 一键安装在线立案运行环境(进度走 court-filing-env-progress 事件)。 */
export function courtFilingEnvInstall(): Promise<CourtFilingEnvReport> {
  return invoke<CourtFilingEnvReport>("court_filing_env_install");
}

export function submitCaptchaAnswer(
  jobId: string,
  taskId: string,
  round: number,
  answer: string,
): Promise<void> {
  return invoke<void>("submit_captcha_answer", { jobId, taskId, round, answer });
}

export function listLawyerProfiles(): Promise<LawyerProfile[]> {
  return invoke<LawyerProfile[]>("list_lawyer_profiles");
}

export function saveLawyerProfile(
  profile: Omit<LawyerProfile, "id" | "created_at" | "updated_at">,
): Promise<LawyerProfile> {
  return invoke<LawyerProfile>("save_lawyer_profile", { profile });
}

export function updateLawyerProfile(
  id: string,
  profile: Omit<LawyerProfile, "id" | "created_at" | "updated_at">,
): Promise<LawyerProfile> {
  return invoke<LawyerProfile>("update_lawyer_profile", { id, profile });
}

export function deleteLawyerProfile(id: string): Promise<void> {
  return invoke<void>("delete_lawyer_profile", { id });
}

export function setDefaultLawyer(id: string): Promise<void> {
  return invoke<void>("set_default_lawyer", { id });
}

/* ------------------------------------------------------------------ */
/* 审级实例(2026-06-11 · case_instances)                             */
/* ------------------------------------------------------------------ */

export function listCaseInstances(caseId: string): Promise<CaseInstance[]> {
  return invoke<CaseInstance[]>("list_case_instances", { caseId });
}

export function addCaseInstance(
  caseId: string,
  inst: NewCaseInstance,
): Promise<CaseInstance> {
  return invoke<CaseInstance>("add_case_instance", { caseId, new: inst });
}

export function updateCaseInstance(
  id: string,
  inst: NewCaseInstance,
): Promise<number> {
  return invoke<number>("update_case_instance", { id, new: inst });
}

export function deleteCaseInstance(id: string): Promise<number> {
  return invoke<number>("delete_case_instance", { id });
}

/** V0.2.2 · 软删一个文档(从材料列表移除,主要给 AI artifact 用)。返回受影响行数。 */
export function deleteDocument(id: string): Promise<number> {
  return invoke<number>("delete_document", { id });
}

/**
 * V0.3 · 强制重抽单个文档(源文件列表「重新抽取」按钮)。
 * 后端重置状态为 pending + 清错误 → 后台重新 OCR/抽取(走 extraction_progress 事件)。
 * ⚠️ PDF 走云端 OCR 会再烧 MinerU 积分。
 */
export function reextractDocument(docId: string): Promise<void> {
  return invoke<void>("reextract_document", { docId });
}

/**
 * 2026-06-13(胡彬律师反馈)· 去水印重新识别。
 * 带大幅水印的工商调档件/章程改用 PP-OCRv6(纯文字)+ 去水印过滤(强制,不回退 VL)。
 * 同样不自动跑全案分析,识别完手动点「重新分析」。
 */
export function reextractDocumentDewatermark(docId: string): Promise<void> {
  return invoke<void>("reextract_document_dewatermark", { docId });
}

/* ------------------------------------------------------------------ */
/* 用户反馈 — MD 文件方案(2026-05-24 e)                              */
/* ------------------------------------------------------------------ */

export interface ConsoleError {
  level: string; // "error" | "warn" | "unhandled"
  message: string;
  at: string | null;
}

export interface SettingsSnapshot {
  setup_completed: boolean;
  user_display_name_set: boolean;
  mineru_api_key: string; // "[SET]" | "[EMPTY]"
  mineru_endpoint: string | null;
  mineru_verified: boolean;
  deepseek_api_key: string;
  deepseek_endpoint: string | null;
  deepseek_verified: boolean;
  yuandian_api_key: string;
  yuandian_verified: boolean;
  local_model_dir: string | null;
  local_server_endpoint: string | null;
  local_server_auto_start: boolean;
}

export interface SystemInfo {
  data_dir: string;
  data_dir_writable: boolean;
  db_size_mb: number | null;
  extracts_files: number | null;
  extracts_size_mb: number | null;
  reports_size_mb: number | null;
  disk_free_gb: number | null;
  pdftotext_available: boolean;
  pdftoppm_available: boolean;
}

export interface MetricSample {
  filename: string;
  ext: string;
  file_size_bytes: number;
  stage: string;
  backend: string;
  outcome: string;
  elapsed_ms: number;
  text_chars: number | null;
  error_short: string | null;
  created_at: string;
}

export interface BackendStat {
  backend: string;
  stage: string;
  samples: number;
  ok_samples: number;
  avg_ms: number;
  p50_ms: number;
  avg_chars: number | null;
}

export interface FeedbackDiagnostic {
  client_id_short: string;
  app_version: string;
  os_version: string;
  language: string;
  llm_provider: string;
  ocr_provider: string;
  local_server_status: string;
  deepseek_balance: number | null;
  stats: {
    cases_total: number;
    documents_total: number;
    documents_done: number;
    documents_skipped: number;
    documents_failed: number;
    documents_pending: number;
  };
  recent_failures: {
    filename: string;
    category: string | null;
    created_at: string;
    last_error: string | null;
  }[];
  settings_snapshot: SettingsSnapshot;
  system_info: SystemInfo;
  stderr_tail: string[];
  console_errors: ConsoleError[];
  /** 2026-05-26 V0.1.12:最近 200 条抽取性能埋点(stage/backend/耗时/字数/成败) */
  metrics_tail: MetricSample[];
  /** 2026-05-26 V0.1.12:按 backend 聚合的统计(本地 vs 云端 A/B) */
  metrics_summary: BackendStat[];
  /** 2026-05-27 V0.1.13+:chat 用量(按 model + task_type 聚合,**不含 content**) */
  chat_usage: ChatUsageBucket[];
  /** 2026-05-27 V0.1.13+:功能模块用量(聚合数字,无业务标识) */
  feature_usage: FeatureUsage;
}

export interface ChatUsageBucket {
  model: string;
  task_type: string;
  samples: number;
  ok_samples: number;
  avg_prompt: number;
  avg_completion: number;
  avg_latency_ms: number;
  avg_chars: number;
}

export interface FeatureUsage {
  chat_messages_total: number;
  chat_artifacts_total: number;
  cases_with_analysis_report: number;
  cases_with_risk_report: number;
  cases_with_deep_dive_report: number;
  cases_with_full_report: number;
  cases_with_user_overrides: number;
  case_types: { key: string; count: number }[];
  document_sources: { key: string; count: number }[];
  yuandian_queried_cases: number;
}

/** 收集反馈用的诊断信息(给弹窗预填)。
 * 2026-05-26 V0.1.11:打开反馈弹窗时把前端累积的 console.error/window.onerror 传过来,
 * Rust 端会跟自家 stderr ring buffer + settings 脱敏快照 + 系统信息一起写进 MD。 */
export function collectFeedbackDiagnostic(
  consoleErrors: ConsoleError[] = [],
): Promise<FeedbackDiagnostic> {
  return invoke<FeedbackDiagnostic>("collect_feedback_diagnostic", {
    consoleErrors,
  });
}

/**
 * 2026-05-27 V0.1.13+ · 调用默认邮件客户端发反馈。
 *
 * 返回值 `[path, warning]`:
 *   - path: "applescript" 表示走 Mail.app 自动带附件
 *   - path: "mailto" 表示 fallback,需要用户手动拖附件
 *   - warning: 走 mailto 时填提示;applescript 时空串
 */
export function sendFeedbackEmail(
  mdPath: string,
  to: string,
  subject: string,
): Promise<[string, string]> {
  return invoke<[string, string]>("send_feedback_email", {
    mdPath,
    to,
    subject,
  });
}

/** 把诊断 + 用户描述拼成 MD,写到桌面。返回最终绝对路径。 */
export function saveFeedbackMd(
  info: FeedbackDiagnostic,
  description: string,
): Promise<string> {
  return invoke<string>("save_feedback_md", { info, description });
}

/* ------------------------------------------------------------------ */
/* DeepSeek 余额(给整个界面右上角 chip 用)                            */
/* ------------------------------------------------------------------ */

/** 对应 Rust `DeepSeekBalance` */
export interface DeepSeekBalance {
  total_balance: number;
  granted_balance: number;
  topped_up_balance: number;
  /** 今日消费(元)— 没有昨日快照时为 null */
  today_consumed: number | null;
  /** ISO 8601 */
  fetched_at: string;
}

/**
 * 拉 DeepSeek 余额 + 算今日消费。
 * `refresh=true` → 发请求拉新数据并落 DB(慢,失败抛错)
 * `refresh=false` → 只读 DB 缓存(瞬时,无数据返 null)
 */
export function getDeepSeekBalance(refresh = false): Promise<DeepSeekBalance | null> {
  return invoke<DeepSeekBalance | null>("get_deepseek_balance", { refresh });
}

/**
 * 2026-05-24 e:手工覆盖案件工作流状态(看板卡片右上角的 chip)。
 *
 * `status = null` → 清空,前端走自动推断;
 * 非 null → 用户手工选过(8 档之一,见 modules/litigation/lib/inferStatus.ts)。
 */
export function updateWorkflowStatus(
  caseId: string,
  status: string | null,
): Promise<void> {
  return invoke<void>("update_workflow_status", { caseId, status });
}

/**
 * 2026-05-26 V0.1.13 · 写入案件 user_overrides JSON(编辑模式手改 overlay)。
 *
 * `overridesJson = null` → 清空所有用户改动,详情页回到纯 LLM 抽取的视图;
 * 非 null → 整段覆盖(前端 debounce 300ms 后整包提交,见 P3 编辑模式)。
 *
 * 结构由前端 `lib/userOverrides.ts` 定义,后端 sqlite 透传不解析。
 * LLM 全局抽永不写这列,所以这里保存的用户值不会被下次重抽覆盖。
 */
export function updateCaseOverrides(
  caseId: string,
  overridesJson: string | null,
): Promise<void> {
  return invoke<void>("update_case_overrides", { caseId, overridesJson });
}

/**
 * 2026-05-26 V0.1.13 · 写入"首页在办案件"用户拖动后的顺序。
 *
 * 空数组等同于"清空" — 后端会把 home_case_order 设为 None,首页回到默认排序。
 * 后端 read-modify-write 只动这一个字段,跟 SettingsModal 同时写不会冲突。
 */
export function updateHomeCaseOrder(caseIds: string[]): Promise<void> {
  return invoke<void>("update_home_case_order", { caseIds });
}

/**
 * 重抽该案件的所有 LLM 抽取(**真的会跑 LLM**,慢)。
 *
 * 用途:升级 prompt 后,让 LLM 按新 prompt 重新抽存量文档的字段。
 * 做法:把 `extraction_status='done'` 的文档重置为 `'pending'` + 清抽取产物,
 *      然后触发后台 pipeline。前端订阅 `extraction_progress` 事件看进度。
 *
 * @returns 被重置的文档数(立即返回,不等 LLM 跑完)
 */
export function recomputeCaseExtraction(caseId: string): Promise<number> {
  return invoke<number>("recompute_case_extraction", { caseId });
}

/**
 * 刷新案件源文件(增量同步)。
 *
 * 用户在案件文件夹里手工加/改/删文件后,点「🔄 刷新源文件」触发。
 * 后端扫文件夹 → diff documents 表 → 新增/修改的标 pending + 软删消失的 →
 * 后台 spawn_extraction 跑 pending(并自动重生成画像 + 报告)。
 *
 * 立即返回 SyncStats,前端 toast 显示;LLM 抽取进度通过 `extraction_progress`
 * 事件订阅(跟首次导入复用同一套进度推送)。
 */
export interface SyncStats {
  added: number;
  updated: number;
  unchanged: number;
  deleted: number;
  moved: number;
}

export function refreshCaseFiles(
  caseId: string,
  referenceMaterials = false,
): Promise<SyncStats> {
  return invoke<SyncStats>("refresh_case_files", { caseId, referenceMaterials });
}

export function relinkCaseFolder(
  caseId: string,
  newFolder: string,
  referenceMaterials = false,
): Promise<SyncStats> {
  return invoke<SyncStats>("relink_case_folder", { caseId, newFolder, referenceMaterials });
}

/* ───────────── 法院短信处理(V0.3 · 一张网 zxfw.court.gov.cn) ───────────── */

/** 一张网送达链接参数(只传这三个,不传时效性的下载地址 wjlj)。 */
export interface ZxfwLink {
  sdbh: string;
  qdbh: string;
  sdsin: string;
}

export interface CourtSmsDocBrief {
  name: string;
  ext: string;
}

export interface CourtSmsPreview {
  court: string | null;
  case_no: string | null;
  has_link: boolean;
  link: ZxfwLink | null;
  docs: CourtSmsDocBrief[];
  matched_case_id: string | null;
  matched_case_name: string | null;
  note: string | null;
  /** 2026-06-11:案号没匹配上时按当事人姓名匹配的候选(命中名多的在前),前端预选第一个让用户确认 */
  name_matches: CourtSmsNameMatch[];
}

export interface CourtSmsNameMatch {
  case_id: string;
  case_name: string;
  matched_names: string[];
}

export interface CourtSmsIngestResult {
  downloaded: string[];
  skipped: string[];
  sync: SyncStats;
}

export interface CourtSmsLocalDownloadResult {
  downloaded: string[];
  skipped: string[];
  folder: string;
}

/** 预览:解析短信 + 拉文书列表 + 匹配在办案件(不下载、无副作用)。 */
export function previewCourtSms(smsText: string): Promise<CourtSmsPreview> {
  return invoke<CourtSmsPreview>("preview_court_sms", { smsText });
}

/** 导入:重新拉新鲜下载地址 → 下载 PDF 进案件源文件夹 → 触发抽取上看板。 */
export function ingestCourtSms(
  caseId: string,
  link: ZxfwLink,
): Promise<CourtSmsIngestResult> {
  return invoke<CourtSmsIngestResult>("ingest_court_sms", { caseId, link });
}

/** 下载:重新拉新鲜下载地址 → 保存到用户指定本地文件夹;不归档案件、不触发抽取。 */
export function downloadCourtSmsToFolder(
  link: ZxfwLink,
  targetFolder: string,
): Promise<CourtSmsLocalDownloadResult> {
  return invoke<CourtSmsLocalDownloadResult>("download_court_sms_to_folder", {
    link,
    targetFolder,
  });
}

/* ───────────── 快递查询(V0.3 · 快递100 实时查询) ───────────── */

export interface ExpressTrackNode {
  time: string;
  context: string;
}

/** 一条被跟踪的快递记录(落本地 express_tracks.json)。 */
export interface ExpressTrack {
  num: string;
  com: string;
  com_name: string;
  /** 收寄件人手机号(顺丰/中通查询必填);旧数据可能无此字段。 */
  phone?: string;
  state: string;
  state_text: string;
  delivered: boolean;
  nodes: ExpressTrackNode[];
  created_at: string;
  last_polled_at: string;
}

/** 查询并跟踪一个单号(查 + 落本地 + 返回最新全列表)。需先在设置填快递100 customer+key。
 *  phone = 收寄件人手机号,顺丰/中通必填(否则快递100 报 408),其它快递可留空。 */
export function queryExpress(
  com: string,
  comName: string,
  num: string,
  phone: string,
): Promise<ExpressTrack[]> {
  return invoke<ExpressTrack[]>("query_express", { com, comName, num, phone });
}

/** 列出本地所有跟踪记录(不联网)。 */
export function listExpressTracks(): Promise<ExpressTrack[]> {
  return invoke<ExpressTrack[]>("list_express_tracks");
}

/** 刷新在跟踪的单号(未签收+30天内+距上次≥6小时;40天内重查免费)。 */
export function refreshExpressTracks(): Promise<ExpressTrack[]> {
  return invoke<ExpressTrack[]>("refresh_express_tracks");
}

/** 删除一个跟踪记录。 */
export function deleteExpressTrack(num: string): Promise<ExpressTrack[]> {
  return invoke<ExpressTrack[]>("delete_express_track", { num });
}

/* ------------------------------------------------------------------ */
/* 健康检查                                                            */
/* ------------------------------------------------------------------ */

export interface DbHealth {
  ok: boolean;
  table_count: number;
  case_count: number;
  db_path: string;
}

export function dbHealth(): Promise<DbHealth> {
  return invoke<DbHealth>("db_health");
}

/* ------------------------------------------------------------------ */
/* 案件 AI 助手 · case-aware chat (V0.1.13+)                          */
/* ------------------------------------------------------------------ */

/**
 * 固定任务的 task_type 枚举。自由问 / 写文书入口传 null。
 *
 * V0.3.3:6 个功能单一的生成型任务(案件总览/证据目录/时间线/客户进展/查付款/待补材料)已删 ——
 * AI 助手已是 agent,用户直接打字提需求,它自己拆解、调工具、产出直答或可编辑文书。现有 5 个
 * 复杂工具/分析型任务(都走 agent_loop):
 *  - compile_legal_basis:围绕诉求查法条+案例(没引用文档时)
 *  - verify_my_draft:核校这份引用的法条/案例是否真实(引用文档时)
 *  - find_similar_cases:找相似案例对比
 *  - simulate_opposition:站对方立场推演抗辩/进攻 + 我方应对
 *  - deep_analysis:请求权基础+鉴定式深度分析(两闸交互确认后逐要件论证,落深度分析报告)
 *  - criminal_deep_analysis:三阶层犯罪论+鉴定式刑事深度分析(仅刑事 tab AI 助手用)
 */
export type CaseChatTaskType =
  | "compile_legal_basis"
  | "verify_my_draft"
  | "find_similar_cases"
  | "simulate_opposition"
  | "deep_analysis"
  | "criminal_deep_analysis";

/** chat_messages 表一行(后端 db::chat::ChatMessage 对应)。 */
export interface ChatMessage {
  id: string;
  case_id: string;
  /** 'user' | 'assistant' */
  role: string;
  content: string;
  task_type: CaseChatTaskType | null;
  model: string | null;
  prompt_tokens: number | null;
  completion_tokens: number | null;
  latency_ms: number | null;
  /** JSON 数组,引用的 document.id */
  based_on: string | null;
  /** 若 assistant 输出落了 artifact,这里是新 documents.id */
  artifact_doc_id: string | null;
  /** 出错时填脱敏错误(content 为空串) */
  error_short: string | null;
  created_at: string;
  /**
   * V0.2 D6.5 · user 消息上的 AttachmentPicker 引用 doc.id 列表(JSON 数组字符串)。
   * 前端 JSON.parse 后给 AttachmentChips 渲染。assistant 消息通常为 null。
   */
  attached_doc_ids: string | null;
  /**
   * V0.2 D6.5 · assistant 消息的 `<CITATIONS>` 解析结果(JSON 数组字符串,`Citation[]`)。
   * 前端 JSON.parse 后给 CitationsCard 渲染。仅 assistant 消息有值。
   */
  citations_json: string | null;
  /**
   * V0.2 D6.5 · 关联 chat_tasks.id;前端可据此 lazy fetch tool_calls trace(任务 V0.3+)。
   */
  task_id: string | null;
}

/** SSE 流式事件,通过 listen("chat-stream-{messageId}") 接收。 */
/** V0.3 · ask_user 选项式追问的单个问题(后端 AskQuestion 镜像) */
export interface AskQuestion {
  question: string;
  /** 预设选项;空数组 → 只显自由输入框 */
  options: string[];
  /** 是否允许自由输入(无选项时前端也强制可输入) */
  allow_input: boolean;
}

export type ChatStreamEvent =
  | { kind: "delta"; text: string }
  | {
      /** V0.3 · thinking 模型推理增量 — 前端显示「深度推理中…(N 字)」进度,不进正文 */
      kind: "reasoning";
      text: string;
    }
  | {
      /** V0.2 D6.5 · 单次工具调用完成 — 前端 ToolCallTrace 追加一行 */
      kind: "tool_call";
      record: import("./types").ToolCallRecord;
    }
  | {
      /** V0.3 · 模型调 ask_user 发起选项式追问 — 前端渲染选项卡片 */
      kind: "ask_user";
      questions: AskQuestion[];
    }
  | {
      kind: "done";
      prompt_tokens: number | null;
      completion_tokens: number | null;
      model: string;
    }
  | { kind: "error"; message: string };

/** case_chat 完成后的元数据(promise 返回值)。 */
export interface CaseChatResult {
  user_message_id: string;
  assistant_message_id: string;
  model: string | null;
  prompt_tokens: number | null;
  completion_tokens: number | null;
  latency_ms: number;
  /** 若产出落了 artifact,这里返回 documents.id */
  artifact_doc_id: string | null;
  /** "lightweight" | "keyword-hits" | "keyword-fallback-lightweight" */
  strategy: string;
  based_on_doc_ids: string[];
  /** V0.2 D6.5 · `<CITATIONS>` 解析结果,流式结束时直接返回前端,免一次 list 回拉 */
  citations: import("./types").Citation[];
  /** V0.2 D6.5 · agent_loop 全程 tool_trace,兜底防漏 emit */
  tool_calls: import("./types").ToolCallRecord[];
  /** V0.2 D6.5 · 本会话 chat_tasks.id(若走了 agent_loop) */
  task_id: string | null;
  /** V0.3 · 本轮模型调 ask_user 发起的选项式追问;前端据此渲染选项卡片。null = 正常回答 */
  ask_user: AskQuestion[] | null;
}

export interface CaseChatInput {
  case_id: string;
  user_message: string;
  task_type: CaseChatTaskType | null;
  /** 前端事先生成的 uuid,作为流式 channel 名后缀 + assistant_message_id */
  message_id: string;
  /**
   * V0.2 D6-D7 · 本轮引用的文档 id 数组(`AttachmentPicker` 勾选的)。
   * null/空数组都表示不引用;非空时后端强制走 agent_loop 工具链路。
   */
  attached_doc_ids?: string[] | null;
  /**
   * V0.3 ADR-0003 Phase 1B · 写作模式下编辑器里正打开的 AI 文书 doc_id。
   * 非空时后端注入 system prompt,让模型知道「要改的是这份」→ 局部 edit_artifact。
   */
  editing_doc_id?: string | null;
}

/**
 * 启动一次案件聊天。
 *
 * 调用前应:
 *   1. 用 crypto.randomUUID() 生成 message_id
 *   2. listen("chat-stream-{message_id}") 拼字 delta
 *   3. 然后 await caseChat(...)
 *
 * Promise resolve 时流式已结束(或被取消 / 出错)。
 */
export function caseChat(input: CaseChatInput): Promise<CaseChatResult> {
  return invoke<CaseChatResult>("case_chat", { input });
}

/** 取案件聊天历史(升序;不传 limit = 全部)。 */
export function listChatHistory(
  caseId: string,
  limit?: number,
): Promise<ChatMessage[]> {
  return invoke<ChatMessage[]>("list_chat_history", {
    caseId,
    limit: limit ?? null,
  });
}

/**
 * 取消进行中的 chat。
 * message_id 必须跟 caseChat 入参的 message_id 相同。
 * 返回 true 表示成功取消,false 表示已经完成(找不到对应 cancel sender)。
 */
export function cancelChat(messageId: string): Promise<boolean> {
  return invoke<boolean>("cancel_chat", { messageId });
}

/** 清空某案件下全部聊天记录(用户主动)。返回删除条数。 */
export function clearChatHistory(caseId: string): Promise<number> {
  return invoke<number>("clear_chat_history", { caseId });
}

/* ------------------------------------------------------------------ */
/* V0.2 D7 · 本地知识库 + 元典积分 Settings 卡片                      */
/* ------------------------------------------------------------------ */

/** 本地 KB 三态(对应 Rust `local_kb::status::KbStatus`)。 */
export type KbStatus =
  | {
      state: "bound";
      root: string;
      cache_dir: string;
      cache_count: number;
      /** `{"法规": 156, "案例": 89, "企业": 242, "其他": 0}` */
      cache_breakdown: Record<string, number>;
      /** 可检索内容篇数(raw/notes + wiki/sources + wiki/topics + gap-log),跟 cache_count 是两回事 */
      content_count: number;
      total_size_bytes: number | null;
      /** RFC3339 时间戳,null = 还没写过任何缓存 */
      last_write_at: string | null;
    }
  | {
      state: "unbound";
      /** 用户配置过但路径不存在,或没配过 */
      configured_root: string | null;
    }
  | {
      state: "permission_denied";
      root: string;
    };

/** 创建空 KB 结果(对应 Rust `local_kb::init::KbInitResult`)。 */
export interface KbInitResult {
  /** RFC3339 */
  created_at: string;
  path: string;
  files_created: number;
  dirs_created: number;
  /** true = 复用已有 KB(只补缺),false = 全新建 */
  reused_existing: boolean;
}

/** 导出结果。 */
export interface KbExportResult {
  output_path: string;
  total_items: number;
  total_size_bytes: number;
}

/** 导入冲突策略。 */
export type KbConflictStrategy = "skip" | "overwrite_older" | "always_overwrite";

/** 单条冲突 / 失败记录。 */
export interface KbConflictRecord {
  path: string;
  /** `"skip"` / `"overwrite"` / `"failed"` */
  action: string;
  reason: string;
}

/** 导入结果。 */
export interface KbImportResult {
  total_in_zip: number;
  added: number;
  skipped: number;
  overwritten: number;
  failed: number;
  conflicts: KbConflictRecord[];
}

/** 检测本地 KB 状态;每次打开 Settings 都调一次(状态实时)。 */
export function detectKbStatus(): Promise<KbStatus> {
  return invoke<KbStatus>("detect_kb_status");
}

/** 在指定路径建空 KB(已有则只补缺失子目录),自动写回 settings 启用。 */
export function createLocalKb(path: string): Promise<KbInitResult> {
  return invoke<KbInitResult>("create_local_kb", { path });
}

/** 从 zip 导入资料包合并进当前 KB。 */
export function importKbFromZip(
  zipPath: string,
  onConflict: KbConflictStrategy,
): Promise<KbImportResult> {
  return invoke<KbImportResult>("import_kb_from_zip", {
    zipPath,
    onConflict,
  });
}

/** 把当前 KB 元典缓存打成 zip 导出到指定路径。 */
export function exportKbToZip(outputPath: string): Promise<KbExportResult> {
  return invoke<KbExportResult>("export_kb_to_zip", { outputPath });
}

// ===== 案件资料包(双人办案材料合并)=====

/** 导入资料包预览(对应 Rust `case_bundle::BundlePreview`)。 */
export interface CaseBundlePreview {
  name: string;
  caseNo: string | null;
  parties: string | null;
  summary: string | null;
  fileCount: number;
  /** 按案号自动匹配到的本地同一案件(建议合并目标),无匹配为 null。 */
  suggestedCaseId: string | null;
  suggestedCaseName: string | null;
}

/** 合并结果(对应 Rust `case_bundle::MergeReport`)。 */
export interface CaseMergeReport {
  targetCaseId: string;
  targetCaseName: string;
  createdNew: boolean;
  added: number;
  deduped: number;
  skipped: number;
  filledFields: string[];
}

/** 把一个案件导出成 zip 资料包,返回打进包的文件数。 */
export function exportCaseBundle(caseId: string, outputPath: string): Promise<number> {
  return invoke<number>("export_case_bundle", { caseId, outputPath });
}

/** 预览资料包 + 按案号建议本地合并目标。 */
export function previewCaseBundle(zipPath: string): Promise<CaseBundlePreview> {
  return invoke<CaseBundlePreview>("preview_case_bundle", { zipPath });
}

/** 合并资料包进目标案件(targetCaseId 为 null → 新建)。 */
export function mergeCaseBundle(
  zipPath: string,
  targetCaseId: string | null,
): Promise<CaseMergeReport> {
  return invoke<CaseMergeReport>("merge_case_bundle", { zipPath, targetCaseId });
}

/** prune_yuandian_cache 回执(对应 Rust `local_kb::cache::PruneStats`)。 */
export interface KbPruneStats {
  /** 清掉的 index 条目数 */
  removed_entries: number;
  /** 删掉的物理文件数(.md + .raw.json) */
  removed_files: number;
}

/**
 * 清理「搜索/向量类、超 maxAgeDays 天」的元典缓存(检索列表)。
 * 法规/案例/企业**全文详情**不动(复用资产)。显式触发,需用户确认。
 */
export function pruneYuandianCache(maxAgeDays: number): Promise<KbPruneStats> {
  return invoke<KbPruneStats>("prune_yuandian_cache", { maxAgeDays });
}

/** 本地知识库语义向量索引规模(对应 Rust `local_kb::semantic::KbIndexStats`)。 */
export interface KbIndexStats {
  /** 已索引文件数(法律 + 案例 + 企业等) */
  files: number;
  /** 切片(向量)数 */
  chunks: number;
}

/** 读语义索引现有规模(不建不改)。 */
export function getLocalKbIndexStats(): Promise<KbIndexStats> {
  return invoke<KbIndexStats>("get_local_kb_index_stats");
}

/** 重建/更新本地知识库语义向量索引(法条+案例+企业;增量,进度走 `kb_index_progress` 事件)。 */
export function buildLocalKbSemanticIndex(): Promise<KbIndexStats> {
  return invoke<KbIndexStats>("build_local_kb_semantic_index");
}

/** 月度元典积分账(对应 Rust `db::credits::MonthlyCredits`)。 */
export interface YuandianMonthlyStats {
  year_month: string;
  credits_used: number;
  api_calls: number;
  /** 本地 KB 命中次数(替元典省了 N 次外查) */
  kb_hits: number;
  updated_at: string;
}

/** 取本月元典积分统计。 */
export function getYuandianMonthlyStats(): Promise<YuandianMonthlyStats> {
  return invoke<YuandianMonthlyStats>("get_yuandian_monthly_stats");
}

/** 积分账总览(当月 + 上月 + 累计;对应 Rust `db::credits::CreditsOverview`)。
 *  当月跨月归 0 时,前端用 prev_month / total 补显示,避免误以为数据丢了。 */
export interface CreditsOverview {
  current: YuandianMonthlyStats;
  prev_month: YuandianMonthlyStats | null;
  total_credits: number;
  total_api_calls: number;
  total_kb_hits: number;
}

/** 取元典积分账总览(当月 + 上月 + 累计)。 */
export function getYuandianCreditsOverview(): Promise<CreditsOverview> {
  return invoke<CreditsOverview>("get_yuandian_credits_overview");
}

/** 验证 embedding 配置(embed 探针词),成功返回向量维度。给设置页验证按钮。
 *  endpoint/model 留空则后端用硅基流动 bge-m3 默认。 */
export function verifyEmbeddingKey(
  endpoint: string,
  model: string,
  apiKey: string,
): Promise<number> {
  return invoke<number>("verify_embedding_key", { endpoint, model, apiKey });
}

/* ============================================================
 * 合同审查(非诉 tab · 2026-06-17)
 * 后端 contract_review 模块:上传 docx → LLM 三层审查 → 风险清单 + 结论 + 审查意见书 docx。
 * 字段与 Rust serde 一致(snake_case)。
 * ============================================================ */

/** 单条风险点(对应 Rust `contract_review::analyze::ReviewRisk`)。 */
export interface ReviewRisk {
  level: string; // P0 / P1 / P2
  title: string;
  clause_ref: string;
  paragraph_index: number | null;
  anchor_text: string;
  consequence: string;
  basis: string;
  suggestion: string;
  recommended_text: string;
  action: string; // revise / comment
}

/** 审查结论(对应 Rust `ReviewConclusion`)。 */
export interface ReviewConclusion {
  verdict: string; // 可签 / 有条件可签 / 不建议签
  preconditions: string[];
  summary: string;
}

/** 完整审查结果(对应 Rust `ContractReviewResult`)。 */
export interface ContractReviewResult {
  contract_type: string;
  conclusion: ReviewConclusion;
  risks: ReviewRisk[];
}

/** 审查命令返回(对应 Rust `ContractReviewResponse`)。 */
export interface ContractReviewResponse {
  contract_name: string;
  paragraph_count: number;
  result: ContractReviewResult;
  opinion_md: string;
}

/** 审查一份合同 .docx。stance: party_a/party_b/neutral;strictness: lenient/normal/aggressive。 */
export function reviewContractDocx(
  docxPath: string,
  stance: string,
  strictness: string,
  contractTypeHint: string,
): Promise<ContractReviewResponse> {
  return invoke<ContractReviewResponse>("review_contract_docx", {
    docxPath,
    stance,
    strictness,
    contractTypeHint,
  });
}

/**
 * 把旧版 .doc / .rtf / .odt 合同转成 .docx,返回转换后(临时目录)路径。
 * macOS 用系统 textutil;其他平台尝试 LibreOffice,没装则透传错误引导另存为 .docx。
 */
export function convertDocToDocx(srcPath: string): Promise<string> {
  return invoke<string>("convert_doc_to_docx", { srcPath });
}

/** 导出审查意见书 Word(把 review 拿到的 result 原样回传)。返回写盘路径。 */
export function exportContractOpinionDocx(
  result: ContractReviewResult,
  contractName: string,
  stance: string,
  strictness: string,
  savePath: string,
): Promise<string> {
  return invoke<string>("export_contract_opinion_docx", {
    result,
    contractName,
    stance,
    strictness,
    savePath,
  });
}

/** 导出修订批注版 docx 的结果摘要(对应 Rust `RedlineSummary`)。 */
export interface RedlineSummary {
  applied_inline: number;
  applied_comment: number;
  skipped: string[];
  saved_path: string;
}

/** 导出修订批注版 Word(在原合同 docx 上落 P2 整段批注 + P3 行内修订痕迹)。 */
export function exportContractRedlineDocx(
  srcDocxPath: string,
  result: ContractReviewResult,
  author: string,
  savePath: string,
): Promise<RedlineSummary> {
  return invoke<RedlineSummary>("export_contract_redline_docx", {
    srcDocxPath,
    result,
    author,
    savePath,
  });
}

/* ============================================================
 * 合同起草(非诉 tab · 2026-06-18 · B1)
 * 后端 contract_draft 模块:口语化需求 → 三观四步法规划(plan)→ 生成合同草案(generate)→ 导出 Word。
 * 方法论借鉴 pa1nrui1/legal-skills(MIT,小潘律师);prompt/引擎自建。字段与 Rust serde 一致(snake_case)。
 * ============================================================ */

/** 引导式采集的单个要素(对应 Rust `RequiredField`)。 */
export interface RequiredField {
  field: string;
  why: string;
  example: string;
  required: boolean;
}

/** 起草前规划(对应 Rust `ContractDraftPlan`)。 */
export interface ContractDraftPlan {
  contract_type: string;
  transaction_essence: string;
  structure_outline: string[];
  required_info: RequiredField[];
  clarifying_questions: string[];
  notes: string;
}

/** 关键条款及理由(对应 Rust `DraftKeyClause`)。 */
export interface DraftKeyClause {
  clause: string;
  rationale: string;
}

/** 合同草案结果(对应 Rust `ContractDraftResult`)。 */
export interface ContractDraftResult {
  contract_type: string;
  contract_name: string;
  draft_md: string;
  key_clauses: DraftKeyClause[];
  risks: string[];
  assumptions: string[];
  missing_info: string[];
}

/** 起草需求附件抽取结果:本地文本抽取,扫描 PDF 不触发云端 OCR。 */
export interface ContractDraftContextFile {
  filename: string;
  path: string;
  text: string;
  char_count: number;
  truncated: boolean;
}

export function extractContractDraftContextFile(
  path: string,
): Promise<ContractDraftContextFile> {
  return invoke<ContractDraftContextFile>("extract_contract_draft_context_file", { path });
}

/** 步骤 1-3:起草前规划(类型判定 + 结构大纲 + 引导式采集清单)。stance: party_a/party_b/neutral。 */
export function planContractDraft(
  requirement: string,
  stance: string,
  contractTypeHint: string,
): Promise<ContractDraftPlan> {
  return invoke<ContractDraftPlan>("plan_contract_draft", {
    requirement,
    stance,
    contractTypeHint,
  });
}

/** 步骤 4:据已采集信息生成完整合同草案。collectedInfo 为采集清单/追问的回答汇总(可空)。 */
export function generateContractDraft(
  requirement: string,
  stance: string,
  contractTypeHint: string,
  collectedInfo: string,
): Promise<ContractDraftResult> {
  return invoke<ContractDraftResult>("generate_contract_draft", {
    requirement,
    stance,
    contractTypeHint,
    collectedInfo,
  });
}

/** 导出合同草案为 Word(把 generate 拿到的 draft_md + contract_name 回传)。返回写盘路径。 */
export function exportContractDraftDocx(
  draftMd: string,
  contractName: string,
  savePath: string,
): Promise<string> {
  return invoke<string>("export_contract_draft_docx", {
    draftMd,
    contractName,
    savePath,
  });
}

/* ── B2 多轮修订 + 版本管理(对应 Rust contract_draft 落库命令 + db::contract_drafts)── */

/** 修订结果(对应 Rust `ContractReviseResult`)。 */
export interface ContractReviseResult {
  draft_md: string;
  change_summary: string;
  key_clauses: DraftKeyClause[];
  risks: string[];
}

/** 一份合同起草事项(对应 Rust `ContractDraft`)。 */
export interface ContractDraft {
  id: string;
  contract_name: string;
  contract_type: string;
  stance: string;
  requirement: string;
  status: string; // working / final
  latest_version: number;
  created_at: string;
  updated_at: string;
}

/** 一版合同稿(对应 Rust `ContractDraftVersion`)。 */
export interface ContractDraftVersion {
  id: string;
  draft_id: string;
  version_no: number;
  source: string;
  based_on_version: number | null;
  purpose: string;
  draft_md: string;
  change_summary: string;
  is_final: number; // 0 / 1
  created_at: string;
}

/** 多轮修订:据修订要求把现有合同改成新版(不落库;落库走 addContractDraftVersion)。 */
export function reviseContractDraft(
  currentMd: string,
  feedback: string,
  stance: string,
): Promise<ContractReviseResult> {
  return invoke<ContractReviseResult>("revise_contract_draft", {
    currentMd,
    feedback,
    stance,
  });
}

/** 保存一份合同草案(同时落第 1 版)。 */
export function saveContractDraft(
  contractName: string,
  contractType: string,
  stance: string,
  requirement: string,
  draftMd: string,
): Promise<ContractDraft> {
  return invoke<ContractDraft>("save_contract_draft", {
    contractName,
    contractType,
    stance,
    requirement,
    draftMd,
  });
}

/** 全部草案列表(最近更新在前)。 */
export function listContractDrafts(): Promise<ContractDraft[]> {
  return invoke<ContractDraft[]>("list_contract_drafts");
}

/** 某草案的全部版本(版本号升序)。 */
export function listContractDraftVersions(
  draftId: string,
): Promise<ContractDraftVersion[]> {
  return invoke<ContractDraftVersion[]>("list_contract_draft_versions", { draftId });
}

/** 追加一版(多轮修订落库)。 */
export function addContractDraftVersion(
  draftId: string,
  source: string,
  basedOnVersion: number | null,
  purpose: string,
  draftMd: string,
  changeSummary: string,
): Promise<ContractDraftVersion> {
  return invoke<ContractDraftVersion>("add_contract_draft_version", {
    draftId,
    source,
    basedOnVersion,
    purpose,
    draftMd,
    changeSummary,
  });
}

/** 标记最终版(用户明确确认才调)。 */
export function markContractDraftFinal(
  draftId: string,
  versionId: string,
): Promise<void> {
  return invoke<void>("mark_contract_draft_final", { draftId, versionId });
}

/** 删除草案及其全部版本。 */
export function deleteContractDraft(id: string): Promise<number> {
  return invoke<number>("delete_contract_draft", { id });
}

/* ── B3 起草偏好库(对应 Rust contract_draft 偏好命令 + db::contract_preferences)── */

/** 一条起草偏好(对应 Rust `ContractPreference`)。contract_type 空 = 通用。 */
export interface ContractPreference {
  id: string;
  contract_type: string;
  topic: string;
  preference: string;
  source: string; // user / ai_suggest
  created_at: string;
}

/** 新增一条起草偏好(contractType 空 = 通用)。 */
export function addContractPreference(
  contractType: string,
  topic: string,
  preference: string,
): Promise<ContractPreference> {
  return invoke<ContractPreference>("add_contract_preference", {
    contractType,
    topic,
    preference,
  });
}

/** 全部起草偏好。 */
export function listContractPreferences(): Promise<ContractPreference[]> {
  return invoke<ContractPreference[]>("list_contract_preferences");
}

/** 删除一条起草偏好。 */
export function deleteContractPreference(id: string): Promise<number> {
  return invoke<number>("delete_contract_preference", { id });
}

export function listCaseLogs(caseId: string): Promise<CaseLog[]> {
  return invoke<CaseLog[]>("list_case_logs", { caseId });
}

export function createCaseLog(
  caseId: string,
  occurredAt: string,
  rawInput: string,
  organizedMarkdown: string | null,
): Promise<CaseLog> {
  return invoke<CaseLog>("create_case_log", {
    caseId,
    occurredAt,
    rawInput,
    organizedMarkdown,
  });
}

export function generateCaseWorkReport(caseId: string): Promise<string> {
  return invoke<string>("generate_case_work_report", { caseId });
}

export function exportCaseWorkReportDocx(
  caseId: string,
  savePath: string,
  contentMd: string | null,
): Promise<string> {
  return invoke<string>("export_case_work_report_docx", {
    caseId,
    savePath,
    contentMd,
  });
}

export function generateClosingMaterials(caseId: string): Promise<Document> {
  return invoke<Document>("generate_closing_materials", { caseId });
}

export function organizeCaseLog(caseId: string, rawInput: string): Promise<string> {
  return invoke<string>("organize_case_log", { caseId, rawInput });
}

/* ------------------------------------------------------------------ */
/* 要素式文书                                                        */
/* ------------------------------------------------------------------ */

export function listElementDocumentTypes(): Promise<ElementDocumentType[]> {
  return invoke<ElementDocumentType[]>("list_element_document_types");
}

export function generateElementDocument(
  sourcePath: string,
  extractedTextPath: string | null,
  templateId: string,
): Promise<ElementDraft> {
  return invoke<ElementDraft>("generate_element_document", {
    sourcePath,
    extractedTextPath,
    templateId,
  });
}

export function saveElementDocument(
  caseId: string,
  templateId: string,
  title: string,
  fields: ElementDraft["fields"],
): Promise<SavedElementDocument> {
  return invoke<SavedElementDocument>("save_element_document", {
    caseId,
    templateId,
    title,
    fields,
  });
}

export function exportElementDocument(
  templateId: string,
  title: string,
  fields: ElementDraft["fields"],
  savePath: string,
): Promise<string> {
  return invoke<string>("export_element_document", { templateId, title, fields, savePath });
}

export function saveExternalElementDocument(
  caseId: string,
  filename: string,
  dataBase64: string,
): Promise<SavedElementDocument> {
  return invoke<SavedElementDocument>("save_external_element_document", {
    caseId,
    filename,
    dataBase64,
  });
}

/** 工具页(无案件):把外部转换的 base64 docx 写到用户选择的路径(由 Rust 写,绕过前端 fs scope)。 */
export function saveElementDocxToPath(savePath: string, dataBase64: string): Promise<string> {
  return invoke<string>("save_element_docx_to_path", { savePath, dataBase64 });
}

import { invoke } from "@tauri-apps/api/core";
import type { Bookmark, DocumentTag, ExtractedFields, SearchHit } from "../types";

/* ------------------------------------------------------------------ */
/* 文件读取                                                            */
/* ------------------------------------------------------------------ */

/** 读一个文本文件(.md/.html/.txt)的全文。仅限 5MB 以内。 */
export function readTextFile(path: string): Promise<string> {
  return invoke<string>("read_text_file", { path });
}

type BinaryPayload = number[] | ArrayBuffer | Uint8Array;

/** 读取案件预览需要的二进制文件。后端按案件源目录/转换缓存做范围校验。 */
export async function readCaseFileBytes(path: string): Promise<Uint8Array> {
  const bytes = await invoke<BinaryPayload>("read_case_file_bytes", { path });
  if (bytes instanceof Uint8Array) return bytes;
  if (bytes instanceof ArrayBuffer) return new Uint8Array(bytes);
  return Uint8Array.from(bytes);
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

/** 用系统默认浏览器打开 URL(Settings 里 token 申请链接、外链等)。 */
export function openUrl(url: string): Promise<void> {
  return invoke<void>("open_url", { url });
}

/** 在 Finder 中显示该路径(选中并打开父目录)。 */
export function revealInFinder(path: string): Promise<void> {
  return invoke<void>("reveal_in_finder", { path });
}

/* ---- 源文件看板 Phase 3:文档标记 ---- */

/** 列出某案件全部文档的标记(重要/忽略 + 原告/被告/第三人)。 */
export function listDocumentTags(caseId: string): Promise<DocumentTag[]> {
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
  documentIds: string | string[],
  value: string | null,
): Promise<void> {
  const ids = Array.isArray(documentIds) ? documentIds : [documentIds];
  return invoke("set_document_category", { documentIds: ids, value });
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

/** AI 自动整理:一次 LLM 调用判整案材料的 重要度+归类+显示名,写 ai_suggest。返回写入数。 */
export function aiOrganizeCase(
  caseId: string,
  renameFiles = true,
): Promise<number> {
  return invoke("ai_organize_case", { caseId, renameFiles });
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
): Promise<SearchHit[]> {
  return invoke("search_in_document", { documentId, query });
}

/** 列某文档的 PDF 页码书签(按页升序)。 */
export function listDocumentBookmarks(documentId: string): Promise<Bookmark[]> {
  return invoke("list_document_bookmarks", { documentId });
}

/** 加一个 PDF 页码书签(page 1-based,label 可空)。返回新书签。 */
export function addDocumentBookmark(
  documentId: string,
  page: number,
  label: string | null,
): Promise<Bookmark> {
  return invoke("add_document_bookmark", { documentId, page, label });
}

/** 删一个 PDF 页码书签。 */
export function deleteDocumentBookmark(id: string): Promise<void> {
  return invoke("delete_document_bookmark", { id });
}

/**
 * 把案件源文件夹加进 asset 协议 scope(运行期、按案件授权),
 * 让源文件查看器能用流式 `asset://` 协议在 iframe 里原生渲染该案 PDF。
 * **打开查看器前必须 await 本调用**,否则 iframe 首次请求会 403(scope 未就绪)。
 */
export function allowCaseAssets(folder: string): Promise<void> {
  return invoke<void>("allow_case_assets", { folder });
}

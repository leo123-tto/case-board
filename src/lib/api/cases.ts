import { invoke } from "@tauri-apps/api/core";
import type { Case, CaseWithDocs, ImportPlan, ImportResult, ScannedDoc } from "../types";

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

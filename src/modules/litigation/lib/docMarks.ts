/**
 * 源文件看板 Phase 3:把扁平的 `DocumentTag[]` 聚合成「每个文档的标记」。
 */
import type { DocumentTag } from "@/lib/types";

export type Importance = "重要" | "忽略";
export type TagSource = "user" | "ai_suggest";

/** 自动归类的固定分类(对齐后端 db::document_tags::CATEGORIES)。 */
export const CATEGORIES = [
  "起诉材料",
  "证据",
  "法院文书",
  "对方材料",
  "程序文书",
  "参考材料",
  "其他",
] as const;
export const UNCATEGORIZED = "未分类";

export const EVIDENCE_ATTITUDES = ["有利", "不利", "中性"] as const;
export type EvidenceAttitude = (typeof EVIDENCE_ATTITUDES)[number];

export const SUBMISSION_STAGES = [
  "起诉/答辩随附",
  "举证期限内",
  "补充提交",
  "二审新证据",
  "未提交或待确认",
] as const;
export type SubmissionStage = (typeof SUBMISSION_STAGES)[number];

export interface DocMark {
  /** 单值:重要 / 忽略 / null(普通) */
  importance: Importance | null;
  importanceSource: TagSource | null;
  /** 单值分类(六类之一)/ null(未分类) */
  category: string | null;
  categorySource: TagSource | null;
  evidenceAttitude: EvidenceAttitude | null;
  evidenceAttitudeSource: TagSource | null;
  submissionStage: SubmissionStage | null;
  submissionStageSource: TagSource | null;
  /** 可多值:原告 / 被告 / 第三人 */
  parties: string[];
}

export type DocMarkMap = Map<string, DocMark>;

export const EMPTY_MARK: DocMark = {
  importance: null,
  importanceSource: null,
  category: null,
  categorySource: null,
  evidenceAttitude: null,
  evidenceAttitudeSource: null,
  submissionStage: null,
  submissionStageSource: null,
  parties: [],
};

/** 排序权重:重要(0)置顶 < 普通(1) < 忽略(2)沉底。同级保持原顺序(稳定排序)。 */
export function importanceRank(mark: DocMark | undefined): number {
  if (mark?.importance === "重要") return 0;
  if (mark?.importance === "忽略") return 2;
  return 1;
}

/** 按重要度稳定排序一批文档(重要置顶、忽略沉底)。 */
export function sortByImportance<T extends { id: string }>(
  docs: T[],
  markMap: DocMarkMap,
): T[] {
  return [...docs].sort(
    (a, b) => importanceRank(markMap.get(a.id)) - importanceRank(markMap.get(b.id)),
  );
}

export function buildMarkMap(tags: DocumentTag[]): DocMarkMap {
  const m: DocMarkMap = new Map();
  const ensure = (id: string): DocMark => {
    let e = m.get(id);
    if (!e) {
      e = { ...EMPTY_MARK, parties: [] };
      m.set(id, e);
    }
    return e;
  };
  for (const t of tags) {
    const e = ensure(t.document_id);
    const src = t.source === "ai_suggest" ? "ai_suggest" : "user";
    if (t.namespace === "importance") {
      e.importance = t.value as Importance;
      e.importanceSource = src;
    } else if (t.namespace === "category") {
      e.category = t.value;
      e.categorySource = src;
    } else if (t.namespace === "evidence_attitude") {
      e.evidenceAttitude = t.value as EvidenceAttitude;
      e.evidenceAttitudeSource = src;
    } else if (t.namespace === "submission_stage") {
      e.submissionStage = t.value as SubmissionStage;
      e.submissionStageSource = src;
    } else if (t.namespace === "party_side") {
      e.parties.push(t.value);
    }
  }
  return m;
}

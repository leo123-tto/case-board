/**
 * 案件工作流状态机(2026-05-24 e · 作者拍板;2026-06-11 审级模型加 仲裁中/再审中 → 11 档)。
 *
 * 11 档枚举 + 颜色配置 + 从已有数据(documents.category / key_dates)的自动推断。
 *
 * 用户在卡片右上角下拉可手工覆盖 → 写入 `cases.workflow_status`,优先用用户值。
 * 注意:StatusId 必须与后端 `ingest/global_pipeline.rs::workflow_status_zh_to_en` 严格对齐。
 */

import type { Case, Document } from "@/lib/types";
import { todayIsoLocal } from "@/lib/date";

/** 工作流状态 ID(对应数据库 cases.workflow_status 字段值) */
export type StatusId =
  | "intake" // 接案
  | "filing" // 立案中
  | "arbitration" // 仲裁中(2026-06-11 审级模型加:劳动仲裁等,诉讼前置程序)
  | "awaiting_hearing" // 待开庭
  | "trial" // 审理中
  | "mediated" // 已调解(调解书无上诉期,直接生效)
  | "appeal_window" // 上诉期(判决书 15 天上诉期内)
  | "appeal" // 二审中
  | "retrial" // 再审中(2026-06-11 审级模型加)
  | "execution" // 执行中
  | "closed"; // 已结案

export interface StatusDef {
  id: StatusId;
  label: string;
  /** Tailwind 颜色类(bg + text),给卡片右上角 chip 用 */
  color: string;
  /** 在卡片列表中的排序权重(已结案排末尾,其他按业务顺序) */
  order: number;
}

/** 11 档状态完整定义 — UI 渲染 + 排序都从这里读 */
export const STATUS_DEFS: Record<StatusId, StatusDef> = {
  intake: {
    id: "intake",
    label: "接案",
    color: "bg-slate-100 text-slate-700",
    order: 1,
  },
  filing: {
    id: "filing",
    label: "立案中",
    color: "bg-blue-100 text-blue-800",
    order: 2,
  },
  arbitration: {
    id: "arbitration",
    label: "仲裁中",
    // 青色:仲裁 = 诉讼前置的独立程序,与法院档位的蓝/琥珀区分
    color: "bg-cyan-100 text-cyan-800",
    order: 2,
  },
  awaiting_hearing: {
    id: "awaiting_hearing",
    label: "待开庭",
    color: "bg-amber-100 text-amber-800",
    order: 3,
  },
  trial: {
    id: "trial",
    label: "审理中",
    color: "bg-amber-100 text-amber-800",
    order: 4,
  },
  mediated: {
    id: "mediated",
    label: "已调解",
    // 青绿:调解 = 平和解决,跟"上诉期 / 二审中"的紫色冲突区分
    color: "bg-teal-100 text-teal-800",
    order: 5,
  },
  appeal_window: {
    id: "appeal_window",
    label: "上诉期",
    color: "bg-violet-100 text-violet-800",
    order: 5,
  },
  appeal: {
    id: "appeal",
    label: "二审中",
    color: "bg-violet-100 text-violet-800",
    order: 6,
  },
  retrial: {
    id: "retrial",
    label: "再审中",
    // 紫红:再审 = 比二审更后段的非常程序
    color: "bg-fuchsia-100 text-fuchsia-800",
    order: 6,
  },
  execution: {
    id: "execution",
    label: "执行中",
    color: "bg-emerald-100 text-emerald-800",
    order: 7,
  },
  closed: {
    id: "closed",
    label: "已结案",
    color: "bg-muted text-muted-foreground",
    order: 99, // 排末尾
  },
};

/** 状态选项数组(给下拉菜单遍历用,按 order 排) */
export const STATUS_LIST: StatusDef[] = Object.values(STATUS_DEFS).sort(
  (a, b) => a.order - b.order,
);

/**
 * 从 case + documents + 聚合 key_dates 推断当前应该处于哪个状态。
 *
 * 规则(从晚到早匹配,命中即返,优先级:执行 > 二审 > 上诉期 > 审理中 > 待开庭 > 立案中 > 接案):
 *   1. 有执行类文档(限消令/失信被执行人/执行申请/终本裁定) → 执行中
 *   2. 有上诉状文档 / key_date(上诉/二审开庭/二审判决) → 二审中
 *   3a. 有调解书文档 → 已调解(调解书无上诉期,直接生效;优先级高于判决书)
 *   3b. 有判决书文档 → 上诉期(判决书 15 天上诉期内)
 *   4. 有 key_date 开庭日期 ≤ 今天 / 有庭审笔录文档 → 审理中
 *   5. 有受理通知 / 传票 / 开庭通知文档 → 待开庭
 *   6. 有起诉状文档 → 立案中
 *   7. 有委托合同文档 → 接案
 *   8. 默认 → 接案
 *
 * 不参与「已结案」的判断 — 已结案是作者手工标记的事,自动推断永远不会归到 closed。
 */
export function inferCaseStatus(
  _caseData: Case,
  documents: Document[],
  keyDates: Array<{ event_type: string; date: string | null }> = [],
): StatusId {
  const hasCategory = (cats: string[]) =>
    documents.some((d) => !d.deleted_at && d.category && cats.includes(d.category));
  const hasKeyDate = (types: string[]) =>
    keyDates.some((k) => types.includes(k.event_type));

  // 1. 执行类
  if (
    hasCategory([
      "执行申请",
      "限制消费令",
      "失信被执行人",
      "终本裁定",
      "执行通知",
    ]) ||
    hasKeyDate(["执行立案"])
  ) {
    return "execution";
  }

  // 2. 再审类(比二审更后段,优先匹配;2026-06-11 审级模型加)
  if (
    hasCategory(["再审申请书", "再审决定书", "再审判决书"]) ||
    hasKeyDate(["再审", "再审开庭", "再审判决"])
  ) {
    return "retrial";
  }

  // 3. 二审类
  if (
    hasCategory(["上诉状"]) ||
    hasKeyDate(["上诉", "二审开庭", "二审判决"])
  ) {
    return "appeal";
  }

  // 3a. 调解书优先 — 调解书无上诉期,出书即生效。
  //     如果同时有判决书 + 调解书,以调解书为准(实务上很少同时出,但有也是调解优先)
  if (hasCategory(["调解书"]) || hasKeyDate(["调解"])) {
    return "mediated";
  }

  // 3b. 判决书 → 上诉期(15 天上诉期内,默认归这;作者要进执行就手工切到执行中)
  if (hasCategory(["判决书"]) || hasKeyDate(["判决"])) {
    return "appeal_window";
  }

  // 4. 开庭已发生(最晚开庭 ≤ 今天) / 有庭审笔录
  const today = todayIsoLocal();
  const hadHearing = keyDates.some(
    (k) => k.event_type === "开庭" && k.date && k.date <= today,
  );
  if (hadHearing || hasCategory(["庭审笔录"])) {
    return "trial";
  }

  // 5. 受理 / 传票 → 待开庭
  if (
    hasCategory(["受理通知", "应诉通知", "传票", "开庭通知"]) ||
    hasKeyDate(["正式立案", "开庭"]) // "开庭" 但日期 > today,在 trial 之前已分流过
  ) {
    return "awaiting_hearing";
  }

  // 6. 已出起诉状
  if (
    hasCategory(["起诉状", "民事起诉状", "财产保全", "财产保全申请"]) ||
    hasKeyDate(["申请立案", "保全"])
  ) {
    return "filing";
  }

  // 7. 仲裁类(2026-06-11 审级模型加)——
  //    放在法院各档之后:案件一旦出现起诉状/受理通知等法院文书,说明已进入诉讼,
  //    上面的规则会先命中;只有纯仲裁材料的案件才落到这里。
  if (
    hasCategory(["仲裁申请书", "仲裁裁决书", "仲裁受理通知", "仲裁开庭通知", "仲裁答辩状"]) ||
    hasKeyDate(["仲裁立案", "仲裁开庭", "仲裁裁决"])
  ) {
    return "arbitration";
  }

  // 8. 委托合同(接案)
  if (
    hasCategory(["委托合同", "代理合同", "律师代理合同"]) ||
    hasKeyDate(["接案"])
  ) {
    return "intake";
  }

  // 9. 默认
  return "intake";
}

/**
 * 取案件应展示的状态:用户手工选过的优先,否则自动推断。
 */
export function resolveCaseStatus(
  caseData: Case,
  documents: Document[],
  keyDates: Array<{ event_type: string; date: string | null }> = [],
): StatusDef {
  const manual = caseData.workflow_status as StatusId | null;
  if (caseData.workflow_status_locked === 1 && manual && manual in STATUS_DEFS) {
    return STATUS_DEFS[manual];
  }
  const effectiveKeyDates =
    keyDates.length > 0 ? keyDates : readAggKeyDates(caseData.agg_key_dates);
  const inferred = inferCaseStatus(caseData, documents, effectiveKeyDates);

  // LLM 写入的 workflow_status 是抽取当日的一次性判断,不能像用户手工锁定一样
  // 永久压过日期推断。只有自动推断完全没有线索时,才把它当兜底显示值。
  if (inferred === "intake" && manual && manual in STATUS_DEFS) {
    return STATUS_DEFS[manual];
  }
  return STATUS_DEFS[inferred];
}

function readAggKeyDates(
  json: string | null,
): Array<{ event_type: string; date: string | null }> {
  if (!json) return [];
  try {
    const parsed = JSON.parse(json);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .map((d) => ({
        event_type:
          typeof d?.event_type === "string"
            ? d.event_type
            : typeof d?.event === "string"
              ? d.event
              : "",
        date: typeof d?.date === "string" ? d.date : null,
      }))
      .filter((d) => d.event_type);
  } catch {
    return [];
  }
}

/** 列表排序(作者 2026-05-24 e):
 *   1. 在办状态(其他 7 档)— 按 updated_at 倒序
 *   2. 已调解 — 排在在办后面(诉讼阶段已结束,等待执行或收尾)
 *   3. 已结案 — 排在最末尾(dim 灰)
 *
 * 同档内按 updated_at 倒序。
 */
function statusBucket(s: StatusId): number {
  if (s === "closed") return 2;
  if (s === "mediated") return 1;
  return 0;
}

export function compareCasesByStatusThenTime(
  aStatus: StatusId,
  aUpdated: string,
  bStatus: StatusId,
  bUpdated: string,
): number {
  const aBucket = statusBucket(aStatus);
  const bBucket = statusBucket(bStatus);
  if (aBucket !== bBucket) return aBucket - bBucket;
  return bUpdated.localeCompare(aUpdated);
}

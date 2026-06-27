/**
 * 案件画像 snapshot — 详情页直接渲染用。
 *
 * 2026-05-24 j-6 · 大砍重写:从「规则聚合 documents.extracted_fields」改成
 * 「直接读 LLM 全局抽出的 cases.agg_* JSON 字段」。
 *
 * 为什么:
 *   - LLM 全局抽方案(2026-05-24 h-3)已经把 party_contacts / court_contacts /
 *     key_dates / fees / resolution 直接以 JSON 写入 cases 表
 *   - 跨文档关联 / 反诉去污 / 去重 全由 LLM prompt 内部处理(比规则准)
 *   - 旧 caseSnapshot 553 行规则代码(PARTY_AUTHORITY_CATEGORIES /
 *     isCounterclaimDoc / FEE_ITEM_PRIORITY / KEY_DATE_WHITELIST 等)全删
 *
 * 当前文件 ~120 行,只做:
 *   - 安全 JSON.parse(LLM 偶尔会输出非数组,parse 失败兜底 [])
 *   - 适配 LLM 输出字段名到 UI 用的 TS 类型(date/event → date/event_type 等)
 *   - 一层防御性 dedup(LLM prompt 已经要求去重,但加一道保险)
 */

import {
  type Case,
  type CourtContact,
  type Document,
  type FeeRecord,
  type KeyDate,
  type PartyContact,
  type Preservation,
  parseJsonArray,
} from "./types";
import { todayIsoLocal } from "./date";

/** 案件 snapshot(详情页直接渲染) */
export interface CaseSnapshot {
  /** 用了多少份文档算出来的(0 = 还没数据) */
  basedOnDocs: number;
  /** 是否后端全局抽已经跑完(用户可以信赖) */
  computedAt: string | null;

  // 案件基本
  case_no: string | null;
  case_type: string | null;
  court: string | null;
  cause: string | null;
  case_stage: string | null;
  case_status: string | null;
  filed_at: string | null;
  expected_close_at: string | null;
  case_note: string | null;
  claim_amount: number | null;

  // 当事人
  plaintiffs: string[];
  defendants: string[];
  third_parties: string[];
  judges: string[];

  // 联系人 / 子表
  court_contacts: CourtContact[];
  party_contacts: PartyContact[];
  fees: FeeRecord[];
  key_dates: KeyDate[];
  preservations: Preservation[];

  // 2026-05-24 i 加(LLM 全局抽多出来的字段,直接给 UI 用)
  summary: string | null;
  resolution: string | null;
  status_text: string | null;

  // 2026-06-13:我方代理立场(原告方/被告方/第三人/反诉混合/null)。
  our_side: string | null;
}

/**
 * 把 cases.agg_* JSON 字段 parse + 适配成 UI 用的 CaseSnapshot。
 *
 * documents 参数保留只为兼容旧签名,实际不读(LLM 已经看过所有文档了)。
 */
export function computeCaseSnapshot(
  caseData: Case,
  documents: Document[],
): CaseSnapshot {
  const statusText = normalizeCaseStatusText(caseData.agg_status_text);
  return {
    basedOnDocs: documents.length,
    computedAt: caseData.agg_computed_at,

    case_no: caseData.agg_case_no,
    case_type: caseData.case_type ?? null,
    court: caseData.agg_court,
    cause: caseData.agg_cause,
    case_stage: null,
    case_status: statusText,
    filed_at: caseData.agg_filed_at,
    expected_close_at: null,
    case_note: null,
    claim_amount: caseData.agg_claim_amount,

    plaintiffs: parseJsonArray(caseData.agg_plaintiffs),
    defendants: parseJsonArray(caseData.agg_defendants),
    third_parties: parseJsonArray(caseData.agg_third_parties),
    judges: parseJsonArray(caseData.agg_judges),

    court_contacts: adaptCourtContacts(caseData.agg_court_contacts),
    party_contacts: adaptPartyContacts(caseData.agg_party_contacts),
    fees: adaptFees(caseData.agg_fees),
    key_dates: adaptKeyDates(caseData.agg_key_dates),
    preservations: [], // V0.1 暂没在 LLM schema 单独要,放在 key_dates 里

    summary: caseData.case_summary,
    resolution: caseData.agg_resolution,
    status_text: statusText,

    our_side: caseData.agg_our_side,
  };
}

export function normalizeCaseStatusText(text: string | null): string | null {
  if (!text) return null;
  const match = text.match(
    /(?:将于\s*)?\d{4}-\d{2}-\d{2}\s*(?:待开庭|已开庭|开庭)/g,
  );
  if (!match) return text;
  let next = text;
  for (const hit of match) {
    const date = hit.match(/\d{4}-\d{2}-\d{2}/)?.[0];
    if (!date) continue;
    next = next.replace(hit, hearingStatusText(date));
  }
  return next;
}

function hearingStatusText(isoDate: string): string {
  const todayKey = todayIsoLocal();
  if (isoDate > todayKey) return `将于 ${isoDate} 开庭`;
  if (isoDate === todayKey) return "今日开庭";
  return `已于 ${isoDate} 开庭`;
}

/* ============ adapter:LLM JSON → UI TS 类型 ============ */

function safeParseArray<T>(json: string | null): T[] {
  if (!json) return [];
  try {
    const parsed = JSON.parse(json);
    return Array.isArray(parsed) ? (parsed as T[]) : [];
  } catch {
    return [];
  }
}

interface LlmCourtContact {
  name?: string;
  role?: string | null;
  phone?: string | null;
}

function adaptCourtContacts(json: string | null): CourtContact[] {
  const arr = safeParseArray<LlmCourtContact>(json);
  return dedupBy(arr, (c) => `${c.name ?? ""}|${c.role ?? ""}`)
    .filter((c) => c.name)
    .map((c) => ({
      name: c.name as string,
      role: c.role ?? null,
      phone: c.phone ?? null,
    }));
}

interface LlmPartyContact {
  name?: string | null;
  role?: string | null;
  id_no?: string | null;
  address?: string | null;
  phone?: string | null;
  is_our_side?: boolean | null;
  /** 2026-05-26 V0.1.12:同人跨文档其它身份 */
  aliases?: string[] | null;
}

function adaptPartyContacts(json: string | null): PartyContact[] {
  const arr = safeParseArray<LlmPartyContact>(json);
  return dedupBy(arr, (p) => `${p.name ?? ""}|${p.role ?? ""}`)
    .filter((p) => p.name)
    .map((p) => ({
      party: p.role ?? "", // UI 用 party 字段当列表分组,放 LLM 给的 role
      name: p.name as string,
      role: p.role ?? null,
      phone: p.phone ?? null,
      email: null,
      is_our_side: p.is_our_side ?? null,
      aliases: Array.isArray(p.aliases) ? p.aliases : undefined,
    }));
}

interface LlmFee {
  item?: string;
  amount?: number | string | null;
  note?: string | null;
}

function adaptFees(json: string | null): FeeRecord[] {
  const arr = safeParseArray<LlmFee>(json);
  return dedupBy(arr, (f) => `${f.item ?? ""}|${f.amount ?? ""}`)
    .filter((f) => f.item)
    .map((f) => ({
      item: f.item as string,
      amount:
        typeof f.amount === "string"
          ? Number.isFinite(parseFloat(f.amount))
            ? parseFloat(f.amount)
            : null
          : typeof f.amount === "number"
            ? f.amount
            : null,
      charged_at: null,
      receipt_no: null,
      note: f.note ?? null,
    }));
}

interface LlmKeyDate {
  date?: string;
  event?: string;
  note?: string | null;
  expires_at?: string | null;
}

function adaptKeyDates(json: string | null): KeyDate[] {
  const arr = safeParseArray<LlmKeyDate>(json);
  // 2026-05-24 j-6 作者要求:**没日期的节点不显示**,只渲染真实抽到的
  return dedupBy(arr, (d) => `${d.date ?? ""}|${d.event ?? ""}`)
    .filter((d) => d.date && d.event)
    .map((d) => ({
      event_type: d.event as string,
      date: d.date as string,
      note: d.note ?? null,
      expires_at: d.expires_at ?? null,
    }));
}

/** 通用 dedup:按 keyFn 输出的字符串去重,保留首次出现的项 */
function dedupBy<T>(arr: T[], keyFn: (x: T) => string): T[] {
  const seen = new Set<string>();
  const out: T[] = [];
  for (const item of arr) {
    const k = keyFn(item);
    if (seen.has(k)) continue;
    seen.add(k);
    out.push(item);
  }
  return out;
}

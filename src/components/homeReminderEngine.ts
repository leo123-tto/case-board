import type {
  Case,
  CourtContact,
  Document,
  ExtractedFields,
  Preservation,
} from "@/lib/types";
import { parseJsonArray } from "@/lib/types";
import type {
  PreservationSchedule,
  PreservationTextInfo,
} from "./homePreservationEvents";
import { addYears } from "./homePreservationEvents";

export type HomeReminderKind = "hearing" | "deadline" | "todo" | "manual";

export interface HomeReminderEvent {
  kind: HomeReminderKind;
  date: string;
  daysFromNow: number;
  type: string;
  note?: string | null;
  timeText?: string | null;
  locationText?: string | null;
  caseName: string;
  caseId: string;
  court?: string | null;
  judges?: string[];
  courtContacts?: CourtContact[];
  sourceDoc?: Document;
  id?: string;
}

export const PRESERVATION_RE = /保全|续封|查封|冻结|续冻/;

const PRESERVATION_REMIND_DAYS = 60;

interface BuildOptions {
  onlyReminderWindow: boolean;
}

interface HearingCandidate {
  event: string;
  date: string;
  note: string | null;
  sourceDoc?: Document;
}

interface PreservationCandidate {
  type: string;
  startedAt: string;
  expiresAt: string;
  durationYears: number | null;
  targetLabel: string | null;
  sourceDoc?: Document;
}

interface UnsealCandidate {
  date: string;
  sourceDoc?: Document;
}

export function buildImportantCaseReminders(
  cases: Case[],
  docsByCase: Record<string, Document[]>,
  textInfoByDoc: Record<string, PreservationTextInfo>,
  now = todayDate(),
): HomeReminderEvent[] {
  const events = cases.flatMap((c) =>
    buildCaseReminderEvents(c, docsByCase[c.id] ?? [], textInfoByDoc, now, {
      onlyReminderWindow: true,
    }),
  );
  const rank = { overdue: 0, urgent: 1, normal: 2 } as const;
  return events
    .sort((a, b) => {
      const ra = rank[eventUrgency(a)];
      const rb = rank[eventUrgency(b)];
      if (ra !== rb) return ra - rb;
      if (a.daysFromNow !== b.daysFromNow) return a.daysFromNow - b.daysFromNow;
      if (a.kind !== b.kind) return a.kind === "hearing" ? -1 : 1;
      return 0;
    })
    .slice(0, 12);
}

export function buildCaseCalendarEvents(
  cases: Case[],
  docsByCase: Record<string, Document[]>,
  textInfoByDoc: Record<string, PreservationTextInfo>,
  now = todayDate(),
): HomeReminderEvent[] {
  const events = cases.flatMap((c) =>
    buildCaseReminderEvents(c, docsByCase[c.id] ?? [], textInfoByDoc, now, {
      onlyReminderWindow: false,
    }),
  );
  return events.sort((a, b) => a.date.localeCompare(b.date));
}

function buildCaseReminderEvents(
  c: Case,
  docs: Document[],
  textInfoByDoc: Record<string, PreservationTextInfo>,
  now: Date,
  options: BuildOptions,
): HomeReminderEvent[] {
  const caseName = c.agg_cause || c.name;
  const judges = parseJsonArray(c.agg_judges);
  const courtContacts = parseCourtContacts(c.agg_court_contacts);
  return [
    ...resolveHearings(c, docs, now, options).map((item) => ({
      kind: "hearing" as const,
      date: item.date,
      daysFromNow: diffDays(parseDate(item.date)!, now),
      type: item.event,
      note: item.note,
      caseName,
      caseId: c.id,
      court: c.agg_court,
      judges,
      courtContacts,
      sourceDoc: item.sourceDoc,
    })),
    ...resolvePreservations(c, docs, textInfoByDoc, now, options).map((item) => ({
      kind: "deadline" as const,
      date: item.expiresAt,
      daysFromNow: diffDays(parseDate(item.expiresAt)!, now),
      type: item.type,
      note: item.targetLabel,
      caseName,
      caseId: c.id,
      court: c.agg_court,
      judges,
      courtContacts,
      sourceDoc: item.sourceDoc,
    })),
    ...resolveNonPreservationDeadlines(c, now, options).map((kd) => ({
      kind: "deadline" as const,
      date: kd.expires_at!,
      daysFromNow: diffDays(parseDate(kd.expires_at!)!, now),
      type: kd.event ?? "到期",
      note: kd.note ?? null,
      caseName,
      caseId: c.id,
      court: c.agg_court,
      judges,
      courtContacts,
    })),
  ];
}

function resolveHearings(
  c: Case,
  docs: Document[],
  now: Date,
  options: BuildOptions,
): HearingCandidate[] {
  const docCandidates = docs.flatMap((doc) =>
    (parseExtractedFields(doc.extracted_fields)?.key_dates ?? [])
      .filter((kd) => isHearingEvent(kd.event_type) && kd.date)
      .map((kd) => ({
        event: kd.event_type,
        date: kd.date!,
        note: kd.note ?? null,
        sourceDoc: doc,
      })),
  );
  const source = docCandidates.length > 0 ? docCandidates : readKeyDates(c)
    .filter((kd) => isHearingEvent(kd.event ?? "") && kd.date)
    .map((kd) => ({
      event: kd.event ?? "开庭",
      date: kd.date!,
      note: kd.note ?? null,
      sourceDoc: findHearingSourceDoc(docs, kd.date),
    }));

  const latestBySession = new Map<string, HearingCandidate>();
  for (const candidate of source) {
    const d = parseDate(candidate.date);
    if (!d) continue;
    const days = diffDays(d, now);
    if (options.onlyReminderWindow && (days < 0 || days > 365)) continue;
    const key = hearingSessionKey(candidate);
    const prev = latestBySession.get(key);
    if (!prev || hearingAuthorityScore(candidate) >= hearingAuthorityScore(prev)) {
      latestBySession.set(key, candidate);
    }
  }
  return [...latestBySession.values()];
}

function resolvePreservations(
  c: Case,
  docs: Document[],
  textInfoByDoc: Record<string, PreservationTextInfo>,
  now: Date,
  options: BuildOptions,
): PreservationCandidate[] {
  const unseals = collectUnsealCandidates(docs, textInfoByDoc);
  const candidates = collectPreservationCandidates(docs, textInfoByDoc);
  const fallback = candidates.length > 0 ? [] : collectPreservationFallback(c);

  return [...candidates, ...fallback].filter((candidate) => {
    const expires = parseDate(candidate.expiresAt);
    if (!expires) return false;
    const days = diffDays(expires, now);
    if (days < 0) return false;
    if (options.onlyReminderWindow && days > PRESERVATION_REMIND_DAYS) return false;
    return !isReleasedByLaterUnseal(candidate, unseals);
  });
}

function resolveNonPreservationDeadlines(
  c: Case,
  now: Date,
  options: BuildOptions,
): Array<{ event?: string | null; note?: string | null; expires_at?: string | null }> {
  return readKeyDates(c).filter((kd) => {
    if (!kd.expires_at || PRESERVATION_RE.test(kd.event ?? "")) return false;
    const d = parseDate(kd.expires_at);
    if (!d) return false;
    const days = diffDays(d, now);
    if (options.onlyReminderWindow) return days >= -30 && days <= 365;
    return true;
  });
}

function collectPreservationCandidates(
  docs: Document[],
  textInfoByDoc: Record<string, PreservationTextInfo>,
): PreservationCandidate[] {
  const out: PreservationCandidate[] = [];
  for (const doc of docs) {
    for (const schedule of textInfoByDoc[doc.id]?.schedules ?? []) {
      out.push(fromSchedule(schedule, doc));
    }
    const fields = parseExtractedFields(doc.extracted_fields);
    for (const p of fields?.preservations ?? []) {
      const candidate = fromExtractedPreservation(p, doc);
      if (candidate) out.push(candidate);
    }
  }
  return dedupePreservations(out);
}

function collectPreservationFallback(c: Case): PreservationCandidate[] {
  return readKeyDates(c)
    .filter((kd) => kd.expires_at && PRESERVATION_RE.test(kd.event ?? ""))
    .map((kd) => ({
      type: preservationTypeFromText(kd.event ?? "保全"),
      startedAt: kd.date ?? kd.expires_at!,
      expiresAt: kd.expires_at!,
      durationYears: null,
      targetLabel: kd.note ?? null,
    }));
}

function collectUnsealCandidates(
  docs: Document[],
  textInfoByDoc: Record<string, PreservationTextInfo>,
): UnsealCandidate[] {
  const out: UnsealCandidate[] = [];
  for (const doc of docs) {
    const textDate = textInfoByDoc[doc.id]?.unsealDate;
    if (textDate) out.push({ date: textDate, sourceDoc: doc });
    const fields = parseExtractedFields(doc.extracted_fields);
    for (const kd of fields?.key_dates ?? []) {
      if (isUnsealEvent(kd.event_type) && kd.date) out.push({ date: kd.date, sourceDoc: doc });
    }
  }
  return out;
}

function isReleasedByLaterUnseal(
  candidate: PreservationCandidate,
  unseals: UnsealCandidate[],
): boolean {
  return unseals.some((unseal) => unseal.date >= candidate.startedAt);
}

function fromSchedule(
  schedule: PreservationSchedule,
  sourceDoc: Document,
): PreservationCandidate {
  return {
    type: schedule.type,
    startedAt: schedule.startedAt,
    expiresAt: schedule.expiresAt,
    durationYears: schedule.durationYears,
    targetLabel: schedule.targetLabel,
    sourceDoc,
  };
}

function fromExtractedPreservation(
  p: Preservation,
  sourceDoc: Document,
): PreservationCandidate | null {
  const startedAt = p.started_at;
  const expiresAt =
    p.expires_at ?? (startedAt && p.duration_years ? addYears(startedAt, p.duration_years) : null);
  if (!startedAt || !expiresAt) return null;
  return {
    type: preservationTypeFromText(p.target),
    startedAt,
    expiresAt,
    durationYears: p.duration_years,
    targetLabel: preservationTargetLabel(p.target),
    sourceDoc,
  };
}

function dedupePreservations(items: PreservationCandidate[]): PreservationCandidate[] {
  const byKey = new Map<string, PreservationCandidate>();
  for (const item of items) {
    const key = `${item.expiresAt}|${item.type}|${item.targetLabel ?? ""}`;
    const prev = byKey.get(key);
    if (!prev || sourceDocTime(item.sourceDoc) >= sourceDocTime(prev.sourceDoc)) {
      byKey.set(key, item);
    }
  }
  return [...byKey.values()];
}

function preservationTypeFromText(value: string): string {
  if (/冻/.test(value) || /银行|账户|存款/.test(value)) return "续冻";
  return "续封";
}

function preservationTargetLabel(value: string | null | undefined): string | null {
  const text = value ?? "";
  if (/银行|账户|存款/.test(text)) return "银行账户";
  if (/车辆|车/.test(text)) return "车辆";
  if (/不动产|房产|房屋|土地/.test(text)) return "不动产";
  return text.trim() || null;
}

function hearingSessionKey(candidate: HearingCandidate): string {
  const text = `${candidate.event} ${candidate.note ?? ""}`;
  if (/第二次|二次|第2次/.test(text)) return "hearing-2";
  if (/第三次|三次|第3次/.test(text)) return "hearing-3";
  if (/庭前|询问|听证/.test(text)) return "pretrial";
  return "hearing-1";
}

function hearingAuthorityScore(candidate: HearingCandidate): number {
  return sourceDocTime(candidate.sourceDoc) || Date.parse(candidate.date) || 0;
}

function sourceDocTime(doc?: Document): number {
  const value = doc?.modified_at ?? doc?.created_at ?? "";
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function isHearingEvent(value: string): boolean {
  return /开庭|传票|询问|听证|庭前会议/.test(value);
}

function isUnsealEvent(value: string): boolean {
  return /解封|解除保全|解除查封|解除冻结|解除扣押|解除续封|解除续冻/.test(value);
}

export function isPreservationOrUnsealDoc(doc: Document): boolean {
  const hay = `${doc.category ?? ""} ${doc.filename}`;
  return /保全|续封|续冻|查封|冻结|解封|解除查封|解除冻结|解除保全|扣押/.test(hay);
}

function parseExtractedFields(json: string | null): ExtractedFields | null {
  if (!json) return null;
  try {
    return JSON.parse(json) as ExtractedFields;
  } catch {
    return null;
  }
}

function readKeyDates(c: Case): Array<{
  date?: string;
  event?: string;
  note?: string;
  expires_at?: string;
}> {
  if (!c.agg_key_dates) return [];
  try {
    const parsed = JSON.parse(c.agg_key_dates);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

function findHearingSourceDoc(docs: Document[], isoDate: string | null | undefined): Document | undefined {
  return docs
    .filter((doc) => /传票|开庭通知|开庭/.test(`${doc.category ?? ""} ${doc.filename}`))
    .filter((doc) => {
      const fields = parseExtractedFields(doc.extracted_fields);
      return !isoDate || fields?.key_dates?.some((kd) => kd.date === isoDate && isHearingEvent(kd.event_type));
    })
    .sort((a, b) => sourceDocTime(b) - sourceDocTime(a))[0];
}

export function parseCourtContacts(json: string | null): CourtContact[] {
  if (!json) return [];
  try {
    const parsed = JSON.parse(json) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed
      .map((item) => {
        if (!item || typeof item !== "object") return null;
        const row = item as Partial<CourtContact>;
        return {
          name: typeof row.name === "string" && row.name.trim() ? row.name.trim() : null,
          role: typeof row.role === "string" && row.role.trim() ? row.role.trim() : null,
          phone: typeof row.phone === "string" && row.phone.trim() ? row.phone.trim() : null,
        };
      })
      .filter((item): item is CourtContact => item !== null);
  } catch {
    return [];
  }
}

export function eventUrgency(e: HomeReminderEvent): "overdue" | "urgent" | "normal" {
  if (e.daysFromNow < 0) return "overdue";
  if (e.kind === "hearing") return e.daysFromNow <= 30 ? "urgent" : "normal";
  return e.daysFromNow <= 90 ? "urgent" : "normal";
}

export function parseDate(value: string): Date | null {
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return null;
  d.setHours(0, 0, 0, 0);
  return d;
}

export function todayDate(): Date {
  const now = new Date();
  now.setHours(0, 0, 0, 0);
  return now;
}

export function diffDays(a: Date, b: Date): number {
  return Math.round((a.getTime() - b.getTime()) / 86400000);
}

import type { Case, CaseInstance } from "./types";

const CASE_NO_RE = /[（(]\s*\d{4}\s*[）)]\s*[^\s，。；;、,]{1,40}?号/g;
const EXECUTION_MARK_RE = /执恢|执保|执异|执复|执监|执转破|恢执|执/;

export function isExecutionCaseNo(value: string | null | undefined): boolean {
  return EXECUTION_MARK_RE.test(normalizeCaseNo(value ?? ""));
}

export function extractExecutionCaseNoFromCase(c: Case): string | null {
  const keyDateHit = extractExecutionCaseNoFromKeyDates(c.agg_key_dates);
  if (keyDateHit) return keyDateHit;
  return firstExecutionCaseNo([
    readOverrideString(c.user_overrides_json, "agg_case_no") ?? c.agg_case_no,
    c.agg_status_text,
    c.agg_resolution,
    c.case_summary,
    c.name,
    c.case_no,
  ]);
}

export function getTrialCaseNo(
  c: Case,
  instances: CaseInstance[] = [],
  currentCaseNo?: string | null,
): string | null {
  const candidates = [
    currentCaseNo,
    readOverrideString(c.user_overrides_json, "agg_case_no") ?? c.agg_case_no,
    c.case_no,
    ...instances.map((item) => item.case_no),
  ];
  return candidates.map((value) => normalizeCaseNo(value ?? "")).find((value) => value && !isExecutionCaseNo(value)) ?? null;
}

export function normalizeCaseNo(value: string): string {
  return value
    .replace(/\s+/g, "")
    .replace(/（/g, "(")
    .replace(/）/g, ")");
}

function extractExecutionCaseNoFromKeyDates(json: string | null): string | null {
  if (!json) return null;
  try {
    const parsed = JSON.parse(json) as unknown;
    if (!Array.isArray(parsed)) return null;
    for (const item of parsed) {
      if (!item || typeof item !== "object") continue;
      const row = item as Record<string, unknown>;
      const text = [row.event, row.event_type, row.note]
        .filter((value): value is string => typeof value === "string")
        .join(" ");
      if (!/执行|执恢|执保|执/.test(text)) continue;
      const hit = firstExecutionCaseNo([text]);
      if (hit) return hit;
    }
  } catch {
    return null;
  }
  return null;
}

function firstExecutionCaseNo(values: Array<string | null | undefined>): string | null {
  for (const value of values) {
    const text = value ?? "";
    const matches = text.match(CASE_NO_RE) ?? [];
    for (const match of matches) {
      const normalized = normalizeCaseNo(match);
      if (isExecutionCaseNo(normalized)) return normalized;
    }
  }
  return null;
}

function readOverrideString(
  json: string | null,
  path: string,
): string | null {
  if (!json) return null;
  try {
    const parsed = JSON.parse(json) as { fields?: Record<string, unknown> };
    const value = parsed.fields?.[path];
    return typeof value === "string" && value.trim() ? value.trim() : null;
  } catch {
    return null;
  }
}

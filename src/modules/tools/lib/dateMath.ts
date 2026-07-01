/**
 * 日期算术 — 给天数计算器 / 利息计算器复用。
 *
 * 完整移植自法律工具箱 daycal.html 的日期处理逻辑。
 *
 * 全部用 `new Date(y, m-1, d)` 构造本地日期(避免 UTC 时区偏移导致跨日错)。
 */

const WEEKDAYS = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"] as const;

/** "2026-05-24" → "2026年05月24日" */
export function formatChineseDate(iso: string | null | undefined): string {
  if (!iso) return "";
  const [y, m, d] = iso.split("-");
  return `${y}年${m}月${d}日`;
}

/** "2026-05-24" → "周日"。invalid 返回空串 */
export function getWeekday(iso: string | null | undefined): string {
  if (!iso) return "";
  const [y, m, d] = iso.split("-").map(Number);
  if (!y || !m || !d) return "";
  return WEEKDAYS[new Date(y, m - 1, d).getDay()];
}

/** 今日 ISO "YYYY-MM-DD" */
export function todayIso(): string {
  const t = new Date();
  return toIso(t);
}

/** Date → "YYYY-MM-DD" */
export function toIso(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** 给 ISO 日期加减 days 天,返回新 ISO */
export function addDaysIso(iso: string, days: number): string {
  const [y, m, d] = iso.split("-").map(Number);
  const result = new Date(y, m - 1, d);
  result.setDate(result.getDate() + days);
  return toIso(result);
}

/** 两个 ISO 日期间的天数差(绝对值)+ 方向(end 在 start 之前 = true) */
export function diffDaysIso(startIso: string, endIso: string): {
  days: number;
  reversed: boolean;
} {
  const s = new Date(startIso);
  const e = new Date(endIso);
  const ms = e.getTime() - s.getTime();
  return {
    days: Math.round(Math.abs(ms) / (1000 * 60 * 60 * 24)),
    reversed: ms < 0,
  };
}

/** 月份差(按年月,不考虑日)— 绝对值 */
export function diffMonths(startIso: string, endIso: string): number {
  const [sy, sm] = startIso.split("-").map(Number);
  const [ey, em] = endIso.split("-").map(Number);
  return Math.abs((ey - sy) * 12 + (em - sm));
}

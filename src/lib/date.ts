/** 本地日历日期工具。避免 `toISOString().slice(0, 10)` 因 UTC 偏移跨日。 */

export function todayIsoLocal(): string {
  return dateToIsoLocal(new Date());
}

export function dateToIsoLocal(date: Date): string {
  return [
    date.getFullYear(),
    String(date.getMonth() + 1).padStart(2, "0"),
    String(date.getDate()).padStart(2, "0"),
  ].join("-");
}

export function parseIsoDateLocal(value: string): Date | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value.trim());
  if (!match) return null;
  const y = Number(match[1]);
  const m = Number(match[2]);
  const d = Number(match[3]);
  const date = new Date(y, m - 1, d);
  if (
    date.getFullYear() !== y ||
    date.getMonth() !== m - 1 ||
    date.getDate() !== d
  ) {
    return null;
  }
  return date;
}

export function daysBetweenIsoLocal(fromIso: string, toIso: string): number | null {
  const from = parseIsoDateLocal(fromIso);
  const to = parseIsoDateLocal(toIso);
  if (!from || !to) return null;
  const start = new Date(from.getFullYear(), from.getMonth(), from.getDate());
  const end = new Date(to.getFullYear(), to.getMonth(), to.getDate());
  return Math.round((end.getTime() - start.getTime()) / 86_400_000);
}

export function daysFromToday(isoDate: string): number | null {
  return daysBetweenIsoLocal(todayIsoLocal(), isoDate);
}

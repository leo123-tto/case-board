export type PreservationAssetKind = "bank" | "vehicle" | "equity" | "realEstate";

export interface PreservationSchedule {
  kind: PreservationAssetKind;
  type: string;
  targetLabel: string;
  startedAt: string;
  expiresAt: string;
  durationYears: number;
}

export interface PreservationTextInfo {
  schedules: PreservationSchedule[];
  unsealDate: string | null;
}

export function extractPreservationTextInfo(raw: string): PreservationTextInfo {
  return {
    schedules: extractPreservationSchedulesFromText(raw),
    unsealDate: extractUnsealDateFromText(raw),
  };
}

export function extractPreservationSchedulesFromText(raw: string): PreservationSchedule[] {
  const text = normalizeText(raw);
  const dateWindow = extractPreservationDateWindow(text);
  const startedAt = dateWindow?.startedAt ?? extractPreservationStartDate(text);
  if (!startedAt) return [];

  const specs: Array<{
    kind: PreservationAssetKind;
    type: string;
    targetLabel: string;
    durationYears: number;
    matches: RegExp[];
  }> = [
    {
      kind: "bank",
      type: "续冻",
      targetLabel: "银行账户",
      durationYears: 1,
      matches: [/银行(?:账户|存款).*?(?:冻结)?期限为?一?年/, /冻结(?:期限)?为?一?年/],
    },
    {
      kind: "vehicle",
      type: "续封",
      targetLabel: "车辆",
      durationYears: 2,
      matches: [/车辆.*?(?:查封)?期限为?二年/, /车辆.*?(?:查封)?期限为?2年/],
    },
    {
      kind: "equity",
      type: "续冻",
      targetLabel: "股权",
      durationYears: 3,
      matches: [/股权.*?(?:冻结)?期限为?三年/, /股权.*?(?:冻结)?期限为?3年/],
    },
    {
      kind: "realEstate",
      type: "续封",
      targetLabel: "不动产",
      durationYears: 3,
      matches: [/不动产.*?(?:查封)?期限为?三年/, /不动产.*?(?:查封)?期限为?3年/],
    },
  ];

  return specs
    .filter((spec) => spec.matches.some((regex) => regex.test(text)))
    .map((spec) => ({
      kind: spec.kind,
      type: spec.type,
      targetLabel: spec.targetLabel,
      startedAt,
      durationYears: spec.durationYears,
      expiresAt: dateWindow?.expiresAt ?? addYears(startedAt, spec.durationYears),
    }));
}

export function extractUnsealDateFromText(raw: string): string | null {
  const text = normalizeText(raw);
  if (!/(解封|解除.*?(?:保全|查封|冻结|扣押|续封|续冻))/.test(text)) {
    return null;
  }
  return extractPreservationStartDate(text);
}

export function addYears(isoDate: string, years: number): string {
  const [year, month, day] = isoDate.split("-").map(Number);
  if (!year || !month || !day) return isoDate;
  return `${year + years}-${String(month).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
}

function extractPreservationDateWindow(
  text: string,
): { startedAt: string; expiresAt: string } | null {
  const date = DATE_EXPR;
  const match = text.match(new RegExp(`自(${date})(?:起|开始)?(?:至|到)(${date})止?`));
  if (!match) return null;
  const startedAt = parseDateExpression(match[1]);
  const expiresAt = parseDateExpression(match[2]);
  return startedAt && expiresAt ? { startedAt, expiresAt } : null;
}

function extractPreservationStartDate(text: string): string | null {
  const explicitStart = text.match(new RegExp(`自(${DATE_EXPR})(?:起|开始)`));
  if (explicitStart) {
    const parsed = parseDateExpression(explicitStart[1]);
    if (parsed) return parsed;
  }

  const dates = [
    ...Array.from(text.matchAll(/(20\d{2})\s*年\s*(\d{1,2})\s*月\s*(\d{1,2})\s*日/g)).map(
      (m) => toIsoDate(Number(m[1]), Number(m[2]), Number(m[3])),
    ),
    ...Array.from(
      text.matchAll(/([二〇零一二三四五六七八九十]{4})年([一二三四五六七八九十]{1,3})月([一二三四五六七八九十]{1,3})日/g),
    ).map((m) => {
      const year = parseChineseYear(m[1]);
      const month = parseChineseNumber(m[2]);
      const day = parseChineseNumber(m[3]);
      return year && month && day ? toIsoDate(year, month, day) : null;
    }),
  ].filter((date): date is string => !!date);
  return dates.length > 0 ? dates[dates.length - 1] : null;
}

function normalizeText(raw: string): string {
  return raw.replace(/\s+/g, "");
}

function toIsoDate(year: number, month: number, day: number): string | null {
  if (year < 1900 || month < 1 || month > 12 || day < 1 || day > 31) return null;
  return `${year}-${String(month).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
}

function parseDateExpression(value: string): string | null {
  const arabic = value.match(/(20\d{2})年(\d{1,2})月(\d{1,2})日/);
  if (arabic) {
    return toIsoDate(Number(arabic[1]), Number(arabic[2]), Number(arabic[3]));
  }
  const chinese = value.match(
    /([二〇零一二三四五六七八九十]{4})年([一二三四五六七八九十]{1,3})月([一二三四五六七八九十]{1,3})日/,
  );
  if (!chinese) return null;
  const year = parseChineseYear(chinese[1]);
  const month = parseChineseNumber(chinese[2]);
  const day = parseChineseNumber(chinese[3]);
  return year && month && day ? toIsoDate(year, month, day) : null;
}

function parseChineseYear(value: string): number | null {
  const digits = [...value].map((char) => {
    if (char === "〇" || char === "零") return "0";
    return String(CHINESE_NUMBER[char] ?? "");
  });
  const year = Number(digits.join(""));
  return Number.isFinite(year) ? year : null;
}

function parseChineseNumber(value: string): number | null {
  if (value === "十") return 10;
  if (value.startsWith("十")) {
    const ones = CHINESE_NUMBER[value.slice(1)] ?? 0;
    return 10 + ones;
  }
  if (value.endsWith("十")) {
    const tens = CHINESE_NUMBER[value.slice(0, -1)] ?? 1;
    return tens * 10;
  }
  if (value.includes("十")) {
    const [left, right] = value.split("十");
    const tens = CHINESE_NUMBER[left] ?? 1;
    const ones = CHINESE_NUMBER[right] ?? 0;
    return tens * 10 + ones;
  }
  return CHINESE_NUMBER[value] ?? null;
}

const CHINESE_NUMBER: Record<string, number> = {
  一: 1,
  二: 2,
  三: 3,
  四: 4,
  五: 5,
  六: 6,
  七: 7,
  八: 8,
  九: 9,
};

const DATE_EXPR =
  "(?:20\\d{2}年\\d{1,2}月\\d{1,2}日|[二〇零一二三四五六七八九十]{4}年[一二三四五六七八九十]{1,3}月[一二三四五六七八九十]{1,3}日)";

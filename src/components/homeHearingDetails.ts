import type { Document, ExtractedFields, KeyDate } from "@/lib/types";

export interface HearingDisplayDetail {
  timeText: string | null;
  locationText: string | null;
}

export async function loadHearingDisplayDetail(
  docs: Document[],
  isoDate: string,
  readMd: (path: string) => Promise<string>,
  courtName?: string | null,
): Promise<HearingDisplayDetail | null> {
  const candidates = docs
    .map((doc) => {
      const fields = parseExtractedFields(doc.extracted_fields);
      const matchedKeyDates = (fields?.key_dates ?? []).filter(
        (kd) => kd.event_type.includes("开庭") && kd.date === isoDate,
      );
      const detailFromFields = extractHearingDetailFromKeyDates(
        matchedKeyDates,
        courtName,
      );
      const score =
        scoreHearingDoc(doc) +
        (matchedKeyDates.length > 0 ? 4 : 0) +
        (detailFromFields.timeText ? 2 : 0) +
        (detailFromFields.locationText ? 2 : 0);
      return {
        doc,
        detailFromFields,
        score,
      };
    })
    .filter((item) => item.score > 0)
    .sort((a, b) => b.score - a.score);

  for (const candidate of candidates) {
    if (hasHearingDetail(candidate.detailFromFields)) {
      if (
        candidate.detailFromFields.timeText &&
        candidate.detailFromFields.locationText
      ) {
        return candidate.detailFromFields;
      }
    }

    if (candidate.doc.extracted_text_path) {
      try {
        const raw = await readMd(candidate.doc.extracted_text_path);
        const detailFromMd = extractHearingDetailFromText(raw, isoDate, courtName);
        const merged = mergeHearingDetails(
          candidate.detailFromFields,
          detailFromMd,
        );
        if (hasHearingDetail(merged)) return merged;
      } catch {
        // 忽略单份 MD 读取失败,继续尝试别的候选文档。
      }
    }

    if (hasHearingDetail(candidate.detailFromFields)) {
      return candidate.detailFromFields;
    }
  }

  return null;
}

export function extractHearingDetailFromText(
  raw: string,
  isoDate: string,
  courtName?: string | null,
): HearingDisplayDetail {
  const text = normalizeText(raw);
  const dateZh = isoToChineseDate(isoDate);
  const escapedDate = escapeRegExp(dateZh);
  const locationSegment = extractHearingLocationSegment(text);

  const arrivalTime =
    firstCapture(text, [
      new RegExp(
        `(?:应到时间|到庭时间|报到时间|应到庭时间)\\s*${escapedDate}\\s*([0-9]{1,2}:[0-9]{2})`,
      ),
      /(应到时间|到庭时间|报到时间|应到庭时间)\s*([0-9]{1,2}:\d{2})/,
    ]) ?? null;

  const hearingSpan =
    firstCapture(text, [
      /(?:庭审时间|开庭时间)\s*([0-9]{1,2}:\d{2}\s*[-—~至]\s*[0-9]{1,2}:\d{2})/,
    ]) ?? null;

  const courtroom =
    firstCapture(text, [
      /(?:应到处所|开庭地点|开庭处所|地点)\s*[:：]?\s*(第\s*[0-9一二三四五六七八九十百]+(?:号)?法庭)/,
      /(第\s*[0-9一二三四五六七八九十百]+(?:号)?法庭)/,
    ]) ?? null;

  const address = extractHearingAddress(text, locationSegment, courtName);

  const timeParts = [
    arrivalTime ? `到场 ${compactTime(arrivalTime)}` : null,
    hearingSpan ? `庭审 ${compactSpan(hearingSpan)}` : null,
  ].filter(Boolean) as string[];
  const locationParts = uniqueStrings([courtroom, address]);

  return {
    timeText: timeParts.length > 0 ? timeParts.join(" · ") : null,
    locationText: locationParts.length > 0 ? locationParts.join(" · ") : null,
  };
}

function extractHearingDetailFromKeyDates(
  keyDates: KeyDate[],
  courtName?: string | null,
): HearingDisplayDetail {
  const merged: HearingDisplayDetail = {
    timeText: null,
    locationText: null,
  };
  for (const kd of keyDates) {
    const note = kd.note?.trim();
    if (!note) continue;
    const parsed = extractHearingDetailFromText(note, kd.date ?? "", courtName);
    if (!merged.timeText && parsed.timeText) merged.timeText = parsed.timeText;
    if (!merged.locationText && parsed.locationText) {
      merged.locationText = parsed.locationText;
    }
  }
  return merged;
}

function parseExtractedFields(json: string | null): ExtractedFields | null {
  if (!json) return null;
  try {
    return JSON.parse(json) as ExtractedFields;
  } catch {
    return null;
  }
}

function scoreHearingDoc(doc: Document): number {
  const hay = `${doc.category ?? ""} ${doc.filename}`;
  if (/传票|开庭通知/.test(hay)) return 3;
  if (/开庭/.test(hay)) return 2;
  if (doc.extracted_text_path) return 1;
  return 0;
}

function mergeHearingDetails(
  primary: HearingDisplayDetail,
  fallback: HearingDisplayDetail,
): HearingDisplayDetail {
  return {
    timeText: primary.timeText ?? fallback.timeText,
    locationText: primary.locationText ?? fallback.locationText,
  };
}

function hasHearingDetail(detail: HearingDisplayDetail | null): boolean {
  return !!(detail?.timeText || detail?.locationText);
}

function firstCapture(text: string, regexes: RegExp[]): string | null {
  for (const regex of regexes) {
    const match = text.match(regex);
    if (!match) continue;
    const value = (match[2] ?? match[1] ?? "").trim();
    if (value) return value;
  }
  return null;
}

function extractHearingLocationSegment(text: string): string | null {
  return firstCapture(text, [
    /(?:应到处所|开庭地点|开庭处所|应到地点)\s*[:：]?\s*(.{0,100}?)(?=(?:被传唤人地址|送达地址|住所地|住址|通讯地址|传唤事由|应到时间|庭审时间|开庭时间|注意事项|$))/,
  ]);
}

function extractHearingAddress(
  text: string,
  locationSegment: string | null,
  courtName?: string | null,
): string | null {
  const courtAddress = firstCapture(text, [
    /(?:法院地址|法庭地址|审判庭地址|开庭地址)\s*[:：]?\s*([^\s，。；]{0,50}(?:路|街|道|巷|大道|号)[^，。；\n]{0,30}(?:室|楼|层|号)?)/,
  ]);
  if (courtAddress && !isPartyAddressFragment(courtAddress, courtName)) {
    return courtAddress;
  }

  const fromLabeledLocation = locationSegment
    ? firstCapture(locationSegment, [
        /((?:[\u4e00-\u9fa5]{2,}(?:省|市|区|县)){0,4}[\u4e00-\u9fa5A-Za-z0-9\-]{2,}(?:路|街|道|巷|大道|号)[^，。；\n]{0,24}(?:室|楼|层|号)?)/,
      ])
    : null;
  if (fromLabeledLocation && !isPartyAddressFragment(fromLabeledLocation, courtName)) {
    return fromLabeledLocation;
  }

  if (/(?:被传唤人地址|送达地址|住所地|住址|通讯地址)/.test(text)) {
    return null;
  }

  const fallback = firstCapture(text, [
    /((?:[\u4e00-\u9fa5]{2,}(?:省|市|区|县)){1,4}[\u4e00-\u9fa5A-Za-z0-9\-]{2,}(?:路|街|道|巷|大道|号)[^，。；\n]{0,24}(?:室|楼|层|号)?)/,
  ]);
  return fallback && !isPartyAddressFragment(fallback, courtName) ? fallback : null;
}

function isPartyAddressFragment(value: string, courtName?: string | null): boolean {
  if (/(?:被传唤人地址|送达地址|住所地|住址|通讯地址)/.test(value)) return true;
  if (!looksLikeAddress(value)) return false;
  const courtToken = courtName?.trim().slice(0, 2);
  return !!courtToken && !value.includes(courtToken);
}

function looksLikeAddress(value: string): boolean {
  return /(?:省|市|区|县).*(?:路|街|道|巷|大道|号)|(?:路|街|道|巷|大道).*(?:号|室|楼|层)/.test(
    value,
  );
}

function compactTime(value: string): string {
  return value.replace(/\s+/g, "");
}

function compactSpan(value: string): string {
  return value.replace(/\s+/g, "").replace(/至/g, "-").replace(/[—~]/g, "-");
}

function normalizeText(raw: string): string {
  return raw.replace(/\s+/g, " ").replace(/[：]/g, ":").trim();
}

function isoToChineseDate(isoDate: string): string {
  const match = isoDate.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!match) return "";
  return `${match[1]}年${match[2]}月${match[3]}日`;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function uniqueStrings(values: Array<string | null>): string[] {
  const out: string[] = [];
  for (const value of values) {
    const next = value?.trim();
    if (!next) continue;
    if (!out.includes(next)) out.push(next);
  }
  return out;
}

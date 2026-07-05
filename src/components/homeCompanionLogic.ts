export const WEATHER_REUSE_WINDOW_MS = 1000 * 60 * 5;
export const NETWORK_LOCATION_RETRY_MS = 1000 * 60 * 5;
export const WEATHER_CACHE_MAX_AGE_MS = 1000 * 60 * 30;

export interface GreetingCacheKeyInput {
  localDate: string;
  timeOfDay: string;
  weatherSummary: string | null;
  assistantMode: string;
  activeCaseCount: number;
  reminderSummaries: string[];
}

export interface CachedWeatherLike {
  generated_at: string;
  summary?: string | null;
  source?: string | null;
  detail?: string | null;
  location_label?: string | null;
}

export function todayLocalIso(now = new Date()): string {
  const y = now.getFullYear();
  const m = `${now.getMonth() + 1}`.padStart(2, "0");
  const d = `${now.getDate()}`.padStart(2, "0");
  return `${y}-${m}-${d}`;
}

export function timeOfDay(now = new Date()): string {
  const hour = now.getHours();
  if (hour < 6) return "夜间";
  if (hour < 12) return "上午";
  if (hour < 14) return "中午";
  if (hour < 18) return "下午";
  return "晚上";
}

export function buildGreetingCacheKey(input: GreetingCacheKeyInput): string {
  const fingerprint = stableHash(
    JSON.stringify({
      date: input.localDate,
      period: input.timeOfDay,
      weather: input.weatherSummary?.trim() || "天气未更新",
      mode: input.assistantMode,
      cases: caseLoadBucket(input.activeCaseCount),
      reminders: reminderBucket(input.reminderSummaries),
    }),
  );
  return `${input.localDate}:${input.timeOfDay}:${fingerprint}`;
}

export function shouldRefreshWeather(cached: CachedWeatherLike | null, now = new Date()): boolean {
  if (!cached?.generated_at) return true;
  const generatedAt = Date.parse(cached.generated_at);
  if (!Number.isFinite(generatedAt)) return true;
  const age = now.getTime() - generatedAt;
  if (age < -WEATHER_REUSE_WINDOW_MS) return true;
  if (age > WEATHER_CACHE_MAX_AGE_MS) return true;
  if (isNetworkFallbackWeather(cached)) return age > NETWORK_LOCATION_RETRY_MS;
  return age > WEATHER_REUSE_WINDOW_MS;
}

export function isUsableCachedWeather(cached: CachedWeatherLike | null, now = new Date()): boolean {
  if (!cached) return false;
  if (cached.source !== "系统定位") return false;
  return !shouldRefreshWeather(cached, now);
}

export function isDisplayableCachedWeather(
  cached: CachedWeatherLike | null,
  now = new Date(),
): boolean {
  if (!cached) return false;
  return !shouldRefreshWeather(cached, now);
}

export function weatherSummaryForGreeting(
  cached: CachedWeatherLike | null,
  now = new Date(),
): string | null {
  if (!isUsableCachedWeather(cached, now)) return null;
  const summary = cached?.summary?.trim();
  return summary || null;
}

export function weatherDisplaySummary(cached: CachedWeatherLike | null): string | null {
  if (!cached) return null;
  if (cached.source === "网络定位") return "定位未确认，天气未更新";
  const summary = cached.summary?.trim();
  return summary || null;
}

export interface WeatherStatusMessageInput {
  value: CachedWeatherLike | null;
  error: string | null;
  weatherFeedsGreeting: boolean;
  weatherNeedsRefresh: boolean;
}

export function weatherStatusMessage(input: WeatherStatusMessageInput): string | null {
  const { value, error, weatherFeedsGreeting, weatherNeedsRefresh } = input;
  if (value?.source === "网络定位") {
    return locationIssueLabel(extractLocationIssue(value.detail) ?? error) ?? "定位未确认";
  }
  if (value && !weatherFeedsGreeting) return "旧天气缓存";
  if (value && weatherNeedsRefresh) return "正在刷新";
  if (error && value) return "使用缓存";
  return error;
}

export function shouldShowLocationSettingsAction(
  cached: CachedWeatherLike | null,
  error: string | null,
): boolean {
  const issue = `${extractLocationIssue(cached?.detail) ?? ""} ${error ?? ""}`;
  if (!issue.trim()) return false;
  return [
    "权限未开启",
    "定位服务未开启",
    "定位服务受限",
    "定位超时",
    "等待定位授权",
    "授权未完成",
    "not_determined",
    "NotDetermined",
  ].some((needle) => issue.includes(needle));
}

export function isGreetingConsistentWithPeriod(
  text: string | null | undefined,
  period: string,
): boolean {
  const value = text?.trim();
  if (!value) return false;
  return !isInconsistentTimeText(value, period);
}

export function isGreetingTextCompatible(
  text: string | null | undefined,
  period: string,
  weatherSummary: string | null = null,
): boolean {
  const value = text?.trim();
  if (!value) return false;
  if (isPromptLeakText(value)) return false;
  if (!isGreetingConsistentWithPeriod(value, period)) return false;
  if (!weatherSummary?.trim() && hasWeatherSmallTalk(value)) return false;
  return true;
}

export function isStaleIso(value: string, ttlMs: number, now = new Date()): boolean {
  const time = Date.parse(value);
  if (!Number.isFinite(time)) return true;
  return now.getTime() - time > ttlMs;
}

function caseLoadBucket(count: number): string {
  if (count <= 0) return "none";
  if (count <= 7) return "normal";
  return "heavy";
}

function reminderBucket(items: string[]): string {
  if (items.some((item) => item.includes("逾期"))) return "overdue";
  if (items.some((item) => item.includes("紧急"))) return "urgent";
  if (items.some((item) => item.trim())) return "some";
  return "none";
}

function isNetworkFallbackWeather(cached: CachedWeatherLike): boolean {
  return cached.source === "网络定位" || Boolean(cached.detail?.includes("系统定位失败"));
}

function extractLocationIssue(detail: string | null | undefined): string | null {
  const value = detail?.trim();
  if (!value) return null;
  const marker = "系统定位失败:";
  const index = value.indexOf(marker);
  if (index < 0) return null;
  const rest = value.slice(index + marker.length).trim();
  if (!rest) return null;
  const parts = rest
    .split(/[;；]/)
    .map((part) => part.replace(/^WebView\s*定位也失败[:：]\s*/, "").trim())
    .filter(Boolean);
  return parts.find(isActionableLocationIssue) ?? parts[0] ?? null;
}

function locationIssueLabel(issue: string | null | undefined): string | null {
  const value = issue?.trim();
  if (!value) return null;
  if (value.includes("定位权限未开启")) return "定位权限未开启";
  if (value.includes("定位服务未开启")) return "定位服务未开启";
  if (value.includes("定位服务受限")) return "定位服务受限";
  if (value.includes("定位超时")) return "定位授权超时";
  if (value.includes("not_determined") || value.includes("NotDetermined")) return "等待定位授权";
  if (value.includes("授权")) return "等待定位授权";
  return value;
}

function isActionableLocationIssue(issue: string): boolean {
  return [
    "权限未开启",
    "定位服务未开启",
    "定位服务受限",
    "定位超时",
    "等待定位授权",
    "授权未完成",
    "not_determined",
    "NotDetermined",
  ].some((needle) => issue.includes(needle));
}

function isInconsistentTimeText(text: string, period: string): boolean {
  if (period === "早上" || period === "上午") {
    return [
      "早点休息",
      "早些休息",
      "晚安",
      "时间不早",
      "明天状态",
      "今天辛苦",
      "辛苦了",
    ].some((needle) => text.includes(needle));
  }
  if (period === "晚上" || period === "夜间") {
    return ["早上", "上午", "开局", "新的一天"].some((needle) => text.includes(needle));
  }
  return false;
}

function hasWeatherSmallTalk(text: string): boolean {
  return [
    "天气",
    "下雨",
    "雨",
    "带伞",
    "伞",
    "添衣",
    "降温",
    "升温",
    "高温",
    "低温",
  ].some((needle) => text.includes(needle));
}

function isPromptLeakText(text: string): boolean {
  return [
    "输出一句中文",
    "只输出一句",
    "30字以内",
    "30 字以内",
    "最多46",
    "最多 46",
    "严格要求",
    "不要解释",
    "不要引号",
    "不要列表",
    "时间段是",
    "日期202",
    "我们要求",
    "要求输出",
  ].some((needle) => text.includes(needle));
}

function stableHash(value: string): string {
  let hash = 2166136261;
  for (const char of value) {
    hash ^= char.charCodeAt(0);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(36);
}

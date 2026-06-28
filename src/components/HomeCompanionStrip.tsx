import { useCallback, useEffect, useMemo, useState } from "react";
import { CloudSun } from "lucide-react";

import poseBriefcase from "@/assets/caseboard-companion/caseboard-companion-pose-briefcase-2026-06-28.png";
import poseChecklist from "@/assets/caseboard-companion/caseboard-companion-pose-checklist-2026-06-28.png";
import poseFiles from "@/assets/caseboard-companion/caseboard-companion-pose-files-2026-06-28.png";
import poseNeutral from "@/assets/caseboard-companion/caseboard-companion-pose-neutral-2026-06-28.png";
import poseWriting from "@/assets/caseboard-companion/caseboard-companion-pose-writing-2026-06-28.png";
import { generateHomeGreeting, type HomeGreetingResponse } from "@/lib/api";

const GREETING_KEY_PREFIX = "caseboard:home-companion:greeting:";
const WEATHER_KEY_PREFIX = "caseboard:home-companion:weather:";
const GREETING_REFRESH_KEY_PREFIX = "caseboard:home-companion:greeting-refresh:";
const GEOLOCATION_TIMEOUT_MS = 5000;
const WEATHER_FETCH_TIMEOUT_MS = 2500;
const IP_LOCATION_TIMEOUT_MS = 2500;
const WEATHER_CACHE_TTL_MS = 1000 * 60 * 60 * 3;
const GREETING_MIN_REFRESH_MS = 1000 * 60 * 60 * 2;
const GREETING_JITTER_MS = 1000 * 60 * 45;

interface CachedGreeting {
  text: string;
  source: string;
  generated_at: string;
}

interface CachedWeather {
  summary: string;
  detail: string;
  generated_at: string;
  source?: "系统定位" | "网络定位";
  location_label?: string | null;
}

interface WeatherLocation {
  latitude: number;
  longitude: number;
  source: "system" | "network";
  label: string | null;
  warning: string | null;
}

type CompanionMode = "urgent" | "caseload" | "briefcase" | "writing" | "neutral";

type WeatherState =
  | { status: "idle"; value: CachedWeather | null; error: string | null }
  | { status: "locating"; value: CachedWeather | null; error: string | null }
  | { status: "fetching"; value: CachedWeather | null; error: string | null };

export function HomeCompanionStrip({
  displayName,
  activeCaseCount,
  reminderSummaries,
}: {
  displayName: string | null;
  activeCaseCount: number;
  reminderSummaries: string[];
}) {
  const today = useMemo(() => todayLocalIso(), []);
  const companionMode = useMemo(
    () => resolveCompanionMode({ today, activeCaseCount, reminderSummaries }),
    [activeCaseCount, reminderSummaries, today],
  );
  const [greeting, setGreeting] = useState<CachedGreeting>(() =>
    readCachedGreeting(today) ?? {
      text: fallbackGreeting(displayName, timeOfDay(), companionMode),
      source: "fallback",
      generated_at: new Date().toISOString(),
    },
  );
  const [greetingRefreshing, setGreetingRefreshing] = useState(false);
  const [weather, setWeather] = useState<WeatherState>(() => ({
    status: "idle",
    value: readCachedWeather(today),
    error: null,
  }));

  const weatherSummary = weather.value?.summary ?? null;

  const refreshGreeting = useCallback(
    async (forceRefresh = false) => {
      setGreetingRefreshing(true);
      try {
        const result = await generateHomeGreeting({
          display_name: displayName,
          weather_summary: weatherSummary,
          active_case_count: activeCaseCount,
          reminder_summaries: reminderSummaries,
          assistant_mode: companionModeLabel(companionMode),
          local_date: today,
          time_of_day: timeOfDay(),
          force_refresh: forceRefresh,
        });
        const next = responseToCache(result);
        setGreeting(next);
        writeCachedGreeting(today, next);
      } catch {
        // 首页问候是环境信息,失败不打扰主流程。
      } finally {
        setGreetingRefreshing(false);
      }
    },
    [activeCaseCount, companionMode, displayName, reminderSummaries, today, weatherSummary],
  );

  useEffect(() => {
    const cached = readCachedGreeting(today);
    if (cached && !shouldRefreshGreeting(today)) return;
    const delay = stableRefreshDelay(today);
    const id = window.setTimeout(() => {
      void refreshGreeting(Boolean(cached));
      writeGreetingRefresh(today);
    }, delay);
    return () => window.clearTimeout(id);
  }, [refreshGreeting, today]);

  const refreshWeather = useCallback(async () => {
    setWeather((prev) => ({ status: "locating", value: prev.value, error: null }));
    try {
      const location = await resolveWeatherLocation();
      setWeather((prev) => ({ status: "fetching", value: prev.value, error: null }));
      const next = await fetchWeather(location);
      writeCachedWeather(today, next);
      setWeather({ status: "idle", value: next, error: null });
      void refreshGreeting(false);
    } catch (e) {
      const message = weatherErrorMessage(e);
      setWeather((prev) => ({ status: "idle", value: prev.value, error: message }));
    }
  }, [today, refreshGreeting]);

  useEffect(() => {
    const cached = readCachedWeather(today);
    if (cached && !isStaleIso(cached.generated_at, WEATHER_CACHE_TTL_MS)) return;
    const id = window.setTimeout(() => {
      void refreshWeather();
    }, 250);
    return () => window.clearTimeout(id);
  }, [refreshWeather, today]);

  const weatherBusy = weather.status === "locating" || weather.status === "fetching";
  const weatherLabel =
    weather.status === "locating"
      ? "正在定位..."
      : weather.status === "fetching"
        ? "正在查询天气..."
        : weather.value?.summary ?? (weather.error ? "天气获取失败" : "天气未更新");
  const companionPose = pickCompanionPose({
    mode: companionMode,
    weatherBusy,
    greetingRefreshing,
  });

  return (
    <div className="mt-3 flex max-w-2xl items-start gap-2.5 text-sm text-muted-foreground">
      <img
        aria-hidden
        src={companionPose.src}
        className="h-12 w-16 shrink-0 object-contain object-center"
        alt=""
        title={companionPose.label}
      />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
          <span className="min-w-0 text-sm leading-snug text-foreground">{greeting.text}</span>
          <span
            className="shrink-0 text-[11px] text-muted-foreground/70"
            title={greeting.source === "ai" ? "案件助手 · AI 生成" : "案件助手 · 本地兜底"}
          >
            案件助手
          </span>
        </div>
        <div className="mt-0.5 flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
          <span className="inline-flex items-center gap-1" title={weather.value?.detail ?? undefined}>
            <CloudSun className="size-3" />
            {weatherLabel}
          </span>
          {weather.error && (
            <span className="text-muted-foreground/80" title={weather.error}>
              {weather.value ? "使用缓存" : weather.error}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}

function resolveCompanionMode({
  today,
  activeCaseCount,
  reminderSummaries,
}: {
  today: string;
  activeCaseCount: number;
  reminderSummaries: string[];
}): CompanionMode {
  if (reminderSummaries.some((item) => item.includes("紧急") || item.includes("逾期"))) return "urgent";
  if (activeCaseCount >= 8) return "caseload";
  return (["neutral", "briefcase", "writing"] as const)[stableIndex(today, 3)];
}

function pickCompanionPose({
  mode,
  weatherBusy,
  greetingRefreshing,
}: {
  mode: CompanionMode;
  weatherBusy: boolean;
  greetingRefreshing: boolean;
}): { src: string; label: string } {
  if (weatherBusy || greetingRefreshing) return { src: poseWriting, label: "案件助手正在更新" };
  if (mode === "urgent") return { src: poseChecklist, label: "案件助手提醒重点事项" };
  if (mode === "caseload") return { src: poseFiles, label: "案件助手整理案件" };
  if (mode === "briefcase") return { src: poseBriefcase, label: "案件助手准备出门" };
  if (mode === "writing") return { src: poseWriting, label: "案件助手记录事项" };
  return { src: poseNeutral, label: "案件助手值守" };
}

function companionModeLabel(mode: CompanionMode): string {
  if (mode === "urgent") return "重点提醒";
  if (mode === "caseload") return "整理案件";
  if (mode === "briefcase") return "准备出门";
  if (mode === "writing") return "记录事项";
  return "日常值守";
}

function fallbackGreeting(
  displayName: string | null,
  period: string,
  mode: CompanionMode = "neutral",
): string {
  const name = displayName?.trim() || "律师";
  if (mode === "urgent") return `${name},先看今天最紧的提醒。`;
  if (mode === "caseload") return `${name},在办案件不少,先抓重点推进。`;
  if (mode === "writing") return `${name},先记清楚关键节点,再动手。`;
  if (mode === "briefcase") return `${name},今天出门前先过一遍提醒。`;
  if (period === "上午") return `${name},早上先看最要紧的一件事。`;
  if (period === "晚上" || period === "夜间") return `${name},晚上收个尾,别把自己绷太紧。`;
  return `${name},今天稳一点,先处理最关键的事。`;
}

function stableIndex(seed: string, modulo: number): number {
  let hash = 0;
  for (const char of seed) {
    hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  }
  return hash % modulo;
}

function responseToCache(result: HomeGreetingResponse): CachedGreeting {
  return {
    text: result.text,
    source: result.source,
    generated_at: result.generated_at,
  };
}

function todayLocalIso(): string {
  const now = new Date();
  const y = now.getFullYear();
  const m = `${now.getMonth() + 1}`.padStart(2, "0");
  const d = `${now.getDate()}`.padStart(2, "0");
  return `${y}-${m}-${d}`;
}

function timeOfDay(): string {
  const hour = new Date().getHours();
  if (hour < 6) return "夜间";
  if (hour < 11) return "上午";
  if (hour < 14) return "中午";
  if (hour < 18) return "下午";
  return "晚上";
}

function readCachedGreeting(today: string): CachedGreeting | null {
  return readJson<CachedGreeting>(GREETING_KEY_PREFIX + today);
}

function writeCachedGreeting(today: string, value: CachedGreeting): void {
  writeJson(GREETING_KEY_PREFIX + today, value);
}

function shouldRefreshGreeting(today: string): boolean {
  const last = readJson<{ refreshed_at: string }>(GREETING_REFRESH_KEY_PREFIX + today);
  if (!last?.refreshed_at) return true;
  return isStaleIso(last.refreshed_at, GREETING_MIN_REFRESH_MS + stableRefreshDelay(today));
}

function writeGreetingRefresh(today: string): void {
  writeJson(GREETING_REFRESH_KEY_PREFIX + today, {
    refreshed_at: new Date().toISOString(),
  });
}

function readCachedWeather(today: string): CachedWeather | null {
  return readJson<CachedWeather>(WEATHER_KEY_PREFIX + today);
}

function writeCachedWeather(today: string, value: CachedWeather): void {
  writeJson(WEATHER_KEY_PREFIX + today, value);
}

function readJson<T>(key: string): T | null {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : null;
  } catch {
    return null;
  }
}

function writeJson(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* UI cache only */
  }
}

function isStaleIso(value: string, ttlMs: number): boolean {
  const time = Date.parse(value);
  if (!Number.isFinite(time)) return true;
  return Date.now() - time > ttlMs;
}

function stableRefreshDelay(seed: string): number {
  return stableIndex(`${seed}:${timeOfDay()}`, GREETING_JITTER_MS);
}

function getCurrentPositionWithTimeout(timeoutMs: number): Promise<GeolocationPosition> {
  if (!navigator.geolocation) {
    return Promise.reject(new Error("当前环境不支持定位"));
  }
  return new Promise((resolve, reject) => {
    navigator.geolocation.getCurrentPosition(resolve, reject, {
      enableHighAccuracy: false,
      maximumAge: 1000 * 60 * 30,
      timeout: timeoutMs,
    });
  });
}

async function resolveWeatherLocation(): Promise<WeatherLocation> {
  try {
    const position = await getCurrentPositionWithTimeout(GEOLOCATION_TIMEOUT_MS);
    return {
      latitude: position.coords.latitude,
      longitude: position.coords.longitude,
      source: "system",
      label: null,
      warning: null,
    };
  } catch (error) {
    const systemError = weatherErrorMessage(error);
    try {
      const fallback = await getIpApproximateLocation();
      return { ...fallback, warning: systemError };
    } catch (fallbackError) {
      throw new Error(`${systemError}; 网络定位也失败: ${weatherErrorMessage(fallbackError)}`);
    }
  }
}

async function getIpApproximateLocation(): Promise<WeatherLocation> {
  const ipapi = await fetchJsonWithTimeout<IpApiLocation>(
    "https://ipapi.co/json/",
    IP_LOCATION_TIMEOUT_MS,
  ).catch(() => null);
  if (typeof ipapi?.latitude === "number" && typeof ipapi.longitude === "number") {
    return {
      latitude: ipapi.latitude,
      longitude: ipapi.longitude,
      source: "network",
      label: [ipapi.city, ipapi.region].filter(Boolean).join(" · ") || null,
      warning: null,
    };
  }

  const ipwho = await fetchJsonWithTimeout<IpWhoLocation>(
    "https://ipwho.is/",
    IP_LOCATION_TIMEOUT_MS,
  );
  if (ipwho.success !== false && typeof ipwho.latitude === "number" && typeof ipwho.longitude === "number") {
    return {
      latitude: ipwho.latitude,
      longitude: ipwho.longitude,
      source: "network",
      label: [ipwho.city, ipwho.region].filter(Boolean).join(" · ") || null,
      warning: null,
    };
  }
  throw new Error(ipwho.message || "网络定位返回无经纬度");
}

async function fetchWeather(location: WeatherLocation): Promise<CachedWeather> {
  const controller = new AbortController();
  const timer = window.setTimeout(() => controller.abort(), WEATHER_FETCH_TIMEOUT_MS);
  try {
    const params = new URLSearchParams({
      latitude: location.latitude.toFixed(4),
      longitude: location.longitude.toFixed(4),
      daily:
        "temperature_2m_max,temperature_2m_min,precipitation_probability_max,precipitation_sum,rain_sum",
      timezone: "auto",
      forecast_days: "1",
    });
    const response = await fetch(`https://api.open-meteo.com/v1/forecast?${params}`, {
      signal: controller.signal,
    });
    if (!response.ok) throw new Error(`天气返回 ${response.status}`);
    const data = (await response.json()) as OpenMeteoDailyResponse;
    return parseWeather(data, location);
  } finally {
    window.clearTimeout(timer);
  }
}

async function fetchJsonWithTimeout<T>(url: string, timeoutMs: number): Promise<T> {
  const controller = new AbortController();
  const timer = window.setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(url, { signal: controller.signal });
    if (!response.ok) throw new Error(`请求返回 ${response.status}`);
    return (await response.json()) as T;
  } finally {
    window.clearTimeout(timer);
  }
}

function weatherErrorMessage(error: unknown): string {
  if (typeof error === "object" && error && "code" in error) {
    const code = Number((error as { code?: unknown }).code);
    if (code === 1) return "定位权限未开启";
    if (code === 2) return "定位不可用";
    if (code === 3) return "定位超时";
  }
  if (error instanceof DOMException && error.name === "AbortError") return "天气请求超时";
  if (error instanceof Error) return error.message;
  return "天气获取失败";
}

interface OpenMeteoDailyResponse {
  daily?: {
    temperature_2m_max?: number[];
    temperature_2m_min?: number[];
    precipitation_probability_max?: number[];
    precipitation_sum?: number[];
    rain_sum?: number[];
  };
}

interface IpApiLocation {
  latitude?: number;
  longitude?: number;
  city?: string;
  region?: string;
}

interface IpWhoLocation {
  success?: boolean;
  latitude?: number;
  longitude?: number;
  city?: string;
  region?: string;
  message?: string;
}

function parseWeather(data: OpenMeteoDailyResponse, location: WeatherLocation): CachedWeather {
  const daily = data.daily ?? {};
  const max = daily.temperature_2m_max?.[0];
  const min = daily.temperature_2m_min?.[0];
  const probability = daily.precipitation_probability_max?.[0] ?? 0;
  const precipitation = daily.precipitation_sum?.[0] ?? daily.rain_sum?.[0] ?? 0;
  const hasRain = precipitation > 0.1 || probability >= 30;
  const temp =
    typeof min === "number" && typeof max === "number"
      ? `${Math.round(min)}-${Math.round(max)}°C`
      : "温度未更新";
  const rainText = hasRain ? "可能有雨" : "少雨";
  const sourceText = location.source === "system" ? "系统定位" : "网络定位";
  const locationText = location.label ? ` · ${location.label}` : "";
  const warningText = location.warning ? ` · 系统定位失败: ${location.warning}` : "";
  return {
    summary: `${temp} · ${rainText}`,
    detail: `${sourceText}${locationText}${warningText} · 降雨概率 ${Math.round(probability)}% · 预计降雨 ${precipitation.toFixed(1)}mm`,
    generated_at: new Date().toISOString(),
    source: sourceText,
    location_label: location.label,
  };
}

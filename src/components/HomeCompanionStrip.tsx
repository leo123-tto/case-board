import { useCallback, useEffect, useMemo, useState } from "react";
import { CloudSun, RefreshCw } from "lucide-react";

import poseBriefcase from "@/assets/caseboard-companion/caseboard-companion-pose-briefcase-2026-06-28.png";
import poseChecklist from "@/assets/caseboard-companion/caseboard-companion-pose-checklist-2026-06-28.png";
import poseFiles from "@/assets/caseboard-companion/caseboard-companion-pose-files-2026-06-28.png";
import poseNeutral from "@/assets/caseboard-companion/caseboard-companion-pose-neutral-2026-06-28.png";
import poseWriting from "@/assets/caseboard-companion/caseboard-companion-pose-writing-2026-06-28.png";
import {
  generateHomeGreeting,
  getNativeLocation,
  getWeatherInfo,
  openLocationPrivacySettings,
  type HomeGreetingResponse,
  type WeatherInfo,
  type WeatherRequest,
} from "@/lib/api";
import { cn } from "@/lib/utils";
import {
  buildGreetingCacheKey,
  isDisplayableCachedWeather,
  isGreetingTextCompatible,
  isStaleIso,
  shouldShowLocationSettingsAction,
  shouldRefreshWeather,
  timeOfDay,
  todayLocalIso,
  weatherDisplaySummary,
  weatherStatusMessage,
  weatherSummaryForGreeting,
} from "./homeCompanionLogic";

const GREETING_KEY_PREFIX = "caseboard:home-companion:greeting:v2:";
const WEATHER_KEY_PREFIX = "caseboard:home-companion:weather:v2:";
const GEOLOCATION_TIMEOUT_MS = 8000;
const GREETING_CACHE_TTL_MS = 1000 * 60 * 90;

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

interface SystemWeatherLocation {
  latitude: number;
  longitude: number;
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
  dailyBrief,
  onDailyBriefAction,
}: {
  displayName: string | null;
  activeCaseCount: number;
  reminderSummaries: string[];
  dailyBrief?: DailyBrief | null;
  onDailyBriefAction?: () => void;
}) {
  const clock = useCompanionClock();
  const localDate = useMemo(() => todayLocalIso(clock), [clock]);
  const currentPeriod = useMemo(() => timeOfDay(clock), [clock]);
  const companionMode = useMemo(
    () => resolveCompanionMode({ today: localDate, activeCaseCount, reminderSummaries }),
    [activeCaseCount, reminderSummaries, localDate],
  );
  const assistantMode = companionModeLabel(companionMode);
  const [greeting, setGreeting] = useState<CachedGreeting>(() => {
    const initialDate = todayLocalIso();
    const initialPeriod = timeOfDay();
    const initialWeather = readCachedWeather(initialDate);
    const initialWeatherSummary = weatherSummaryForGreeting(initialWeather);
    const initialMode = resolveCompanionMode({
      today: initialDate,
      activeCaseCount,
      reminderSummaries,
    });
    const initialKey = buildGreetingCacheKey({
      localDate: initialDate,
      timeOfDay: initialPeriod,
      weatherSummary: initialWeatherSummary,
      assistantMode: companionModeLabel(initialMode),
      activeCaseCount,
      reminderSummaries,
    });
    return (
      readCachedGreeting(initialKey, initialPeriod, weatherSummaryForGreeting(initialWeather)) ?? {
        text: fallbackGreeting(displayName, initialPeriod, initialMode),
        source: "fallback",
        generated_at: new Date().toISOString(),
      }
    );
  });
  const [greetingRefreshing, setGreetingRefreshing] = useState(false);
  const [weather, setWeather] = useState<WeatherState>(() => {
    const initialDate = todayLocalIso();
    return {
      status: "idle",
      value: readCachedWeather(initialDate),
      error: null,
    };
  });

  const weatherSummary = useMemo(
    () => weatherSummaryForGreeting(weather.value, clock),
    [
      clock,
      weather.value?.detail,
      weather.value?.generated_at,
      weather.value?.source,
      weather.value?.summary,
    ],
  );
  const greetingCacheKey = useMemo(
    () =>
      buildGreetingCacheKey({
        localDate,
        timeOfDay: currentPeriod,
        weatherSummary,
        assistantMode,
        activeCaseCount,
        reminderSummaries,
      }),
    [activeCaseCount, assistantMode, currentPeriod, localDate, reminderSummaries, weatherSummary],
  );

  const refreshGreeting = useCallback(
    async ({
      forceRefresh = false,
      weatherSummaryOverride,
    }: {
      forceRefresh?: boolean;
      weatherSummaryOverride?: string | null;
    } = {}) => {
      const requestDate = todayLocalIso();
      const requestPeriod = timeOfDay();
      const requestMode = resolveCompanionMode({
        today: requestDate,
        activeCaseCount,
        reminderSummaries,
      });
      const requestAssistantMode = companionModeLabel(requestMode);
      const effectiveWeatherSummary = weatherSummaryOverride ?? weatherSummary;
      const requestCacheKey = buildGreetingCacheKey({
        localDate: requestDate,
        timeOfDay: requestPeriod,
        weatherSummary: effectiveWeatherSummary,
        assistantMode: requestAssistantMode,
        activeCaseCount,
        reminderSummaries,
      });

      setGreetingRefreshing(true);
      try {
        const result = await generateHomeGreeting({
          display_name: displayName,
          weather_summary: effectiveWeatherSummary,
          active_case_count: activeCaseCount,
          reminder_summaries: reminderSummaries,
          assistant_mode: requestAssistantMode,
          local_date: requestDate,
          time_of_day: requestPeriod,
          force_refresh: forceRefresh,
        });
        const next = responseToCache(result);
        setGreeting(next);
        writeCachedGreeting(requestCacheKey, next);
      } catch {
        // 首页问候是环境信息,失败不打扰主流程。
      } finally {
        setGreetingRefreshing(false);
      }
    },
    [activeCaseCount, displayName, reminderSummaries, weatherSummary],
  );

  useEffect(() => {
    const cached = readCachedGreeting(greetingCacheKey, currentPeriod, weatherSummary);
    if (cached && !isStaleIso(cached.generated_at, GREETING_CACHE_TTL_MS)) {
      setGreeting(cached);
      return;
    }
    if (greetingRefreshing) return;
    setGreeting(
      cached ?? {
        text: fallbackGreeting(displayName, currentPeriod, companionMode),
        source: "fallback",
        generated_at: new Date().toISOString(),
      },
    );
    const id = window.setTimeout(() => {
      void refreshGreeting({ forceRefresh: Boolean(cached) });
    }, 250);
    return () => window.clearTimeout(id);
  }, [companionMode, currentPeriod, displayName, greetingCacheKey, greetingRefreshing, refreshGreeting]);

  const refreshWeather = useCallback(
    async (forceGreetingRefresh = false) => {
      setWeather((prev) => ({ status: "locating", value: prev.value, error: null }));
      try {
        let request: WeatherRequest;
        try {
          const location = await getSystemLocation();
          request = { latitude: location.latitude, longitude: location.longitude };
        } catch (error) {
          request = { warning: weatherErrorMessage(error) };
        }
        setWeather((prev) => ({ status: "fetching", value: prev.value, error: null }));
        const next = weatherInfoToCache(await getWeatherInfo(request));
        if (isDisplayableCachedWeather(next)) writeCachedWeather(todayLocalIso(), next);
        setWeather({ status: "idle", value: next, error: null });
        void refreshGreeting({
          forceRefresh: forceGreetingRefresh,
          weatherSummaryOverride: weatherSummaryForGreeting(next),
        });
      } catch (e) {
        const message = weatherErrorMessage(e);
        setWeather((prev) => ({ status: "idle", value: prev.value, error: message }));
        if (forceGreetingRefresh) void refreshGreeting({ forceRefresh: true });
      }
    },
    [refreshGreeting],
  );

  useEffect(() => {
    const cached = readCachedWeather(localDate);
    if (cached && cached.generated_at !== weather.value?.generated_at) {
      setWeather({ status: "idle", value: cached, error: null });
    }
    if (cached && !shouldRefreshWeather(cached)) return;
    const id = window.setTimeout(() => {
      void refreshWeather(false);
    }, 250);
    return () => window.clearTimeout(id);
  }, [localDate, refreshWeather, weather.value?.generated_at]);

  const handleManualRefresh = useCallback(() => {
    void refreshWeather(true);
  }, [refreshWeather]);

  const weatherBusy = weather.status === "locating" || weather.status === "fetching";
  const weatherNeedsRefresh = weather.value ? shouldRefreshWeather(weather.value, clock) : false;
  const weatherFeedsGreeting = weather.value ? Boolean(weatherSummaryForGreeting(weather.value, clock)) : false;
  const weatherLabel =
    weather.status === "locating"
      ? "正在定位..."
      : weather.status === "fetching"
        ? "正在查询天气..."
        : weatherDisplaySummary(weather.value) ?? (weather.error ? "天气获取失败" : "天气未更新");
  const weatherStatusText = weatherStatusMessage({
    value: weather.value,
    error: weather.error,
    weatherFeedsGreeting,
    weatherNeedsRefresh,
  });
  const showLocationSettingsAction = shouldShowLocationSettingsAction(weather.value, weather.error);
  const companionPose = pickCompanionPose({
    mode: companionMode,
    weatherBusy,
    greetingRefreshing,
  });

  const handleOpenLocationSettings = useCallback(() => {
    void openLocationPrivacySettings();
  }, []);

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
            title={greeting.source === "ai" ? "看板助手 · AI 生成" : "看板助手 · 本地兜底"}
          >
            看板助手
          </span>
        </div>
        <div className="mt-0.5 flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
          <span
            className="inline-flex items-center gap-1"
            title={
              weather.value?.source === "网络定位"
                ? "系统定位失败，当前显示网络估算天气；该天气不会用于 AI 问候"
                : weather.value?.detail ?? undefined
            }
          >
            <CloudSun className="size-3" />
            {weatherLabel}
          </span>
          {weatherStatusText && (
            <span
              className="text-muted-foreground/80"
              title={
                weather.error ??
                (!weatherFeedsGreeting && weather.value
                  ? "天气缓存已过期,不会用于看板助手问候"
                  : undefined)
              }
            >
              {weatherStatusText}
            </span>
          )}
          <button
            type="button"
            className="inline-flex size-5 items-center justify-center rounded border border-transparent text-muted-foreground/70 transition hover:border-border hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
            title="刷新看板助手"
            onClick={handleManualRefresh}
            disabled={weatherBusy || greetingRefreshing}
          >
            <RefreshCw className={weatherBusy || greetingRefreshing ? "size-3 animate-spin" : "size-3"} />
            <span className="sr-only">刷新看板助手</span>
          </button>
          {showLocationSettingsAction && (
            <button
              type="button"
              className="rounded border border-border bg-background px-1.5 py-0.5 text-[11px] text-muted-foreground transition hover:bg-muted hover:text-foreground"
              title="打开系统定位服务设置"
              onClick={handleOpenLocationSettings}
            >
              开启定位
            </button>
          )}
        </div>
        {dailyBrief && (
          <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-muted-foreground">
            <span className="min-w-0 truncate">{dailyBrief.text}</span>
            <button
              type="button"
              className={cn(
                "shrink-0 rounded border px-1.5 py-0.5 transition",
                "border-border bg-background text-muted-foreground hover:bg-muted hover:text-foreground",
              )}
              onClick={onDailyBriefAction}
            >
              {dailyBrief.actionLabel}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

export interface DailyBrief {
  text: string;
  actionLabel: string;
  level: "red" | "orange" | "calm";
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
  if (weatherBusy || greetingRefreshing) return { src: poseWriting, label: "看板助手正在更新" };
  if (mode === "urgent") return { src: poseChecklist, label: "看板助手提醒重点事项" };
  if (mode === "caseload") return { src: poseFiles, label: "看板助手整理案件" };
  if (mode === "briefcase") return { src: poseBriefcase, label: "看板助手准备出门" };
  if (mode === "writing") return { src: poseWriting, label: "看板助手记录事项" };
  return { src: poseNeutral, label: "看板助手值守" };
}

function companionModeLabel(mode: CompanionMode): string {
  if (mode === "urgent") return "日常关心(有提醒背景)";
  if (mode === "caseload") return "日常关心(案件较多)";
  if (mode === "briefcase") return "日常关心(准备出门)";
  if (mode === "writing") return "日常关心(记录节奏)";
  return "日常关心";
}

function fallbackGreeting(
  displayName: string | null,
  period: string,
  mode: CompanionMode = "neutral",
): string {
  const name = displayName?.trim() || "律师";
  if (mode === "urgent") return `${name},今天先稳住节奏,提醒区稍后扫一眼。`;
  if (mode === "caseload") return `${name},案子不少,也别一口气全扛完。`;
  if (mode === "writing") return `${name},先慢一点写,思路清楚最省力。`;
  if (mode === "briefcase") return `${name},出门前带好东西,路上别太赶。`;
  if (period === "上午") return `${name},早上开局不错,今天也稳稳推进。`;
  if (period === "中午") return `${name},中午缓一口气,下午继续稳住。`;
  if (period === "晚上") return `${name},晚上收个尾,差不多就早点休息。`;
  if (period === "夜间") return `${name},时间不早了,先把自己照顾好。`;
  return `${name},今天稳一点,不用一下子全扛完。`;
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

function useCompanionClock(): Date {
  const [clock, setClock] = useState(() => new Date());
  useEffect(() => {
    const id = window.setInterval(() => setClock(new Date()), 1000 * 60);
    return () => window.clearInterval(id);
  }, []);
  return clock;
}

function readCachedGreeting(
  today: string,
  period: string,
  weatherSummary: string | null,
): CachedGreeting | null {
  const cached = readJson<CachedGreeting>(GREETING_KEY_PREFIX + today);
  if (!cached) return null;
  return isGreetingTextCompatible(cached.text, period, weatherSummary) ? cached : null;
}

function writeCachedGreeting(today: string, value: CachedGreeting): void {
  writeJson(GREETING_KEY_PREFIX + today, value);
}

function readCachedWeather(today: string): CachedWeather | null {
  const cached = readJson<CachedWeather>(WEATHER_KEY_PREFIX + today);
  return isDisplayableCachedWeather(cached) ? cached : null;
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

function getCurrentPositionWithTimeout(timeoutMs: number): Promise<GeolocationPosition> {
  if (!navigator.geolocation) {
    return Promise.reject(new Error("当前环境不支持定位"));
  }
  return new Promise((resolve, reject) => {
    navigator.geolocation.getCurrentPosition(resolve, reject, {
      enableHighAccuracy: true,
      maximumAge: 1000 * 60 * 5,
      timeout: timeoutMs,
    });
  });
}

async function getSystemLocation(): Promise<SystemWeatherLocation> {
  try {
    const location = await getNativeLocation(GEOLOCATION_TIMEOUT_MS);
    return {
      latitude: location.latitude,
      longitude: location.longitude,
    };
  } catch (nativeError) {
    try {
      const position = await getCurrentPositionWithTimeout(GEOLOCATION_TIMEOUT_MS);
      return {
        latitude: position.coords.latitude,
        longitude: position.coords.longitude,
      };
    } catch (webError) {
      throw new Error(
        `${weatherErrorMessage(nativeError)}; WebView 定位也失败: ${weatherErrorMessage(webError)}`,
      );
    }
  }
}

function weatherInfoToCache(info: WeatherInfo): CachedWeather {
  return {
    summary: info.summary,
    detail: info.detail,
    generated_at: new Date().toISOString(),
    source: info.source,
    location_label: info.label,
  };
}

function weatherErrorMessage(error: unknown): string {
  if (typeof error === "object" && error && "code" in error) {
    const code = Number((error as { code?: unknown }).code);
    if (code === 1) return "定位权限未开启";
    if (code === 2) return "定位不可用";
    if (code === 3) return "定位超时";
  }
  if (typeof error === "string") return error;
  if (error instanceof DOMException && error.name === "AbortError") return "天气请求超时";
  if (error instanceof Error) return error.message;
  return "天气获取失败";
}

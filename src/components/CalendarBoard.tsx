/**
 * 月历板组件(2026-06-10 V0.3.7)。
 *
 * 在首页 ImportantDates 下方展示一个月历视图，数据来源：
 * 1. 飞书日历事件（主要来源）
 * 2. 本地案件的 agg_key_dates（补充来源）
 *
 * 点击日期可查看当日事件列表，点击事件可跳转案件详情或打开飞书日历。
 */

import { useEffect, useMemo, useState } from "react";
import {
  Calendar,
  ChevronLeft,
  ChevronRight,
  FolderOpen,
  Gavel,
  Loader2,
  ShieldAlert,
} from "lucide-react";

import { fetchFeishuCalendar } from "@/lib/api";
import type { FeishuCalendarEvent } from "@/lib/types";
import { cn } from "@/lib/utils";

import type { UpcomingEvent } from "./HomeView";

/* ------------------------------------------------------------------ */
/* 类型                                                                  */
/* ------------------------------------------------------------------ */

interface CalendarEvent {
  date: string; // YYYY-MM-DD
  title: string;
  source: "feishu" | "local";
  kind?: "hearing" | "deadline" | "todo" | "manual";
  caseId?: string;
  eventId?: string; // 飞书事件ID
  appLink?: string; // 飞书日历链接
  description?: string;
  location?: string;
}

interface CalendarDay {
  date: Date;
  dayOfMonth: number;
  isCurrentMonth: boolean;
  isToday: boolean;
  events: CalendarEvent[];
}

/* ------------------------------------------------------------------ */
/* 工具函数                                                              */
/* ------------------------------------------------------------------ */

function startOfMonth(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), 1);
}

function startOfWeek(d: Date): Date {
  const day = d.getDay();
  const diff = day === 0 ? 6 : day - 1;
  const result = new Date(d);
  result.setDate(result.getDate() - diff);
  return result;
}

function isSameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

function formatDateKey(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function formatDateISO(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

const WEEKDAY_LABELS = ["一", "二", "三", "四", "五", "六", "日"];

const MONTH_LABELS = [
  "一月", "二月", "三月", "四月", "五月", "六月",
  "七月", "八月", "九月", "十月", "十一月", "十二月",
];

/* ------------------------------------------------------------------ */
/* 组件                                                                  */
/* ------------------------------------------------------------------ */

export function CalendarBoard({
  localEvents,
  onPickCase,
  onImportFolder,
}: {
  localEvents: UpcomingEvent[];
  onPickCase: (caseId: string) => void;
  /** 点击飞书日历事件后导入本地文件夹 */
  onImportFolder?: (eventTitle: string) => void;
}) {
  const [viewMonth, setViewMonth] = useState(() => {
    const now = new Date();
    return new Date(now.getFullYear(), now.getMonth(), 1);
  });
  const [selectedDate, setSelectedDate] = useState<Date | null>(null);
  const [feishuEvents, setFeishuEvents] = useState<FeishuCalendarEvent[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expandedEvent, setExpandedEvent] = useState<number | null>(null); // 展开的飞书事件索引

  // 计算当前月份的日期范围（前后各扩展一周）
  const dateRange = useMemo(() => {
    const monthStart = startOfMonth(viewMonth);
    const gridStart = startOfWeek(monthStart);
    const gridEnd = new Date(gridStart);
    gridEnd.setDate(gridEnd.getDate() + 41); // 6周 = 42天

    return {
      start: formatDateISO(gridStart),
      end: formatDateISO(gridEnd),
    };
  }, [viewMonth]);

  // 从飞书日历获取事件（可手动刷新）
  const [refreshKey, setRefreshKey] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    fetchFeishuCalendar(dateRange.start, dateRange.end)
      .then((events) => {
        if (!cancelled) {
          setFeishuEvents(events);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setError(`获取飞书日历失败: ${e}`);
          console.warn("fetch feishu calendar failed:", e);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [dateRange, refreshKey]);

  const handleRefresh = () => setRefreshKey((k) => k + 1);

  // 合并本地案件事件和飞书日历事件
  const allEvents = useMemo((): CalendarEvent[] => {
    const events: CalendarEvent[] = [];

    // 添加飞书日历事件
    for (const fe of feishuEvents) {
      events.push({
        date: fe.start_date,
        title: fe.summary,
        source: "feishu",
        eventId: fe.event_id,
        appLink: fe.app_link ?? undefined,
        description: fe.description ?? undefined,
        location: fe.location ?? undefined,
      });
    }

    // 添加本地案件事件
    for (const le of localEvents) {
      events.push({
        date: le.date,
        title: `${le.caseName} · ${le.type}`,
        source: "local",
        kind: le.kind,
        caseId: le.caseId,
      });
    }

    return events;
  }, [feishuEvents, localEvents]);

  // 按日期分组事件
  const eventsByDate = useMemo(() => {
    const map = new Map<string, CalendarEvent[]>();
    for (const e of allEvents) {
      const key = e.date;
      if (!map.has(key)) map.set(key, []);
      map.get(key)!.push(e);
    }
    return map;
  }, [allEvents]);

  // 生成日历格子
  const days = useMemo((): CalendarDay[] => {
    const today = new Date();
    today.setHours(0, 0, 0, 0);

    const monthStart = startOfMonth(viewMonth);
    const gridStart = startOfWeek(monthStart);

    const result: CalendarDay[] = [];
    const cursor = new Date(gridStart);

    for (let i = 0; i < 42; i++) {
      const dateKey = formatDateKey(cursor);
      result.push({
        date: new Date(cursor),
        dayOfMonth: cursor.getDate(),
        isCurrentMonth: cursor.getMonth() === viewMonth.getMonth(),
        isToday: isSameDay(cursor, today),
        events: eventsByDate.get(dateKey) ?? [],
      });
      cursor.setDate(cursor.getDate() + 1);
    }
    return result;
  }, [viewMonth, eventsByDate]);

  // 当前选中日期的事件
  const selectedEvents = selectedDate
    ? eventsByDate.get(formatDateKey(selectedDate)) ?? []
    : [];

  const goPrev = () =>
    setViewMonth(new Date(viewMonth.getFullYear(), viewMonth.getMonth() - 1, 1));
  const goNext = () =>
    setViewMonth(new Date(viewMonth.getFullYear(), viewMonth.getMonth() + 1, 1));
  const goToday = () => {
    const now = new Date();
    setViewMonth(new Date(now.getFullYear(), now.getMonth(), 1));
    setSelectedDate(null);
  };

  return (
    <div className="rounded-xl border border-border bg-card p-5">
      {/* 头部：月份导航 */}
      <div className="mb-4 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Calendar className="size-4 text-muted-foreground" />
          <h2 className="text-sm font-semibold tracking-tight">
            {viewMonth.getFullYear()}年{MONTH_LABELS[viewMonth.getMonth()]}
          </h2>
          <button
            type="button"
            onClick={goToday}
            className="rounded px-1.5 py-0.5 text-caption text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            今天
          </button>
          <button
            type="button"
            onClick={handleRefresh}
            className="rounded px-1.5 py-0.5 text-caption text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            ⟳ 刷新
          </button>
          {loading && (
            <Loader2 className="size-3.5 animate-spin text-muted-foreground" />
          )}
        </div>
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={goPrev}
            className="rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            aria-label="上个月"
          >
            <ChevronLeft className="size-4" />
          </button>
          <button
            type="button"
            onClick={goNext}
            className="rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            aria-label="下个月"
          >
            <ChevronRight className="size-4" />
          </button>
        </div>
      </div>

      {/* 错误提示 */}
      {error && (
        <div className="mb-3 rounded-md bg-destructive/10 px-3 py-2 text-caption text-destructive">
          {error}
        </div>
      )}

      {/* 星期标签 */}
      <div className="mb-1 grid grid-cols-7 gap-px">
        {WEEKDAY_LABELS.map((label) => (
          <div
            key={label}
            className="py-1 text-center text-caption font-medium text-muted-foreground"
          >
            {label}
          </div>
        ))}
      </div>

      {/* 日历格子 */}
      <div className="grid grid-cols-7 gap-px">
        {days.map((day, i) => {
          const hasEvents = day.events.length > 0;
          const isSelected =
            selectedDate && isSameDay(day.date, selectedDate);
          const hasFeishu = day.events.some((e) => e.source === "feishu");
          const hasLocal = day.events.some((e) => e.source === "local");

          return (
            <button
              key={i}
              type="button"
              onClick={() => setSelectedDate(isSelected ? null : day.date)}
              className={cn(
                "relative flex h-10 flex-col items-center justify-center rounded-md text-sm transition-colors",
                day.isCurrentMonth
                  ? "text-foreground"
                  : "text-muted-foreground/40",
                day.isToday && "font-bold",
                isSelected && "bg-foreground text-background",
                !isSelected && hasEvents && "bg-accent/50",
                !isSelected && !hasEvents && "hover:bg-accent/30",
              )}
            >
              <span
                className={cn(
                  day.isToday &&
                    !isSelected &&
                    "inline-flex size-5 items-center justify-center rounded-full bg-foreground text-background",
                )}
              >
                {day.dayOfMonth}
              </span>
              {/* 事件指示点 */}
              {hasEvents && (
                <div className="absolute bottom-1 flex gap-0.5">
                  {hasFeishu && (
                    <div
                      className={cn(
                        "size-1.5 rounded-full",
                        isSelected ? "bg-background" : "bg-blue-500",
                      )}
                    />
                  )}
                  {hasLocal && (
                    <div
                      className={cn(
                        "size-1.5 rounded-full",
                        isSelected ? "bg-background" : "bg-amber-500",
                      )}
                    />
                  )}
                </div>
              )}
            </button>
          );
        })}
      </div>

      {/* 图例 */}
      <div className="mt-3 flex items-center gap-4 text-caption text-muted-foreground">
        <div className="flex items-center gap-1">
          <div className="size-2 rounded-full bg-blue-500" />
          <span>飞书日历</span>
        </div>
        <div className="flex items-center gap-1">
          <div className="size-2 rounded-full bg-amber-500" />
          <span>案件事件</span>
        </div>
      </div>

      {/* 选中日期的事件列表 */}
      {selectedDate && selectedEvents.length > 0 && (
        <div className="mt-3 border-t border-border pt-3">
          <p className="mb-2 text-caption text-muted-foreground">
            {selectedDate.getMonth() + 1}月{selectedDate.getDate()}日 · {selectedEvents.length} 个事件
          </p>
          <ul className="space-y-1.5">
            {selectedEvents.map((e, i) => (
              <li key={i}>
                {/* 事件标题行 */}
                <button
                  type="button"
                  onClick={() => {
                    if (e.source === "local" && e.caseId) {
                      onPickCase(e.caseId);
                      setSelectedDate(null);
                    } else if (e.source === "feishu") {
                      setExpandedEvent(expandedEvent === i ? null : i);
                    }
                  }}
                  className={cn(
                    "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent cursor-pointer",
                    expandedEvent === i && "rounded-b-none bg-accent/50",
                  )}
                >
                  {e.source === "feishu" ? (
                    <Calendar className="size-3.5 shrink-0 text-blue-600" />
                  ) : e.kind === "hearing" ? (
                    <Gavel className="size-3.5 shrink-0 text-amber-600" />
                  ) : (
                    <ShieldAlert className="size-3.5 shrink-0 text-amber-600" />
                  )}
                  <span className="min-w-0 flex-1 truncate">
                    {e.title}
                    {e.location && (
                      <span className="ml-1 text-muted-foreground">
                        · {e.location}
                      </span>
                    )}
                  </span>
                </button>

                {/* 飞书事件展开的操作面板 */}
                {e.source === "feishu" && expandedEvent === i && (
                  <div className="flex gap-1.5 rounded-b-md border border-t-0 border-border bg-muted/30 px-2 py-1.5">
                    {onImportFolder && (
                      <button
                        type="button"
                        onClick={() => {
                          onImportFolder(e.title);
                          setExpandedEvent(null);
                          setSelectedDate(null);
                        }}
                        className="inline-flex items-center gap-1 rounded px-2 py-1 text-caption text-foreground transition-colors hover:bg-foreground/10"
                      >
                        <FolderOpen className="size-3" />
                        导入案件文件夹
                      </button>
                    )}
                  </div>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}

      {/* 选中日期但无事件 */}
      {selectedDate && selectedEvents.length === 0 && (
        <div className="mt-3 border-t border-border pt-3 text-center text-caption text-muted-foreground">
          {selectedDate.getMonth() + 1}月{selectedDate.getDate()}日暂无事件
        </div>
      )}
    </div>
  );
}

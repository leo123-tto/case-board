import { useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  ArrowUpDown,
  CalendarClock,
  CalendarDays,
  Check,
  CheckSquare,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  FolderOpen,
  Gavel,
  GripVertical,
  LayoutGrid,
  List,
  Plus,
  Search,
  ShieldAlert,
  Square,
  Trash2,
  X,
} from "lucide-react";
import { toast } from "@/components/ui/toast";
import {
  DndContext,
  type DragEndEvent,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  rectSortingStrategy,
  useSortable,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";

import { Button } from "@/components/ui/button";
import { formatYuan } from "@/lib/format";
import {
  addCalendarEvent,
  type CalendarEvent,
  deleteCalendarEvent,
  deleteTodo,
  getCaseWithDocs,
  getSettings,
  listCalendarEvents,
  listOpenTodos,
  type OpenTodoRow,
  readTextFile,
  updateHomeCaseOrder,
  updateCalendarEvent,
  updateCaseCalendarEventOverride,
  updateTodo,
  updateWorkflowStatus,
} from "@/lib/api";
import { isCriminalCase } from "@/lib/caseDomain";
import {
  ttStatus,
  ttListItems,
  ttToggleItem,
  ttAddItem,
  ttDeleteItem,
  type TickTickItem,
} from "@/lib/ticktickApi";
import type { Case, Document } from "@/lib/types";
import { parseJsonArray } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useFeatureFlag } from "@/lib/featureFlags";
import { CalendarBoard } from "./CalendarBoard";
import {
  CalendarEventActions,
  type CalendarEventEditInput,
} from "./CalendarEventActions";
import { DashboardAssistantCard } from "./DashboardAssistantCard";
import { buildDashboardAssistantContext } from "./dashboardAssistantContext";
import { countOpenCaseRows, isOpenCaseStatus } from "./homeCaseCounts";
import {
  loadHomeListPreferences,
  saveHomeListPreferences,
  type HomeSortDir,
  type HomeSortKey,
  type HomeViewMode,
} from "./homeListPreferences";
import type { DailyBrief } from "./HomeCompanionStrip";
import {
  loadHearingDisplayDetail,
  type HearingDisplayDetail,
} from "./homeHearingDetails";
import {
  extractPreservationTextInfo,
  type PreservationTextInfo,
} from "./homePreservationEvents";
import {
  buildCaseCalendarEvents,
  buildImportantCaseReminders,
  diffDays,
  eventUrgency,
  formatReminderCountdown,
  isPreservationOrUnsealDoc,
  parseDate,
  PRESERVATION_RE,
  todayDate,
  type HomeReminderEvent,
} from "./homeReminderEngine";
import {
  compareCasesByStatusThenTime,
  resolveCaseStatus,
  STATUS_LIST,
  type StatusDef,
  type StatusId,
} from "@/modules/litigation/lib/inferStatus";

export interface HomeViewProps {
  cases: Case[];
  userDisplayName: string | null;
  onPickCase: (caseId: string) => void;
  onOpenEvent?: (event: UpcomingEvent) => void;
  onImport: () => void;
  /** 右键卡片「删除」→ 删除案件(只删数据库记录,不动原始文件夹)。由 App 弹确认 + 刷新列表。 */
  onDeleteCase: (caseId: string) => void;
  /** 批量删除选中案件(筛选工具栏「多选」模式)。由 App 弹一次确认 + 逐个删 + 刷新列表。 */
  onDeleteCases: (caseIds: string[]) => void;
  /** 状态修改成功后通知顶层案件列表同步,避免其他模块拿旧 workflow_status。 */
  onCaseStatusChanged?: (caseId: string, status: StatusId | null) => void;
  /** 飞书日历:点日历事件后导入对应案件文件夹(反查案件池表→有则直接导,否则弹选择器)。 */
  onImportFolder?: (eventTitle: string) => void;
}

type ViewMode = HomeViewMode;
type SortKey = HomeSortKey;
type SortDir = HomeSortDir;

interface CaseDisplayFields {
  caseNo: string | null;
  court: string | null;
  cause: string | null;
  claimAmount: number | null;
  plaintiffs: string[];
  defendants: string[];
  judges: string[];
  partySummary: string;
  amountText: string | null;
}

interface CaseRow {
  caseData: Case;
  status: StatusDef;
  display: CaseDisplayFields;
  nearestHearing: string | null;
}

export type UpcomingEvent = HomeReminderEvent;

export function HomeView({
  cases,
  userDisplayName,
  onPickCase,
  onOpenEvent,
  onImport,
  onDeleteCase,
  onDeleteCases,
  onCaseStatusChanged,
  onImportFolder,
}: HomeViewProps) {
  const greeting = getGreeting(userDisplayName);
  const monthLabel = new Date()
    .toLocaleString("en-US", { month: "short", year: "numeric" })
    .toUpperCase();

  const [docsByCase, setDocsByCase] = useState<Record<string, Document[]>>({});
  const [statusOverride, setStatusOverride] = useState<Record<string, StatusId | null>>({});
  const [userOrder, setUserOrder] = useState<string[] | null>(null);
  const [initialListPreferences] = useState(loadHomeListPreferences);
  const [viewMode, setViewMode] = useState<ViewMode>(initialListPreferences.viewMode);
  const [sortKey, setSortKey] = useState<SortKey>(initialListPreferences.sortKey);
  const [sortDir, setSortDir] = useState<SortDir>(initialListPreferences.sortDir);
  const [statusFilters, setStatusFilters] = useState<Set<StatusId>>(new Set());
  const [courtFilter, setCourtFilter] = useState("");
  // 2026-06-16 · 首页模糊搜索(原告/被告名,公司或人名都可子串匹配)
  const [search, setSearch] = useState("");
  // 带日期的待办 → 汇入日程日历(2026-06-14:手动日程 = 带日期的待办)
  const [openTodos, setOpenTodos] = useState<OpenTodoRow[]>([]);
  // 独立日历日程(不绑案件,日历右键添加)
  const [manualEvents, setManualEvents] = useState<CalendarEvent[]>([]);
  const [calendarOverrideJsonByCase, setCalendarOverrideJsonByCase] = useState<
    Record<string, string | null>
  >({});
  // 日程日历功能开关(默认关闭,设置里手动开)
  const [calendarEnabled, setCalendarEnabled] = useState(false);
  // 飞书日历开关(法律工具→飞书日历卡里开;开了用飞书月历替代本地日程卡)
  const [feishuEnabled, setFeishuEnabled] = useState(false);
  const [hearingDetails, setHearingDetails] = useState<
    Record<string, HearingDisplayDetail>
  >({});
  const [preservationTextInfo, setPreservationTextInfo] = useState<
    Record<string, PreservationTextInfo>
  >({});
  // 2026-06-16 · 首页清爽开关(设置页「功能开关」tab,默认关,逐设备生效)
  const [filterBarOn] = useFeatureFlag("home_filter_bar");
  const [homeCompanionOn] = useFeatureFlag("home_companion");
  const [ticktickOn] = useFeatureFlag("home_ticktick");

  useEffect(() => {
    saveHomeListPreferences({ viewMode, sortKey, sortDir });
  }, [sortDir, sortKey, viewMode]);

  const reloadManualEvents = () => {
    listCalendarEvents()
      .then(setManualEvents)
      .catch(() => {});
  };
  const handleAddCalendarEvent = async (date: string, title: string) => {
    const t = title.trim();
    if (!t) return;
    try {
      await addCalendarEvent({ date, title: t });
      reloadManualEvents();
    } catch (e) {
      alert(`添加日程失败:${e}`);
    }
  };
  const handleEditCalendarEvent = async (
    event: UpcomingEvent,
    input: CalendarEventEditInput,
  ) => {
    if (event.kind === "manual" && event.id) {
      const updated = await updateCalendarEvent(event.id, input);
      setManualEvents((prev) => prev.map((row) => (row.id === updated.id ? updated : row)));
      return;
    }
    if (event.kind === "todo" && event.id) {
      await updateTodo(event.id, {
        title: input.title,
        due_date: input.date,
        note: input.note,
      });
      setOpenTodos((prev) =>
        prev.map((row) =>
          row.id === event.id
            ? { ...row, title: input.title, due_date: input.date, note: input.note }
            : row,
        ),
      );
      return;
    }
    const nextJson = await updateCaseCalendarEventOverride({
      caseId: event.caseId,
      sourceKey: event.sourceKey,
      rowKey: event.rowKey ?? null,
      date: input.date,
      title: input.title,
      note: input.note,
    });
    setCalendarOverrideJsonByCase((prev) => ({ ...prev, [event.caseId]: nextJson }));
  };
  const handleDeleteCalendarEvent = async (event: UpcomingEvent) => {
    if (event.kind === "manual" && event.id) {
      await deleteCalendarEvent(event.id);
      setManualEvents((prev) => prev.filter((row) => row.id !== event.id));
      return;
    }
    if (event.kind === "todo" && event.id) {
      await deleteTodo(event.id);
      setOpenTodos((prev) => prev.filter((row) => row.id !== event.id));
      return;
    }
    const nextJson = await updateCaseCalendarEventOverride({
      caseId: event.caseId,
      sourceKey: event.sourceKey,
      rowKey: event.rowKey ?? null,
      hidden: true,
    });
    setCalendarOverrideJsonByCase((prev) => ({ ...prev, [event.caseId]: nextJson }));
  };

  useEffect(() => {
    let cancelled = false;
    Promise.all(
      cases.map(async (c) => {
        try {
          const r = await getCaseWithDocs(c.id);
          return [c.id, r.documents] as const;
        } catch {
          return [c.id, [] as Document[]] as const;
        }
      }),
    ).then((pairs) => {
      if (!cancelled) setDocsByCase(Object.fromEntries(pairs));
    });
    return () => {
      cancelled = true;
    };
  }, [cases]);

  useEffect(() => {
    let cancelled = false;
    listOpenTodos()
      .then((r) => {
        if (!cancelled) setOpenTodos(r);
      })
      .catch(() => {});
    listCalendarEvents()
      .then((r) => {
        if (!cancelled) setManualEvents(r);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [cases]);

  useEffect(() => {
    let cancelled = false;
    getSettings()
      .then((s) => {
        if (cancelled) return;
        setUserOrder(s.home_case_order);
        setCalendarEnabled(s.home_calendar_enabled);
        setFeishuEnabled(s.feishu_enabled === true);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  const casesWithOverride = cases.map((c) =>
    c.id in statusOverride
      ? {
          ...c,
          workflow_status: statusOverride[c.id],
          workflow_status_locked: statusOverride[c.id] == null ? 0 : 1,
        }
      : c,
  );

  const caseRows = useMemo<CaseRow[]>(
    () =>
      casesWithOverride.map((c) => ({
        caseData: c,
        status: resolveCaseStatus(c, docsByCase[c.id] ?? []),
        display: buildCaseDisplay(c),
        nearestHearing: findNearestFutureHearing(c),
      })),
    [casesWithOverride, docsByCase],
  );

  const defaultSorted = [...caseRows].sort((a, b) =>
    compareCasesByStatusThenTime(
      a.status.id,
      a.caseData.updated_at,
      b.status.id,
      b.caseData.updated_at,
    ),
  );

  // 用户拖过 → 按 userOrder 重排,没排过的(新案件 / userOrder 没覆盖到的)按默认顺序追加。
  // 已删的 case id 留在 userOrder 里也无害(idMap 找不到自动 filter)。
  const userOrderedRows = (() => {
    let ordered = defaultSorted;
    if (userOrder && userOrder.length > 0) {
      const byId = new Map(defaultSorted.map((row) => [row.caseData.id, row]));
      const result: CaseRow[] = [];
      const seen = new Set<string>();
      for (const id of userOrder) {
        const row = byId.get(id);
        if (row && !seen.has(id)) {
          result.push(row);
          seen.add(id);
        }
      }
      for (const row of defaultSorted) {
        if (!seen.has(row.caseData.id)) {
          result.push(row);
          seen.add(row.caseData.id);
        }
      }
      ordered = result;
    }
    // 2026-06-13(胡彬律师反馈):已结案的一律沉到最后 —— 即便用户之前把它拖到了前面。
    // 稳定分区:非结案保持原顺序在前,结案保持原顺序在后。
    const active = ordered.filter((row) => row.status.id !== "closed");
    const closed = ordered.filter((row) => row.status.id === "closed");
    return [...active, ...closed];
  })();

  // 2026-06-16 · 筛选工具栏关闭(清爽默认)时:强制 grid + 默认序、绕过筛选。
  // 用 effective 值保证「关闭 = grid+status+asc」→ canUseUserOrder 为真,拖拽仍能持久化。
  const effViewMode = filterBarOn ? viewMode : "grid";
  const effSortKey = filterBarOn ? sortKey : "status";
  const effSortDir = filterBarOn ? sortDir : "asc";
  const canUseUserOrder =
    effViewMode === "grid" && effSortKey === "status" && effSortDir === "asc";
  const sortedRows = canUseUserOrder
    ? userOrderedRows
    : [...caseRows].sort((a, b) => compareCaseRows(a, b, effSortKey, effSortDir));

  const courtOptions = Array.from(
    new Set(caseRows.map((row) => row.display.court).filter(Boolean) as string[]),
  ).sort((a, b) => a.localeCompare(b, "zh-Hans-CN"));

  const searchQuery = search.trim().toLowerCase();
  const filteredRows = sortedRows.filter((row) => {
    if (!filterBarOn) return true; // 工具栏关闭 → 不过滤,显示全部案件
    if (statusFilters.size > 0 && !statusFilters.has(row.status.id)) return false;
    if (courtFilter && row.display.court !== courtFilter) return false;
    // 模糊搜索:原告/被告名(公司或人名)+ 当事人摘要,子串匹配(不分大小写)
    if (searchQuery) {
      const hay = [
        ...row.display.plaintiffs,
        ...row.display.defendants,
        row.display.partySummary,
      ]
        .join(" ")
        .toLowerCase();
      if (!hay.includes(searchQuery)) return false;
    }
    return true;
  });
  const openCaseCount = countOpenCaseRows(caseRows);
  const visibleOpenCaseCount = countOpenCaseRows(filteredRows);

  const activeCases = defaultSorted
    .filter(({ status }) => isOpenCaseStatus(status.id) && status.id !== "mediated")
    .map(({ caseData }) => caseData);
  const activeCasesForReminders = activeCases.map((caseData) =>
    Object.prototype.hasOwnProperty.call(calendarOverrideJsonByCase, caseData.id)
      ? { ...caseData, user_overrides_json: calendarOverrideJsonByCase[caseData.id] }
      : caseData,
  );
  const calendarCases = defaultSorted
    .filter(({ status }) => isOpenCaseStatus(status.id))
    .map(({ caseData }) =>
      Object.prototype.hasOwnProperty.call(calendarOverrideJsonByCase, caseData.id)
        ? { ...caseData, user_overrides_json: calendarOverrideJsonByCase[caseData.id] }
        : caseData,
    );
  const upcomingEventsBase = buildImportantCaseReminders(
    activeCasesForReminders,
    docsByCase,
    preservationTextInfo,
  );
  const upcomingEvents = useMemo(
    () =>
      upcomingEventsBase.map((event) => ({
        ...event,
        ...(hearingDetails[upcomingEventKey(event)] ?? {}),
      })),
    [hearingDetails, upcomingEventsBase],
  );
  const assistantReminderSummaries = useMemo(
    () => buildAssistantReminderSummaries(upcomingEvents),
    [upcomingEvents],
  );
  const dailyBriefContext = useMemo(
    () => buildHomeDailyBrief(upcomingEvents, activeCases, docsByCase),
    [activeCases, docsByCase, upcomingEvents],
  );
  const dashboardAssistantContext = useMemo(
    () =>
      buildDashboardAssistantContext({
        cases: caseRows.map(({ caseData, status }) => ({
          statusId: status.id,
          statusLabel: status.label,
          isCriminal: isCriminalCase(caseData),
        })),
        documents: Object.values(docsByCase)
          .flat()
          .map((document) => ({
            extractionStatus: document.extraction_status,
            deleted: !!document.deleted_at,
          })),
        openTodoCount: openTodos.length,
        reminderUrgencies: upcomingEvents.map((event) => eventUrgency(event)),
        snapshotComplete: cases.every((caseData) => caseData.id in docsByCase),
      }),
    [caseRows, cases, docsByCase, openTodos.length, upcomingEvents],
  );
  const hearingEventsSeed = upcomingEventsBase
    .filter((event) => event.kind === "hearing")
    .map(upcomingEventKey)
    .join("||");
  const calendarEvents = [
    ...buildCaseCalendarEvents(calendarCases, docsByCase, preservationTextInfo),
    ...buildTodoEvents(openTodos),
    ...buildManualEvents(manualEvents),
  ];
  const openEvent = (event: UpcomingEvent) => {
    if (!event.caseId) return;
    if (onOpenEvent) onOpenEvent(event);
    else onPickCase(event.caseId);
  };

  function buildAssistantReminderSummaries(events: UpcomingEvent[]): string[] {
    return events.slice(0, 4).map((event) => {
      const distance =
        event.daysFromNow < 0
          ? `已过 ${Math.abs(event.daysFromNow)} 天`
          : event.daysFromNow === 0
            ? "今天"
            : `${event.daysFromNow} 天后`;
      const label =
        event.kind === "hearing"
          ? "开庭"
          : event.type?.trim() || (event.kind === "deadline" ? "期限" : "日程");
      const urgency = eventUrgency(event);
      const urgentText = urgency === "urgent" ? "紧急" : urgency === "overdue" ? "逾期" : "提醒";
      return `${urgentText} · ${distance} · ${formatShortDate(event.date)} · ${label}`;
    });
  }

  function formatShortDate(date: string): string {
    const match = date.match(/^\d{4}-(\d{2})-(\d{2})$/);
    return match ? `${match[1]}/${match[2]}` : date;
  }

  useEffect(() => {
    let cancelled = false;
    const hearingEvents = upcomingEventsBase.filter((event) => event.kind === "hearing");
    if (hearingEvents.length === 0) {
      setHearingDetails({});
      return () => {
        cancelled = true;
      };
    }

    (async () => {
      const pairs = await Promise.all(
        hearingEvents.map(async (event) => {
          const detail = await loadHearingDisplayDetail(
            docsByCase[event.caseId] ?? [],
            event.date,
            readTextFile,
            event.court,
          );
          return [upcomingEventKey(event), detail] as const;
        }),
      );
      if (cancelled) return;
      const next = Object.fromEntries(
        pairs.filter((entry): entry is [string, HearingDisplayDetail] => !!entry[1]),
      );
      setHearingDetails(next);
    })();

    return () => {
      cancelled = true;
    };
  }, [docsByCase, hearingEventsSeed]);

  useEffect(() => {
    let cancelled = false;
    const candidates = Object.values(docsByCase)
      .flat()
      .filter((doc) => doc.extracted_text_path && isPreservationOrUnsealDoc(doc));
    if (candidates.length === 0) {
      setPreservationTextInfo({});
      return () => {
        cancelled = true;
      };
    }

    (async () => {
      const pairs = await Promise.all(
        candidates.map(async (doc) => {
          try {
            const raw = await readTextFile(doc.extracted_text_path!);
            const info = extractPreservationTextInfo(raw);
            return [doc.id, info] as const;
          } catch {
            return [doc.id, { schedules: [], unsealDate: null }] as const;
          }
        }),
      );
      if (!cancelled) setPreservationTextInfo(Object.fromEntries(pairs));
    })();

    return () => {
      cancelled = true;
    };
  }, [docsByCase]);

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
  );
  const sortedIds = filteredRows.map((row) => row.caseData.id);

  const handleChangeStatus = async (caseId: string, status: StatusId | null) => {
    setStatusOverride((m) => ({ ...m, [caseId]: status }));
    try {
      await updateWorkflowStatus(caseId, status);
      onCaseStatusChanged?.(caseId, status);
      if (status === "closed") {
        toast(
          "案件已结案。可进详情页点「沉淀为办案经验」存入知识库,日后同类案可检索复用",
          "info",
        );
      }
    } catch (e) {
      console.warn("updateWorkflowStatus failed", e);
    }
  };

  const handleDragEnd = async (event: DragEndEvent) => {
    if (!canUseUserOrder) return;
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const oldIdx = sortedIds.indexOf(String(active.id));
    const newIdx = sortedIds.indexOf(String(over.id));
    if (oldIdx === -1 || newIdx === -1) return;
    const newOrder = arrayMove(sortedIds, oldIdx, newIdx);
    setUserOrder(newOrder);
    try {
      await updateHomeCaseOrder(newOrder);
    } catch (e) {
      console.warn("updateHomeCaseOrder failed", e);
    }
  };

  const toggleStatusFilter = (id: StatusId) => {
    setStatusFilters((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const clearFilters = () => {
    setStatusFilters(new Set());
    setCourtFilter("");
    setSearch("");
  };

  // 批量多选删除(只在筛选工具栏打开时可用;首页默认简洁状态不出现)。
  const [selectMode, setSelectMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const effSelectMode = filterBarOn && selectMode;

  // 工具栏关闭 → 退出多选并清空选择(批量删除只属于工具栏)。
  useEffect(() => {
    if (!filterBarOn) {
      setSelectMode(false);
      setSelectedIds(new Set());
    }
  }, [filterBarOn]);

  // 案件列表变化(如删除后刷新)→ 剔除已不存在的选中 id,避免悬空。
  useEffect(() => {
    setSelectedIds((prev) => {
      if (prev.size === 0) return prev;
      const live = new Set(cases.map((c) => c.id));
      const next = new Set([...prev].filter((id) => live.has(id)));
      return next.size === prev.size ? prev : next;
    });
  }, [cases]);

  const toggleSelect = (id: string) =>
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  // 右键卡片菜单(目前只有「删除」)。位置用鼠标坐标 fixed 定位,点别处 / 滚动 / Esc 关闭。
  const [ctxMenu, setCtxMenu] = useState<{
    id: string;
    name: string;
    x: number;
    y: number;
  } | null>(null);

  const openCtxMenu = (e: React.MouseEvent, row: CaseRow) => {
    e.preventDefault();
    e.stopPropagation();
    // 简单防溢出:菜单宽约 160 / 高约 90,贴近右下边缘时往内收
    const x = Math.min(e.clientX, window.innerWidth - 168);
    const y = Math.min(e.clientY, window.innerHeight - 96);
    setCtxMenu({
      id: row.caseData.id,
      name: row.display.cause || row.caseData.name,
      x,
      y,
    });
  };

  useEffect(() => {
    if (!ctxMenu) return;
    const close = () => setCtxMenu(null);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setCtxMenu(null);
    };
    // 捕获阶段监听:任何点击 / 右键 / 滚动都关菜单(菜单项自身的点击在 onClick 里先执行)
    window.addEventListener("click", close);
    window.addEventListener("contextmenu", close);
    window.addEventListener("scroll", close, true);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("contextmenu", close);
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("keydown", onKey);
    };
  }, [ctxMenu]);

  return (
    <main className="app-shell flex h-full w-full flex-col">
      <header className="app-subheader border-b px-4 py-3 sm:px-6 xl:px-8">
        <div className="mx-auto flex max-w-6xl items-center">
          <h1 className="text-sm font-semibold tracking-tight text-foreground">案件看板</h1>
        </div>
      </header>

      <div className="flex-1 overflow-auto">
        <div className="app-page-enter mx-auto max-w-6xl px-4 py-6 sm:px-6 xl:px-8 xl:py-8">
          <div className="mb-10 grid grid-cols-1 gap-6 md:grid-cols-2">
            {homeCompanionOn ? (
              <DashboardAssistantCard
                greeting={greeting}
                monthLabel={monthLabel}
                openCaseCount={openCaseCount}
                displayName={userDisplayName}
                activeCaseCount={activeCases.length}
                assistantContext={dashboardAssistantContext}
                reminderSummaries={assistantReminderSummaries}
                dailyBrief={dailyBriefContext.brief}
                onDailyBriefAction={() => {
                  if (dailyBriefContext.actionEvent) {
                    openEvent(dailyBriefContext.actionEvent);
                  } else if (dailyBriefContext.actionCaseId) {
                    onPickCase(dailyBriefContext.actionCaseId);
                  } else {
                    onImport();
                  }
                }}
              />
            ) : (
              <div className="overflow-hidden rounded-xl border border-brand/10 bg-brand-soft/55 p-5 shadow-[inset_0_1px_0_oklch(1_0_0/0.72)]">
                <p className="font-mono text-caption uppercase tracking-wider text-brand">
                  OVERVIEW · {monthLabel}
                </p>
                <h1 className="mt-2 text-3xl font-semibold tracking-tight text-foreground xl:text-4xl">
                  {greeting}
                </h1>
                <p className="mt-2 text-sm text-muted-foreground">
                  你正在办 {openCaseCount} 个案件,扫一眼今天的进度。
                </p>
              </div>
            )}
            <ImportantDates events={upcomingEvents} onPickCase={openEvent} />
          </div>

          {/* 飞书日历开启 → 月历视图(替代本地日程日历卡);否则按本地开关显示原日程卡 */}
          {cases.length > 0 && feishuEnabled && (
            <div className="mb-8">
              <CalendarBoard
                localEvents={upcomingEvents}
                onPickCase={onPickCase}
                onImportFolder={onImportFolder}
              />
            </div>
          )}

          {cases.length > 0 && !feishuEnabled && calendarEnabled && (
            <div className="mb-8">
              <CalendarPanel
                events={calendarEvents}
                onPickCase={openEvent}
                onAddEvent={handleAddCalendarEvent}
                onEditEvent={handleEditCalendarEvent}
                onDeleteEvent={handleDeleteCalendarEvent}
              />
            </div>
          )}

          {/* 待办两卡:左=案件待办汇总,右=我的待办(滴答同步)。整个「在办案件」区上方;各自空/未连接时自动隐藏。
              右卡(滴答)受「功能开关」tab 的 home_ticktick 控制(默认关=清爽)。 */}
          <div className="mb-6 grid grid-cols-1 gap-4 md:grid-cols-2">
            <TodoSummary onPickCase={onPickCase} />
            {ticktickOn && <MyTodosCard />}
          </div>

          <section>
            <div className="mb-4 flex flex-col gap-3">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="flex items-baseline gap-3">
                  <h2 className="text-lg font-semibold tracking-tight">在办案件</h2>
                  <span className="font-mono text-caption uppercase tracking-wider text-muted-foreground">
                    {visibleOpenCaseCount} / {cases.length} CASES
                  </span>
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  <Button type="button" size="sm" onClick={onImport}>
                    <FolderOpen className="size-3.5" />
                    导入案件
                  </Button>
                  {filterBarOn && (
                    <>
                      <IconToggle
                        active={viewMode === "grid"}
                        label="卡片视图"
                        onClick={() => setViewMode("grid")}
                      >
                        <LayoutGrid className="size-3.5" />
                      </IconToggle>
                      <IconToggle
                        active={viewMode === "list"}
                        label="列表视图"
                        onClick={() => setViewMode("list")}
                      >
                        <List className="size-3.5" />
                      </IconToggle>
                      <IconToggle
                        active={selectMode}
                        label="多选删除"
                        onClick={() => {
                          setSelectMode((on) => !on);
                          setSelectedIds(new Set());
                        }}
                      >
                        <CheckSquare className="size-3.5" />
                      </IconToggle>
                    </>
                  )}
                </div>
              </div>

              {filterBarOn && (
              <div className="surface-card flex flex-wrap items-center gap-2 bg-card/70 p-3">
                <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
                  排序
                  <select
                    value={sortKey}
                    onChange={(e) => setSortKey(e.target.value as SortKey)}
                    className="rounded-md border border-border bg-background px-2 py-1 text-xs text-foreground"
                  >
                    <option value="status">按状态</option>
                    <option value="amount">按诉讼金额</option>
                    <option value="filed_at">按立案时间</option>
                    <option value="hearing">按最近开庭日</option>
                  </select>
                </label>
                {/* 与左右两个 select 同尺寸(px-2 py-1 text-xs),别用 Button size=sm(更高) */}
                <button
                  type="button"
                  onClick={() => setSortDir((d) => (d === "asc" ? "desc" : "asc"))}
                  className="flex items-center gap-1 rounded-md border border-border bg-background px-2 py-1 text-xs text-foreground transition-colors hover:bg-accent"
                >
                  <ArrowUpDown className="size-3.5" />
                  {sortDir === "asc" ? "升序" : "降序"}
                </button>
                <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
                  法院
                  <select
                    value={courtFilter}
                    onChange={(e) => setCourtFilter(e.target.value)}
                    className="max-w-44 rounded-md border border-border bg-background px-2 py-1 text-xs text-foreground"
                  >
                    <option value="">全部</option>
                    {courtOptions.map((court) => (
                      <option key={court} value={court}>
                        {court}
                      </option>
                    ))}
                  </select>
                </label>
                <div className="flex flex-wrap items-center gap-1">
                  {STATUS_LIST.map((s) => (
                    <button
                      key={s.id}
                      type="button"
                      onClick={() => toggleStatusFilter(s.id)}
                      className={cn(
                        "rounded-full px-2 py-1 text-caption font-medium transition-opacity hover:opacity-80",
                        statusFilters.has(s.id) ? s.color : "bg-muted text-muted-foreground",
                      )}
                    >
                      {s.label}
                    </button>
                  ))}
                </div>
                {(statusFilters.size > 0 || courtFilter || search.trim()) && (
                  <Button type="button" variant="ghost" size="sm" onClick={clearFilters}>
                    <X className="size-3.5" />
                    清空筛选
                  </Button>
                )}
                {/* 模糊搜索:放排序那一排的最后,搜原告/被告名(公司或人名,子串匹配) */}
                <div className="relative ml-auto">
                  <Search className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
                  <input
                    type="text"
                    value={search}
                    onChange={(e) => setSearch(e.target.value)}
                    placeholder="搜原告/被告名…"
                    className="h-7 w-44 rounded-md border border-border bg-background pl-7 pr-6 text-xs text-foreground placeholder:text-muted-foreground/60 focus:border-foreground focus:outline-none focus:ring-1 focus:ring-foreground/20"
                  />
                  {search && (
                    <button
                      type="button"
                      onClick={() => setSearch("")}
                      aria-label="清除搜索"
                      className="absolute right-1.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                    >
                      <X className="size-3.5" />
                    </button>
                  )}
                </div>
                <p className="w-full text-[11px] leading-relaxed text-muted-foreground">
                  点上方 <CheckSquare className="mb-0.5 inline size-3" />
                  「多选」只删除看板记录，不动原始文件夹。
                </p>
              </div>
              )}

              {/* 多选操作条:仅在「多选」模式出现 */}
              {effSelectMode &&
                (() => {
                  const visibleIds = filteredRows.map((r) => r.caseData.id);
                  const selectedVisible = visibleIds.filter((id) =>
                    selectedIds.has(id),
                  );
                  const allVisibleSelected =
                    visibleIds.length > 0 &&
                    selectedVisible.length === visibleIds.length;
                  return (
                    <div className="flex flex-wrap items-center gap-2 rounded-xl border border-brand/20 bg-brand-soft/75 px-3 py-2.5">
                      <span className="text-sm font-medium text-foreground">
                        已选 {selectedVisible.length} 个
                      </span>
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        onClick={() =>
                          setSelectedIds(
                            allVisibleSelected ? new Set() : new Set(visibleIds),
                          )
                        }
                      >
                        {allVisibleSelected ? "取消全选" : "全选(当前可见)"}
                      </Button>
                      <button
                        type="button"
                        disabled={selectedVisible.length === 0}
                        onClick={() => onDeleteCases(selectedVisible)}
                        className="inline-flex items-center gap-1.5 rounded-md bg-red-600 px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-red-700 disabled:opacity-40"
                      >
                        <Trash2 className="size-3.5" />
                        删除所选
                      </button>
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        onClick={() => {
                          setSelectMode(false);
                          setSelectedIds(new Set());
                        }}
                      >
                        退出多选
                      </Button>
                    </div>
                  );
                })()}

            </div>

            {cases.length === 0 ? (
              <EmptyCases onImport={onImport} />
            ) : filteredRows.length === 0 ? (
              <div className="rounded-xl border border-dashed border-border bg-card/45 px-6 py-12 text-center text-sm text-muted-foreground">
                没有符合筛选条件的案件
              </div>
            ) : effViewMode === "list" ? (
              <div className="surface-card overflow-hidden">
                {filteredRows.map((row) => (
                  <CaseListRow
                    key={row.caseData.id}
                    row={row}
                    onClick={() => onPickCase(row.caseData.id)}
                    onChangeStatus={(s) => handleChangeStatus(row.caseData.id, s)}
                    onContextMenu={(e) => openCtxMenu(e, row)}
                    selectMode={effSelectMode}
                    selected={selectedIds.has(row.caseData.id)}
                    onToggleSelect={() => toggleSelect(row.caseData.id)}
                  />
                ))}
              </div>
            ) : (
              <DndContext
                sensors={sensors}
                collisionDetection={closestCenter}
                onDragEnd={handleDragEnd}
              >
                <SortableContext items={sortedIds} strategy={rectSortingStrategy}>
                  <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
                    {filteredRows.map((row) => (
                      <SortableCaseCard
                        key={row.caseData.id}
                        row={row}
                        onClick={() => onPickCase(row.caseData.id)}
                        onChangeStatus={(s) => handleChangeStatus(row.caseData.id, s)}
                        onContextMenu={(e) => openCtxMenu(e, row)}
                        selectMode={effSelectMode}
                        selected={selectedIds.has(row.caseData.id)}
                        onToggleSelect={() => toggleSelect(row.caseData.id)}
                      />
                    ))}
                  </div>
                </SortableContext>
              </DndContext>
            )}
          </section>
        </div>
      </div>

      {ctxMenu && (
        <div
          className="fixed z-[200] min-w-[160px] overflow-hidden rounded-lg border border-border bg-popover py-1 shadow-xl"
          style={{ top: ctxMenu.y, left: ctxMenu.x }}
          // 菜单自身的右键/点击不冒泡到 window 的关闭监听之外的逻辑(关闭仍由 window 捕获处理)
          onContextMenu={(e) => e.preventDefault()}
        >
          <button
            type="button"
            onClick={() => {
              const id = ctxMenu.id;
              setCtxMenu(null);
              onPickCase(id);
            }}
            className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-foreground hover:bg-accent"
          >
            <FolderOpen className="size-4 text-muted-foreground" />
            打开案件
          </button>
          <button
            type="button"
            onClick={() => {
              const id = ctxMenu.id;
              setCtxMenu(null);
              // 2026-06-23 v0.3.26.1 Windows fix:右键菜单关闭动画期间 onDeleteCase 立刻
              // 弹 confirmDialog,Windows WebView 模态对话框会被菜单收尾动画遮挡/时序竞态
              // 吞掉事件(用户看到菜单关了、对话框没弹)。延一帧让菜单先彻底关。
              setTimeout(() => onDeleteCase(id), 0);
            }}
            className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-red-600 hover:bg-red-50 dark:hover:bg-red-950/40"
          >
            <Trash2 className="size-4" />
            从看板删除
          </button>
        </div>
      )}
    </main>
  );
}

function IconToggle({
  active,
  label,
  onClick,
  children,
}: {
  active: boolean;
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={cn(
        "inline-flex size-8 items-center justify-center rounded-md border border-border transition-colors",
        active ? "bg-foreground text-background" : "bg-background text-muted-foreground hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}

function SortableCaseCard(props: {
  row: CaseRow;
  onClick: () => void;
  onChangeStatus: (s: StatusId | null) => void;
  onContextMenu: (e: React.MouseEvent) => void;
  selectMode: boolean;
  selected: boolean;
  onToggleSelect: () => void;
}) {
  const [isStatusPickerOpen, setIsStatusPickerOpen] = useState(false);
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: props.row.caseData.id });
  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
    // 卡片 hover 的 transform 会创建独立 stacking context。状态菜单展开时抬高
    // 整个 grid item，避免后面的卡片在 hover 后覆盖菜单。
    zIndex: isDragging ? 10 : isStatusPickerOpen ? 20 : undefined,
  };
  return (
    <div ref={setNodeRef} style={style} className="relative">
      <CaseCard
        {...props}
        dragHandleProps={{ attributes, listeners }}
        isDragging={isDragging}
        onStatusPickerOpenChange={setIsStatusPickerOpen}
      />
    </div>
  );
}

function CaseCard({
  row,
  onClick,
  onChangeStatus,
  onContextMenu,
  selectMode,
  selected,
  onToggleSelect,
  dragHandleProps,
  isDragging,
  onStatusPickerOpenChange,
}: {
  row: CaseRow;
  onClick: () => void;
  onChangeStatus: (s: StatusId | null) => void;
  onContextMenu?: (e: React.MouseEvent) => void;
  selectMode?: boolean;
  selected?: boolean;
  onToggleSelect?: () => void;
  dragHandleProps?: {
    attributes: ReturnType<typeof useSortable>["attributes"];
    listeners: ReturnType<typeof useSortable>["listeners"];
  };
  isDragging?: boolean;
  onStatusPickerOpenChange?: (open: boolean) => void;
}) {
  const { caseData, status, display } = row;
  const isClosed = status.id === "closed";
  // 多选模式:整卡点击 = 勾选/取消(不进详情);非多选 = 打开案件
  const handleActivate = selectMode ? onToggleSelect : onClick;
  return (
    <div
      className={cn(
        "interactive-surface group relative flex cursor-pointer flex-col rounded-xl border border-border bg-card/92 p-5 text-left shadow-sm hover:border-brand/25 hover:bg-card",
        isDragging && "border-dashed",
        isClosed && "opacity-60",
        selectMode && selected && "border-brand bg-brand-soft/70 ring-2 ring-brand/25",
      )}
      onClick={handleActivate}
      onContextMenu={onContextMenu}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          handleActivate?.();
        }
      }}
      role="button"
      tabIndex={0}
      aria-label={
        selectMode
          ? `${selected ? "取消选择" : "选择"}案件 ${display.cause || caseData.name}`
          : `打开案件 ${display.cause || caseData.name}`
      }
    >
      {selectMode ? (
        <div className="absolute left-2.5 top-2.5 text-sky-500">
          {selected ? (
            <CheckSquare className="size-5" />
          ) : (
            <Square className="size-5 text-muted-foreground/50" />
          )}
        </div>
      ) : (
        dragHandleProps && (
          <button
            type="button"
            aria-label="拖动调整顺序"
            title="按住拖动调整卡片顺序"
            onClick={(e) => e.stopPropagation()}
            className="absolute left-1.5 top-1.5 cursor-grab touch-none rounded p-1 text-muted-foreground/30 opacity-20 transition-all hover:bg-accent hover:text-foreground group-hover:opacity-100 active:cursor-grabbing"
            {...dragHandleProps.attributes}
            {...dragHandleProps.listeners}
          >
            <GripVertical className="size-3.5" />
          </button>
        )
      )}

      <div className="absolute right-3 top-3">
        <StatusPicker
          status={status}
          isManual={caseData.workflow_status_locked === 1}
          onPick={onChangeStatus}
          onOpenChange={onStatusPickerOpenChange}
        />
      </div>

      <h3
        className={cn(
          "pr-16 text-lg font-semibold leading-tight text-foreground",
          selectMode && "pl-7",
        )}
      >
        {caseData.source_folder === "__DEMO__" && (
          <span className="mr-2 inline-flex items-center rounded bg-amber-100 px-1.5 py-0.5 text-caption font-medium text-amber-800 align-middle dark:bg-amber-900/40 dark:text-amber-200">
            示例
          </span>
        )}
        {display.cause || caseData.name}
      </h3>
      <p className="mt-1 text-sm text-muted-foreground">{display.partySummary}</p>
      <dl className="mt-4 grid grid-cols-2 gap-x-4 gap-y-2 text-xs">
        <Item label="案号" value={display.caseNo} mono />
        <Item
          label={caseData.agg_court_type === "仲裁委" ? "仲裁委" : "法院"}
          value={display.court}
        />
        <Item
          label={caseData.agg_court_type === "仲裁委" ? "仲裁员" : "承办法官"}
          value={display.judges.length > 0 ? display.judges.join("、") : null}
        />
        <Item label="诉讼金额" value={display.amountText} mono highlight />
      </dl>
      <div className="mt-4 flex items-center justify-between border-t border-border pt-3 text-caption text-muted-foreground">
        <span className="font-mono">{caseData.agg_computed_at ? "已抽取" : "抽取中..."}</span>
        <span className="inline-flex items-center gap-0.5 text-foreground/60 transition-colors group-hover:text-foreground">
          打开 <ChevronRight className="size-3" />
        </span>
      </div>
    </div>
  );
}

function CaseListRow({
  row,
  onClick,
  onChangeStatus,
  onContextMenu,
  selectMode,
  selected,
  onToggleSelect,
}: {
  row: CaseRow;
  onClick: () => void;
  onChangeStatus: (s: StatusId | null) => void;
  onContextMenu: (e: React.MouseEvent) => void;
  selectMode?: boolean;
  selected?: boolean;
  onToggleSelect?: () => void;
}) {
  const { caseData, status, display } = row;
  const handleActivate = selectMode ? onToggleSelect : onClick;
  return (
    <div
      className={cn(
        "grid cursor-pointer grid-cols-[minmax(0,1.35fr)_minmax(0,1fr)_minmax(0,1fr)_minmax(0,1fr)_auto] items-center gap-3 border-b border-border px-4 py-3 text-left transition-[transform,background-color] duration-150 last:border-b-0 hover:bg-brand-soft/45 active:scale-[0.997]",
        status.id === "closed" && "opacity-60",
        selectMode && selected && "bg-sky-50/70 dark:bg-sky-950/30",
      )}
      onClick={handleActivate}
      onContextMenu={onContextMenu}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          handleActivate?.();
        }
      }}
      role="button"
      tabIndex={0}
      aria-label={
        selectMode
          ? `${selected ? "取消选择" : "选择"}案件 ${display.cause || caseData.name}`
          : `打开案件 ${display.cause || caseData.name}`
      }
    >
      <div className="flex min-w-0 items-center gap-2">
        {selectMode &&
          (selected ? (
            <CheckSquare className="size-4 shrink-0 text-sky-500" />
          ) : (
            <Square className="size-4 shrink-0 text-muted-foreground/50" />
          ))}
        <div className="min-w-0">
          <div className="truncate text-sm font-semibold text-foreground">
            {display.cause || caseData.name}
          </div>
          <div className="truncate text-xs text-muted-foreground">{display.partySummary}</div>
        </div>
      </div>
      <div className="min-w-0 text-xs">
        <div className="truncate font-mono text-foreground">{display.caseNo || "-"}</div>
        <div className="truncate text-muted-foreground">{display.court || "-"}</div>
      </div>
      <div className="min-w-0 text-xs">
        <div className="truncate text-foreground">
          {display.judges.length > 0 ? display.judges.join("、") : "-"}
        </div>
        <div className="font-mono text-muted-foreground">{display.amountText || "-"}</div>
      </div>
      <StatusPicker
        status={status}
        isManual={caseData.workflow_status_locked === 1}
        onPick={onChangeStatus}
      />
      <span className="inline-flex items-center gap-0.5 text-caption text-muted-foreground">
        打开 <ChevronRight className="size-3" />
      </span>
    </div>
  );
}

function StatusPicker({
  status,
  isManual,
  onPick,
  onOpenChange,
}: {
  status: StatusDef;
  isManual: boolean;
  onPick: (s: StatusId | null) => void;
  onOpenChange?: (open: boolean) => void;
}) {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (!open) return;
    const onClick = () => {
      setOpen(false);
      onOpenChange?.(false);
    };
    window.addEventListener("click", onClick);
    return () => window.removeEventListener("click", onClick);
  }, [onOpenChange, open]);

  return (
    <div
      className={cn("relative inline-flex justify-end", open ? "z-50" : "z-10")}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => e.stopPropagation()}
    >
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          const nextOpen = !open;
          setOpen(nextOpen);
          onOpenChange?.(nextOpen);
        }}
        className={cn(
          "inline-flex items-center gap-1 rounded-full px-2.5 py-0.5 text-caption font-medium transition-opacity hover:opacity-80",
          status.color,
        )}
        title={isManual ? "手工设置 · 点击修改" : "自动推断 · 点击手工选择"}
      >
        {status.label}
        <ChevronDown className="size-3 opacity-70" />
      </button>
      {open && (
        <div className="absolute right-0 top-full z-20 mt-1 w-32 overflow-hidden rounded-md border border-border bg-card shadow-lg">
          {STATUS_LIST.map((s) => (
            <button
              key={s.id}
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                onPick(s.id);
                setOpen(false);
                onOpenChange?.(false);
              }}
              className="flex w-full items-center justify-between px-3 py-1.5 text-left text-xs hover:bg-accent"
            >
              <span className="flex items-center gap-1.5">
                <span className={cn("inline-block size-2 rounded-full", s.color.split(" ")[0])} />
                {s.label}
              </span>
              {s.id === status.id && <Check className="size-3 text-foreground" />}
            </button>
          ))}
          {isManual && (
            <>
              <div className="border-t border-border" />
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  onPick(null);
                  setOpen(false);
                  onOpenChange?.(false);
                }}
                className="block w-full px-3 py-1.5 text-left text-label text-muted-foreground hover:bg-accent hover:text-foreground"
              >
                恢复自动推断
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}

function Item({
  label,
  value,
  mono = false,
  highlight = false,
}: {
  label: string;
  value: string | null;
  mono?: boolean;
  highlight?: boolean;
}) {
  return (
    <div>
      <dt className="text-caption uppercase tracking-wider text-muted-foreground">{label}</dt>
      <dd
        className={cn(
          "mt-0.5 truncate text-foreground",
          mono && "font-mono",
          highlight && value && "font-semibold",
        )}
      >
        {value || <span className="text-muted-foreground/40">-</span>}
      </dd>
    </div>
  );
}

/**
 * 自动向上翻页滚动(2026-06-16:重要日期卡片反馈会无限变长 → 固定高度 + 自动翻页)。
 * 每 intervalMs 把滚动容器平滑向上翻一页(durationMs 缓动过渡,不瞬变);到底回到顶部循环。
 * 鼠标悬停时暂停(用户可手动上下滚动);内容不溢出时不滚。
 * 返回 pausedRef —— 调用方把它接到容器的 onMouseEnter/Leave 上。
 */
function useAutoPageScroll(
  ref: React.RefObject<HTMLDivElement | null>,
  { intervalMs = 5000, durationMs = 500 } = {},
) {
  const pausedRef = useRef(false);
  useEffect(() => {
    let raf = 0;
    const easeInOutQuad = (t: number) =>
      t < 0.5 ? 2 * t * t : -1 + (4 - 2 * t) * t;
    const animateTo = (el: HTMLDivElement, to: number) => {
      cancelAnimationFrame(raf);
      const start = el.scrollTop;
      const change = to - start;
      if (Math.abs(change) < 1) return;
      const t0 = performance.now();
      const step = (now: number) => {
        const t = Math.min(1, (now - t0) / durationMs);
        el.scrollTop = start + change * easeInOutQuad(t);
        if (t < 1) raf = requestAnimationFrame(step);
      };
      raf = requestAnimationFrame(step);
    };
    const timer = window.setInterval(() => {
      const el = ref.current;
      if (!el || pausedRef.current) return;
      const maxScroll = el.scrollHeight - el.clientHeight;
      if (maxScroll <= 4) return; // 内容不溢出,不滚
      // 向上翻页:已到底 → 回到顶部;否则下翻一页(不足一页则停在底部)
      const next =
        el.scrollTop >= maxScroll - 4
          ? 0
          : Math.min(el.scrollTop + el.clientHeight, maxScroll);
      animateTo(el, next);
    }, intervalMs);
    return () => {
      window.clearInterval(timer);
      cancelAnimationFrame(raf);
    };
  }, [ref, intervalMs, durationMs]);
  return pausedRef;
}

function ImportantDates({
  events,
  onPickCase,
}: {
  events: UpcomingEvent[];
  onPickCase: (event: UpcomingEvent) => void;
}) {
  const prominent = events.filter((e) => eventUrgency(e) !== "normal");
  const later = events.filter((e) => eventUrgency(e) === "normal");
  // 2026-06-16:固定卡片高度 + 自动向上翻页(5s/页,0.5s 缓动);鼠标悬停暂停、可手动滚动。
  const scrollRef = useRef<HTMLDivElement>(null);
  const pausedRef = useAutoPageScroll(scrollRef, {
    intervalMs: 5000,
    durationMs: 500,
  });
  return (
    <div className="surface-card p-5">
      <div className="mb-3 flex items-baseline justify-between">
        <h2 className="text-sm font-semibold tracking-tight">重要提醒</h2>
        <span className="font-mono text-caption uppercase tracking-wider text-muted-foreground">
          {events.length} ALERTS
        </span>
      </div>
      {events.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-8 text-center">
          <CalendarClock className="size-6 text-muted-foreground/40" />
          <p className="mt-2 text-xs text-muted-foreground">暂无近期事件</p>
          <p className="mt-1 text-caption text-muted-foreground/70">
            导入后，开庭和续封提醒会显示在这里。
          </p>
        </div>
      ) : (
        <div
          ref={scrollRef}
          onMouseEnter={() => {
            pausedRef.current = true;
          }}
          onMouseLeave={() => {
            pausedRef.current = false;
          }}
          className="max-h-72 space-y-3 overflow-y-auto pr-1">
          {prominent.length > 0 && (
            <ul className="space-y-2">
              {prominent.map((e, i) => (
                <EventRow
                  key={`${e.caseId}-${e.date}-p${i}`}
                  e={e}
                  variant="prominent"
                  onPick={() => onPickCase(e)}
                />
              ))}
            </ul>
          )}
          {later.length > 0 && (
            <div>
              {prominent.length > 0 && (
                <p className="mb-1.5 mt-1 text-caption uppercase tracking-wider text-muted-foreground/50">
                  其他日程
                </p>
              )}
              <ul className="space-y-0.5">
                {later.map((e, i) => (
                  <EventRow
                    key={`${e.caseId}-${e.date}-l${i}`}
                    e={e}
                    variant="compact"
                    onPick={() => onPickCase(e)}
                  />
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/** 待办汇总 widget(2026-06-13 胡彬律师反馈):跨案件未完成待办,按案分组,打钩即完成消失。 */
function TodoSummary({ onPickCase }: { onPickCase: (caseId: string) => void }) {
  const [rows, setRows] = useState<OpenTodoRow[]>([]);

  useEffect(() => {
    let cancelled = false;
    listOpenTodos()
      .then((r) => {
        if (!cancelled) setRows(r);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const handleComplete = async (id: string) => {
    // 乐观移除(打钩消失)
    const prev = rows;
    setRows((r) => r.filter((t) => t.id !== id));
    try {
      await updateTodo(id, { done: 1 });
    } catch (e) {
      setRows(prev); // 回滚
      alert(`完成失败:${e}`);
    }
  };

  const todayKey = toDateKey(new Date());
  // 按案件分组(后端已按 case_name、组内创建倒序)
  const groups: { caseId: string; caseName: string; items: OpenTodoRow[] }[] = [];
  for (const r of rows) {
    const last = groups[groups.length - 1];
    if (last && last.caseId === r.case_id) last.items.push(r);
    else groups.push({ caseId: r.case_id, caseName: r.case_name, items: [r] });
  }

  // 没待办就不显示(不占首页卡片格子)。
  if (rows.length === 0) return null;

  return (
    <div className="surface-card p-5">
      <div className="mb-3 flex items-baseline justify-between">
        <h2 className="text-sm font-semibold tracking-tight">待办汇总</h2>
        <span className="font-mono text-caption uppercase tracking-wider text-muted-foreground">
          {rows.length} TODO
        </span>
      </div>
      {/* 固定成一张卡片高度,待办多了内部滚动(不再随条数无限变长)。 */}
      <div className="max-h-64 space-y-3 overflow-y-auto pr-1">
        {groups.map((g) => (
            <div key={g.caseId}>
              <button
                type="button"
                onClick={() => onPickCase(g.caseId)}
                className="mb-1 text-xs font-medium text-sky-700 hover:underline"
              >
                {g.caseName}
              </button>
              <ul className="space-y-0.5">
                {g.items.map((t) => (
                  <li
                    key={t.id}
                    className="group flex items-center gap-2.5 rounded-md px-1.5 py-1 hover:bg-muted/40"
                  >
                    <button
                      type="button"
                      onClick={() => void handleComplete(t.id)}
                      aria-label="标记完成"
                      title="打钩完成"
                      className="flex size-4 shrink-0 items-center justify-center rounded-[4px] border border-muted-foreground/50 hover:border-sky-600 hover:bg-sky-50"
                    />
                    <span className="flex-1 truncate text-sm text-foreground">
                      {t.title}
                    </span>
                    {t.due_date && (
                      <span
                        className={cn(
                          "shrink-0 font-mono text-[11px]",
                          t.due_date < todayKey
                            ? "text-red-600 dark:text-red-400"
                            : "text-muted-foreground",
                        )}
                        title={`日期 ${t.due_date}`}
                      >
                        {t.due_date.slice(5)}
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
    </div>
  );
}

/** 我的待办(滴答同步)首页卡:跟手机滴答双向同步的个人待办,连接后才显示。
 *  增删改 + 连接管理在设置页;这里可快速加、打钩。每 30s 刷新一次(后台每分钟同步)。 */
function MyTodosCard() {
  const [items, setItems] = useState<TickTickItem[]>([]);
  const [connected, setConnected] = useState(false);
  const [title, setTitle] = useState("");

  const reload = async () => {
    try {
      const s = await ttStatus();
      const ok = s.connected && !!s.projectId;
      setConnected(ok);
      if (ok) setItems(await ttListItems());
    } catch {
      setConnected(false);
    }
  };

  useEffect(() => {
    let cancelled = false;
    const tick = () => {
      if (!cancelled) void reload();
    };
    tick();
    const h = window.setInterval(tick, 30_000);
    return () => {
      cancelled = true;
      window.clearInterval(h);
    };
  }, []);

  const add = async () => {
    const t = title.trim();
    if (!t) return;
    setTitle("");
    try {
      await ttAddItem(t, null);
      await reload();
    } catch (e) {
      alert(`添加失败:${e}`);
    }
  };

  const toggle = async (it: TickTickItem) => {
    setItems((arr) => arr.map((x) => (x.id === it.id ? { ...x, done: !x.done } : x)));
    try {
      await ttToggleItem(it.id, !it.done);
    } catch {
      void reload();
    }
  };

  const remove = async (it: TickTickItem) => {
    setItems((arr) => arr.filter((x) => x.id !== it.id));
    try {
      await ttDeleteItem(it.id);
    } catch {
      void reload();
    }
  };

  // 未连接滴答 → 不占首页格子(去设置页连接)。
  if (!connected) return null;

  const open = items.filter((i) => !i.done);

  return (
    <div className="surface-card p-5">
      <div className="mb-3 flex items-baseline justify-between">
        <h2 className="text-sm font-semibold tracking-tight">我的待办</h2>
        <span className="font-mono text-caption uppercase tracking-wider text-muted-foreground">
          滴答同步
        </span>
      </div>
      <div className="mb-2 flex items-center gap-2">
        <input
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && void add()}
          placeholder="加一条(回车),自动同步到手机"
          className="flex-1 rounded-md border border-border bg-background px-2.5 py-1 text-sm outline-none focus:border-sky-400"
        />
      </div>
      <div className="max-h-64 space-y-0.5 overflow-y-auto pr-1">
        {open.map((t) => (
          <div
            key={t.id}
            className="group flex items-center gap-2.5 rounded-md px-1.5 py-1 hover:bg-muted/40"
          >
            <button
              type="button"
              onClick={() => void toggle(t)}
              aria-label="标记完成"
              title="打钩完成"
              className="flex size-4 shrink-0 items-center justify-center rounded-[4px] border border-muted-foreground/50 hover:border-sky-600 hover:bg-sky-50"
            />
            <span className="flex-1 truncate text-sm text-foreground">{t.title}</span>
            {t.due && (
              <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
                {t.due.slice(5, 10)}
              </span>
            )}
            <button
              type="button"
              onClick={() => void remove(t)}
              title="删除"
              className="shrink-0 text-muted-foreground opacity-0 transition-opacity hover:text-red-600 group-hover:opacity-100"
            >
              ✕
            </button>
          </div>
        ))}
        {open.length === 0 && (
          <p className="px-1 py-2 text-xs text-muted-foreground">暂无待办,上面加一条。</p>
        )}
      </div>
    </div>
  );
}

function EventRow({
  e,
  variant,
  onPick,
}: {
  e: UpcomingEvent;
  variant: "prominent" | "compact";
  onPick: () => void;
}) {
  const urgency = eventUrgency(e);
  const tone =
    urgency === "overdue" || (urgency === "urgent" && e.daysFromNow <= 7)
      ? "red"
      : urgency === "urgent"
        ? "amber"
        : "muted";
  const isPreserv = e.kind === "deadline" && PRESERVATION_RE.test(e.type);
  const titleText = eventTitle(e, isPreserv);
  const Icon =
    e.kind === "hearing"
      ? Gavel
      : e.kind === "todo" || e.kind === "manual"
        ? CalendarClock
        : isPreserv
          ? ShieldAlert
          : AlertTriangle;
  const countdown = formatReminderCountdown(e.daysFromNow);

  if (variant === "compact") {
    const cdCls =
      tone === "red"
        ? "text-red-700 dark:text-red-300"
        : tone === "amber"
          ? "text-amber-700 dark:text-amber-300"
          : "text-muted-foreground";
    return (
      <li>
        <button
          type="button"
          onClick={onPick}
          className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-muted/50"
          title={`打开案件 · ${e.caseName}`}
        >
          <Icon className="size-3 shrink-0 text-muted-foreground/60" />
          <span className={`shrink-0 font-mono text-caption font-medium ${cdCls}`}>{countdown}</span>
          <span className="shrink-0 text-xs font-medium text-foreground">{titleText}</span>
          <span className="truncate text-caption text-muted-foreground">
            · {e.partySummary || e.caseName}
          </span>
          {e.caseNo && (
            <span className="hidden shrink-0 font-mono text-caption text-muted-foreground/70 sm:inline">
              {e.caseNo}
            </span>
          )}
        </button>
      </li>
    );
  }

  const box = {
    red: "bg-red-50 ring-1 ring-red-300/60 dark:bg-red-950/30 dark:ring-red-700/40",
    amber: "bg-amber-50 ring-1 ring-amber-300/60 dark:bg-amber-950/30 dark:ring-amber-700/40",
    muted: "bg-muted/40",
  }[tone];
  const cdCls = {
    red: "text-red-700 dark:text-red-300",
    amber: "text-amber-800 dark:text-amber-300",
    muted: "text-muted-foreground",
  }[tone];
  const iconCls = {
    red: "text-red-600 dark:text-red-400",
    amber: "text-amber-600 dark:text-amber-400",
    muted: "text-foreground/60",
  }[tone];
  const hint =
    urgency === "overdue"
      ? isPreserv
        ? "已超期,尽快续封"
        : "已逾期"
      : urgency === "urgent" && isPreserv
        ? "需提前申请续封"
        : null;
  const hintCls =
    tone === "red"
      ? "bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-300"
      : "bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300";
  const primaryDetail =
    e.kind === "hearing"
      ? [e.timeText, e.locationText].filter(Boolean).join(" · ") || null
      : isPreserv
        ? preservationDetail(e)
        : null;
  const displayNote = e.kind === "hearing" ? e.note : isPreserv ? null : e.note;
  const metaLines = caseMetaLines(e);

  return (
    <li>
      <button
        type="button"
        onClick={onPick}
        className={`grid w-full grid-cols-[4rem_minmax(0,1fr)] gap-3 rounded-lg px-3.5 py-3 text-left transition-colors hover:brightness-95 sm:grid-cols-[4.25rem_minmax(0,1fr)_minmax(12rem,0.9fr)] dark:hover:brightness-110 ${box}`}
        title={`打开案件 · ${e.caseName}`}
      >
        <div className="shrink-0 text-center">
          <div className={`font-mono text-xl font-bold leading-none ${cdCls}`}>{countdown}</div>
          <div className="mt-1 font-mono text-caption text-muted-foreground">{e.date.slice(5)}</div>
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-1.5">
            <Icon className={`size-3.5 shrink-0 ${iconCls}`} />
            <span className="text-sm font-semibold text-foreground">{titleText}</span>
            {hint && <span className={`rounded px-1.5 py-0.5 text-caption font-medium ${hintCls}`}>{hint}</span>}
          </div>
          {primaryDetail && (
            <p className="mt-0.5 truncate text-xs font-medium text-foreground/85">
              {primaryDetail}
            </p>
          )}
          {displayNote && displayNote !== primaryDetail && (
            <p className="mt-0.5 truncate text-xs text-muted-foreground">{displayNote}</p>
          )}
        </div>
        <div className="col-span-2 min-w-0 space-y-0.5 border-t border-foreground/10 pt-2 sm:col-span-1 sm:border-l sm:border-t-0 sm:py-0 sm:pl-3">
          {metaLines.map((line, index) => (
            <p
              key={`${line.text}-${index}`}
              className={cn(
                "truncate text-caption text-muted-foreground",
                index === 0 && "text-xs font-medium text-foreground/85",
                line.mono && "font-mono",
              )}
              title={line.text}
            >
              {line.text}
            </p>
          ))}
        </div>
      </button>
    </li>
  );
}

function eventTitle(e: UpcomingEvent, isPreserv: boolean): string {
  if (e.kind === "hearing") return "开庭";
  return isPreserv ? preservationTitle(e.type) : e.type;
}

function preservationTitle(type: string): string {
  if (/续封/.test(type)) return "续封到期";
  if (/查封/.test(type)) return "查封到期";
  if (/冻结/.test(type)) return "冻结到期";
  return "保全到期";
}

function preservationDetail(e: UpcomingEvent): string | null {
  const judgeText = preservationJudgeText(e);
  const phoneText = preservationPhoneText(e);
  const targetText = e.note ? `标的 ${e.note}` : null;
  return [`到期 ${e.date}`, targetText, judgeText, phoneText].filter(Boolean).join(" · ") || null;
}

function preservationJudgeText(e: UpcomingEvent): string | null {
  const judges = e.judges?.filter(Boolean) ?? [];
  if (judges.length > 0) return `法官 ${judges.join("、")}`;
  const contact = e.courtContacts?.find((c) => isJudgeRole(c.role) && c.name);
  return contact?.name ? `法官 ${contact.name}` : null;
}

function preservationPhoneText(e: UpcomingEvent): string | null {
  const judgeNames = new Set((e.judges ?? []).filter(Boolean));
  const contact =
    e.courtContacts?.find(
      (c) => c.phone && ((c.name && judgeNames.has(c.name)) || isJudgeRole(c.role)),
    ) ?? e.courtContacts?.find((c) => c.phone);
  return contact?.phone ? `电话 ${contact.phone}` : null;
}

function isJudgeRole(role: string | null | undefined): boolean {
  return !!role && /法官|审判员|审判长|承办人|法官助理|书记员/.test(role);
}

function caseMetaLines(e: UpcomingEvent): Array<{ text: string; mono?: boolean }> {
  const raw = [
    e.caseName,
    e.partySummary,
    e.caseNo ? { text: e.caseNo, mono: true } : null,
    e.court,
  ];
  const seen = new Set<string>();
  const lines: Array<{ text: string; mono?: boolean }> = [];
  for (const item of raw) {
    const line =
      typeof item === "string" ? { text: item, mono: false } : item;
    if (!line) continue;
    const text = line.text.trim();
    if (!text || seen.has(text)) continue;
    seen.add(text);
    lines.push({ text, mono: line.mono });
  }
  return lines;
}

function CalendarPanel({
  events,
  onPickCase,
  onAddEvent,
  onEditEvent,
  onDeleteEvent,
}: {
  events: UpcomingEvent[];
  onPickCase: (event: UpcomingEvent) => void;
  onAddEvent: (date: string, title: string) => void | Promise<void>;
  onEditEvent: (event: UpcomingEvent, input: CalendarEventEditInput) => Promise<void>;
  onDeleteEvent: (event: UpcomingEvent) => Promise<void>;
}) {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const [monthCursor, setMonthCursor] = useState(
    () => new Date(today.getFullYear(), today.getMonth(), 1),
  );
  const [selectedDate, setSelectedDate] = useState(toDateKey(today));
  const days = buildCalendarDays(monthCursor);
  const eventsByDate = new Map<string, UpcomingEvent[]>();
  for (const event of events) {
    const arr = eventsByDate.get(event.date) ?? [];
    arr.push(event);
    eventsByDate.set(event.date, arr);
  }
  const selectedEvents = eventsByDate.get(selectedDate) ?? [];
  const monthLabel = `${monthCursor.getFullYear()} 年 ${monthCursor.getMonth() + 1} 月`;

  const moveMonth = (offset: number) => {
    setMonthCursor((d) => new Date(d.getFullYear(), d.getMonth() + offset, 1));
  };

  // 折叠态(2026-06-14:作者反馈整月网格太占地方)。默认折叠成摘要卡,按用户选择持久化。
  const [collapsed, setCollapsed] = useState(
    () => localStorage.getItem("caseboard.home.calendarCollapsed") !== "false",
  );
  const toggleCollapsed = () => {
    setCollapsed((v) => {
      const next = !v;
      localStorage.setItem("caseboard.home.calendarCollapsed", next ? "true" : "false");
      return next;
    });
  };
  // 摘要列表:近 30 天内逾期 + 所有未来日程,按临近程度升序(最紧迫在最前)。
  const summaryEvents = [...events]
    .filter((e) => e.daysFromNow >= -30)
    .sort((a, b) => a.daysFromNow - b.daysFromNow);

  // 右键某天 → 菜单 →「添加日程」(独立日程,不绑案件)
  const [dateMenu, setDateMenu] = useState<{ date: string; x: number; y: number } | null>(null);
  const [eventMenu, setEventMenu] = useState<{
    event: UpcomingEvent;
    x: number;
    y: number;
  } | null>(null);
  const [addDate, setAddDate] = useState<string | null>(null);
  const [addInput, setAddInput] = useState("");
  const submitAdd = async () => {
    const title = addInput.trim();
    if (!addDate || !title) return;
    await onAddEvent(addDate, title);
    setAddInput("");
    setAddDate(null);
  };

  return (
    <section className="surface-card p-5">
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <CalendarDays className="size-4 text-muted-foreground" />
          <h2 className="text-sm font-semibold tracking-tight">日程日历</h2>
          <span className="font-mono text-caption uppercase tracking-wider text-muted-foreground">
            {events.length} EVENTS
          </span>
        </div>
        <div className="flex items-center gap-2">
          {!collapsed && (
            <>
              <Button type="button" variant="outline" size="icon" onClick={() => moveMonth(-1)} title="上一月">
                <ChevronLeft className="size-4" />
              </Button>
              <div className="w-28 text-center text-sm font-medium">{monthLabel}</div>
              <Button type="button" variant="outline" size="icon" onClick={() => moveMonth(1)} title="下一月">
                <ChevronRight className="size-4" />
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => {
                  setMonthCursor(new Date(today.getFullYear(), today.getMonth(), 1));
                  setSelectedDate(toDateKey(today));
                }}
              >
                回到本月
              </Button>
            </>
          )}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={toggleCollapsed}
            title={collapsed ? "展开整月日历" : "折叠成摘要卡"}
          >
            {collapsed ? "展开" : "折叠"}
            <ChevronDown className={cn("size-4 transition-transform", !collapsed && "rotate-180")} />
          </Button>
        </div>
      </div>
      {collapsed ? (
        // 折叠态:固定高度摘要卡,日程多了内部上下滚动
        <div className="max-h-52 space-y-1.5 overflow-y-auto rounded-lg border border-border bg-background/60 p-3">
          {summaryEvents.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              暂无近期日程(开庭 / 到期日由案件分析自动汇总到这里)
            </p>
          ) : (
            summaryEvents.map((event) => {
              const isManual = event.kind === "manual";
              const countdown = formatReminderCountdown(event.daysFromNow);
              return (
                <div
                  key={event.sourceKey}
                  onContextMenu={(contextEvent) => {
                    contextEvent.preventDefault();
                    contextEvent.stopPropagation();
                    setDateMenu(null);
                    setEventMenu({
                      event,
                      x: contextEvent.clientX,
                      y: contextEvent.clientY,
                    });
                  }}
                  title="右键编辑或删除日程"
                  className="flex items-center gap-2 rounded-md px-2 py-1.5 text-xs hover:bg-muted/60"
                >
                  <button
                    type="button"
                    onClick={() => {
                      if (!isManual) onPickCase(event);
                    }}
                    className="flex min-w-0 flex-1 items-center gap-2 text-left"
                  >
                    <span className={cn("size-2 shrink-0 rounded-full", calendarDotClass(event))} />
                    <span className="shrink-0 font-mono text-caption text-muted-foreground">
                      {event.date.slice(5)}
                    </span>
                    <span className="shrink-0 font-medium text-foreground">{event.type}</span>
                    <span className="truncate text-muted-foreground">
                      {[event.caseName, event.note].filter(Boolean).join(" · ")}
                    </span>
                    <span className="ml-auto shrink-0 font-mono text-caption text-muted-foreground">
                      {countdown}
                    </span>
                  </button>
                </div>
              );
            })
          )}
        </div>
      ) : (
        <>
      <div className="grid grid-cols-7 gap-px overflow-hidden rounded-lg border border-border bg-border">
        {["一", "二", "三", "四", "五", "六", "日"].map((d) => (
          <div key={d} className="bg-muted/70 px-2 py-1 text-center text-caption text-muted-foreground">
            周{d}
          </div>
        ))}
        {days.map((day) => {
          const key = toDateKey(day.date);
          const dayEvents = eventsByDate.get(key) ?? [];
          const isCurrentMonth = day.date.getMonth() === monthCursor.getMonth();
          const isToday = key === toDateKey(today);
          const isSelected = key === selectedDate;
          return (
            <button
              key={key}
              type="button"
              onClick={() => setSelectedDate(key)}
              onContextMenu={(e) => {
                e.preventDefault();
                setSelectedDate(key);
                setEventMenu(null);
                setDateMenu({ date: key, x: e.clientX, y: e.clientY });
              }}
              title="左键看当天日程 · 右键添加日程"
              className={cn(
                "min-h-16 bg-card p-2 text-left transition-colors hover:bg-sky-50 dark:hover:bg-sky-950/20",
                !isCurrentMonth && "bg-muted/20 text-muted-foreground/50",
                isSelected && "ring-2 ring-inset ring-sky-400",
                isToday && "bg-blue-50 dark:bg-blue-950/20",
              )}
            >
              <div className="flex items-center justify-between">
                <span className={cn("text-xs", isToday && "font-bold text-blue-700 dark:text-blue-300")}>
                  {day.date.getDate()}
                </span>
                {dayEvents.length > 0 && (
                  <span className="rounded-full bg-foreground px-1.5 py-0.5 font-mono text-caption text-background">
                    {dayEvents.length}
                  </span>
                )}
              </div>
              <div className="mt-2 flex flex-wrap gap-1">
                {dayEvents.slice(0, 4).map((event, index) => (
                  <span key={`${event.caseId}-${index}`} className={cn("size-1.5 rounded-full", calendarDotClass(event))} />
                ))}
              </div>
            </button>
          );
        })}
      </div>
      <div className="mt-4 rounded-lg border border-border bg-background/60 p-3">
        <div className="mb-2 flex items-center justify-between">
          <span className="text-xs font-medium text-foreground">{selectedDate} 日程</span>
          <button
            type="button"
            onClick={() => {
              setAddDate(selectedDate);
              setAddInput("");
            }}
            className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-sky-700 transition-colors hover:bg-sky-50 dark:text-sky-300 dark:hover:bg-sky-950/30"
          >
            <Plus className="size-3" /> 添加
          </button>
        </div>
        {addDate && (
          <div className="mb-2 flex items-center gap-2 rounded-md border border-sky-200 bg-sky-50/70 px-2 py-1.5 dark:border-sky-900 dark:bg-sky-950/20">
            <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
              {addDate.slice(5)}
            </span>
            <input
              autoFocus
              value={addInput}
              onChange={(e) => setAddInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void submitAdd();
                } else if (e.key === "Escape") {
                  setAddDate(null);
                }
              }}
              placeholder="写个日程…回车保存"
              className="flex-1 bg-transparent text-xs outline-none placeholder:text-muted-foreground"
            />
            <button
              type="button"
              onClick={() => void submitAdd()}
              className="shrink-0 rounded bg-sky-600 px-2 py-0.5 text-[11px] font-medium text-white hover:bg-sky-700"
            >
              保存
            </button>
            <button
              type="button"
              onClick={() => setAddDate(null)}
              className="shrink-0 rounded px-1.5 py-0.5 text-[11px] text-muted-foreground hover:bg-muted"
            >
              取消
            </button>
          </div>
        )}
        {selectedEvents.length === 0 ? (
          <p className="text-xs text-muted-foreground">
            当天暂无日程 · 右键日期或点「添加」可加一条
          </p>
        ) : (
          <ul className="space-y-1.5">
            {selectedEvents.map((event) => {
              const isManual = event.kind === "manual";
              const countdown = formatReminderCountdown(event.daysFromNow);
              return (
                <li
                  key={event.sourceKey}
                  onContextMenu={(contextEvent) => {
                    contextEvent.preventDefault();
                    contextEvent.stopPropagation();
                    setDateMenu(null);
                    setEventMenu({
                      event,
                      x: contextEvent.clientX,
                      y: contextEvent.clientY,
                    });
                  }}
                  title="右键编辑或删除日程"
                  className="group flex items-center gap-2 rounded-md px-2 py-1.5 text-xs hover:bg-muted/60"
                >
                  <span className={cn("size-2 shrink-0 rounded-full", calendarDotClass(event))} />
                  {isManual ? (
                    <div className="min-w-0 flex-1">
                      <span className="font-medium text-foreground">{event.type}</span>
                      {event.note && (
                        <span className="ml-2 truncate text-muted-foreground">{event.note}</span>
                      )}
                    </div>
                  ) : (
                      <button
                        type="button"
                        onClick={() => onPickCase(event)}
                        className="flex flex-1 items-center gap-2 overflow-hidden text-left"
                      >
                      <span className="shrink-0 font-medium text-foreground">{event.type}</span>
                      <span className="truncate text-muted-foreground">
                        {[event.caseName, event.note].filter(Boolean).join(" · ")}
                      </span>
                    </button>
                  )}
                  <span className="ml-auto shrink-0 font-mono text-caption text-muted-foreground">
                    {countdown}
                  </span>
                </li>
              );
            })}
          </ul>
        )}
      </div>
        </>
      )}

      {/* 右键某天弹出的菜单:点「添加日程」→ 打开当天添加输入 */}
      {dateMenu && (
        <>
          <div className="fixed inset-0 z-40" onClick={() => setDateMenu(null)} />
          <div
            className="fixed z-50 overflow-hidden rounded-md border border-border bg-card shadow-lg"
            style={{ left: dateMenu.x, top: dateMenu.y }}
          >
            <button
              type="button"
              onClick={() => {
                setSelectedDate(dateMenu.date);
                setAddDate(dateMenu.date);
                setAddInput("");
                setDateMenu(null);
              }}
              className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs hover:bg-sky-50 dark:hover:bg-sky-950/30"
            >
              <Plus className="size-3.5 text-sky-600" />
              在 {dateMenu.date.slice(5)} 添加日程
            </button>
          </div>
        </>
      )}
      {eventMenu && (
        <CalendarEventActions
          event={eventMenu.event}
          position={{ x: eventMenu.x, y: eventMenu.y }}
          onClose={() => setEventMenu(null)}
          onEdit={(input) => onEditEvent(eventMenu.event, input)}
          onDelete={() => onDeleteEvent(eventMenu.event)}
        />
      )}
    </section>
  );
}

function buildCaseDisplay(caseData: Case): CaseDisplayFields {
  const plaintiffs = parseJsonArray(caseData.agg_plaintiffs);
  const defendants = parseJsonArray(caseData.agg_defendants);
  const judges = parseJsonArray(caseData.agg_judges);
  const ovFields: Record<string, string | null> = (() => {
    if (!caseData.user_overrides_json) return {};
    try {
      const parsed = JSON.parse(caseData.user_overrides_json) as {
        fields?: Record<string, string | null>;
      };
      return parsed.fields ?? {};
    } catch {
      return {};
    }
  })();
  const ovStr = (path: string, base: string | null): string | null =>
    path in ovFields ? ovFields[path] : base;
  const claimAmount = (() => {
    const ov = ovFields["agg_claim_amount"];
    if (ov === undefined) return caseData.agg_claim_amount;
    const n = ov != null ? parseFloat(ov) : NaN;
    return Number.isFinite(n) ? n : null;
  })();
  const left = plaintiffs[0] || "-";
  const right = defendants[0] || "-";
  const leftMore = plaintiffs.length > 1 ? `等${plaintiffs.length}人` : "";
  const rightMore = defendants.length > 1 ? `等${defendants.length}人` : "";
  return {
    caseNo: ovStr("agg_case_no", caseData.agg_case_no),
    court: ovStr("agg_court", caseData.agg_court),
    cause: ovStr("agg_cause", caseData.agg_cause),
    claimAmount,
    plaintiffs,
    defendants,
    judges,
    partySummary: `${left}${leftMore} vs ${right}${rightMore}`,
    amountText: claimAmount ? formatYuan(claimAmount) : null,
  };
}

function buildHomeDailyBrief(
  events: UpcomingEvent[],
  activeCases: Case[],
  docsByCase: Record<string, Document[]>,
): {
  brief: DailyBrief;
  actionEvent: UpcomingEvent | null;
  actionCaseId: string | null;
} {
  const redEvents = events.filter((event) => eventUrgency(event) === "overdue");
  const orangeEvents = events.filter((event) => eventUrgency(event) === "urgent");
  const failedDocCaseId = firstExtractionFailedCaseId(activeCases, docsByCase);
  const recentCaseId = firstRecentlyUpdatedCaseId(activeCases);
  const recentCount = recentlyUpdatedCaseCount(activeCases);

  if (redEvents.length > 0) {
    return {
      brief: {
        text: `今日简报: ${redEvents.length} 个红色提醒 · ${orangeEvents.length} 个橙色提醒`,
        actionLabel: "查看提醒",
        level: "red",
      },
      actionEvent: redEvents[0],
      actionCaseId: null,
    };
  }

  if (orangeEvents.length > 0) {
    return {
      brief: {
        text: `今日简报: 无红色风险 · ${orangeEvents.length} 个橙色提醒`,
        actionLabel: "扫一眼",
        level: "orange",
      },
      actionEvent: orangeEvents[0],
      actionCaseId: null,
    };
  }

  if (failedDocCaseId) {
    return {
      brief: {
        text: "今日简报: 无红色风险 · 有材料抽取需处理",
        actionLabel: "查看材料",
        level: "orange",
      },
      actionEvent: null,
      actionCaseId: failedDocCaseId,
    };
  }

  if (recentCount > 0 && recentCaseId) {
    return {
      brief: {
        text: `今日简报: 无红色风险 · 最近更新 ${recentCount} 案`,
        actionLabel: "看更新",
        level: "calm",
      },
      actionEvent: null,
      actionCaseId: recentCaseId,
    };
  }

  return {
    brief: {
      text: "今日简报: 暂无红色风险 · 可以按自己的节奏来",
      actionLabel: "导入材料",
      level: "calm",
    },
    actionEvent: null,
    actionCaseId: null,
  };
}

function firstExtractionFailedCaseId(
  activeCases: Case[],
  docsByCase: Record<string, Document[]>,
): string | null {
  for (const c of activeCases) {
    if ((docsByCase[c.id] ?? []).some((doc) => doc.extraction_status === "failed")) {
      return c.id;
    }
  }
  return null;
}

function firstRecentlyUpdatedCaseId(activeCases: Case[]): string | null {
  return [...activeCases]
    .filter((c) => isRecentlyUpdated(c.updated_at))
    .sort((a, b) => b.updated_at.localeCompare(a.updated_at))[0]?.id ?? null;
}

function recentlyUpdatedCaseCount(activeCases: Case[]): number {
  return activeCases.filter((c) => isRecentlyUpdated(c.updated_at)).length;
}

function isRecentlyUpdated(value: string | null | undefined): boolean {
  if (!value) return false;
  const time = Date.parse(value);
  if (!Number.isFinite(time)) return false;
  return Date.now() - time <= 1000 * 60 * 60 * 48;
}

function compareCaseRows(a: CaseRow, b: CaseRow, key: SortKey, dir: SortDir): number {
  const sign = dir === "asc" ? 1 : -1;
  if (key === "status") {
    const base = compareCasesByStatusThenTime(
      a.status.id,
      a.caseData.updated_at,
      b.status.id,
      b.caseData.updated_at,
    );
    return sign * base;
  }
  const av = sortValue(a, key);
  const bv = sortValue(b, key);
  if (av == null && bv == null) return 0;
  if (av == null) return 1;
  if (bv == null) return -1;
  if (av < bv) return -1 * sign;
  if (av > bv) return 1 * sign;
  return b.caseData.updated_at.localeCompare(a.caseData.updated_at);
}

function sortValue(row: CaseRow, key: SortKey): number | string | null {
  if (key === "amount") return row.display.claimAmount;
  if (key === "filed_at") return row.caseData.agg_filed_at;
  if (key === "hearing") return row.nearestHearing;
  return row.status.order;
}

function findNearestFutureHearing(c: Case): string | null {
  const now = todayDate();
  let best: string | null = null;
  for (const kd of readKeyDates(c)) {
    if (!kd.event?.includes("开庭") || !kd.date) continue;
    const d = parseDate(kd.date);
    if (!d) continue;
    const days = diffDays(d, now);
    if (days < 0) continue;
    if (!best || kd.date < best) best = kd.date;
  }
  return best;
}

/** 带日期的待办(绑案件)→ 日历事件,kind="todo"。 */
function buildTodoEvents(todos: OpenTodoRow[]): UpcomingEvent[] {
  const now = todayDate();
  const out: UpcomingEvent[] = [];
  for (const t of todos) {
    if (!t.due_date) continue;
    const d = parseDate(t.due_date);
    if (!d) continue;
    out.push({
      kind: "todo",
      date: t.due_date,
      daysFromNow: diffDays(d, now),
      type: t.title,
      note: t.note,
      caseName: t.case_name,
      caseId: t.case_id,
      court: null,
      id: t.id,
      sourceKey: `todo:${t.id}`,
    });
  }
  return out;
}

/** 独立日历日程(不绑案件)→ 日历事件,kind="manual",带 id 供删除。 */
function buildManualEvents(rows: CalendarEvent[]): UpcomingEvent[] {
  const now = todayDate();
  const out: UpcomingEvent[] = [];
  for (const e of rows) {
    const d = parseDate(e.date);
    if (!d) continue;
    out.push({
      kind: "manual",
      date: e.date,
      daysFromNow: diffDays(d, now),
      type: e.title,
      note: e.note,
      caseName: "",
      caseId: "",
      court: null,
      id: e.id,
      sourceKey: `manual:${e.id}`,
    });
  }
  return out;
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

function calendarDotClass(e: UpcomingEvent): string {
  if (e.daysFromNow < 0 || e.daysFromNow <= 7) return "bg-red-500";
  if (e.daysFromNow <= 30) return "bg-amber-500";
  if (e.kind === "hearing") return "bg-blue-500";
  if (e.kind === "todo") return "bg-violet-500";
  if (e.kind === "manual") return "bg-emerald-500";
  return "bg-slate-400";
}

function buildCalendarDays(cursor: Date): Array<{ date: Date }> {
  const first = new Date(cursor.getFullYear(), cursor.getMonth(), 1);
  const mondayOffset = (first.getDay() + 6) % 7;
  const start = new Date(first);
  start.setDate(first.getDate() - mondayOffset);
  return Array.from({ length: 42 }, (_, index) => {
    const date = new Date(start);
    date.setDate(start.getDate() + index);
    return { date };
  });
}

function upcomingEventKey(event: UpcomingEvent): string {
  return `${event.caseId}|${event.type}|${event.date}`;
}

function toDateKey(d: Date): string {
  const year = d.getFullYear();
  const month = `${d.getMonth() + 1}`.padStart(2, "0");
  const day = `${d.getDate()}`.padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function getGreeting(name: string | null): string {
  const who = name && name.trim().length > 0 ? name.trim() : "律师";
  const h = new Date().getHours();
  if (h < 6) return `深夜好，${who}`;
  if (h < 12) return `上午好，${who}`;
  if (h < 14) return `中午好，${who}`;
  if (h < 18) return `下午好，${who}`;
  return `晚上好，${who}`;
}

function EmptyCases({ onImport }: { onImport: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center rounded-xl border border-dashed border-border bg-card/45 px-6 py-16 text-center">
      <FolderOpen className="size-10 text-muted-foreground/40" />
      <p className="mt-4 text-base font-medium text-foreground">还没有导入任何案件</p>
      <p className="mt-1 text-sm text-muted-foreground">选择案件文件夹即可开始。</p>
      <Button onClick={onImport} className="mt-6">
        <FolderOpen className="size-3.5" />
        导入案件文件夹
      </Button>
    </div>
  );
}

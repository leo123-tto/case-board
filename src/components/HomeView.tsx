import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import {
  AlertTriangle,
  ArrowUpDown,
  CalendarClock,
  CalendarDays,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  FolderOpen,
  Gavel,
  GripVertical,
  LayoutGrid,
  List,
  Plus,
  ShieldAlert,
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
  getCaseWithDocs,
  getSettings,
  listCalendarEvents,
  listOpenTodos,
  type OpenTodoRow,
  updateHomeCaseOrder,
  updateTodo,
  updateWorkflowStatus,
} from "@/lib/api";
import type { Case, Document } from "@/lib/types";
import { parseJsonArray } from "@/lib/types";
import { cn } from "@/lib/utils";
import {
  compareCasesByStatusThenTime,
  resolveCaseStatus,
  STATUS_LIST,
  type StatusDef,
  type StatusId,
} from "@/modules/litigation/lib/inferStatus";
import { CalendarBoard } from "./CalendarBoard";

export interface HomeViewProps {
  cases: Case[];
  userDisplayName: string | null;
  onPickCase: (caseId: string) => void;
  onImport: () => void;
  /** 点击飞书日历事件后导入对应文件夹 */
  onImportFolder?: (eventTitle: string) => void;
}

type ViewMode = "grid" | "list";
type SortKey = "status" | "amount" | "filed_at" | "hearing";
type SortDir = "asc" | "desc";
type EventKind = "hearing" | "deadline" | "todo" | "manual";

interface CaseDisplayFields {
  title: string;
  subtitle: string | null;
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

export interface UpcomingEvent {
  kind: EventKind;
  date: string;
  daysFromNow: number;
  type: string;
  note?: string | null;
  caseName: string;
  caseId: string;
  court?: string | null;
  /** 仅 kind="manual"(独立日历日程):calendar_events.id,用于删除 */
  id?: string;
}

const PRESERVATION_RE = /保全|续封|查封|冻结/;

export function HomeView({ cases, userDisplayName, onPickCase, onImport, onImportFolder }: HomeViewProps) {
  const greeting = getGreeting(userDisplayName);
  const monthLabel = new Date()
    .toLocaleString("en-US", { month: "short", year: "numeric" })
    .toUpperCase();

  const [docsByCase, setDocsByCase] = useState<Record<string, Document[]>>({});
  const [statusOverride, setStatusOverride] = useState<Record<string, StatusId | null>>({});
  const [userOrder, setUserOrder] = useState<string[] | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>("grid");
  const [sortKey, setSortKey] = useState<SortKey>("status");
  const [sortDir, setSortDir] = useState<SortDir>("asc");
  const [statusFilters, setStatusFilters] = useState<Set<StatusId>>(new Set());
  const [courtFilter, setCourtFilter] = useState("");
  // 带日期的待办 → 汇入日程日历(2026-06-14:手动日程 = 带日期的待办)
  const [openTodos, setOpenTodos] = useState<OpenTodoRow[]>([]);
  // 独立日历日程(不绑案件,日历右键添加)
  const [manualEvents, setManualEvents] = useState<CalendarEvent[]>([]);
  // 日程日历功能开关(默认关闭,设置里手动开)
  const [calendarEnabled, setCalendarEnabled] = useState(false);
  const topLeftRef = useRef<HTMLDivElement | null>(null);
  const [topLeftHeight, setTopLeftHeight] = useState<number | null>(null);

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
  const handleDeleteCalendarEvent = async (id: string) => {
    try {
      await deleteCalendarEvent(id);
      setManualEvents((prev) => prev.filter((e) => e.id !== id));
    } catch (e) {
      alert(`删除日程失败:${e}`);
    }
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
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const el = topLeftRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const update = () => setTopLeftHeight(Math.ceil(el.getBoundingClientRect().height));
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    window.addEventListener("resize", update);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", update);
    };
  }, []);

  const casesWithOverride = cases.map((c) =>
    c.id in statusOverride ? { ...c, workflow_status: statusOverride[c.id] } : c,
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

  const canUseUserOrder = viewMode === "grid" && sortKey === "status" && sortDir === "asc";
  const sortedRows = canUseUserOrder
    ? userOrderedRows
    : [...caseRows].sort((a, b) => compareCaseRows(a, b, sortKey, sortDir));

  const courtOptions = Array.from(
    new Set(caseRows.map((row) => row.display.court).filter(Boolean) as string[]),
  ).sort((a, b) => a.localeCompare(b, "zh-Hans-CN"));

  const filteredRows = sortedRows.filter((row) => {
    if (statusFilters.size > 0 && !statusFilters.has(row.status.id)) return false;
    if (courtFilter && row.display.court !== courtFilter) return false;
    return true;
  });

  const activeCases = defaultSorted
    .filter(({ status }) => status.id !== "closed" && status.id !== "mediated")
    .map(({ caseData }) => caseData);
  const upcomingEvents = buildUpcomingEvents(activeCases);
  const calendarEvents = [
    ...buildAllCalendarEvents(activeCases),
    ...buildTodoEvents(openTodos),
    ...buildManualEvents(manualEvents),
  ];

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
  );
  const sortedIds = filteredRows.map((row) => row.caseData.id);

  const handleChangeStatus = async (caseId: string, status: StatusId | null) => {
    setStatusOverride((m) => ({ ...m, [caseId]: status }));
    try {
      await updateWorkflowStatus(caseId, status);
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
  };

  return (
    <main className="flex h-full w-full flex-col bg-background">
      <header className="border-b border-border bg-card/50 px-8 py-3">
        <div className="mx-auto flex max-w-6xl items-center">
          <h1 className="text-sm font-semibold tracking-tight text-foreground">案件看板</h1>
        </div>
      </header>

      <div className="flex-1 overflow-auto">
        <div className="mx-auto max-w-6xl px-8 py-8">
          {/* 左边(2/3): 问候语 + 飞书日历; 右边(1/3): 重要日期 */}
          <div className="mb-10 grid grid-cols-1 items-start gap-6 lg:grid-cols-[minmax(0,1.85fr)_minmax(20rem,1fr)]">
            <div ref={topLeftRef} className="min-w-0 space-y-6">
              {/* 问候语 */}
              <div>
                <p className="font-mono text-caption uppercase tracking-wider text-muted-foreground">
                  OVERVIEW · {monthLabel}
                </p>
                <h1 className="mt-2 text-4xl font-semibold tracking-tight text-foreground">
                  {greeting}
                </h1>
                <p className="mt-2 text-sm text-muted-foreground">
                  你正在办 {cases.length} 个案件,扫一眼今天的进度。
                </p>
                <div className="mt-5 flex gap-2">
                  <Button
                    onClick={onImport}
                    className="bg-foreground text-background hover:bg-foreground/90"
                  >
                    <FolderOpen className="size-3.5" />
                    导入案件文件夹
                  </Button>
                </div>
              </div>
              {/* 飞书日历 */}
              <CalendarBoard
                localEvents={upcomingEvents}
                onPickCase={onPickCase}
                onImportFolder={onImportFolder}
              />
            </div>
            {/* 右边: 重要日期 */}
            <div
              className="flex min-h-0 min-w-0 overflow-hidden lg:h-[var(--top-left-height)]"
              style={
                topLeftHeight
                  ? ({ "--top-left-height": `${topLeftHeight}px` } as CSSProperties)
                  : undefined
              }
            >
              <ImportantDates events={upcomingEvents} onPickCase={onPickCase} />
            </div>
          </div>

          {cases.length > 0 && calendarEnabled && (
            <div className="mb-8">
              <CalendarPanel
                events={calendarEvents}
                onPickCase={onPickCase}
                onAddEvent={handleAddCalendarEvent}
                onDeleteEvent={handleDeleteCalendarEvent}
              />
            </div>
          )}

          <section>
            <div className="mb-4 flex flex-col gap-3">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="flex items-baseline gap-3">
                  <h2 className="text-lg font-semibold tracking-tight">在办案件</h2>
                  <span className="font-mono text-caption uppercase tracking-wider text-muted-foreground">
                    {filteredRows.length} / {cases.length} CASES
                  </span>
                </div>
                <div className="flex flex-wrap items-center gap-2">
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
                </div>
              </div>

              <div className="flex flex-wrap items-center gap-2 rounded-xl border border-border bg-card/60 p-3">
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
                {(statusFilters.size > 0 || courtFilter) && (
                  <Button type="button" variant="ghost" size="sm" onClick={clearFilters}>
                    <X className="size-3.5" />
                    清空筛选
                  </Button>
                )}
              </div>

            </div>

            {cases.length === 0 ? (
              <EmptyCases onImport={onImport} />
            ) : filteredRows.length === 0 ? (
              <div className="rounded-xl border border-dashed border-border bg-card/30 px-6 py-12 text-center text-sm text-muted-foreground">
                没有符合筛选条件的案件
              </div>
            ) : viewMode === "list" ? (
              <div className="overflow-hidden rounded-xl border border-border bg-card">
                {filteredRows.map((row) => (
                  <CaseListRow
                    key={row.caseData.id}
                    row={row}
                    onClick={() => onPickCase(row.caseData.id)}
                    onChangeStatus={(s) => handleChangeStatus(row.caseData.id, s)}
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
                    {/* 待办汇总固定置顶第一格(胡彬律师反馈):不进 SortableContext,不可拖、永远在最前 */}
                    <TodoSummary onPickCase={onPickCase} />
                    {filteredRows.map((row) => (
                      <SortableCaseCard
                        key={row.caseData.id}
                        row={row}
                        onClick={() => onPickCase(row.caseData.id)}
                        onChangeStatus={(s) => handleChangeStatus(row.caseData.id, s)}
                      />
                    ))}
                  </div>
                </SortableContext>
              </DndContext>
            )}
          </section>
        </div>
      </div>
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
}) {
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
    zIndex: isDragging ? 10 : undefined,
  };
  return (
    <div ref={setNodeRef} style={style}>
      <CaseCard
        {...props}
        dragHandleProps={{ attributes, listeners }}
        isDragging={isDragging}
      />
    </div>
  );
}

function CaseCard({
  row,
  onClick,
  onChangeStatus,
  dragHandleProps,
  isDragging,
}: {
  row: CaseRow;
  onClick: () => void;
  onChangeStatus: (s: StatusId | null) => void;
  dragHandleProps?: {
    attributes: ReturnType<typeof useSortable>["attributes"];
    listeners: ReturnType<typeof useSortable>["listeners"];
  };
  isDragging?: boolean;
}) {
  const { caseData, status, display } = row;
  const isClosed = status.id === "closed";
  return (
    <div
      className={cn(
        "group relative flex cursor-pointer flex-col rounded-xl border border-border bg-card p-5 text-left shadow-sm transition-all hover:border-foreground/30 hover:bg-foreground/[0.025] hover:shadow-lg",
        isDragging && "border-dashed",
        isClosed && "opacity-60",
      )}
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick();
        }
      }}
      role="button"
      tabIndex={0}
      aria-label={`打开案件 ${display.title}`}
    >
      {dragHandleProps && (
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
      )}

      <div className="absolute right-3 top-3">
        <StatusPicker
          status={status}
          isManual={caseData.workflow_status != null}
          onPick={onChangeStatus}
        />
      </div>

      <h3 className="pr-16 text-lg font-semibold leading-tight text-foreground">
        {caseData.source_folder === "__DEMO__" && (
          <span className="mr-2 inline-flex items-center rounded bg-amber-100 px-1.5 py-0.5 text-caption font-medium text-amber-800 align-middle dark:bg-amber-900/40 dark:text-amber-200">
            示例
          </span>
        )}
        {display.title}
      </h3>
      <p className="mt-1 truncate text-sm text-muted-foreground">
        {display.subtitle || display.cause || caseData.name}
      </p>
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
}: {
  row: CaseRow;
  onClick: () => void;
  onChangeStatus: (s: StatusId | null) => void;
}) {
  const { caseData, status, display } = row;
  return (
    <div
      className={cn(
        "grid cursor-pointer grid-cols-[minmax(0,1.35fr)_minmax(0,1fr)_minmax(0,1fr)_minmax(0,1fr)_auto] items-center gap-3 border-b border-border px-4 py-3 text-left transition-colors last:border-b-0 hover:bg-muted/50",
        status.id === "closed" && "opacity-60",
      )}
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick();
        }
      }}
      role="button"
      tabIndex={0}
      aria-label={`打开案件 ${display.title}`}
    >
      <div className="min-w-0">
        <div className="truncate text-sm font-semibold text-foreground">
          {display.title}
        </div>
        <div className="truncate text-xs text-muted-foreground">
          {display.subtitle || display.cause || caseData.name}
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
        isManual={caseData.workflow_status != null}
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
}: {
  status: StatusDef;
  isManual: boolean;
  onPick: (s: StatusId | null) => void;
}) {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (!open) return;
    const onClick = () => setOpen(false);
    window.addEventListener("click", onClick);
    return () => window.removeEventListener("click", onClick);
  }, [open]);

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
          setOpen((v) => !v);
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

function ImportantDates({
  events,
  onPickCase,
}: {
  events: UpcomingEvent[];
  onPickCase: (caseId: string) => void;
}) {
  const prominent = events.filter((e) => eventUrgency(e) !== "normal");
  const later = events.filter((e) => eventUrgency(e) === "normal");
  return (
    <div className="flex h-full min-h-0 w-full flex-col overflow-hidden rounded-xl border border-border bg-card p-5">
      <div className="mb-3 flex items-baseline justify-between">
        <h2 className="text-sm font-semibold tracking-tight">重要日期</h2>
        <span className="font-mono text-caption uppercase tracking-wider text-muted-foreground">
          {events.length} EVENTS
        </span>
      </div>
      {events.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center py-8 text-center">
          <CalendarClock className="size-6 text-muted-foreground/40" />
          <p className="mt-2 text-xs text-muted-foreground">暂无近期事件</p>
          <p className="mt-1 text-caption text-muted-foreground/70">
            导入案件后,开庭日 / 保全续封会自动出现在这里
          </p>
        </div>
      ) : (
        <div className="min-h-0 flex-1 space-y-3 overflow-y-auto pr-1">
          {prominent.length > 0 && (
            <ul className="space-y-2">
              {prominent.map((e, i) => (
                <EventRow
                  key={`${e.caseId}-${e.date}-p${i}`}
                  e={e}
                  variant="prominent"
                  onPick={() => onPickCase(e.caseId)}
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
                    onPick={() => onPickCase(e.caseId)}
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
    <div className="rounded-xl border border-border bg-card p-5">
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
  const Icon =
    e.kind === "hearing"
      ? Gavel
      : e.kind === "todo" || e.kind === "manual"
        ? CalendarClock
        : isPreserv
          ? ShieldAlert
          : AlertTriangle;
  const countdown =
    e.daysFromNow === 0 ? "D-DAY" : e.daysFromNow > 0 ? `D-${e.daysFromNow}` : `逾期${-e.daysFromNow}天`;

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
          <span className="shrink-0 text-xs text-foreground">{e.type}</span>
          <span className="truncate text-caption text-muted-foreground">· {e.caseName}</span>
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

  return (
    <li>
      <button
        type="button"
        onClick={onPick}
        className={`flex w-full items-center gap-3.5 rounded-lg px-3.5 py-3 text-left transition-colors hover:brightness-95 dark:hover:brightness-110 ${box}`}
        title={`打开案件 · ${e.caseName}`}
      >
        <div className="shrink-0 text-center">
          <div className={`font-mono text-xl font-bold leading-none ${cdCls}`}>{countdown}</div>
          <div className="mt-1 font-mono text-caption text-muted-foreground">{e.date.slice(5)}</div>
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-1.5">
            <Icon className={`size-3.5 shrink-0 ${iconCls}`} />
            <span className="text-sm font-semibold text-foreground">{e.type}</span>
            {hint && <span className={`rounded px-1.5 py-0.5 text-caption font-medium ${hintCls}`}>{hint}</span>}
          </div>
          {e.note && <p className="mt-0.5 truncate text-xs text-muted-foreground">{e.note}</p>}
          <p className="mt-0.5 truncate text-xs text-foreground/80">{e.caseName}</p>
          {e.court && <p className="mt-0.5 truncate text-caption text-muted-foreground/70">{e.court}</p>}
        </div>
      </button>
    </li>
  );
}

function CalendarPanel({
  events,
  onPickCase,
  onAddEvent,
  onDeleteEvent,
}: {
  events: UpcomingEvent[];
  onPickCase: (caseId: string) => void;
  onAddEvent: (date: string, title: string) => void | Promise<void>;
  onDeleteEvent: (id: string) => void | Promise<void>;
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
  const [menu, setMenu] = useState<{ date: string; x: number; y: number } | null>(null);
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
    <section className="rounded-xl border border-border bg-card p-5">
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
            summaryEvents.map((event, index) => (
              <button
                key={`${event.caseId}-${event.date}-${index}`}
                type="button"
                onClick={() => onPickCase(event.caseId)}
                className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs hover:bg-muted/60"
              >
                <span className={cn("size-2 shrink-0 rounded-full", calendarDotClass(event))} />
                <span className="shrink-0 font-mono text-caption text-muted-foreground">
                  {event.date.slice(5)}
                </span>
                <span className="font-medium text-foreground">{event.type}</span>
                <span className="truncate text-muted-foreground">{event.caseName}</span>
                <span className="ml-auto shrink-0 font-mono text-caption text-muted-foreground">
                  {event.daysFromNow === 0
                    ? "D-DAY"
                    : event.daysFromNow > 0
                      ? `D-${event.daysFromNow}`
                      : `逾期${-event.daysFromNow}天`}
                </span>
              </button>
            ))
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
                setMenu({ date: key, x: e.clientX, y: e.clientY });
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
            {selectedEvents.map((event, index) => {
              const isManual = event.kind === "manual";
              const countdown =
                event.daysFromNow === 0
                  ? "D-DAY"
                  : event.daysFromNow > 0
                    ? `D-${event.daysFromNow}`
                    : `逾期${-event.daysFromNow}天`;
              return (
                <li
                  key={`${event.id ?? event.caseId}-${event.date}-${index}`}
                  className="group flex items-center gap-2 rounded-md px-2 py-1.5 text-xs hover:bg-muted/60"
                >
                  <span className={cn("size-2 shrink-0 rounded-full", calendarDotClass(event))} />
                  {isManual ? (
                    <span className="flex-1 font-medium text-foreground">{event.type}</span>
                  ) : (
                    <button
                      type="button"
                      onClick={() => onPickCase(event.caseId)}
                      className="flex flex-1 items-center gap-2 overflow-hidden text-left"
                    >
                      <span className="shrink-0 font-medium text-foreground">{event.type}</span>
                      <span className="truncate text-muted-foreground">{event.caseName}</span>
                    </button>
                  )}
                  <span className="ml-auto shrink-0 font-mono text-caption text-muted-foreground">
                    {countdown}
                  </span>
                  {isManual && event.id && (
                    <button
                      type="button"
                      onClick={() => void onDeleteEvent(event.id!)}
                      aria-label="删除日程"
                      className="shrink-0 rounded p-0.5 text-muted-foreground opacity-0 transition hover:bg-destructive/10 hover:text-destructive group-hover:opacity-100"
                    >
                      <Trash2 className="size-3.5" />
                    </button>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>
        </>
      )}

      {/* 右键某天弹出的菜单:点「添加日程」→ 打开当天添加输入 */}
      {menu && (
        <>
          <div className="fixed inset-0 z-40" onClick={() => setMenu(null)} />
          <div
            className="fixed z-50 overflow-hidden rounded-md border border-border bg-card shadow-lg"
            style={{ left: menu.x, top: menu.y }}
          >
            <button
              type="button"
              onClick={() => {
                setSelectedDate(menu.date);
                setAddDate(menu.date);
                setAddInput("");
                setMenu(null);
              }}
              className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs hover:bg-sky-50 dark:hover:bg-sky-950/30"
            >
              <Plus className="size-3.5 text-sky-600" />
              在 {menu.date.slice(5)} 添加日程
            </button>
          </div>
        </>
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
  const partySummary = `${left}${leftMore} vs ${right}${rightMore}`;
  const hasParties = left !== "-" || right !== "-";
  const cause = ovStr("agg_cause", caseData.agg_cause);
  return {
    title: hasParties ? partySummary : caseData.name,
    subtitle: cause && cause !== caseData.name ? cause : null,
    caseNo: ovStr("agg_case_no", caseData.agg_case_no),
    court: ovStr("agg_court", caseData.agg_court),
    cause,
    claimAmount,
    plaintiffs,
    defendants,
    judges,
    partySummary,
    amountText: claimAmount ? formatYuan(claimAmount) : null,
  };
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

function buildUpcomingEvents(cases: Case[]): UpcomingEvent[] {
  const events: UpcomingEvent[] = [];
  const now = todayDate();
  for (const c of cases) {
    const caseName = buildCaseDisplay(c).title;
    let nearestHearing: UpcomingEvent | null = null;
    for (const kd of readKeyDates(c)) {
      if (kd.event?.includes("开庭") && kd.date) {
        const d = parseDate(kd.date);
        if (d) {
          const daysFromNow = diffDays(d, now);
          if (daysFromNow >= 0 && daysFromNow <= 365) {
            if (!nearestHearing || daysFromNow < nearestHearing.daysFromNow) {
              nearestHearing = {
                kind: "hearing",
                date: kd.date,
                daysFromNow,
                type: kd.event,
                note: kd.note ?? null,
                caseName,
                caseId: c.id,
                court: c.agg_court,
              };
            }
          }
        }
      }
      // 2026-06-14:重要提醒只留"开庭 + 续封/保全"两类真要紧的。其余到期项
      // (上诉期/举证期限等)交给底下日程日历显示,这里不重复堆。
      if (kd.expires_at && PRESERVATION_RE.test(kd.event ?? "到期")) {
        const d = parseDate(kd.expires_at);
        if (d) {
          const daysFromNow = diffDays(d, now);
          if (daysFromNow >= -30 && daysFromNow <= 365) {
            events.push({
              kind: "deadline",
              date: kd.expires_at,
              daysFromNow,
              type: kd.event ?? "到期",
              note: kd.note ?? null,
              caseName,
              caseId: c.id,
              court: c.agg_court,
            });
          }
        }
      }
    }
    if (nearestHearing) events.push(nearestHearing);
  }
  const rank = { overdue: 0, urgent: 1, normal: 2 } as const;
  return events
    .sort((a, b) => {
      const ra = rank[eventUrgency(a)];
      const rb = rank[eventUrgency(b)];
      if (ra !== rb) return ra - rb;
      if (a.daysFromNow !== b.daysFromNow) return a.daysFromNow - b.daysFromNow;
      if (a.kind !== b.kind) return a.kind === "hearing" ? -1 : 1;
      return 0;
    })
    .slice(0, 12);
}

function buildAllCalendarEvents(cases: Case[]): UpcomingEvent[] {
  const events: UpcomingEvent[] = [];
  const now = todayDate();
  for (const c of cases) {
    const caseName = buildCaseDisplay(c).title;
    for (const kd of readKeyDates(c)) {
      if (kd.event?.includes("开庭") && kd.date) {
        const d = parseDate(kd.date);
        if (d) {
          events.push({
            kind: "hearing",
            date: kd.date,
            daysFromNow: diffDays(d, now),
            type: kd.event,
            note: kd.note ?? null,
            caseName,
            caseId: c.id,
            court: c.agg_court,
          });
        }
      }
      if (kd.expires_at) {
        const d = parseDate(kd.expires_at);
        if (d) {
          events.push({
            kind: "deadline",
            date: kd.expires_at,
            daysFromNow: diffDays(d, now),
            type: kd.event ?? "到期",
            note: kd.note ?? null,
            caseName,
            caseId: c.id,
            court: c.agg_court,
          });
        }
      }
    }
  }
  return events.sort((a, b) => a.date.localeCompare(b.date));
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
      note: null,
      caseName: t.case_name,
      caseId: t.case_id,
      court: null,
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
      note: null,
      caseName: "",
      caseId: "",
      court: null,
      id: e.id,
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

function eventUrgency(e: UpcomingEvent): "overdue" | "urgent" | "normal" {
  if (e.daysFromNow < 0) return "overdue";
  if (e.kind === "hearing") return e.daysFromNow <= 30 ? "urgent" : "normal";
  return e.daysFromNow <= 90 ? "urgent" : "normal";
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

function parseDate(value: string): Date | null {
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return null;
  d.setHours(0, 0, 0, 0);
  return d;
}

function todayDate(): Date {
  const now = new Date();
  now.setHours(0, 0, 0, 0);
  return now;
}

function diffDays(a: Date, b: Date): number {
  return Math.round((a.getTime() - b.getTime()) / 86400000);
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
  if (h < 6) return `深夜好,${who}`;
  if (h < 12) return `上午好,${who}`;
  if (h < 14) return `中午好,${who}`;
  if (h < 18) return `下午好,${who}`;
  return `晚上好,${who}`;
}

function EmptyCases({ onImport }: { onImport: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center rounded-xl border border-dashed border-border bg-card/30 px-6 py-16 text-center">
      <FolderOpen className="size-10 text-muted-foreground/40" />
      <p className="mt-4 text-base font-medium text-foreground">还没有导入任何案件</p>
      <p className="mt-1 text-sm text-muted-foreground">选择一个案件文件夹开始</p>
      <Button onClick={onImport} className="mt-6">
        <FolderOpen className="size-3.5" />
        导入案件文件夹
      </Button>
    </div>
  );
}

import { useEffect, useMemo, useState } from "react";
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
  ShieldAlert,
  Square,
  X,
} from "lucide-react";
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
  getCaseWithDocs,
  getSettings,
  updateHomeCaseOrder,
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

export interface HomeViewProps {
  cases: Case[];
  userDisplayName: string | null;
  onPickCase: (caseId: string) => void;
  onImport: () => void;
}

type ViewMode = "grid" | "list";
type SortKey = "status" | "amount" | "filed_at" | "hearing";
type SortDir = "asc" | "desc";
type EventKind = "hearing" | "deadline";

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

interface UpcomingEvent {
  kind: EventKind;
  date: string;
  daysFromNow: number;
  type: string;
  note?: string | null;
  caseName: string;
  caseId: string;
  court?: string | null;
}

const PRESERVATION_RE = /保全|续封|查封|冻结/;

export function HomeView({ cases, userDisplayName, onPickCase, onImport }: HomeViewProps) {
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
  const [selectMode, setSelectMode] = useState(false);
  const [selectedCaseIds, setSelectedCaseIds] = useState<Set<string>>(new Set());

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
    getSettings()
      .then((s) => {
        if (!cancelled) setUserOrder(s.home_case_order);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
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

  const userOrderedRows = (() => {
    if (!userOrder || userOrder.length === 0) return defaultSorted;
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
    return result;
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
  const calendarEvents = buildAllCalendarEvents(activeCases);

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
  );
  const sortedIds = filteredRows.map((row) => row.caseData.id);
  const visibleIds = sortedIds;
  const allVisibleSelected =
    visibleIds.length > 0 && visibleIds.every((id) => selectedCaseIds.has(id));

  const handleChangeStatus = async (caseId: string, status: StatusId | null) => {
    setStatusOverride((m) => ({ ...m, [caseId]: status }));
    try {
      await updateWorkflowStatus(caseId, status);
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

  const toggleSelected = (caseId: string) => {
    setSelectedCaseIds((prev) => {
      const next = new Set(prev);
      if (next.has(caseId)) next.delete(caseId);
      else next.add(caseId);
      return next;
    });
  };

  const clearFilters = () => {
    setStatusFilters(new Set());
    setCourtFilter("");
  };

  const selectAllVisible = () => {
    setSelectedCaseIds((prev) => {
      const next = new Set(prev);
      for (const id of visibleIds) next.add(id);
      return next;
    });
  };

  const invertVisible = () => {
    setSelectedCaseIds((prev) => {
      const next = new Set(prev);
      for (const id of visibleIds) {
        if (next.has(id)) next.delete(id);
        else next.add(id);
      }
      return next;
    });
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
          <div className="mb-10 grid grid-cols-1 gap-6 md:grid-cols-2">
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
            <ImportantDates events={upcomingEvents} onPickCase={onPickCase} />
          </div>

          {cases.length > 0 && (
            <div className="mb-8">
              <CalendarPanel events={calendarEvents} onPickCase={onPickCase} />
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
                  <Button
                    type="button"
                    variant={selectMode ? "default" : "outline"}
                    size="sm"
                    onClick={() => setSelectMode((v) => !v)}
                  >
                    {selectMode ? <CheckSquare className="size-3.5" /> : <Square className="size-3.5" />}
                    多选
                  </Button>
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
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => setSortDir((d) => (d === "asc" ? "desc" : "asc"))}
                >
                  <ArrowUpDown className="size-3.5" />
                  {sortDir === "asc" ? "升序" : "降序"}
                </Button>
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

              {selectMode && (
                <div className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-dashed border-border bg-muted/30 p-3">
                  <span className="text-sm text-foreground">
                    已选 <strong>{selectedCaseIds.size}</strong> 个案件
                  </span>
                  <div className="flex flex-wrap gap-2">
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={allVisibleSelected ? () => setSelectedCaseIds(new Set()) : selectAllVisible}
                    >
                      {allVisibleSelected ? "取消全选" : "全选当前结果"}
                    </Button>
                    <Button type="button" variant="outline" size="sm" onClick={invertVisible}>
                      反选当前结果
                    </Button>
                    <Button type="button" variant="ghost" size="sm" onClick={() => setSelectedCaseIds(new Set())}>
                      清空
                    </Button>
                  </div>
                </div>
              )}
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
                    selectMode={selectMode}
                    selected={selectedCaseIds.has(row.caseData.id)}
                    onToggleSelected={() => toggleSelected(row.caseData.id)}
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
                    {filteredRows.map((row) => (
                      <SortableCaseCard
                        key={row.caseData.id}
                        row={row}
                        selectMode={selectMode}
                        selected={selectedCaseIds.has(row.caseData.id)}
                        onToggleSelected={() => toggleSelected(row.caseData.id)}
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
  selectMode: boolean;
  selected: boolean;
  onToggleSelected: () => void;
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
  selectMode,
  selected,
  onToggleSelected,
  onClick,
  onChangeStatus,
  dragHandleProps,
  isDragging,
}: {
  row: CaseRow;
  selectMode: boolean;
  selected: boolean;
  onToggleSelected: () => void;
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
      aria-label={`打开案件 ${display.cause || caseData.name}`}
    >
      {selectMode && (
        <button
          type="button"
          aria-label={selected ? "取消选择案件" : "选择案件"}
          onClick={(e) => {
            e.stopPropagation();
            onToggleSelected();
          }}
          className="absolute left-2 top-2 z-20 rounded-md bg-card/90 p-1 text-muted-foreground shadow-sm transition-colors hover:text-foreground"
        >
          {selected ? <CheckSquare className="size-4" /> : <Square className="size-4" />}
        </button>
      )}
      {dragHandleProps && !selectMode && (
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
  selectMode,
  selected,
  onToggleSelected,
  onClick,
  onChangeStatus,
}: {
  row: CaseRow;
  selectMode: boolean;
  selected: boolean;
  onToggleSelected: () => void;
  onClick: () => void;
  onChangeStatus: (s: StatusId | null) => void;
}) {
  const { caseData, status, display } = row;
  return (
    <div
      className={cn(
        "grid cursor-pointer grid-cols-[minmax(0,1.35fr)_minmax(0,1fr)_minmax(0,1fr)_minmax(0,1fr)_auto] items-center gap-3 border-b border-border px-4 py-3 text-left transition-colors last:border-b-0 hover:bg-muted/50",
        selectMode && "grid-cols-[auto_minmax(0,1.35fr)_minmax(0,1fr)_minmax(0,1fr)_minmax(0,1fr)_auto]",
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
      aria-label={`打开案件 ${display.cause || caseData.name}`}
    >
      {selectMode && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onToggleSelected();
          }}
          className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
          aria-label={selected ? "取消选择案件" : "选择案件"}
        >
          {selected ? <CheckSquare className="size-4" /> : <Square className="size-4" />}
        </button>
      )}
      <div className="min-w-0">
        <div className="truncate text-sm font-semibold text-foreground">
          {display.cause || caseData.name}
        </div>
        <div className="truncate text-xs text-muted-foreground">{display.partySummary}</div>
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
    <div className="rounded-xl border border-border bg-card p-5">
      <div className="mb-3 flex items-baseline justify-between">
        <h2 className="text-sm font-semibold tracking-tight">重要日期</h2>
        <span className="font-mono text-caption uppercase tracking-wider text-muted-foreground">
          {events.length} EVENTS
        </span>
      </div>
      {events.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-8 text-center">
          <CalendarClock className="size-6 text-muted-foreground/40" />
          <p className="mt-2 text-xs text-muted-foreground">暂无近期事件</p>
          <p className="mt-1 text-caption text-muted-foreground/70">
            导入案件后,开庭日 / 保全续封会自动出现在这里
          </p>
        </div>
      ) : (
        <div className="space-y-3">
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
  const Icon = e.kind === "hearing" ? Gavel : isPreserv ? ShieldAlert : AlertTriangle;
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
}: {
  events: UpcomingEvent[];
  onPickCase: (caseId: string) => void;
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
        </div>
      </div>
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
              className={cn(
                "min-h-20 bg-card p-2 text-left transition-colors hover:bg-muted/50",
                !isCurrentMonth && "bg-muted/20 text-muted-foreground/50",
                isSelected && "ring-2 ring-inset ring-foreground/40",
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
        <div className="mb-2 text-xs font-medium text-foreground">{selectedDate} 日程</div>
        {selectedEvents.length === 0 ? (
          <p className="text-xs text-muted-foreground">当天暂无日程</p>
        ) : (
          <ul className="space-y-1.5">
            {selectedEvents.map((event, index) => (
              <li key={`${event.caseId}-${event.date}-${index}`}>
                <button
                  type="button"
                  onClick={() => onPickCase(event.caseId)}
                  className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs hover:bg-muted/60"
                >
                  <span className={cn("size-2 rounded-full", calendarDotClass(event))} />
                  <span className="font-medium text-foreground">{event.type}</span>
                  <span className="truncate text-muted-foreground">{event.caseName}</span>
                  {event.court && <span className="hidden truncate text-muted-foreground/70 md:inline">{event.court}</span>}
                  <span className="ml-auto shrink-0 font-mono text-caption text-muted-foreground">
                    {event.daysFromNow === 0 ? "D-DAY" : event.daysFromNow > 0 ? `D-${event.daysFromNow}` : `逾期${-event.daysFromNow}天`}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
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
    const caseName = c.agg_cause || c.name;
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
      if (kd.expires_at) {
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
    const caseName = c.agg_cause || c.name;
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
  return e.kind === "hearing" ? "bg-blue-500" : "bg-slate-400";
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

import { EyeOff, Eye, GripVertical, RotateCcw, X } from "lucide-react";
import type { DraggableAttributes } from "@dnd-kit/core";
import type { SyntheticListenerMap } from "@dnd-kit/core/dist/hooks/utilities";

import { cn } from "@/lib/utils";

import { EditableField } from "./EditableField";

/** 卡片拖把手所需的 sortable 元数据,由 useSortable hook 提供。 */
export interface DragHandleProps {
  attributes: DraggableAttributes;
  listeners: SyntheticListenerMap | undefined;
}

export function Dash() {
  return <span className="text-muted-foreground/40">—</span>;
}

/* ============================================================ */
/* CardSection — 卡片框架(编辑模式可隐藏)                       */
/* ============================================================ */

export function CardSection({
  title,
  subtitle,
  children,
  isEditMode = false,
  hidden = false,
  onToggleHidden,
  dragHandle,
}: {
  title: string;
  subtitle?: string;
  children: React.ReactNode;
  /** 编辑模式:右上角加 EyeOff 按钮可隐藏整张卡片(只删显示) */
  isEditMode?: boolean;
  /** 是否被用户隐藏。隐藏后非编辑态完全不渲染,编辑态显示折叠占位卡 */
  hidden?: boolean;
  /** 切换隐藏状态 */
  onToggleHidden?: () => void;
  /** 编辑模式拖把手 sortable props,标题左侧渲染 GripVertical(只在编辑态) */
  dragHandle?: DragHandleProps;
}) {
  // 非编辑态 + 已隐藏 → 完全不渲染
  if (hidden && !isEditMode) return null;

  // 编辑态 + 已隐藏 → 折叠占位(让用户能找回来,但仍参与拖拽排序)
  if (hidden && isEditMode) {
    return (
      <section className="rounded-lg border border-dashed border-border bg-muted/30 px-5 py-3">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            {dragHandle && (
              <button
                type="button"
                aria-label="拖动调整顺序"
                title="按住拖动调整卡片顺序"
                className="cursor-grab touch-none rounded p-0.5 text-muted-foreground/40 hover:text-foreground active:cursor-grabbing"
                {...dragHandle.attributes}
                {...dragHandle.listeners}
              >
                <GripVertical className="size-4" />
              </button>
            )}
            <h3 className="text-sm font-medium text-muted-foreground">
              {title}{" "}
              <span className="text-caption text-muted-foreground/60">
                (已隐藏)
              </span>
            </h3>
          </div>
          <button
            type="button"
            onClick={onToggleHidden}
            className="inline-flex items-center gap-1 rounded p-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            title="显示这张卡片"
            aria-label="显示这张卡片"
          >
            <Eye className="size-3.5" />
            <span>显示</span>
          </button>
        </div>
      </section>
    );
  }

  return (
    <section className="surface-card px-5 py-4 sm:px-6">
      <div className="mb-3 flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          {dragHandle && (
            <button
              type="button"
              aria-label="拖动调整顺序"
              title="按住拖动调整卡片顺序"
              className="cursor-grab touch-none rounded p-0.5 text-muted-foreground/40 hover:text-foreground active:cursor-grabbing"
              {...dragHandle.attributes}
              {...dragHandle.listeners}
            >
              <GripVertical className="size-4" />
            </button>
          )}
          <h3 className="text-sm font-semibold text-foreground">{title}</h3>
        </div>
        <div className="flex items-baseline gap-2">
          {subtitle && (
            <span className="truncate text-caption text-muted-foreground">
              {subtitle}
            </span>
          )}
          {isEditMode && onToggleHidden && (
            <button
              type="button"
              onClick={onToggleHidden}
              className="rounded p-1 text-muted-foreground/60 transition-colors hover:bg-destructive/10 hover:text-destructive"
              title="隐藏这张卡片(只删显示,不删数据)"
              aria-label="隐藏这张卡片"
            >
              <EyeOff className="size-3.5" />
            </button>
          )}
        </div>
      </div>
      {children}
    </section>
  );
}

/* ============================================================ */
/* FactRow — 单个 label + value 字段(编辑模式可改)              */
/* ============================================================ */

export function FactRow({
  label,
  value,
  mono = false,
  pill = false,
  isEditMode = false,
  fieldPath,
  caseId,
  onEdit,
  hasOverride = false,
  onReset,
}: {
  label: string;
  value: string | null;
  mono?: boolean;
  pill?: boolean;
  /** 进编辑模式后允许 inline 改 */
  isEditMode?: boolean;
  /** override path,如 "agg_cause"(必须跟 hook 的 setField 一致) */
  fieldPath?: string;
  /** 用作 EditableField key,case 切换时强制 remount 防 cursor jump */
  caseId?: string;
  /** 提交编辑(失焦)。null 表示用户清空 */
  onEdit?: (path: string, value: string | null) => void;
  /** 当前值是不是用户改过的(决定是否显示 ↺ 恢复按钮) */
  hasOverride?: boolean;
  /** 点 ↺ 恢复按钮:清掉这个字段的 override,回到 LLM 抽取值 */
  onReset?: () => void;
}) {
  // 非编辑态 / 缺 fieldPath → 旧行为
  if (!isEditMode || !fieldPath || !onEdit) {
    return (
      <div>
        <dt className="text-caption uppercase tracking-wider text-muted-foreground">
          {label}
        </dt>
        <dd className={cn("mt-0.5 text-sm text-foreground", mono && "font-mono")}>
          {value ? (
            pill ? (
              <span className="inline-block rounded-full bg-muted px-2 py-0.5 text-xs">
                {value}
              </span>
            ) : (
              value
            )
          ) : (
            <Dash />
          )}
        </dd>
      </div>
    );
  }

  return (
    <div>
      <dt className="text-caption uppercase tracking-wider text-muted-foreground">
        {label}
      </dt>
      <dd className={cn("mt-0.5 text-sm text-foreground", mono && "font-mono")}>
        <EditableField
          key={`${caseId ?? ""}:${fieldPath}`}
          initialValue={value}
          editable
          onCommit={(next) => onEdit(fieldPath, next)}
          ariaLabel={`编辑 ${label}`}
          hasOverride={hasOverride}
          onReset={onReset}
        />
      </dd>
    </div>
  );
}

/* ============================================================ */
/* TablePeople — 子表(编辑模式:行右侧 × 删除 + 部分 cell 可改)  */
/* ============================================================ */

export interface TablePeopleRow {
  cells: (string | null)[];
  /** 用 rowKeyOf 生成的稳定 key(不传则该行不可删 / 不可改) */
  rowKey?: string;
}

/**
 * 单元格可编辑配置。inner 是 LLM schema 里的字段名(如 phone / email / note),
 * 配合 rowKey 和子表 path 拼成 `agg_party_contacts.{rowKey}.<inner>`。
 *
 * **不要把 row-key 字段列在这里**(name / role / event / date / item / amount):
 * 改这些会改 rowKey,同行其他 overrides 立刻变孤儿(找不到对应行)。
 */
export interface CellEditableConfig {
  /** cells 数组里第几列 */
  colIndex: number;
  /** LLM schema 字段名(用于拼 dotted path) */
  inner: string;
  /** UI 占位 */
  placeholder?: string;
}

export function TablePeople({
  headers,
  rows,
  emptyText,
  isEditMode = false,
  onDeleteRow,
  editableCells,
  onEditCell,
  hasCellOverride,
  onResetCell,
  caseId,
  deletedRows,
  onUndeleteRow,
}: {
  headers: string[];
  rows: TablePeopleRow[];
  emptyText: string;
  isEditMode?: boolean;
  /** 删除某行回调(rowKey 由 rows 携带,不在这里再生成) */
  onDeleteRow?: (rowKey: string) => void;
  /** 已删行清单(供卡片底部还原 chip 用)。label 是用户友好展示("李四 | 被告") */
  deletedRows?: Array<{ rowKey: string; label: string }>;
  /** 点 chip 还原一行(去掉 deleted_rows 里的这个 key) */
  onUndeleteRow?: (rowKey: string) => void;
  /** 哪些列可编辑(父组件配)。改 row-key 字段安全前提:rowKey 用 rawSnap 算 */
  editableCells?: CellEditableConfig[];
  /** 单元格编辑提交(rowKey + inner field name + value);null = 用户清空 */
  onEditCell?: (rowKey: string, inner: string, value: string | null) => void;
  /** 查这个 cell 是否被用户改过(决定是否显示 ↺) */
  hasCellOverride?: (rowKey: string, inner: string) => boolean;
  /** 点 ↺ 恢复 cell 到 LLM 值 */
  onResetCell?: (rowKey: string, inner: string) => void;
  /** EditableField key 用,case 切换时强制 remount */
  caseId?: string;
}) {
  const showDeleteCol = isEditMode && !!onDeleteRow;
  const cellEditableMap = new Map<number, CellEditableConfig>();
  if (isEditMode && editableCells && onEditCell) {
    for (const c of editableCells) cellEditableMap.set(c.colIndex, c);
  }
  const showUndeleteChips =
    isEditMode &&
    !!onUndeleteRow &&
    deletedRows &&
    deletedRows.length > 0;

  // 空表 — 仍要给已删行显示还原 chip(全部删完时用户得有路找回)
  if (rows.length === 0) {
    return (
      <>
        <div className="rounded-md border border-dashed border-border bg-muted/20 px-3 py-4 text-center text-xs text-muted-foreground">
          {emptyText}
        </div>
        {showUndeleteChips && (
          <DeletedRowChips rows={deletedRows!} onUndelete={onUndeleteRow!} />
        )}
      </>
    );
  }

  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-border text-caption uppercase tracking-wider text-muted-foreground">
            {headers.map((h) => (
              <th key={h} className="py-2 pr-4 text-left font-normal">
                {h}
              </th>
            ))}
            {showDeleteCol && <th className="w-8 py-2 text-left font-normal" aria-label="删除" />}
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr
              key={r.rowKey ?? `idx-${i}`}
              className="group border-b border-border/50 last:border-0"
            >
              {r.cells.map((c, j) => {
                const editCfg = r.rowKey ? cellEditableMap.get(j) : undefined;
                return (
                  <td key={j} className="py-2 pr-4 text-foreground">
                    {editCfg && r.rowKey ? (
                      <EditableField
                        key={`${caseId ?? ""}:${r.rowKey}:${editCfg.inner}`}
                        initialValue={c}
                        editable
                        placeholder={editCfg.placeholder ?? "未填"}
                        onCommit={(v) => onEditCell!(r.rowKey!, editCfg.inner, v)}
                        ariaLabel={`编辑 ${headers[j]}`}
                        hasOverride={hasCellOverride?.(r.rowKey, editCfg.inner) ?? false}
                        onReset={
                          onResetCell
                            ? () => onResetCell(r.rowKey!, editCfg.inner)
                            : undefined
                        }
                      />
                    ) : c == null || c === "" ? (
                      <Dash />
                    ) : (
                      c
                    )}
                  </td>
                );
              })}
              {showDeleteCol && (
                <td className="py-2 pr-2">
                  {r.rowKey && (
                    <button
                      type="button"
                      onClick={() => onDeleteRow?.(r.rowKey!)}
                      className="rounded p-1 text-muted-foreground/40 opacity-0 transition-all hover:bg-destructive/10 hover:text-destructive group-hover:opacity-100"
                      title="删除这一行(只删显示,不删数据)"
                      aria-label="删除这一行"
                    >
                      <X className="size-3.5" />
                    </button>
                  )}
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
      {showUndeleteChips && (
        <DeletedRowChips rows={deletedRows!} onUndelete={onUndeleteRow!} />
      )}
    </div>
  );
}

/* ============================================================ */
/* DeletedRowChips — 已删行清单,点 ↺ 还原(只删显示的反向操作)     */
/* ============================================================ */

export function DeletedRowChips({
  rows,
  onUndelete,
}: {
  rows: Array<{ rowKey: string; label: string }>;
  onUndelete: (rowKey: string) => void;
}) {
  return (
    <div className="mt-3 flex flex-wrap items-center gap-1.5 border-t border-dashed border-border pt-2 text-caption">
      <span className="text-muted-foreground">已隐藏:</span>
      {rows.map((r) => (
        <button
          key={r.rowKey}
          type="button"
          onClick={() => onUndelete(r.rowKey)}
          className="inline-flex items-center gap-1 rounded-full border border-border bg-muted/50 px-2 py-0.5 text-foreground transition-colors hover:bg-accent hover:text-foreground"
          title="还原这一行"
          aria-label={`还原 ${r.label}`}
        >
          <span className="max-w-[180px] truncate">{r.label}</span>
          <RotateCcw className="size-2.5 shrink-0" />
        </button>
      ))}
    </div>
  );
}

/* ============================================================ */
/* KeyMetric — 关键数字(Hero 区,编辑模式可改值)                */
/* ============================================================ */

export function KeyMetric({
  label,
  value,
  mono = false,
  isEditMode = false,
  fieldPath,
  caseId,
  onEdit,
  hasOverride = false,
  onReset,
}: {
  label: string;
  value: string | null;
  mono?: boolean;
  isEditMode?: boolean;
  fieldPath?: string;
  caseId?: string;
  onEdit?: (path: string, value: string | null) => void;
  hasOverride?: boolean;
  onReset?: () => void;
}) {
  const editable = isEditMode && !!fieldPath && !!onEdit;
  return (
    <div>
      <div className="text-caption uppercase tracking-wider text-muted-foreground">
        {label}
      </div>
      <div
        className={cn(
          "mt-1 text-lg font-semibold text-foreground",
          mono && "font-mono",
        )}
      >
        {editable ? (
          <EditableField
            key={`${caseId ?? ""}:${fieldPath ?? ""}`}
            initialValue={value}
            editable
            onCommit={(next) => onEdit!(fieldPath!, next)}
            ariaLabel={`编辑 ${label}`}
            editableClassName="text-lg font-semibold"
            hasOverride={hasOverride}
            onReset={onReset}
          />
        ) : (
          value || <span className="text-muted-foreground/40">—</span>
        )}
      </div>
    </div>
  );
}

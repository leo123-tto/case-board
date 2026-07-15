import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { Pencil, Trash2, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { confirmDialog } from "@/lib/dialog";
import type { HomeReminderEvent } from "./homeReminderEngine";

export interface CalendarEventEditInput {
  date: string;
  title: string;
  note: string | null;
}

export function CalendarEventActions({
  event,
  position,
  onClose,
  onEdit,
  onDelete,
}: {
  event: HomeReminderEvent;
  position: { x: number; y: number };
  onClose: () => void;
  onEdit: (input: CalendarEventEditInput) => Promise<void>;
  onDelete: () => Promise<void>;
}) {
  const [editing, setEditing] = useState(false);
  const [date, setDate] = useState(event.date);
  const [title, setTitle] = useState(event.type);
  const [note, setNote] = useState(event.note ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const menuPosition = useMemo(() => {
    const width = 176;
    const height = 92;
    const viewportWidth = typeof window === "undefined" ? position.x + width : window.innerWidth;
    const viewportHeight = typeof window === "undefined" ? position.y + height : window.innerHeight;
    return {
      left: Math.max(8, Math.min(position.x, viewportWidth - width - 8)),
      top: Math.max(8, Math.min(position.y, viewportHeight - height - 8)),
    };
  }, [position.x, position.y]);

  useEffect(() => {
    const onKeyDown = (keyboardEvent: KeyboardEvent) => {
      if (keyboardEvent.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const submit = async () => {
    const cleanTitle = title.trim();
    if (!date || !cleanTitle || saving) return;
    setSaving(true);
    setError(null);
    try {
      await onEdit({
        date,
        title: cleanTitle,
        note: note.trim() || null,
      });
      onClose();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    if (saving) return;
    const isAiEvent = event.kind !== "manual" && event.kind !== "todo";
    const message = isAiEvent
      ? `隐藏日程“${event.type}”?\n\n仅隐藏该提醒，不删除原始识别数据。`
      : `删除日程“${event.type}”?\n\n该操作会删除对应的${event.kind === "todo" ? "案件待办" : "个人日程"}记录。`;
    if (!(await confirmDialog(message, { okLabel: "删除", danger: true }))) return;
    setSaving(true);
    setError(null);
    try {
      await onDelete();
      onClose();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setSaving(false);
    }
  };

  if (!editing) {
    return createPortal(
      <>
        <button
          type="button"
          aria-label="关闭日程菜单"
          className="fixed inset-0 z-40 cursor-default"
          onClick={onClose}
        />
        <div
          role="menu"
          aria-label={`${event.type}日程操作`}
          className="fixed z-50 w-44 overflow-hidden rounded-md border border-border bg-card p-1 shadow-lg"
          style={menuPosition}
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => setEditing(true)}
            className="flex w-full items-center gap-2 rounded px-2.5 py-2 text-left text-xs hover:bg-muted"
          >
            <Pencil className="size-3.5 text-muted-foreground" />
            编辑
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => void remove()}
            disabled={saving}
            className="flex w-full items-center gap-2 rounded px-2.5 py-2 text-left text-xs text-destructive hover:bg-destructive/10 disabled:opacity-50"
          >
            <Trash2 className="size-3.5" />
            删除
          </button>
          {error && <p className="px-2.5 py-1 text-caption text-destructive">{error}</p>}
        </div>
      </>,
      document.body,
    );
  }

  return createPortal(
    <div className="fixed inset-0 z-[70] flex items-center justify-center bg-black/30 p-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-label="编辑日程"
        className="w-full max-w-md rounded-xl border border-border bg-card p-5 shadow-2xl"
      >
        <div className="mb-4 flex items-start justify-between gap-3">
          <div>
            <h3 className="text-base font-semibold text-foreground">编辑日程</h3>
            <p className="mt-1 text-xs text-muted-foreground">
              {event.caseName || "个人日程"}
              {event.sourceDoc?.filename ? ` · 来源：${event.sourceDoc.filename}` : ""}
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭编辑日程"
            className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <X className="size-4" />
          </button>
        </div>

        <div className="space-y-3">
          <label className="block text-xs font-medium text-foreground">
            日期
            <input
              aria-label="日期"
              type="date"
              value={date}
              onChange={(changeEvent) => setDate(changeEvent.target.value)}
              className="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-sky-500"
            />
          </label>
          <label className="block text-xs font-medium text-foreground">
            事项名称
            <input
              aria-label="事项名称"
              value={title}
              onChange={(changeEvent) => setTitle(changeEvent.target.value)}
              className="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-sky-500"
            />
          </label>
          <label className="block text-xs font-medium text-foreground">
            备注
            <textarea
              aria-label="备注"
              rows={3}
              value={note}
              onChange={(changeEvent) => setNote(changeEvent.target.value)}
              className="mt-1 w-full resize-none rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-sky-500"
            />
          </label>
          {error && <p className="text-xs text-destructive">保存失败：{error}</p>}
        </div>

        <div className="mt-5 flex justify-end gap-2">
          <Button type="button" variant="outline" onClick={onClose} disabled={saving}>
            取消
          </Button>
          <Button
            type="button"
            onClick={() => void submit()}
            disabled={saving || !date || !title.trim()}
          >
            {saving ? "保存中…" : "保存修改"}
          </Button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

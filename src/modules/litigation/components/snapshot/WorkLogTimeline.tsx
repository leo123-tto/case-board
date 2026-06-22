/**
 * 工作记录时间轴 — 记录律师为案件付出的具体劳动。
 *
 * 2026-06-14:
 *   - 琥珀色(Amber)视觉主题
 *   - 顶部常驻输入框:日期时间选择器 + 内容输入
 *   - 倒序排列(从近到远)
 *   - 内联编辑:支持修改日期时间 + 内容
 */
import { useState, useRef, useCallback, useEffect } from "react";
import { Plus, X, Calendar, Clock } from "lucide-react";
import type { WorkLog } from "@/lib/types";

/** 获取当前时间的 ISO 字符串（本地时区） */
function nowISO(): string {
  const d = new Date();
  const year = d.getFullYear();
  const month = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  const hours = String(d.getHours()).padStart(2, "0");
  const minutes = String(d.getMinutes()).padStart(2, "0");
  const seconds = String(d.getSeconds()).padStart(2, "0");
  const ms = String(d.getMilliseconds()).padStart(3, "0");
  const offset = -d.getTimezoneOffset();
  const sign = offset >= 0 ? "+" : "-";
  const offsetHours = String(Math.floor(Math.abs(offset) / 60)).padStart(2, "0");
  const offsetMinutes = String(Math.abs(offset) % 60).padStart(2, "0");
  return `${year}-${month}-${day}T${hours}:${minutes}:${seconds}.${ms}${sign}${offsetHours}:${offsetMinutes}`;
}

/** 将 ISO 时间字符串拆分为 date (YYYY-MM-DD) 和 time (HH:mm) */
function splitDateTime(iso: string): { date: string; time: string } {
  const d = new Date(iso);
  const year = d.getFullYear();
  const month = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  const hours = String(d.getHours()).padStart(2, "0");
  const minutes = String(d.getMinutes()).padStart(2, "0");
  return {
    date: `${year}-${month}-${day}`,
    time: `${hours}:${minutes}`,
  };
}

/** 从 date 和 time 字符串合并为 ISO 时间戳 */
function mergeDateTime(date: string, time: string): string {
  const [y, m, d] = date.split("-").map(Number);
  const [h, min] = time.split(":").map(Number);
  const dt = new Date(y, m - 1, d, h, min, 0, 0);
  // 生成带时区偏移的 ISO 字符串
  const offset = -dt.getTimezoneOffset();
  const sign = offset >= 0 ? "+" : "-";
  const offsetHours = String(Math.floor(Math.abs(offset) / 60)).padStart(2, "0");
  const offsetMinutes = String(Math.abs(offset) % 60).padStart(2, "0");
  return `${date}T${time}:00.000${sign}${offsetHours}:${offsetMinutes}`;
}

/**
 * 将 ISO 时间戳格式化为展示用时间。
 * - 今天:显示 HH:mm
 * - 非今天:显示 MM-DD HH:mm
 */
function formatDisplayTime(iso: string): string {
  const d = new Date(iso);
  const now = new Date();
  const isToday =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();

  const time = d.toLocaleTimeString("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });

  if (isToday) return time;

  const month = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${month}-${day} ${time}`;
}

interface Props {
  logs: WorkLog[];
  onAdd: (content: string, logTime?: string) => void;
  onDelete: (id: string) => void;
  /** 内联编辑后:更新内容或时间 */
  onEdit?: (oldLog: WorkLog, newContent: string, newLogTime: string) => void;
}

export function WorkLogTimeline({ logs, onAdd, onDelete, onEdit }: Props) {
  const [inputValue, setInputValue] = useState("");
  const [selectedDate, setSelectedDate] = useState(() => splitDateTime(nowISO()).date);
  const [selectedTime, setSelectedTime] = useState(() => splitDateTime(nowISO()).time);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editContent, setEditContent] = useState("");
  const [editDate, setEditDate] = useState("");
  const [editTime, setEditTime] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  // 每分钟更新一次默认时间显示
  const resetToNow = useCallback(() => {
    const now = splitDateTime(nowISO());
    setSelectedDate(now.date);
    setSelectedTime(now.time);
  }, []);

  useEffect(() => {
    const timer = setInterval(resetToNow, 60_000);
    return () => clearInterval(timer);
  }, [resetToNow]);

  const handleSave = () => {
    const trimmed = inputValue.trim();
    if (!trimmed) return;
    const logTime = mergeDateTime(selectedDate, selectedTime);
    onAdd(trimmed, logTime);
    setInputValue("");
    resetToNow();
  };

  const handleStartEdit = (log: WorkLog) => {
    const { date, time } = splitDateTime(log.log_time);
    setEditingId(log.id);
    setEditContent(log.content);
    setEditDate(date);
    setEditTime(time);
  };

  const handleConfirmEdit = (oldLog: WorkLog) => {
    const trimmed = editContent.trim();
    if (!trimmed) {
      setEditingId(null);
      return;
    }
    const newLogTime = mergeDateTime(editDate, editTime);
    if (trimmed !== oldLog.content || newLogTime !== oldLog.log_time) {
      onEdit?.(oldLog, trimmed, newLogTime);
    }
    setEditingId(null);
  };

  const handleCancelEdit = () => {
    setEditingId(null);
  };

  return (
    <div className="space-y-1">
      {/* 顶部常驻输入框 */}
      <div className="flex items-stretch gap-2 py-2 border-b border-border mb-2">
        {/* 日期时间选择器 */}
        <div className="flex items-center gap-1 shrink-0">
          <div className="flex items-center gap-1 rounded-md border border-border bg-muted/40 px-2 py-1.5">
            <Calendar className="h-3.5 w-3.5 text-amber-600" />
            <input
              type="date"
              className="bg-transparent text-xs font-mono text-amber-700 w-[7rem] focus:outline-none"
              value={selectedDate}
              onChange={(e) => setSelectedDate(e.target.value)}
            />
          </div>
          <div className="flex items-center gap-1 rounded-md border border-border bg-muted/40 px-2 py-1.5">
            <Clock className="h-3.5 w-3.5 text-amber-600" />
            <input
              type="time"
              className="bg-transparent text-xs font-mono text-amber-700 w-[4.5rem] focus:outline-none"
              value={selectedTime}
              onChange={(e) => setSelectedTime(e.target.value)}
            />
          </div>
        </div>
        <div className="flex-1 flex items-center gap-1">
          <input
            ref={inputRef}
            className="flex-1 rounded-md border border-border px-2.5 py-1.5 text-xs placeholder:text-muted-foreground/60 focus:outline-none focus:ring-1 focus:ring-amber-400 focus:border-amber-400"
            value={inputValue}
            onChange={(e) => setInputValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") handleSave();
            }}
            placeholder="记录工作内容，如：电话沟通案情、撰写起诉状、会见当事人..."
          />
          <button
            className="shrink-0 rounded-md bg-amber-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-amber-600 transition-colors disabled:opacity-40"
            onClick={handleSave}
            disabled={!inputValue.trim()}
          >
            保存
          </button>
        </div>
      </div>

      {/* 空状态 */}
      {logs.length === 0 && (
        <div className="flex flex-col items-center gap-2 py-4 text-sm text-muted-foreground">
          <span>暂无工作记录</span>
        </div>
      )}

      {/* 时间轴列表(倒序:已按后端 DESC 排列) */}
      {logs.map((log) => {
        const isEditing = editingId === log.id;

        return (
          <div key={log.id} className="group flex items-start gap-3 py-1.5">
            {/* 时间轴点 + 线 */}
            <div className="relative flex flex-col items-center pt-1.5">
              <div className="h-2.5 w-2.5 rounded-full bg-amber-500 ring-2 ring-background" />
            </div>

            {/* 时间 */}
            <div className="w-16 shrink-0 pt-0.5 text-xs text-muted-foreground font-mono">
              {isEditing ? (
                <div className="flex items-center gap-0.5">
                  <input
                    type="date"
                    className="w-[6rem] rounded border border-border px-0.5 py-0.5 text-[10px] font-mono focus:outline-none focus:ring-1 focus:ring-amber-400"
                    value={editDate}
                    onChange={(e) => setEditDate(e.target.value)}
                  />
                  <input
                    type="time"
                    className="w-[4rem] rounded border border-border px-0.5 py-0.5 text-[10px] font-mono focus:outline-none focus:ring-1 focus:ring-amber-400"
                    value={editTime}
                    onChange={(e) => setEditTime(e.target.value)}
                  />
                </div>
              ) : (
                formatDisplayTime(log.log_time)
              )}
            </div>

            {/* 工作内容 */}
            <div className="min-w-0 flex-1">
              {isEditing ? (
                <div className="flex items-start gap-1">
                  <input
                    className="flex-1 rounded border border-border px-1.5 py-0.5 text-xs focus:outline-none focus:ring-1 focus:ring-amber-400"
                    value={editContent}
                    onChange={(e) => setEditContent(e.target.value)}
                    placeholder="工作内容"
                    autoFocus
                    onKeyDown={(e) => {
                      if (e.key === "Enter") handleConfirmEdit(log);
                      if (e.key === "Escape") handleCancelEdit();
                    }}
                  />
                  <button
                    className="rounded p-0.5 hover:bg-muted text-xs"
                    onClick={() => handleConfirmEdit(log)}
                    title="保存"
                  >
                    ✓
                  </button>
                  <button
                    className="rounded p-0.5 hover:bg-muted text-xs"
                    onClick={handleCancelEdit}
                    title="取消"
                  >
                    ✕
                  </button>
                </div>
              ) : (
                <div
                  className="text-xs leading-relaxed cursor-default rounded px-1.5 py-0.5 -ml-1.5 hover:bg-amber-50/60 transition-colors"
                  onDoubleClick={() => handleStartEdit(log)}
                  title="双击编辑"
                >
                  {log.content}
                </div>
              )}
            </div>

            {/* 删除按钮 */}
            <div className="shrink-0 pt-0.5">
              <button
                className="rounded p-0.5 text-muted-foreground/40 hover:text-red-500 hover:bg-red-50 opacity-0 group-hover:opacity-100 transition-all"
                onClick={() => onDelete(log.id)}
                title="删除"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          </div>
        );
      })}

      {/* 底部快速入口 */}
      <div className="pt-1">
        <button
          className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs text-amber-600 hover:bg-amber-50 transition-colors"
          onClick={() => {
            inputRef.current?.focus();
          }}
        >
          <Plus className="h-3.5 w-3.5" />
          记录工作
        </button>
      </div>
    </div>
  );
}

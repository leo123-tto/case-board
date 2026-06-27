import { useEffect, useMemo, useState } from "react";
import {
  Check,
  FileText,
  FolderOpen,
  Pencil,
  Plus,
  RefreshCw,
} from "lucide-react";

import {
  loadMemoryVault,
  revealInFinder,
  saveMemoryNote,
} from "@/lib/api";
import type { MemoryNote, MemoryVaultStatus, SaveMemoryNoteInput } from "@/lib/types";
import { cn } from "@/lib/utils";

const CATEGORIES = [
  { id: "all", label: "全部" },
  { id: "cold_start", label: "冷启动" },
  { id: "global", label: "全局" },
  { id: "case", label: "案件" },
  { id: "function", label: "功能" },
  { id: "writing", label: "写作" },
  { id: "workflow", label: "流程" },
  { id: "other", label: "其他" },
];

const INJECT_MODES = [
  { id: "manual_select", label: "只存档" },
  { id: "global_prompt", label: "全局注入" },
  { id: "case_prompt", label: "案件注入" },
  { id: "writing_prompt", label: "写作注入" },
  { id: "tool_prompt", label: "工具注入" },
  { id: "never", label: "不注入" },
];

const EMPTY_DRAFT: SaveMemoryNoteInput = {
  id: null,
  title: "",
  category: "global",
  content: "",
  source: "manual",
  inject_mode: "manual_select",
};

export function MemoryModule() {
  const [status, setStatus] = useState<MemoryVaultStatus | null>(null);
  const [selectedCategory, setSelectedCategory] = useState("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [draft, setDraft] = useState<SaveMemoryNoteInput>(EMPTY_DRAFT);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const notes = status?.notes ?? [];
  const filteredNotes = useMemo(
    () =>
      selectedCategory === "all"
        ? notes
        : notes.filter((note) => note.category === selectedCategory),
    [notes, selectedCategory],
  );

  useEffect(() => {
    void refresh();
  }, []);

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      const next = await loadMemoryVault();
      setStatus(next);
      if (selectedId && !next.notes.some((note) => note.id === selectedId)) {
        setSelectedId(null);
        setDraft(EMPTY_DRAFT);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  function editNote(note: MemoryNote) {
    setSelectedId(note.id);
    setDraft({
      id: note.id,
      title: note.title,
      category: note.category,
      content: note.content,
      source: note.source,
      inject_mode: note.inject_mode,
    });
  }

  function newNote(category = selectedCategory === "all" ? "global" : selectedCategory) {
    setSelectedId(null);
    setDraft({ ...EMPTY_DRAFT, category });
  }

  async function save() {
    setSaving(true);
    setError(null);
    try {
      const saved = await saveMemoryNote(draft);
      const next = await loadMemoryVault();
      setStatus(next);
      setSelectedId(saved.id);
      setDraft({
        id: saved.id,
        title: saved.title,
        category: saved.category,
        content: saved.content,
        source: saved.source,
        inject_mode: saved.inject_mode,
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  const pack = status?.prompt_pack;
  const canSave = draft.title.trim() && draft.content.trim() && !saving;

  return (
    <div className="flex h-full min-h-0 flex-col bg-background">
      <div className="border-b border-border bg-card/40 px-8 py-4">
        <div className="mx-auto flex w-full max-w-6xl flex-wrap items-center justify-between gap-3">
          <div className="min-w-0">
            <h1 className="text-xl font-semibold tracking-normal text-foreground">记忆</h1>
            <p className="mt-1 truncate text-xs text-muted-foreground">
              {status?.root_path ?? "正在定位记忆目录…"}
            </p>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => status?.root_path && revealInFinder(status.root_path)}
              disabled={!status?.root_path}
              className="inline-flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-xs text-foreground hover:bg-accent disabled:opacity-50"
            >
              <FolderOpen className="size-3.5" />
              打开目录
            </button>
            <button
              type="button"
              onClick={refresh}
              disabled={loading}
              className="inline-flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-xs text-foreground hover:bg-accent disabled:opacity-50"
            >
              <RefreshCw className={cn("size-3.5", loading && "animate-spin")} />
              刷新
            </button>
            <button
              type="button"
              onClick={() => newNote()}
              className="inline-flex items-center gap-1.5 rounded-md bg-foreground px-3 py-1.5 text-xs font-medium text-background hover:opacity-90"
            >
              <Plus className="size-3.5" />
              新建
            </button>
          </div>
        </div>
      </div>

      <div className="mx-auto grid min-h-0 w-full max-w-6xl flex-1 grid-cols-[260px_minmax(0,1fr)] gap-5 px-8 py-5">
        <aside className="min-h-0 overflow-auto border-r border-border pr-4">
          {pack && (
            <div className="mb-4 rounded-md border border-border bg-card p-3">
              <div className="text-xs font-medium text-foreground">Prompt 预算</div>
              <div className="mt-2 space-y-1 text-xs text-muted-foreground">
                <div>
                  {pack.used_chars}/{pack.char_budget} 字 · {pack.source_count} 条可注入
                </div>
                <div>
                  {pack.compressed ? `已压缩，省略 ${pack.omitted_count} 条` : "未触发压缩"}
                </div>
              </div>
            </div>
          )}

          <div className="space-y-1">
            {CATEGORIES.map((category) => {
              const count =
                category.id === "all"
                  ? notes.length
                  : notes.filter((note) => note.category === category.id).length;
              return (
                <button
                  key={category.id}
                  type="button"
                  onClick={() => setSelectedCategory(category.id)}
                  className={cn(
                    "flex w-full items-center justify-between rounded-md px-3 py-2 text-left text-sm",
                    selectedCategory === category.id
                      ? "bg-accent text-foreground"
                      : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
                  )}
                >
                  <span>{category.label}</span>
                  <span className="text-xs tabular-nums">{count}</span>
                </button>
              );
            })}
          </div>
        </aside>

        <main className="grid min-h-0 grid-cols-[minmax(260px,360px)_minmax(0,1fr)] gap-5">
          <section className="min-h-0 overflow-auto">
            {filteredNotes.length === 0 ? (
              <div className="rounded-md border border-dashed border-border p-4 text-sm text-muted-foreground">
                暂无记忆。
              </div>
            ) : (
              <div className="space-y-2">
                {filteredNotes.map((note) => (
                  <button
                    key={note.id}
                    type="button"
                    onClick={() => editNote(note)}
                    className={cn(
                      "w-full rounded-md border border-border bg-card p-3 text-left hover:bg-accent/40",
                      selectedId === note.id && "border-foreground",
                    )}
                  >
                    <div className="flex items-start gap-2">
                      <FileText className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-sm font-medium text-foreground">
                          {note.title}
                        </div>
                        <div className="mt-1 truncate text-xs text-muted-foreground">
                          {note.category} · {note.inject_mode}
                        </div>
                        <p className="mt-2 line-clamp-3 text-xs leading-5 text-muted-foreground">
                          {note.content}
                        </p>
                      </div>
                    </div>
                  </button>
                ))}
              </div>
            )}
          </section>

          <section className="min-h-0 overflow-auto rounded-md border border-border bg-card p-4">
            <div className="mb-4 flex items-center justify-between gap-3">
              <div>
                <div className="text-sm font-medium text-foreground">
                  {draft.id ? "编辑记忆" : "新建记忆"}
                </div>
                <div className="mt-1 text-xs text-muted-foreground">
                  {draft.id ? draft.id : "保存后生成 Markdown 文件"}
                </div>
              </div>
              <button
                type="button"
                onClick={save}
                disabled={!canSave}
                className="inline-flex items-center gap-1.5 rounded-md bg-foreground px-3 py-1.5 text-xs font-medium text-background hover:opacity-90 disabled:opacity-50"
              >
                {saving ? (
                  <RefreshCw className="size-3.5 animate-spin" />
                ) : draft.id ? (
                  <Check className="size-3.5" />
                ) : (
                  <Pencil className="size-3.5" />
                )}
                保存
              </button>
            </div>

            <div className="grid grid-cols-2 gap-3">
              <label className="col-span-2 text-xs font-medium text-muted-foreground">
                标题
                <input
                  value={draft.title}
                  onChange={(e) => setDraft((d) => ({ ...d, title: e.target.value }))}
                  className="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm text-foreground outline-none focus:border-foreground"
                />
              </label>
              <label className="text-xs font-medium text-muted-foreground">
                分类
                <select
                  value={draft.category}
                  onChange={(e) => setDraft((d) => ({ ...d, category: e.target.value }))}
                  className="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm text-foreground outline-none focus:border-foreground"
                >
                  {CATEGORIES.filter((c) => c.id !== "all").map((category) => (
                    <option key={category.id} value={category.id}>
                      {category.label}
                    </option>
                  ))}
                </select>
              </label>
              <label className="text-xs font-medium text-muted-foreground">
                注入模式
                <select
                  value={draft.inject_mode ?? "manual_select"}
                  onChange={(e) => setDraft((d) => ({ ...d, inject_mode: e.target.value }))}
                  className="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm text-foreground outline-none focus:border-foreground"
                >
                  {INJECT_MODES.map((mode) => (
                    <option key={mode.id} value={mode.id}>
                      {mode.label}
                    </option>
                  ))}
                </select>
              </label>
              <label className="col-span-2 text-xs font-medium text-muted-foreground">
                内容
                <textarea
                  value={draft.content}
                  onChange={(e) => setDraft((d) => ({ ...d, content: e.target.value }))}
                  className="mt-1 min-h-[380px] w-full resize-y rounded-md border border-border bg-background px-3 py-2 font-mono text-sm leading-6 text-foreground outline-none focus:border-foreground"
                />
              </label>
            </div>

            {error && <div className="mt-3 text-xs text-destructive">{error}</div>}
          </section>
        </main>
      </div>
    </div>
  );
}

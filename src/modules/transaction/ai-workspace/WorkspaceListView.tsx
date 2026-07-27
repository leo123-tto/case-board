import { useCallback, useEffect, useState } from "react";
import {
  Archive,
  Check,
  Clock3,
  FileText,
  FolderOpen,
  Loader2,
  MessageSquare,
  Pencil,
  Plus,
  Search,
  Star,
  X,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { confirmDialog } from "@/lib/dialog";
import { cn } from "@/lib/utils";

import {
  archiveAiWorkspace,
  createAiWorkspace,
  listAiWorkspaces,
  updateAiWorkspace,
} from "./api";
import type {
  AiWorkspace,
  AiWorkspaceSummary,
  WorkspaceListViewMode,
} from "./types";

interface Props {
  onOpen: (workspace: AiWorkspace) => void;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function WorkspaceListView({ onOpen }: Props) {
  const [items, setItems] = useState<AiWorkspaceSummary[]>([]);
  const [query, setQuery] = useState("");
  const [view, setView] = useState<WorkspaceListViewMode>("all");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [creating, setCreating] = useState(false);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameTitle, setRenameTitle] = useState("");
  const [renaming, setRenaming] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setItems(
        await listAiWorkspaces({
          query: query.trim() || null,
          view,
          include_archived: false,
        }),
      );
    } catch (loadError) {
      setError(`工作区加载失败：${errorText(loadError)}`);
    } finally {
      setLoading(false);
    }
  }, [query, view]);

  useEffect(() => {
    void load();
  }, [load]);

  const create = async () => {
    const cleanTitle = title.trim();
    if (!cleanTitle || creating) return;
    setCreating(true);
    setError(null);
    try {
      const workspace = await createAiWorkspace({
        title: cleanTitle,
        description: description.trim() || null,
      });
      setTitle("");
      setDescription("");
      setShowCreate(false);
      onOpen(workspace);
    } catch (createError) {
      setError(`创建工作区失败：${errorText(createError)}`);
    } finally {
      setCreating(false);
    }
  };

  const toggleFavorite = async (workspace: AiWorkspaceSummary) => {
    try {
      await updateAiWorkspace(workspace.id, {
        is_favorite: workspace.is_favorite !== 1,
      });
      await load();
    } catch (updateError) {
      setError(`更新收藏失败：${errorText(updateError)}`);
    }
  };

  const beginRename = (workspace: AiWorkspaceSummary) => {
    setRenamingId(workspace.id);
    setRenameTitle(workspace.title);
    setError(null);
  };

  const cancelRename = () => {
    if (renaming) return;
    setRenamingId(null);
    setRenameTitle("");
  };

  const saveRename = async () => {
    const cleanTitle = renameTitle.trim();
    if (!renamingId || !cleanTitle || renaming) return;
    setRenaming(true);
    setError(null);
    try {
      const updated = await updateAiWorkspace(renamingId, { title: cleanTitle });
      setItems((current) =>
        current.map((workspace) =>
          workspace.id === renamingId
            ? { ...workspace, title: updated.title, updated_at: updated.updated_at }
            : workspace,
        ),
      );
      setRenamingId(null);
      setRenameTitle("");
    } catch (updateError) {
      setError(`重命名失败：${errorText(updateError)}`);
    } finally {
      setRenaming(false);
    }
  };

  const archive = async (workspace: AiWorkspaceSummary) => {
    if (!(await confirmDialog(`确认归档“${workspace.title}”？归档不会删除其中的材料、文稿或对话。`, { okLabel: "归档" }))) {
      return;
    }
    try {
      await archiveAiWorkspace(workspace.id);
      await load();
    } catch (archiveError) {
      setError(`归档失败：${errorText(archiveError)}`);
    }
  };

  return (
    <div className="mx-auto flex h-full w-full max-w-6xl flex-col gap-5 overflow-auto px-5 py-6 sm:px-7">
      <header className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="text-xl font-semibold tracking-tight text-foreground">
            事务工作区
          </h1>
          <p className="mt-1 text-xs text-muted-foreground">
            围绕一件独立事务持续整理材料、对话和起草文稿。
          </p>
        </div>
        <Button onClick={() => setShowCreate(true)}>
          <Plus className="size-4" />
          新建工作区
        </Button>
      </header>

      <div className="flex flex-wrap items-center gap-2">
        <div className="flex rounded-lg bg-muted p-0.5">
          {(["all", "recent"] as const).map((mode) => (
            <button
              key={mode}
              type="button"
              onClick={() => setView(mode)}
              className={cn(
                "rounded-md px-3 py-1.5 text-xs transition-colors",
                view === mode
                  ? "bg-card font-medium text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {mode === "all" ? "全部" : "最近"}
            </button>
          ))}
        </div>
        <label className="flex min-w-52 flex-1 items-center gap-2 rounded-lg border border-border bg-card px-3 py-2 text-xs text-muted-foreground sm:max-w-sm">
          <Search className="size-3.5" />
          <span className="sr-only">搜索工作区</span>
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="搜索工作区"
            className="min-w-0 flex-1 bg-transparent text-foreground outline-none placeholder:text-muted-foreground/60"
          />
        </label>
      </div>

      {showCreate ? (
        <section className="rounded-xl border border-brand/20 bg-card p-4 shadow-sm">
          <div className="grid gap-3 sm:grid-cols-2">
            <label className="grid gap-1.5 text-xs text-foreground">
              工作区名称
              <input
                autoFocus
                value={title}
                maxLength={120}
                onChange={(event) => setTitle(event.target.value)}
                className="rounded-md border border-border bg-background px-3 py-2 outline-none focus:border-brand"
                placeholder="如：设备停机紧急通知"
              />
            </label>
            <label className="grid gap-1.5 text-xs text-foreground">
              一句话背景（可选）
              <input
                value={description}
                maxLength={1000}
                onChange={(event) => setDescription(event.target.value)}
                className="rounded-md border border-border bg-background px-3 py-2 outline-none focus:border-brand"
                placeholder="后续可以随时修改"
              />
            </label>
          </div>
          <div className="mt-3 flex justify-end gap-2">
            <Button variant="ghost" onClick={() => setShowCreate(false)}>
              取消
            </Button>
            <Button disabled={!title.trim() || creating} onClick={() => void create()}>
              {creating ? <Loader2 className="size-4 animate-spin" /> : null}
              创建并进入
            </Button>
          </div>
        </section>
      ) : null}

      {error ? (
        <div className="flex items-center justify-between gap-3 rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-xs text-destructive">
          <span>{error}</span>
          <Button size="sm" variant="outline" onClick={() => void load()}>
            重试
          </Button>
        </div>
      ) : null}

      {loading ? (
        <div className="flex flex-1 items-center justify-center py-16 text-muted-foreground">
          <Loader2 className="size-5 animate-spin" />
        </div>
      ) : items.length === 0 && !error ? (
        <div className="flex flex-1 flex-col items-center justify-center rounded-xl border border-dashed border-border py-16 text-center">
          <FolderOpen className="size-8 text-muted-foreground/50" />
          <p className="mt-3 text-sm font-medium text-foreground">
            {view === "recent" ? "还没有最近使用的工作区" : "还没有工作区"}
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            可以不上传材料，直接新建后让 AI 起草。
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
          {items.map((workspace) => (
            <article
              key={workspace.id}
              className="group rounded-xl border border-border bg-card p-4 transition-colors hover:border-brand/25"
            >
              <div className="flex items-start gap-2">
                {renamingId === workspace.id ? (
                  <div className="min-w-0 flex-1">
                    <input
                      autoFocus
                      aria-label="工作区新名称"
                      value={renameTitle}
                      maxLength={120}
                      disabled={renaming}
                      onChange={(event) => setRenameTitle(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") void saveRename();
                        if (event.key === "Escape") cancelRename();
                      }}
                      className="w-full rounded-md border border-brand/40 bg-background px-2 py-1 text-sm text-foreground outline-none focus:border-brand"
                    />
                    <div className="mt-1 flex items-center gap-1">
                      <button
                        type="button"
                        aria-label="保存工作区名称"
                        title="保存工作区名称"
                        disabled={!renameTitle.trim() || renaming}
                        onClick={() => void saveRename()}
                        className="rounded p-1 text-brand hover:bg-muted disabled:opacity-40"
                      >
                        {renaming ? <Loader2 className="size-3.5 animate-spin" /> : <Check className="size-3.5" />}
                      </button>
                      <button
                        type="button"
                        aria-label="取消重命名"
                        title="取消重命名"
                        disabled={renaming}
                        onClick={cancelRename}
                        className="rounded p-1 text-muted-foreground hover:bg-muted disabled:opacity-40"
                      >
                        <X className="size-3.5" />
                      </button>
                    </div>
                  </div>
                ) : (
                  <>
                    <button
                      type="button"
                      aria-label={`打开${workspace.title}`}
                      onClick={() => onOpen(workspace)}
                      className="min-w-0 flex-1 text-left"
                    >
                      <h2 className="truncate text-sm font-medium text-foreground">
                        {workspace.title}
                      </h2>
                      <p className="mt-1 line-clamp-2 min-h-8 text-xs leading-relaxed text-muted-foreground">
                        {workspace.description || "未填写背景说明"}
                      </p>
                    </button>
                    <button
                      type="button"
                      aria-label={`重命名${workspace.title}`}
                      title="重命名工作区"
                      onClick={() => beginRename(workspace)}
                      className="rounded-md p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                    >
                      <Pencil className="size-4" />
                    </button>
                  </>
                )}
                <button
                  type="button"
                  aria-label={`${workspace.is_favorite === 1 ? "取消收藏" : "收藏"}${workspace.title}`}
                  title={workspace.is_favorite === 1 ? "取消收藏" : "收藏"}
                  onClick={() => void toggleFavorite(workspace)}
                  className="rounded-md p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                >
                  <Star
                    className={cn(
                      "size-4",
                      workspace.is_favorite === 1 && "fill-amber-400 text-amber-500",
                    )}
                  />
                </button>
                <button
                  type="button"
                  aria-label={`归档${workspace.title}`}
                  title="归档工作区"
                  onClick={() => void archive(workspace)}
                  className="rounded-md p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                >
                  <Archive className="size-4" />
                </button>
              </div>
              <div className="mt-4 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
                <span className="flex items-center gap-1">
                  <FolderOpen className="size-3" /> {workspace.source_count} 份材料
                </span>
                <span className="flex items-center gap-1">
                  <FileText className="size-3" /> {workspace.artifact_count} 份文稿
                </span>
                <span className="flex items-center gap-1">
                  <MessageSquare className="size-3" /> {workspace.conversation_count} 段对话
                </span>
              </div>
              <div className="mt-3 flex items-center gap-1 border-t border-border/60 pt-3 text-[10px] text-muted-foreground/75">
                <Clock3 className="size-3" />
                最后更新 {workspace.updated_at}
              </div>
            </article>
          ))}
        </div>
      )}
    </div>
  );
}

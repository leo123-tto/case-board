import { useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ArrowLeft, BriefcaseBusiness, Check, Loader2, Pencil, X } from "lucide-react";

import { Button } from "@/components/ui/button";

import type { AiWorkspace } from "./types";
import { updateAiWorkspace } from "./api";
import { WorkspaceListView } from "./WorkspaceListView";
import { WorkspaceShell, type WorkspaceShellHandle } from "./WorkspaceShell";

export function AiWorkspaceTool({
  registerBeforeLeave,
  onBackToTransaction,
}: {
  registerBeforeLeave?: (handler: () => Promise<boolean>) => void;
  onBackToTransaction: () => void;
}) {
  const [activeWorkspace, setActiveWorkspace] = useState<AiWorkspace | null>(null);
  const shellRef = useRef<WorkspaceShellHandle>(null);
  const closingRef = useRef(false);
  const [renamingWorkspace, setRenamingWorkspace] = useState(false);
  const [workspaceTitle, setWorkspaceTitle] = useState("");
  const [savingWorkspaceTitle, setSavingWorkspaceTitle] = useState(false);
  const [renameError, setRenameError] = useState<string | null>(null);

  const beginWorkspaceRename = () => {
    if (!activeWorkspace) return;
    setWorkspaceTitle(activeWorkspace.title);
    setRenameError(null);
    setRenamingWorkspace(true);
  };

  const cancelWorkspaceRename = () => {
    if (savingWorkspaceTitle) return;
    setRenamingWorkspace(false);
    setWorkspaceTitle("");
    setRenameError(null);
  };

  const saveWorkspaceRename = async () => {
    const cleanTitle = workspaceTitle.trim();
    if (!activeWorkspace || !cleanTitle || savingWorkspaceTitle) return;
    setSavingWorkspaceTitle(true);
    setRenameError(null);
    try {
      const updated = await updateAiWorkspace(activeWorkspace.id, { title: cleanTitle });
      setActiveWorkspace(updated);
      setRenamingWorkspace(false);
      setWorkspaceTitle("");
    } catch (error) {
      setRenameError(error instanceof Error ? error.message : String(error));
    } finally {
      setSavingWorkspaceTitle(false);
    }
  };

  useEffect(() => {
    registerBeforeLeave?.(() => shellRef.current?.flush() ?? Promise.resolve(true));
  }, [registerBeforeLeave]);

  useEffect(() => {
    if (!activeWorkspace) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    getCurrentWindow()
      .onCloseRequested(async (event) => {
        if (closingRef.current) return;
        event.preventDefault();
        const saved = await (shellRef.current?.flush() ?? Promise.resolve(true));
        if (saved) {
          closingRef.current = true;
          await getCurrentWindow().close();
        }
      })
      .then((cleanup) => {
        if (disposed) cleanup();
        else unlisten = cleanup;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [activeWorkspace]);

  if (!activeWorkspace) {
    return (
      <div className="flex h-full flex-col">
        <header className="app-subheader flex shrink-0 items-center gap-3 border-b px-4 py-2.5 sm:px-6">
          <Button variant="ghost" size="sm" onClick={onBackToTransaction}>
            <ArrowLeft className="size-3.5" />
            返回非诉
          </Button>
          <span className="h-4 w-px bg-border" />
          <BriefcaseBusiness className="size-4 text-brand" />
          <h2 className="text-sm font-medium text-foreground">AI 事务工作区</h2>
        </header>
        <div className="min-h-0 flex-1">
          <WorkspaceListView onOpen={setActiveWorkspace} />
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <header className="app-subheader flex shrink-0 items-center gap-3 border-b px-4 py-2.5 sm:px-6">
        <Button
          variant="ghost"
          size="sm"
          onClick={() => void (async () => {
            if (await (shellRef.current?.flush() ?? Promise.resolve(true))) {
              setActiveWorkspace(null);
            }
          })()}
        >
          <ArrowLeft className="size-3.5" />
          返回工作区列表
        </Button>
        <span className="h-4 w-px bg-border" />
        <BriefcaseBusiness className="size-4 text-brand" />
        {renamingWorkspace ? (
          <div className="flex min-w-0 items-center gap-1.5">
            <input
              autoFocus
              aria-label="工作区新名称"
              value={workspaceTitle}
              maxLength={120}
              disabled={savingWorkspaceTitle}
              onChange={(event) => setWorkspaceTitle(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") void saveWorkspaceRename();
                if (event.key === "Escape") cancelWorkspaceRename();
              }}
              className="min-w-48 rounded-md border border-brand/40 bg-background px-2 py-1 text-sm text-foreground outline-none focus:border-brand"
            />
            <button
              type="button"
              aria-label="保存工作区名称"
              title="保存工作区名称"
              disabled={!workspaceTitle.trim() || savingWorkspaceTitle}
              onClick={() => void saveWorkspaceRename()}
              className="rounded p-1 text-brand hover:bg-muted disabled:opacity-40"
            >
              {savingWorkspaceTitle ? <Loader2 className="size-3.5 animate-spin" /> : <Check className="size-3.5" />}
            </button>
            <button
              type="button"
              aria-label="取消重命名"
              title="取消重命名"
              disabled={savingWorkspaceTitle}
              onClick={cancelWorkspaceRename}
              className="rounded p-1 text-muted-foreground hover:bg-muted disabled:opacity-40"
            >
              <X className="size-3.5" />
            </button>
            {renameError ? <span className="text-[11px] text-destructive">{renameError}</span> : null}
          </div>
        ) : (
          <div className="flex min-w-0 items-center gap-1">
            <h2 className="truncate text-sm font-medium text-foreground">
              {activeWorkspace.title}
            </h2>
            <button
              type="button"
              aria-label={`重命名${activeWorkspace.title}`}
              title="重命名工作区"
              onClick={beginWorkspaceRename}
              className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
            >
              <Pencil className="size-3.5" />
            </button>
          </div>
        )}
      </header>
      <WorkspaceShell ref={shellRef} workspace={activeWorkspace} />
    </div>
  );
}

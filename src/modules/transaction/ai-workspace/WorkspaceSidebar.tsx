import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Archive,
  FilePlus2,
  FileText,
  Loader2,
  RefreshCw,
  RotateCcw,
  Upload,
  Trash2,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { confirmDialog } from "@/lib/dialog";
import { cn } from "@/lib/utils";

import {
  addAiWorkspaceSources,
  archiveAiWorkspaceDocument,
  createAiWorkspaceArtifact,
  listAiWorkspaceDocuments,
  relinkAiWorkspaceSource,
  retryAiWorkspaceSource,
} from "./api";
import type {
  AiWorkspaceDocument,
  AiWorkspaceDocumentProgress,
} from "./types";

interface Props {
  workspaceId: string;
  refreshToken?: number;
  selectedDocumentId?: string | null;
  onSelectDocument: (document: AiWorkspaceDocument) => void;
  onDocumentRemoved?: (documentId: string) => void;
  headerActions?: ReactNode;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function statusText(document: AiWorkspaceDocument): string | null {
  switch (document.extraction_status) {
    case "queued":
      return "等待处理";
    case "processing":
      return "正在识别";
    case "review":
      return "建议核对识别结果";
    case "failed":
      return document.last_error || "处理失败";
    case "missing":
      return document.last_error || "原文件已移动";
    default:
      return null;
  }
}

export function WorkspaceSidebar({
  workspaceId,
  refreshToken = 0,
  selectedDocumentId,
  onSelectDocument,
  onDocumentRemoved,
  headerActions,
}: Props) {
  const rootRef = useRef<HTMLElement>(null);
  const [documents, setDocuments] = useState<AiWorkspaceDocument[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<{
    document: AiWorkspaceDocument;
    x: number;
    y: number;
  } | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setDocuments(await listAiWorkspaceDocuments(workspaceId));
    } catch (loadError) {
      setError(`材料加载失败：${errorText(loadError)}`);
    } finally {
      setLoading(false);
    }
  }, [workspaceId]);

  useEffect(() => {
    void load();
  }, [load, refreshToken]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen<AiWorkspaceDocumentProgress>(
      "ai-workspace-document-progress",
      (event) => {
        if (event.payload.workspace_id === workspaceId) void load();
      },
    ).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [load, workspaceId]);

  const addPaths = useCallback(
    async (paths: string[]) => {
      if (paths.length === 0) return;
      setError(null);
      try {
        const result = await addAiWorkspaceSources(workspaceId, paths);
        if (result.errors.length > 0) {
          setError(
            result.errors
              .map((item) => `${item.path.split(/[\\/]/).pop() ?? item.path}：${item.error}`)
              .join("；"),
          );
        }
        await load();
      } catch (addError) {
        setError(`添加材料失败：${errorText(addError)}`);
      }
    },
    [load, workspaceId],
  );

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        const rect = rootRef.current?.getBoundingClientRect();
        const scale = window.devicePixelRatio || 1;
        const inside =
          !rect || !("position" in payload) || payload.position.x <= rect.right * scale;
        if (payload.type === "enter" || payload.type === "over") {
          setDragging(inside);
        } else if (payload.type === "drop") {
          setDragging(false);
          if (inside) void addPaths(payload.paths);
        } else {
          setDragging(false);
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
  }, [addPaths]);

  useEffect(() => {
    if (!contextMenu) return;
    const close = () => setContextMenu(null);
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [contextMenu]);

  const chooseFiles = async () => {
    const picked = await open({
      directory: false,
      multiple: true,
      title: "选择工作区材料",
    });
    const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    await addPaths(paths);
  };

  const createArtifact = async () => {
    setBusyId("__new_artifact__");
    setError(null);
    try {
      const created = await createAiWorkspaceArtifact(workspaceId, {
        title: "未命名文稿",
        initial_markdown: null,
      });
      await load();
      onSelectDocument(created.document);
    } catch (createError) {
      setError(`新建文稿失败：${errorText(createError)}`);
    } finally {
      setBusyId(null);
    }
  };

  const retry = async (document: AiWorkspaceDocument) => {
    setBusyId(document.id);
    setError(null);
    try {
      await retryAiWorkspaceSource(workspaceId, document.id);
      await load();
    } catch (retryError) {
      setError(`重试失败：${errorText(retryError)}`);
    } finally {
      setBusyId(null);
    }
  };

  const relink = async (document: AiWorkspaceDocument) => {
    const picked = await open({ directory: false, multiple: false, title: "重新关联原文件" });
    if (typeof picked !== "string") return;
    setBusyId(document.id);
    try {
      await relinkAiWorkspaceSource(workspaceId, document.id, picked);
      await load();
    } catch (relinkError) {
      setError(`重新关联失败：${errorText(relinkError)}`);
    } finally {
      setBusyId(null);
    }
  };

  const archive = async (document: AiWorkspaceDocument) => {
    const confirmed = await confirmDialog(`确认从工作区移除“${document.title}”？原文件不会被删除。`, {
      title: "移除工作区文档",
      okLabel: "移除",
    });
    if (!confirmed) return;
    setBusyId(document.id);
    try {
      await archiveAiWorkspaceDocument(workspaceId, document.id);
      onDocumentRemoved?.(document.id);
      await load();
    } catch (archiveError) {
      setError(`归档失败：${errorText(archiveError)}`);
    } finally {
      setBusyId(null);
    }
  };

  const sources = documents.filter((document) => document.kind === "source");
  const artifacts = documents.filter((document) => document.kind === "artifact");

  const renderDocuments = (items: AiWorkspaceDocument[]) =>
    items.map((document) => {
      const status = statusText(document);
      const processing = ["queued", "processing"].includes(document.extraction_status);
      return (
        <div
          key={document.id}
          onContextMenu={(event) => {
            if (document.kind !== "source") return;
            event.preventDefault();
            setContextMenu({ document, x: event.clientX, y: event.clientY });
          }}
          className={cn(
            "group rounded-lg border px-2.5 py-2",
            selectedDocumentId === document.id
              ? "border-brand/30 bg-brand/5"
              : "border-transparent hover:bg-muted/70",
          )}
        >
          <button
            type="button"
            aria-label={`打开${document.title}`}
            onClick={() => onSelectDocument(document)}
            className="flex w-full min-w-0 items-start gap-2 text-left"
          >
            {processing ? (
              <Loader2 className="mt-0.5 size-3.5 shrink-0 animate-spin text-brand" />
            ) : (
              <FileText className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
            )}
            <span className="min-w-0 flex-1">
              <span className="block truncate text-xs font-medium text-foreground">
                {document.title}
              </span>
              {status ? (
                <span
                  className={cn(
                    "mt-0.5 block line-clamp-2 text-[11px]",
                    ["failed", "missing"].includes(document.extraction_status)
                      ? "text-destructive"
                      : "text-muted-foreground",
                  )}
                >
                  {status}
                </span>
              ) : null}
            </span>
          </button>
          <div className="mt-1.5 flex justify-end gap-1 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
            {["failed", "review"].includes(document.extraction_status) ? (
              <button
                type="button"
                aria-label={`重试${document.title}`}
                title="重试处理"
                disabled={busyId === document.id}
                onClick={() => void retry(document)}
                className="rounded p-1 text-muted-foreground hover:bg-background hover:text-foreground"
              >
                <RotateCcw className="size-3.5" />
              </button>
            ) : null}
            {document.extraction_status === "missing" ? (
              <button
                type="button"
                aria-label={`重新关联${document.title}`}
                title="重新关联原文件"
                disabled={busyId === document.id}
                onClick={() => void relink(document)}
                className="rounded p-1 text-muted-foreground hover:bg-background hover:text-foreground"
              >
                <RefreshCw className="size-3.5" />
              </button>
            ) : null}
            <button
              type="button"
              aria-label={`从工作区移除${document.title}`}
              title="从工作区移除（不删除原文件）"
              disabled={busyId === document.id}
              onClick={() => void archive(document)}
              className="rounded p-1 text-muted-foreground hover:bg-background hover:text-foreground"
            >
              <Archive className="size-3.5" />
            </button>
          </div>
        </div>
      );
    });

  return (
    <aside
      ref={rootRef}
      className="relative flex h-full w-full shrink-0 flex-col border-r border-border bg-muted/20"
    >
      <div data-testid="workspace-sidebar-header" className="border-b border-border px-3 py-2.5">
        <div className="flex h-6 items-center justify-between">
          <span className="text-xs font-medium text-foreground">文件</span>
          <div className="flex items-center gap-0.5">{headerActions}</div>
        </div>
        <div className="mt-2 grid grid-cols-2 gap-1.5">
          <Button size="sm" variant="ghost" aria-label="新建文稿" disabled={busyId === "__new_artifact__"} onClick={() => void createArtifact()} className="w-full px-2">
            {busyId === "__new_artifact__" ? <Loader2 className="size-3.5 animate-spin" /> : <FilePlus2 className="size-3.5" />}
            新建文稿
          </Button>
          <Button size="sm" variant="outline" onClick={() => void chooseFiles()} className="w-full px-2">
            <Upload className="size-3.5" />
            添加材料
          </Button>
        </div>
      </div>

      {error ? (
        <div className="m-2 rounded-md border border-destructive/30 bg-destructive/5 p-2 text-[11px] text-destructive">
          <p>{error}</p>
          <button type="button" className="mt-1 underline" onClick={() => void load()}>
            重试加载
          </button>
        </div>
      ) : null}

      <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
        {loading ? (
          <div className="flex justify-center py-8 text-muted-foreground">
            <Loader2 className="size-4 animate-spin" />
          </div>
        ) : (
          <>
            <section>
              <h3 className="px-1 pb-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                材料 {sources.length}
              </h3>
              {sources.length > 0 ? renderDocuments(sources) : (
                <p className="px-2 py-3 text-[11px] leading-relaxed text-muted-foreground">
                  拖入或选择文件，原文件保持不变。
                </p>
              )}
            </section>
            <section className="mt-4">
              <h3 className="px-1 pb-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                文稿 {artifacts.length}
              </h3>
              {artifacts.length > 0 ? renderDocuments(artifacts) : (
                <p className="px-2 py-3 text-[11px] text-muted-foreground">
                  AI 生成的文稿会保存在这里。
                </p>
              )}
            </section>
          </>
        )}
      </div>

      {dragging ? (
        <div className="pointer-events-none absolute inset-1 z-20 flex flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed border-brand bg-background/90 text-brand backdrop-blur-sm">
          <FilePlus2 className="size-7" />
          <span className="text-xs font-medium">松开添加材料</span>
        </div>
      ) : null}
      {contextMenu ? (
        <div
          role="menu"
          aria-label={`${contextMenu.document.title}操作`}
          className="fixed z-[100] min-w-40 rounded-md border border-border bg-popover p-1 shadow-lg"
          style={{
            left: Math.min(contextMenu.x, window.innerWidth - 176),
            top: Math.min(contextMenu.y, window.innerHeight - 52),
          }}
          onPointerDown={(event) => event.stopPropagation()}
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              const document = contextMenu.document;
              setContextMenu(null);
              void archive(document);
            }}
            className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs text-destructive hover:bg-destructive/10"
          >
            <Trash2 className="size-3.5" />
            从工作区移除
          </button>
        </div>
      ) : null}
    </aside>
  );
}

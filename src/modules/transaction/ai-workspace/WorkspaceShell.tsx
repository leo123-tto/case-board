import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  ChevronLeft,
  ChevronRight,
  Maximize2,
  MessageSquare,
  Minimize2,
  PanelLeftOpen,
  PanelRightOpen,
} from "lucide-react";

import { cn } from "@/lib/utils";

import { listAiWorkspaceDocuments } from "./api";
import type { AiWorkspace, AiWorkspaceDocument } from "./types";
import {
  WorkspaceDocumentPane,
  type WorkspaceDocumentPaneHandle,
} from "./WorkspaceDocumentPane";
import {
  isCompactWorkspace,
  loadWorkspaceLayout,
  normalizeWorkspaceLayout,
  resizeWorkspacePane,
  saveWorkspaceLayout,
  toggleWorkspaceFullscreen,
  toggleWorkspacePane,
  type WorkspacePane,
} from "./workspaceLayout";
import { WorkspaceSidebar } from "./WorkspaceSidebar";
import { WorkspaceChatPane } from "./WorkspaceChatPane";

export interface WorkspaceShellHandle {
  flush: () => Promise<boolean>;
}

export const WorkspaceShell = forwardRef<WorkspaceShellHandle, { workspace: AiWorkspace }>(
function WorkspaceShell({ workspace }, ref) {
  const rootRef = useRef<HTMLDivElement>(null);
  const documentPaneRef = useRef<WorkspaceDocumentPaneHandle>(null);
  const [layout, setLayout] = useState(loadWorkspaceLayout);
  const [selectedDocument, setSelectedDocument] = useState<AiWorkspaceDocument | null>(null);
  const [documentsRevision, setDocumentsRevision] = useState(0);
  const [proposalRevision, setProposalRevision] = useState(0);
  const [windowWidth, setWindowWidth] = useState(() => window.innerWidth);
  const [overlay, setOverlay] = useState<"sidebar" | "chat" | null>(null);
  const compact = isCompactWorkspace(windowWidth);

  useImperativeHandle(ref, () => ({
    flush: () => documentPaneRef.current?.flush() ?? Promise.resolve(true),
  }), []);

  useEffect(() => {
    saveWorkspaceLayout(layout);
  }, [layout]);

  useEffect(() => {
    const onResize = () => {
      setWindowWidth(window.innerWidth);
      if (rootRef.current) {
        setLayout((current) =>
          normalizeWorkspaceLayout(current, rootRef.current?.clientWidth ?? window.innerWidth),
        );
      }
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  useEffect(() => {
    let cancelled = false;
    void listAiWorkspaceDocuments(workspace.id)
      .then((documents) => {
        if (cancelled || documents.length === 0) return;
        const restored = documents.find((document) => document.id === workspace.last_document_id);
        const fallback =
          restored ??
          documents.find((document) => document.kind === "artifact") ??
          documents[0];
        setSelectedDocument(fallback);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [workspace.id, workspace.last_document_id]);

  const startResize = (pane: "sidebar" | "chat", event: React.PointerEvent) => {
    event.preventDefault();
    const startX = event.clientX;
    const startLayout = layout;
    const onMove = (moveEvent: PointerEvent) => {
      const width = rootRef.current?.clientWidth ?? window.innerWidth;
      setLayout(resizeWorkspacePane(startLayout, pane, moveEvent.clientX - startX, width));
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  const selectDocument = async (document: AiWorkspaceDocument) => {
    if (selectedDocument?.id !== document.id) {
      const saved = await (documentPaneRef.current?.flush() ?? Promise.resolve(true));
      if (!saved) return;
    }
    setSelectedDocument(document);
    setOverlay(null);
  };

  const visible = useMemo(() => {
    const fullscreen = layout.fullscreen;
    return {
      sidebar: !compact && !layout.collapsed.sidebar && (!fullscreen || fullscreen === "sidebar"),
      document: !fullscreen || fullscreen === "document",
      chat: !compact && !layout.collapsed.chat && (!fullscreen || fullscreen === "chat"),
    };
  }, [compact, layout]);

  const fullscreenButton = (pane: WorkspacePane) => {
    const names: Record<WorkspacePane, string> = {
      sidebar: "文件栏",
      document: "文档栏",
      chat: "对话栏",
    };
    const label = layout.fullscreen === pane
      ? `退出${names[pane]}全屏`
      : `${names[pane]}全屏`;
    return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={() => setLayout((current) => toggleWorkspaceFullscreen(current, pane))}
      className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
    >
      {layout.fullscreen === pane ? <Minimize2 className="size-3.5" /> : <Maximize2 className="size-3.5" />}
    </button>
    );
  };

  return (
    <div ref={rootRef} className="relative flex min-h-0 flex-1 overflow-hidden bg-background">
      {!compact && layout.collapsed.sidebar && !layout.fullscreen ? (
        <button
          type="button"
          aria-label="展开文件栏"
          title="展开文件栏"
          onClick={() => setLayout((current) => toggleWorkspacePane(current, "sidebar"))}
          className="absolute left-2 top-2 z-20 rounded-md border border-border bg-card p-1.5 text-muted-foreground shadow-sm"
        >
          <PanelLeftOpen className="size-4" />
        </button>
      ) : null}

      {visible.sidebar ? (
        <section aria-label="工作区文件栏" style={{ width: layout.fullscreen === "sidebar" ? "100%" : layout.left }} className="relative shrink-0">
          <WorkspaceSidebar
            workspaceId={workspace.id}
            refreshToken={documentsRevision}
            selectedDocumentId={selectedDocument?.id}
            onSelectDocument={(document) => void selectDocument(document)}
            onDocumentRemoved={(documentId) => {
              if (selectedDocument?.id === documentId) setSelectedDocument(null);
              setDocumentsRevision((revision) => revision + 1);
            }}
            headerActions={
              <>
                {fullscreenButton("sidebar")}
                {!layout.fullscreen ? (
                  <button type="button" aria-label="收起文件栏" title="收起文件栏" onClick={() => setLayout((current) => toggleWorkspacePane(current, "sidebar"))} className="rounded p-1 text-muted-foreground hover:bg-muted"><ChevronLeft className="size-3.5" /></button>
                ) : null}
              </>
            }
          />
        </section>
      ) : null}

      {visible.sidebar && visible.document && !layout.fullscreen ? (
        <div role="separator" aria-orientation="vertical" onPointerDown={(event) => startResize("sidebar", event)} className="w-1 shrink-0 cursor-col-resize border-r border-border hover:bg-brand/20" />
      ) : null}

      {visible.document ? (
        <main aria-label="工作区文档栏" className="relative min-w-0 flex-1">
          {compact ? (
            <div className="absolute left-2 top-2 z-20 flex gap-1">
              <button type="button" aria-label="打开文件栏" title="打开文件栏" onClick={() => setOverlay("sidebar")} className="rounded-md border border-border bg-card p-1.5 shadow-sm"><PanelLeftOpen className="size-4" /></button>
              <button type="button" aria-label="打开对话栏" title="打开对话栏" onClick={() => setOverlay("chat")} className="rounded-md border border-border bg-card p-1.5 shadow-sm"><PanelRightOpen className="size-4" /></button>
            </div>
          ) : null}
          <WorkspaceDocumentPane
            ref={documentPaneRef}
            workspaceId={workspace.id}
            document={selectedDocument}
            onDocumentChanged={setSelectedDocument}
            proposalRefreshToken={proposalRevision}
            headerActions={fullscreenButton("document")}
          />
        </main>
      ) : null}

      {visible.document && visible.chat && !layout.fullscreen ? (
        <div role="separator" aria-orientation="vertical" onPointerDown={(event) => startResize("chat", event)} className="w-1 shrink-0 cursor-col-resize border-l border-border hover:bg-brand/20" />
      ) : null}

      {visible.chat ? (
        <aside aria-label="工作区对话栏" style={{ width: layout.fullscreen === "chat" ? "100%" : layout.right }} className="relative flex shrink-0 flex-col bg-muted/10">
          <div className="flex h-11 items-center gap-2 border-b border-border px-3 text-xs font-medium">
            <MessageSquare className="size-3.5 text-brand" />AI 助手
            <div className="ml-auto flex items-center gap-0.5">
              {!layout.fullscreen ? <button type="button" aria-label="收起对话栏" title="收起对话栏" onClick={() => setLayout((current) => toggleWorkspacePane(current, "chat"))} className="rounded p-1 text-muted-foreground hover:bg-muted"><ChevronRight className="size-3.5" /></button> : null}
              {fullscreenButton("chat")}
            </div>
          </div>
          <WorkspaceChatPane
            workspaceId={workspace.id}
            editingDocumentId={selectedDocument?.kind === "artifact" ? selectedDocument.id : null}
            onDocumentCreated={(document) => {
              setDocumentsRevision((revision) => revision + 1);
              void selectDocument(document);
            }}
            onProposalCreated={() => setProposalRevision((revision) => revision + 1)}
            beforeSend={() => documentPaneRef.current?.flush() ?? Promise.resolve(true)}
          />
        </aside>
      ) : !compact && layout.collapsed.chat && !layout.fullscreen ? (
        <button type="button" aria-label="展开对话栏" title="展开对话栏" onClick={() => setLayout((current) => toggleWorkspacePane(current, "chat"))} className="absolute right-2 top-2 z-20 rounded-md border border-border bg-card p-1.5 text-muted-foreground shadow-sm"><PanelRightOpen className="size-4" /></button>
      ) : null}

      {compact && overlay ? (
        <div className="absolute inset-0 z-40 flex bg-black/20" onClick={() => setOverlay(null)}>
          <div className={cn("h-full bg-background shadow-xl", overlay === "sidebar" ? "w-[min(82vw,320px)]" : "ml-auto w-[min(88vw,420px)]")} onClick={(event) => event.stopPropagation()}>
            {overlay === "sidebar" ? (
              <WorkspaceSidebar workspaceId={workspace.id} refreshToken={documentsRevision} selectedDocumentId={selectedDocument?.id} onSelectDocument={(document) => void selectDocument(document)} onDocumentRemoved={(documentId) => {
                if (selectedDocument?.id === documentId) setSelectedDocument(null);
                setDocumentsRevision((revision) => revision + 1);
              }} />
            ) : (
              <aside aria-label="工作区移动对话栏" className="flex h-full flex-col">
                <WorkspaceChatPane
                  workspaceId={workspace.id}
                  editingDocumentId={selectedDocument?.kind === "artifact" ? selectedDocument.id : null}
                  onDocumentCreated={(document) => {
                    setDocumentsRevision((revision) => revision + 1);
                    void selectDocument(document);
                    setOverlay(null);
                  }}
                  onProposalCreated={() => setProposalRevision((revision) => revision + 1)}
                  beforeSend={() => documentPaneRef.current?.flush() ?? Promise.resolve(true)}
                />
              </aside>
            )}
          </div>
        </div>
      ) : null}
    </div>
  );
});

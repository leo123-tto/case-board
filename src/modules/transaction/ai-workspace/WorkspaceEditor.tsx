import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { History, Loader2, RotateCcw, Save } from "lucide-react";

import { MilkdownEditor } from "@/components/editor/MilkdownEditor";
import { DiffReview } from "@/components/editor/DiffReview";
import { EditorExportMenu, type EditorExportFormat } from "@/components/editor/EditorExportMenu";
import { Button } from "@/components/ui/button";
import { diffParts } from "@/lib/textDiff";

import {
  createAiWorkspaceArtifactVersion,
  listAiWorkspaceDocumentProposals,
  listAiWorkspaceArtifactVersions,
  readAiWorkspaceArtifact,
  restoreAiWorkspaceArtifactVersion,
  resolveAiWorkspaceDocumentProposal,
  saveAiWorkspaceArtifact,
} from "./api";
import type {
  AiWorkspaceDocument,
  AiWorkspaceDocumentProposal,
  AiWorkspaceDocumentVersion,
} from "./types";

export interface WorkspaceEditorHandle {
  flush: () => Promise<boolean>;
  isDirty: () => boolean;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export const WorkspaceEditor = forwardRef<
  WorkspaceEditorHandle,
  {
    workspaceId: string;
    document: AiWorkspaceDocument;
    onDocumentChanged?: (document: AiWorkspaceDocument) => void;
    proposalRefreshToken?: number;
    headerActions?: ReactNode;
  }
>(function WorkspaceEditor({ workspaceId, document, onDocumentChanged, proposalRefreshToken = 0, headerActions }, ref) {
  const [loaded, setLoaded] = useState(false);
  const [title, setTitle] = useState(document.title);
  const [markdown, setMarkdown] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [savedAt, setSavedAt] = useState<string | null>(null);
  const [versions, setVersions] = useState<AiWorkspaceDocumentVersion[]>([]);
  const [showVersions, setShowVersions] = useState(false);
  const [proposal, setProposal] = useState<AiWorkspaceDocumentProposal | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const documentRef = useRef(document);
  const titleRef = useRef(title);
  const markdownRef = useRef(markdown);
  const dirtyRef = useRef(dirty);
  const savePromiseRef = useRef<Promise<boolean> | null>(null);
  const loadedRevisionRef = useRef<{
    id: string;
    revision: number;
    proposalRefreshToken: number;
  } | null>(null);

  documentRef.current = documentRef.current.id === document.id ? documentRef.current : document;
  titleRef.current = title;
  markdownRef.current = markdown;
  dirtyRef.current = dirty;

  useEffect(() => {
    if (
      loadedRevisionRef.current?.id === document.id &&
      loadedRevisionRef.current.revision === document.working_copy_revision &&
      loadedRevisionRef.current.proposalRefreshToken === proposalRefreshToken
    ) {
      return;
    }
    let cancelled = false;
    setLoaded(false);
    setError(null);
    void Promise.all([
      readAiWorkspaceArtifact(workspaceId, document.id),
      listAiWorkspaceDocumentProposals(workspaceId, document.id),
    ])
      .then(([content, proposals]) => {
        if (cancelled) return;
        documentRef.current = content.document;
        loadedRevisionRef.current = {
          id: content.document.id,
          revision: content.document.working_copy_revision,
          proposalRefreshToken,
        };
        setTitle(content.document.title);
        setMarkdown(content.markdown);
        setDirty(false);
        setProposal(proposals[0] ?? null);
        setLoaded(true);
      })
      .catch((loadError) => {
        if (!cancelled) setError(`读取文稿失败：${errorText(loadError)}`);
      });
    return () => {
      cancelled = true;
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [document.id, document.working_copy_revision, proposalRefreshToken, workspaceId]);

  const flush = useCallback(async (): Promise<boolean> => {
    if (!dirtyRef.current) return true;
    if (savePromiseRef.current) return savePromiseRef.current;
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    setSaving(true);
    setError(null);
    const promise = saveAiWorkspaceArtifact(workspaceId, documentRef.current.id, {
      title: titleRef.current.trim() || documentRef.current.title,
      markdown: markdownRef.current,
      expected_revision: documentRef.current.working_copy_revision,
    })
      .then((saved) => {
        documentRef.current = saved.document;
        loadedRevisionRef.current = {
          id: saved.document.id,
          revision: saved.document.working_copy_revision,
          proposalRefreshToken,
        };
        setTitle(saved.document.title);
        setMarkdown(saved.markdown);
        setDirty(false);
        dirtyRef.current = false;
        setSavedAt(new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }));
        onDocumentChanged?.(saved.document);
        return true;
      })
      .catch((saveError) => {
        setError(`保存失败：${errorText(saveError)}`);
        return false;
      })
      .finally(() => {
        setSaving(false);
        savePromiseRef.current = null;
      });
    savePromiseRef.current = promise;
    return promise;
  }, [onDocumentChanged, workspaceId]);

  useImperativeHandle(ref, () => ({ flush, isDirty: () => dirtyRef.current }), [flush]);

  const scheduleSave = useCallback(() => {
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => void flush(), 800);
  }, [flush]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        void flush();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [flush]);

  const markChanged = (nextMarkdown: string) => {
    if (nextMarkdown === markdownRef.current) return;
    setMarkdown(nextMarkdown);
    markdownRef.current = nextMarkdown;
    setDirty(true);
    dirtyRef.current = true;
    scheduleSave();
  };

  const changeTitle = (nextTitle: string) => {
    setTitle(nextTitle);
    titleRef.current = nextTitle;
    setDirty(true);
    dirtyRef.current = true;
    scheduleSave();
  };

  const saveVersion = async () => {
    if (!(await flush())) return;
    try {
      await createAiWorkspaceArtifactVersion(workspaceId, document.id, {
        trigger: "manual",
        summary: null,
      });
      setVersions(await listAiWorkspaceArtifactVersions(workspaceId, document.id));
      setShowVersions(true);
    } catch (versionError) {
      setError(`保存版本失败：${errorText(versionError)}`);
    }
  };

  const restoreVersion = async (version: AiWorkspaceDocumentVersion) => {
    if (!(await flush())) return;
    try {
      const restored = await restoreAiWorkspaceArtifactVersion(
        workspaceId,
        document.id,
        version.id,
        documentRef.current.working_copy_revision,
      );
      documentRef.current = restored.document;
      loadedRevisionRef.current = {
        id: restored.document.id,
        revision: restored.document.working_copy_revision,
        proposalRefreshToken,
      };
      setTitle(restored.document.title);
      setMarkdown(restored.markdown);
      setDirty(false);
      onDocumentChanged?.(restored.document);
      setVersions(await listAiWorkspaceArtifactVersions(workspaceId, document.id));
    } catch (restoreError) {
      setError(`恢复版本失败：${errorText(restoreError)}`);
    }
  };

  const prepareExport = async (format: EditorExportFormat): Promise<boolean> => {
    void format;
    return flush();
  };

  const createExportVersion = async (format: EditorExportFormat): Promise<boolean> => {
    setError(null);
    try {
      await createAiWorkspaceArtifactVersion(workspaceId, documentRef.current.id, {
        trigger: "export",
        summary: `导出 ${format === "html" ? "HTML" : "Word"} 前版本`,
      });
      return true;
    } catch (exportError) {
      setError(`创建导出前版本失败：${errorText(exportError)}`);
      return false;
    }
  };

  const applyProposal = async (finalMarkdown: string) => {
    if (!proposal) return;
    setSaving(true);
    setError(null);
    try {
      const resolved = await resolveAiWorkspaceDocumentProposal(
        workspaceId,
        proposal.id,
        "accepted",
        finalMarkdown,
      );
      if (!resolved) throw new Error("后端未返回更新后的文稿");
      documentRef.current = resolved.document;
      loadedRevisionRef.current = {
        id: resolved.document.id,
        revision: resolved.document.working_copy_revision,
        proposalRefreshToken,
      };
      markdownRef.current = resolved.markdown;
      dirtyRef.current = false;
      setMarkdown(resolved.markdown);
      setDirty(false);
      setProposal(null);
      onDocumentChanged?.(resolved.document);
      setVersions(await listAiWorkspaceArtifactVersions(workspaceId, document.id));
    } catch (proposalError) {
      setError(`应用 AI 修改失败：${errorText(proposalError)}`);
    } finally {
      setSaving(false);
    }
  };

  const rejectProposal = async () => {
    if (!proposal) return;
    setSaving(true);
    setError(null);
    try {
      await resolveAiWorkspaceDocumentProposal(
        workspaceId,
        proposal.id,
        "rejected",
        null,
      );
      setProposal(null);
    } catch (proposalError) {
      setError(`放弃 AI 修改失败：${errorText(proposalError)}`);
    } finally {
      setSaving(false);
    }
  };

  if (!loaded) {
    return (
      <div className="flex h-full items-center justify-center">
        {error ? <span className="text-xs text-destructive">{error}</span> : <Loader2 className="size-5 animate-spin text-muted-foreground" />}
      </div>
    );
  }

  if (proposal) {
    return (
      <DiffReview
        parts={diffParts(markdown, proposal.proposed_markdown)}
        onApply={(finalMarkdown) => void applyProposal(finalMarkdown)}
        onCancel={() => void rejectProposal()}
      />
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-2">
        <input
          aria-label="文稿标题"
          value={title}
          onChange={(event) => changeTitle(event.target.value)}
          className="min-w-0 flex-1 bg-transparent text-sm font-medium outline-none"
        />
        <span className="text-[11px] text-muted-foreground">
          {saving ? "保存中…" : dirty ? "未保存" : savedAt ? `已保存 ${savedAt}` : "已保存"}
        </span>
        <Button size="sm" variant="outline" aria-label="保存版本" onClick={() => void saveVersion()}>
          <Save className="size-3.5" />保存版本
        </Button>
        <EditorExportMenu
          title={title}
          mdPath={documentRef.current.content_path ?? ""}
          beforeExport={prepareExport}
          beforeWrite={createExportVersion}
          onError={(message) => setError(message || null)}
          disabled={saving || !documentRef.current.content_path}
        />
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            const next = !showVersions;
            setShowVersions(next);
            if (next) void listAiWorkspaceArtifactVersions(workspaceId, document.id).then(setVersions);
          }}
        >
          <History className="size-3.5" />版本
        </Button>
        {headerActions}
      </header>
      {error ? (
        <div className="flex items-center justify-between border-b border-destructive/20 bg-destructive/5 px-3 py-2 text-xs text-destructive">
          <span>{error}</span>
          <button type="button" onClick={() => void flush()} className="underline">重试保存</button>
        </div>
      ) : null}
      {showVersions ? (
        <div className="max-h-36 shrink-0 overflow-auto border-b border-border bg-muted/20 px-3 py-2">
          {versions.map((version) => (
            <div key={version.id} className="flex items-center justify-between py-1 text-xs">
              <span>v{version.version_no} · {version.change_summary || version.trigger}</span>
              <button type="button" onClick={() => void restoreVersion(version)} className="flex items-center gap-1 text-brand hover:underline"><RotateCcw className="size-3" />恢复</button>
            </div>
          ))}
        </div>
      ) : null}
      <MilkdownEditor
        key={`${document.id}-${documentRef.current.working_copy_revision}`}
        value={markdown}
        onChange={markChanged}
        className="min-h-0 flex-1"
      />
    </div>
  );
});

import { forwardRef, useImperativeHandle, useMemo, useRef, type ReactNode } from "react";
import { FileText } from "lucide-react";

import { DocumentViewer } from "@/components/document-viewer/DocumentViewer";
import type {
  DocumentViewerAccess,
  ViewerDocument,
} from "@/components/document-viewer/documentViewerTypes";
import { openInDefaultApp, revealInFinder } from "@/lib/api";

import {
  allowAiWorkspaceAssets,
  readAiWorkspaceFileBytes,
  readAiWorkspaceText,
} from "./api";
import type { AiWorkspaceDocument } from "./types";
import { WorkspaceEditor, type WorkspaceEditorHandle } from "./WorkspaceEditor";

export interface WorkspaceDocumentPaneHandle {
  flush: () => Promise<boolean>;
}

export const WorkspaceDocumentPane = forwardRef<WorkspaceDocumentPaneHandle, {
  workspaceId: string;
  document: AiWorkspaceDocument | null;
  onDocumentChanged?: (document: AiWorkspaceDocument) => void;
  proposalRefreshToken?: number;
  headerActions?: ReactNode;
}>(function WorkspaceDocumentPane({
  workspaceId,
  document,
  onDocumentChanged,
  proposalRefreshToken = 0,
  headerActions,
}, ref) {
  const editorRef = useRef<WorkspaceEditorHandle>(null);
  useImperativeHandle(ref, () => ({
    flush: () => editorRef.current?.flush() ?? Promise.resolve(true),
  }), []);
  const viewerDocument = useMemo<ViewerDocument | null>(() => {
    if (!document || document.kind !== "source" || !document.source_path) return null;
    return {
      id: document.id,
      filename: document.filename,
      displayName: document.title,
      sourcePath: document.source_path,
      extractedTextPath: document.extracted_text_path,
      extractionStatus: document.extraction_status,
    };
  }, [document]);

  const access = useMemo<DocumentViewerAccess | null>(() => {
    if (!document || !document.source_path) return null;
    return {
      allowOriginal: () => allowAiWorkspaceAssets(workspaceId, document.id),
      readText: async (kind) => {
        if (kind === "derived") return readAiWorkspaceText(workspaceId, document.id);
        const bytes = await readAiWorkspaceFileBytes(workspaceId, document.id);
        return new TextDecoder().decode(new Uint8Array(bytes));
      },
      readBytes: () => readAiWorkspaceFileBytes(workspaceId, document.id),
      openOriginal: () => openInDefaultApp(document.source_path!),
      revealOriginal: () => revealInFinder(document.source_path!),
    };
  }, [document, workspaceId]);

  if (!document) {
    return (
      <div className="relative flex h-full flex-col items-center justify-center gap-2 text-muted-foreground">
        <div className="absolute right-3 top-2">{headerActions}</div>
        <FileText className="size-8 opacity-30" />
        <p className="text-xs">选择左侧材料查看，或新建一份文稿。</p>
      </div>
    );
  }
  if (document.kind === "artifact") {
    return <WorkspaceEditor ref={editorRef} workspaceId={workspaceId} document={document} onDocumentChanged={onDocumentChanged} proposalRefreshToken={proposalRefreshToken} headerActions={headerActions} />;
  }
  if (!viewerDocument || !access) {
    return <div className="flex h-full items-center justify-center text-xs text-destructive">材料路径不可用</div>;
  }
  return <DocumentViewer document={viewerDocument} access={access} headerActions={headerActions} />;
});

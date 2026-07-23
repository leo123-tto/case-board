import { BookOpen, TriangleAlert } from "lucide-react";

import type { RetrievedWorkspaceSource } from "./types";

export function WorkspaceCitations({
  citations,
  onOpen,
}: {
  citations: RetrievedWorkspaceSource[];
  onOpen?: (citation: RetrievedWorkspaceSource) => void;
}) {
  if (citations.length === 0) return null;
  const documents = Array.from(
    citations.reduce((byId, citation) => {
      const existing = byId.get(citation.document_id);
      if (!existing || (!existing.source_missing && citation.source_missing)) {
        byId.set(citation.document_id, citation);
      }
      return byId;
    }, new Map<string, RetrievedWorkspaceSource>()).values(),
  );
  return (
    <section className="mt-3 rounded-lg border border-border bg-muted/20 p-2.5">
      <h4 className="mb-1.5 flex items-center gap-1.5 text-[11px] font-medium text-muted-foreground">
        <BookOpen className="size-3.5" />本轮参考材料
      </h4>
      <div className="flex flex-wrap gap-1.5">
        {documents.map((citation) => {
          const label = (
            <span className="flex items-center gap-1">
              {citation.title}
              {citation.source_missing ? <TriangleAlert className="size-3 text-amber-500" /> : null}
            </span>
          );
          return onOpen ? (
            <button
              key={citation.document_id}
              type="button"
              onClick={() => onOpen(citation)}
              className="rounded-md bg-card px-2 py-1 text-[11px] font-medium text-foreground hover:bg-background"
            >
              {label}
            </button>
          ) : (
            <div
              key={citation.document_id}
              className="rounded-md bg-card px-2 py-1 text-[11px] font-medium text-foreground"
            >
              {label}
            </div>
          );
        })}
      </div>
    </section>
  );
}

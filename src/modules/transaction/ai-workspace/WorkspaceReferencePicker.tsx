import { AtSign, FileText, X } from "lucide-react";

import type { AiWorkspaceDocument, WorkspaceReference } from "./types";

export function WorkspaceReferencePicker({
  documents,
  value,
  onChange,
}: {
  documents: AiWorkspaceDocument[];
  value: WorkspaceReference[];
  onChange: (references: WorkspaceReference[]) => void;
}) {
  const available = documents.filter(
    (document) =>
      document.kind === "artifact" ||
      ["ready", "review"].includes(document.extraction_status),
  );

  const add = (document: AiWorkspaceDocument) => {
    if (value.some((reference) => reference.document_id === document.id && reference.page_no === null)) {
      return;
    }
    onChange([...value, { document_id: document.id, page_no: null }]);
  };

  return (
    <div className="rounded-lg border border-border bg-card p-2 shadow-sm">
      {value.length > 0 ? (
        <div className="mb-2 flex flex-wrap gap-1">
          {value.map((reference) => {
            const document = documents.find((item) => item.id === reference.document_id);
            return (
              <span key={`${reference.document_id}-${reference.page_no ?? "all"}`} className="flex items-center gap-1 rounded-md bg-muted px-2 py-1 text-[11px] text-foreground">
                <AtSign className="size-3 text-brand" />
                {document?.title ?? "已失效引用"}
                {reference.page_no ? ` P${reference.page_no}` : ""}
                <button type="button" aria-label={`移除${document?.title ?? "引用"}`} onClick={() => onChange(value.filter((item) => item !== reference))}><X className="size-3 text-muted-foreground" /></button>
              </span>
            );
          })}
        </div>
      ) : null}
      <p className="mb-1.5 px-1 text-[11px] text-muted-foreground">引用当前工作区文件</p>
      <div className="max-h-44 overflow-auto">
        {available.map((document) => (
          <button
            key={document.id}
            type="button"
            aria-label={`引用${document.title}`}
            onClick={() => add(document)}
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs hover:bg-muted"
          >
            <FileText className="size-3.5 text-muted-foreground" />
            <span className="truncate">{document.title}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

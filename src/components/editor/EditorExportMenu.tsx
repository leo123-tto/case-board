import { useEffect, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { ChevronDown, FileCode2, FileDown, FileText, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { exportEditorDocument, revealInFinder } from "@/lib/api";

export type EditorExportFormat = "docx" | "html";

interface Props {
  title: string;
  mdPath: string;
  /** 导出前把当前编辑内容刷新到 mdPath；false 表示保存失败。 */
  beforeExport: (format: EditorExportFormat) => Promise<boolean>;
  /** 用户已选定目标路径后、真正写文件前执行，用于记录导出版本。 */
  beforeWrite?: (format: EditorExportFormat) => Promise<boolean>;
  onError: (message: string) => void;
  disabled?: boolean;
}

function safeDocumentTitle(title: string): string {
  return (title.trim() || "未命名文稿").replace(/[\\/:*?"<>|]/g, "_");
}

export function EditorExportMenu({ title, mdPath, beforeExport, beforeWrite, onError, disabled = false }: Props) {
  const [open, setOpen] = useState(false);
  const [exporting, setExporting] = useState<EditorExportFormat | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const closeOutside = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", closeOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [open]);

  const runExport = async (format: EditorExportFormat) => {
    setOpen(false);
    setExporting(format);
    onError("");
    try {
      if (!(await beforeExport(format))) return;
      const safeTitle = safeDocumentTitle(title);
      const isHtml = format === "html";
      const savePath = await save({
        defaultPath: `${safeTitle}.${isHtml ? "html" : "docx"}`,
        filters: [
          isHtml
            ? { name: "HTML 文档", extensions: ["html"] }
            : { name: "Word 文档", extensions: ["docx"] },
        ],
      });
      if (!savePath) return;
      if (beforeWrite && !(await beforeWrite(format))) return;
      const written = await exportEditorDocument(mdPath, safeTitle, format, savePath);
      try {
        await revealInFinder(written);
      } catch {
        // 导出已成功，Finder 定位失败不应把结果伪装成导出失败。
      }
    } catch (error) {
      onError(`导出失败:${String(error)}`);
    } finally {
      setExporting(null);
    }
  };

  return (
    <div ref={rootRef} className="relative">
      <Button
        type="button"
        variant="outline"
        size="sm"
        aria-label="导出"
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={disabled || exporting !== null}
        onClick={() => setOpen((value) => !value)}
      >
        {exporting ? <Loader2 className="size-3.5 animate-spin" /> : <FileDown className="size-3.5" />}
        {exporting ? "导出中…" : "导出"}
        {!exporting ? <ChevronDown className="size-3" /> : null}
      </Button>
      {open ? (
        <div
          role="menu"
          aria-label="导出格式"
          className="absolute right-0 z-50 mt-1.5 min-w-40 rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-lg"
        >
          <button
            type="button"
            role="menuitem"
            className="flex w-full items-center gap-2 rounded px-2.5 py-2 text-left text-sm hover:bg-accent focus:bg-accent focus:outline-none"
            onClick={() => void runExport("docx")}
          >
            <FileText className="size-4" />导出为 Word
          </button>
          <button
            type="button"
            role="menuitem"
            className="flex w-full items-center gap-2 rounded px-2.5 py-2 text-left text-sm hover:bg-accent focus:bg-accent focus:outline-none"
            onClick={() => void runExport("html")}
          >
            <FileCode2 className="size-4" />导出为 HTML
          </button>
        </div>
      ) : null}
    </div>
  );
}

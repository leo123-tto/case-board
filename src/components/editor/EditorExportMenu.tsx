import { useEffect, useRef, useState } from "react";
import { join } from "@tauri-apps/api/path";
import { save } from "@tauri-apps/plugin-dialog";
import { ChevronDown, FileCode2, FileDown, FileText, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { exportEditorDocument, revealInFinder, type WordTemplate } from "@/lib/api";

export type EditorExportFormat = "docx" | "html";

interface Props {
  title: string;
  mdPath: string;
  /** 系统保存对话框首次打开时使用的目录；用户仍可自由修改。 */
  defaultDirectory?: string | null;
  /** 案件内导出时用于后端读取可信页眉元数据；独立工作区不传。 */
  caseId?: string;
  /** 导出前把当前编辑内容刷新到 mdPath；false 表示保存失败。 */
  beforeExport: (format: EditorExportFormat) => Promise<boolean>;
  /** 用户已选定目标路径后、真正写文件前执行，用于记录导出版本。 */
  beforeWrite?: (format: EditorExportFormat) => Promise<boolean>;
  /** 文件实际写入成功后通知调用方记录最终路径。 */
  onExported?: (
    format: EditorExportFormat,
    writtenPath: string,
    wordTemplate?: WordTemplate,
  ) => Promise<void>;
  onError: (message: string) => void;
  disabled?: boolean;
}

function documentTitle(title: string): string {
  return title.trim() || "未命名文稿";
}

function safeFilenameTitle(title: string): string {
  return title.replace(/[\\/:*?"<>|]/g, "_");
}

export function EditorExportMenu({
  title,
  mdPath,
  defaultDirectory,
  caseId,
  beforeExport,
  beforeWrite,
  onExported,
  onError,
  disabled = false,
}: Props) {
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

  const runExport = async (
    format: EditorExportFormat,
    wordTemplate?: WordTemplate,
  ) => {
    setOpen(false);
    setExporting(format);
    onError("");
    try {
      if (!(await beforeExport(format))) return;
      const exportTitle = documentTitle(title);
      const safeTitle = safeFilenameTitle(exportTitle);
      const isHtml = format === "html";
      const filename = `${safeTitle}.${isHtml ? "html" : "docx"}`;
      const defaultPath = defaultDirectory
        ? await join(defaultDirectory, filename)
        : filename;
      const savePath = await save({
        defaultPath,
        filters: [
          isHtml
            ? { name: "HTML 文档", extensions: ["html"] }
            : { name: "Word 文档", extensions: ["docx"] },
        ],
      });
      if (!savePath) return;
      if (beforeWrite && !(await beforeWrite(format))) return;
      const written = caseId
        ? await exportEditorDocument(
            mdPath,
            exportTitle,
            format,
            savePath,
            wordTemplate,
            caseId,
          )
        : await exportEditorDocument(
            mdPath,
            exportTitle,
            format,
            savePath,
            wordTemplate,
          );
      await onExported?.(format, written, wordTemplate);
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
            onClick={() => void runExport("docx", "editor")}
          >
            <FileText className="size-4" />Word（保留当前排版）
          </button>
          <button
            type="button"
            role="menuitem"
            className="flex w-full items-center gap-2 rounded px-2.5 py-2 text-left text-sm hover:bg-accent focus:bg-accent focus:outline-none"
            onClick={() => void runExport("docx", "legal_filing")}
          >
            <FileText className="size-4" />Word（法律文书模板）
          </button>
          <button
            type="button"
            role="menuitem"
            className="flex w-full items-center gap-2 rounded px-2.5 py-2 text-left text-sm hover:bg-accent focus:bg-accent focus:outline-none"
            onClick={() => void runExport("html")}
          >
            <FileCode2 className="size-4" />HTML
          </button>
        </div>
      ) : null}
    </div>
  );
}

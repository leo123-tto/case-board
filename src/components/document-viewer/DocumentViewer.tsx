import { useEffect, useRef, useState, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { convertFileSrc } from "@tauri-apps/api/core";
import { ExternalLink, FileText, FolderOpen, Loader2 } from "lucide-react";

import {
  workbookToPreviewSheets,
  type SpreadsheetPreviewSheet,
  type SpreadsheetPreviewUtils,
} from "@/lib/spreadsheetPreview";
import { cn } from "@/lib/utils";

import type {
  DocumentViewerAccess,
  OriginalRenderer,
  ViewerDocument,
} from "./documentViewerTypes";

type Tab = "derived" | "original";

const isPdf = (name: string) => /\.pdf$/i.test(name);
const isImage = (name: string) => /\.(png|jpe?g|webp|tiff?|bmp|gif|jp2)$/i.test(name);
const isNativeText = (name: string) => /\.(md|markdown|txt|html?)$/i.test(name);
const isDocx = (name: string) => /\.docx$/i.test(name);
const isSpreadsheet = (name: string) => /\.(xlsx?|csv)$/i.test(name);

function safeAssetUrl(path: string): string {
  try {
    return convertFileSrc(path);
  } catch {
    return path;
  }
}

export function DocumentViewer({
  document,
  access,
  renderOriginal,
  headerActions,
}: {
  document: ViewerDocument;
  access: DocumentViewerAccess;
  renderOriginal?: OriginalRenderer;
  headerActions?: ReactNode;
}) {
  const canShowDerived =
    isNativeText(document.filename) ||
    (!!document.extractedTextPath && ["done", "ready", "review"].includes(document.extractionStatus));
  const [tab, setTab] = useState<Tab>(canShowDerived ? "derived" : "original");
  const [text, setText] = useState<string | null>(null);
  const [textError, setTextError] = useState<string | null>(null);
  const [assetUrl, setAssetUrl] = useState<string | null>(null);
  const [assetError, setAssetError] = useState<string | null>(null);

  useEffect(() => {
    setTab(canShowDerived ? "derived" : "original");
    setText(null);
    setTextError(null);
    setAssetUrl(null);
    setAssetError(null);
  }, [canShowDerived, document.id]);

  useEffect(() => {
    if (tab !== "derived" || !canShowDerived || text !== null) return;
    let cancelled = false;
    access
      .readText(isNativeText(document.filename) ? "original" : "derived")
      .then((value) => !cancelled && setText(value))
      .catch((error) => !cancelled && setTextError(String(error)));
    return () => {
      cancelled = true;
    };
  }, [access, canShowDerived, document.filename, tab, text]);

  useEffect(() => {
    if (tab !== "original" || assetUrl || assetError) return;
    let cancelled = false;
    access
      .allowOriginal()
      .then(() => !cancelled && setAssetUrl(safeAssetUrl(document.sourcePath)))
      .catch((error) => !cancelled && setAssetError(String(error)));
    return () => {
      cancelled = true;
    };
  }, [access, assetError, assetUrl, document.sourcePath, tab]);

  return (
    <div className="flex h-full min-h-0 flex-col bg-background">
      <div className="flex shrink-0 items-center border-b border-border px-3">
        <ViewerTab active={tab === "derived"} disabled={!canShowDerived} onClick={() => setTab("derived")}>
          处理后文本
        </ViewerTab>
        <ViewerTab active={tab === "original"} onClick={() => setTab("original")}>
          原件
        </ViewerTab>
        <div className="ml-auto flex items-center gap-1">
          <button
            type="button"
            title="用系统默认程序打开原文件"
            aria-label="用系统默认程序打开原文件"
            onClick={() => void access.openOriginal()}
            className="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-muted-foreground hover:bg-muted"
          >
            <ExternalLink className="size-3" />打开
          </button>
          <button
            type="button"
            title="在 Finder 中定位原文件"
            aria-label="在 Finder 中定位原文件"
            onClick={() => void access.revealOriginal()}
            className="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-muted-foreground hover:bg-muted"
          >
            <FolderOpen className="size-3" />定位
          </button>
          {headerActions}
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-auto bg-muted/20">
        {tab === "derived" ? (
          textError ? (
            <ViewerEmpty text={`读取失败：${textError}`} />
          ) : text === null ? (
            <ViewerLoading />
          ) : (
            <div className={cn(
              "px-6 py-5 text-sm leading-relaxed text-foreground",
              "[&_h1]:mb-3 [&_h1]:mt-5 [&_h1]:text-xl [&_h1]:font-semibold",
              "[&_h2]:mb-2 [&_h2]:mt-4 [&_h2]:text-base [&_h2]:font-semibold",
              "[&_p]:my-2 [&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-6",
              "[&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-6",
              "[&_table]:my-3 [&_table]:w-full [&_table]:border-collapse [&_table]:text-xs",
              "[&_th]:border [&_th]:border-border [&_th]:bg-muted/50 [&_th]:px-2 [&_th]:py-1.5",
              "[&_td]:border [&_td]:border-border [&_td]:px-2 [&_td]:py-1.5",
            )}>
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{text}</ReactMarkdown>
            </div>
          )
        ) : renderOriginal ? (
          renderOriginal({ document, assetUrl, assetError, access })
        ) : (
          <GenericOriginalView document={document} access={access} assetUrl={assetUrl} assetError={assetError} />
        )}
      </div>
    </div>
  );
}

function GenericOriginalView({
  document,
  access,
  assetUrl,
  assetError,
}: {
  document: ViewerDocument;
  access: DocumentViewerAccess;
  assetUrl: string | null;
  assetError: string | null;
}) {
  if (assetError) return <ViewerEmpty text={`无法加载原件：${assetError}`} />;
  if (!assetUrl) return <ViewerLoading />;
  if (isPdf(document.filename)) {
    return <iframe src={assetUrl} title={document.filename} className="h-full min-h-[480px] w-full border-0" />;
  }
  if (isImage(document.filename)) {
    return <div className="flex min-h-full justify-center p-4"><img src={assetUrl} alt={document.filename} className="max-w-full" /></div>;
  }
  if (isDocx(document.filename) || isSpreadsheet(document.filename)) {
    return <OfficePreview filename={document.filename} access={access} />;
  }
  if (isNativeText(document.filename)) {
    return <OriginalText access={access} />;
  }
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 text-muted-foreground">
      <FileText className="size-9 opacity-40" />
      <p className="text-xs">这种格式暂不支持板内预览，可用系统程序打开。</p>
    </div>
  );
}

function OriginalText({ access }: { access: DocumentViewerAccess }) {
  const [text, setText] = useState<string | null>(null);
  useEffect(() => {
    void access.readText("original").then(setText).catch((error) => setText(`读取失败：${error}`));
  }, [access]);
  return text === null ? <ViewerLoading /> : <pre className="whitespace-pre-wrap p-5 text-xs">{text}</pre>;
}

function OfficePreview({ filename, access }: { filename: string; access: DocumentViewerAccess }) {
  const ref = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [sheets, setSheets] = useState<SpreadsheetPreviewSheet[]>([]);
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const bytes = await access.readBytes();
        if (cancelled) return;
        if (isSpreadsheet(filename)) {
          const XLSX = await import("xlsx");
          const workbook = XLSX.read(bytes, { type: "array" });
          setSheets(workbookToPreviewSheets(workbook, XLSX.utils as SpreadsheetPreviewUtils));
        } else {
          const { renderAsync } = await import("docx-preview");
          if (!cancelled && ref.current) {
            await renderAsync(new Uint8Array(bytes), ref.current);
          }
        }
      } catch (loadError) {
        if (!cancelled) setError(String(loadError));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [access, filename]);
  if (error) return <ViewerEmpty text={`板内预览失败：${error}`} />;
  if (isSpreadsheet(filename) && sheets.length > 0) {
    return <div className="space-y-4 p-4">{sheets.map((sheet) => <section key={sheet.name}><h3 className="mb-1 text-xs font-medium">{sheet.name}</h3><div className="overflow-auto"><table className="border-collapse text-xs"><tbody>{sheet.rows.map((row, rowIndex) => <tr key={rowIndex}>{row.map((cell, cellIndex) => <td key={cellIndex} className="border border-border px-2 py-1">{cell}</td>)}</tr>)}</tbody></table></div></section>)}</div>;
  }
  return <div ref={ref} className="min-h-full bg-white p-4" />;
}

function ViewerTab({ active, disabled, onClick, children }: { active: boolean; disabled?: boolean; onClick: () => void; children: React.ReactNode }) {
  return <button type="button" disabled={disabled} onClick={onClick} className={cn("border-b-2 px-3 py-2 text-xs", active ? "border-brand text-foreground" : "border-transparent text-muted-foreground", disabled && "opacity-40")}>{children}</button>;
}

function ViewerLoading() {
  return <div className="flex h-full min-h-48 items-center justify-center"><Loader2 className="size-5 animate-spin text-muted-foreground" /></div>;
}

function ViewerEmpty({ text }: { text: string }) {
  return <div className="flex h-full min-h-48 items-center justify-center px-6 text-center text-xs text-muted-foreground">{text}</div>;
}

import { useState } from "react";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { ChevronDown, ChevronUp, FileText, Loader2, RefreshCw } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { Button } from "@/components/ui/button";
import { toast } from "@/components/ui/toast";
import {
  exportCaseWorkReportDocx,
  generateCaseWorkReport,
  revealInFinder,
} from "@/lib/api";

export function CaseWorkReportSection({
  caseId,
  caseName,
}: {
  caseId: string;
  caseName: string;
}) {
  const [content, setContent] = useState<string | null>(null);
  const [previewOpen, setPreviewOpen] = useState(false);
  const [busy, setBusy] = useState<"generate" | "export" | null>(null);

  async function generate() {
    if (busy) return;
    setBusy("generate");
    try {
      setContent(await generateCaseWorkReport(caseId));
      setPreviewOpen(true);
      toast("工作汇报已生成，可先在下方预览", "success");
    } catch (error) {
      toast(`生成工作汇报失败:${error}`, "error");
    } finally {
      setBusy(null);
    }
  }

  async function exportWord() {
    if (busy || !content) return;
    let savePath: string | null;
    try {
      savePath = await saveDialog({
        defaultPath: `${caseName}_工作汇报.docx`,
        filters: [{ name: "Word 文档", extensions: ["docx"] }],
      });
    } catch (error) {
      toast(`打开保存对话框失败:${error}`, "error");
      return;
    }
    if (!savePath) return;

    setBusy("export");
    try {
      const written = await exportCaseWorkReportDocx(caseId, savePath, content);
      toast(`工作汇报 Word 已导出:${written}`, "success", 8000);
      await revealInFinder(written).catch(() => {});
    } catch (error) {
      toast(`导出工作汇报失败:${error}`, "error");
    } finally {
      setBusy(null);
    }
  }

  return (
    <section className="rounded-lg border border-border bg-card px-6 py-4 shadow-sm">
      <div className="mb-3 flex items-start justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold text-foreground">工作汇报</h3>
          <p className="mt-0.5 text-caption text-muted-foreground">
            按需汇总案件概况、办案时间轴和工作记录，先预览内容，确认后导出 Word。
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {content && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => setPreviewOpen((value) => !value)}
              disabled={!!busy}
            >
              {previewOpen ? (
                <ChevronUp className="size-3.5" />
              ) : (
                <ChevronDown className="size-3.5" />
              )}
              {previewOpen ? "收起预览" : "展开预览"}
            </Button>
          )}
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => void generate()}
            disabled={!!busy}
          >
            {busy === "generate" ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <RefreshCw className="size-3.5" />
            )}
            {content ? "重新生成" : "生成汇报"}
          </Button>
          <Button
            type="button"
            size="sm"
            onClick={() => void exportWord()}
            disabled={!!busy || !content}
          >
            {busy === "export" ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <FileText className="size-3.5" />
            )}
            导出 Word
          </Button>
        </div>
      </div>

      {content && previewOpen ? (
        <div className="max-h-96 overflow-y-auto rounded-md border border-border bg-muted/20 px-4 py-3">
          <div className="prose-md text-sm leading-relaxed text-foreground [&_h1]:mb-3 [&_h1]:mt-1 [&_h1]:text-lg [&_h1]:font-semibold [&_h2]:mb-2 [&_h2]:mt-4 [&_h2]:text-base [&_h2]:font-semibold [&_h3]:mb-1.5 [&_h3]:mt-3 [&_h3]:text-sm [&_h3]:font-semibold [&_li]:my-1 [&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-6 [&_p]:my-2 [&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-6">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{content}</ReactMarkdown>
          </div>
        </div>
      ) : content ? (
        <div className="rounded-md border border-dashed border-border bg-muted/20 px-4 py-4 text-center text-xs text-muted-foreground">
          工作汇报已生成，预览已收起。可展开查看，或直接导出 Word。
        </div>
      ) : (
        <div className="rounded-md border border-dashed border-border bg-muted/20 px-4 py-6 text-center text-xs text-muted-foreground">
          点击“生成汇报”后，这里会显示结合办案时间轴和工作记录生成的汇报正文。
        </div>
      )}
    </section>
  );
}

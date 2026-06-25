/**
 * 法院短信处理(V0.3 · 仅「人民法院在线服务/一张网」zxfw.court.gov.cn)。
 *
 * 流程:粘贴法院送达短信 → 解析(法院/案号/链接)+ 拉文书列表 + 自动匹配在办案件
 * → 确认归档到某案件 → 下载 PDF 进案件源文件夹 → 复用「刷新源文件」抽取上看板。
 *
 * wjlj 下载地址有时效:预览只拿文书名,确认导入时后端重新拉新鲜地址再下。
 */
import { useEffect, useState } from "react";
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  CheckCircle2,
  Download,
  FileText,
  FolderOpen,
  Gavel,
  Loader2,
} from "lucide-react";

import {
  downloadCourtSmsToFolder,
  listCases,
  previewCourtSms,
  ingestCourtSms,
  revealInFinder,
  type CourtSmsPreview,
} from "@/lib/api";
import type { Case } from "@/lib/types";
import { toast } from "@/components/ui/toast";

type DownloadDone =
  | { mode: "case"; files: string[]; skipped: string[] }
  | { mode: "folder"; files: string[]; skipped: string[]; folder: string };

export function CourtSmsTool() {
  const [sms, setSms] = useState("");
  const [loading, setLoading] = useState(false);
  const [preview, setPreview] = useState<CourtSmsPreview | null>(null);
  const [cases, setCases] = useState<Case[]>([]);
  const [pickedCaseId, setPickedCaseId] = useState<string>("");
  const [ingesting, setIngesting] = useState(false);
  const [downloadingLocal, setDownloadingLocal] = useState(false);
  const [done, setDone] = useState<DownloadDone | null>(null);

  useEffect(() => {
    listCases().then(setCases).catch(() => {});
  }, []);

  const reset = () => {
    setPreview(null);
    setDone(null);
    setPickedCaseId("");
  };

  const handleParse = async () => {
    if (!sms.trim()) return;
    setLoading(true);
    reset();
    try {
      const p = await previewCourtSms(sms);
      setPreview(p);
      if (p.matched_case_id) {
        setPickedCaseId(p.matched_case_id);
      } else if (p.name_matches.length > 0) {
        // 2026-06-11:案号没中(典型:短信是执行案号)→ 按当事人姓名匹配,
        // 预选最可信候选,用户确认后再归档
        setPickedCaseId(p.name_matches[0].case_id);
      }
    } catch (e) {
      toast(`解析失败:${e}`, "error");
    } finally {
      setLoading(false);
    }
  };

  const handleIngest = async () => {
    if (!preview?.link || !pickedCaseId) return;
    setIngesting(true);
    try {
      const r = await ingestCourtSms(pickedCaseId, preview.link);
      setDone({ mode: "case", files: r.downloaded, skipped: r.skipped });
      if (r.downloaded.length > 0) {
        toast(
          `已下载 ${r.downloaded.length} 份文书进案件,正在后台抽取,稍后在案件里查看`,
          "success",
        );
      } else {
        toast("没有下载到文书(链接可能已失效)", "error");
      }
    } catch (e) {
      toast(`导入失败:${e}`, "error");
    } finally {
      setIngesting(false);
    }
  };

  const handleDownloadToFolder = async () => {
    if (!preview?.link) return;
    try {
      const picked = await dialogOpen({
        directory: true,
        multiple: false,
        title: "选择法院文书保存文件夹",
      });
      if (typeof picked !== "string" || !picked.trim()) return;
      setDownloadingLocal(true);
      const r = await downloadCourtSmsToFolder(preview.link, picked);
      setDone({ mode: "folder", files: r.downloaded, skipped: r.skipped, folder: r.folder });
      if (r.downloaded.length > 0) {
        toast(`已下载 ${r.downloaded.length} 份文书到本地文件夹`, "success");
        await revealInFinder(r.folder).catch(() => {});
      } else {
        toast("没有下载到文书(链接可能已失效)", "error");
      }
    } catch (e) {
      toast(`下载失败:${e}`, "error");
    } finally {
      setDownloadingLocal(false);
    }
  };

  // 2026-06-11:当事人名字前置 —— 律师记得住名字记不住案号,下拉先看人名
  const caseLabel = (c: Case) => {
    const first = (json: string | null): string | null => {
      if (!json) return null;
      try {
        const arr = JSON.parse(json) as string[];
        return Array.isArray(arr) && arr.length > 0 ? arr[0] : null;
      } catch {
        return null;
      }
    };
    const p = first(c.agg_plaintiffs);
    const d = first(c.agg_defendants);
    const parties = p || d ? `${p ?? "—"} vs ${d ?? "—"} · ` : "";
    return `${parties}${c.agg_cause || c.name}${c.agg_case_no ? ` · ${c.agg_case_no}` : ""}`;
  };

  return (
    <div className="space-y-4">
      {/* 说明 */}
      <div className="rounded-lg border border-sky-200 bg-sky-50 px-4 py-3 text-xs text-sky-800 dark:border-sky-800/50 dark:bg-sky-950/30 dark:text-sky-200">
        把法院发来的<strong>送达短信</strong>整条粘进来 —— 自动识别案号、下载文书 PDF、
        可归档进对应案件并触发抽取上看板;匹配不到案件时也可直接下载到本地文件夹。目前仅支持
        <strong>「人民法院在线服务/一张网」(zxfw.court.gov.cn)</strong>的链接(无需登录)。
      </div>

      {/* 输入 */}
      <div className="space-y-2">
        <textarea
          value={sms}
          onChange={(e) => setSms(e.target.value)}
          placeholder="粘贴法院短信全文,例:【XX区人民法院】…向您发送了(2024)苏0000民初0000号案件相关文书…点击链接查阅：https://zxfw.court.gov.cn/zxfw/#/…?qdbh=…&sdbh=…&sdsin=…"
          rows={4}
          className="w-full resize-y rounded-md border border-border bg-background px-3 py-2 text-sm placeholder:text-muted-foreground/50 focus:border-foreground focus:outline-none focus:ring-1 focus:ring-foreground/20"
        />
        <button
          type="button"
          onClick={handleParse}
          disabled={loading || !sms.trim()}
          className="inline-flex items-center gap-1.5 rounded-md bg-foreground px-3.5 py-2 text-xs font-medium text-background transition-opacity hover:opacity-90 disabled:opacity-40"
        >
          {loading ? <Loader2 className="size-3.5 animate-spin" /> : <Gavel className="size-3.5" />}
          解析短信
        </button>
      </div>

      {/* 预览结果 */}
      {preview && (
        <div className="space-y-3 rounded-lg border border-border bg-card p-4">
          {!preview.has_link ? (
            <div className="flex items-start gap-2 text-xs text-amber-700 dark:text-amber-300">
              <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
              <span>{preview.note}</span>
            </div>
          ) : (
            <>
              <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-xs">
                <span className="text-muted-foreground">法院</span>
                <span className="text-foreground">{preview.court ?? "—"}</span>
                <span className="text-muted-foreground">案号</span>
                <span className="font-mono text-foreground">{preview.case_no ?? "—"}</span>
                <span className="text-muted-foreground">文书</span>
                <span className="text-foreground">
                  {preview.docs.length > 0 ? (
                    <span className="flex flex-wrap gap-1.5">
                      {preview.docs.map((d, i) => (
                        <span
                          key={i}
                          className="inline-flex items-center gap-1 rounded bg-muted px-1.5 py-0.5"
                        >
                          <FileText className="size-3 text-muted-foreground" />
                          {d.name}
                        </span>
                      ))}
                    </span>
                  ) : (
                    "(未拉到文书,链接可能已失效)"
                  )}
                </span>
              </div>

              {/* 归档目标案件 */}
              <div className="space-y-1.5 border-t border-border pt-3">
                {preview.matched_case_id ? (
                  <div className="flex items-center gap-1.5 rounded-md border border-sky-200 bg-sky-50 px-2.5 py-1.5 text-xs text-sky-800 dark:border-sky-800/50 dark:bg-sky-950/30 dark:text-sky-200">
                    <CheckCircle2 className="size-3.5 shrink-0" />
                    自动匹配到案件:<strong>{preview.matched_case_name}</strong>
                  </div>
                ) : (
                  <div className="space-y-1">
                    {preview.name_matches.length > 0 ? (
                      <p className="flex items-start gap-1.5 rounded-md border border-sky-200 bg-sky-50 px-2.5 py-1.5 text-xs text-sky-800 dark:border-sky-800/50 dark:bg-sky-950/30 dark:text-sky-200">
                        <CheckCircle2 className="mt-0.5 size-3.5 shrink-0" />
                        <span>
                          按当事人姓名
                          <strong>「{preview.name_matches[0].matched_names.join("、")}」</strong>
                          匹配到:<strong>{preview.name_matches[0].case_name}</strong>
                          ,已预选,请确认无误后归档(不对可在下拉里换)。
                        </span>
                      </p>
                    ) : (
                      <p className="flex items-center gap-1.5 text-xs text-amber-700 dark:text-amber-300">
                        <AlertTriangle className="size-3.5 shrink-0" />
                        {preview.note ?? "未匹配到案件,请手动选择"}
                      </p>
                    )}
                    <select
                      value={pickedCaseId}
                      onChange={(e) => setPickedCaseId(e.target.value)}
                      className="w-full rounded-md border border-border bg-background px-2.5 py-1.5 text-xs focus:border-foreground focus:outline-none"
                    >
                      <option value="">— 选择要归档到的案件 —</option>
                      {cases.map((c) => (
                        <option key={c.id} value={c.id}>
                          {caseLabel(c)}
                        </option>
                      ))}
                    </select>
                  </div>
                )}

                <button
                  type="button"
                  onClick={handleIngest}
                  disabled={ingesting || !pickedCaseId || preview.docs.length === 0}
                  className="inline-flex items-center gap-1.5 rounded-md bg-sky-600 px-3.5 py-2 text-xs font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-40"
                >
                  {ingesting ? (
                    <Loader2 className="size-3.5 animate-spin" />
                  ) : (
                    <FileText className="size-3.5" />
                  )}
                  下载并归档到案件
                </button>
                <button
                  type="button"
                  onClick={handleDownloadToFolder}
                  disabled={downloadingLocal || preview.docs.length === 0}
                  className="ml-2 inline-flex items-center gap-1.5 rounded-md border border-border bg-background px-3.5 py-2 text-xs font-medium text-foreground transition-colors hover:bg-muted disabled:opacity-40"
                >
                  {downloadingLocal ? (
                    <Loader2 className="size-3.5 animate-spin" />
                  ) : (
                    <FolderOpen className="size-3.5" />
                  )}
                  下载到本地文件夹
                </button>
              </div>
            </>
          )}
        </div>
      )}

      {/* 导入结果 */}
      {done && (
        <div className="rounded-lg border border-emerald-200 bg-emerald-50 p-4 text-xs text-emerald-800 dark:border-emerald-800/50 dark:bg-emerald-950/30 dark:text-emerald-200">
          <p className="flex items-center gap-1.5 font-medium">
            {done.mode === "folder" ? (
              <Download className="size-3.5" />
            ) : (
              <CheckCircle2 className="size-3.5" />
            )}
            {done.mode === "folder"
              ? `已下载 ${done.files.length} 份文书到本地文件夹`
              : `已归档 ${done.files.length} 份文书,后台正在抽取`}
          </p>
          {done.mode === "folder" && (
            <p className="mt-1.5 break-all text-emerald-700/80 dark:text-emerald-300/70">
              保存位置:{done.folder}
            </p>
          )}
          {done.files.length > 0 && (
            <ul className="mt-1.5 list-inside list-disc space-y-0.5 pl-1">
              {done.files.map((f, i) => (
                <li key={i}>{f}</li>
              ))}
            </ul>
          )}
          {done.skipped.length > 0 && (
            <p className="mt-1.5 text-amber-700 dark:text-amber-300">
              跳过 {done.skipped.length} 份:{done.skipped.join("；")}
            </p>
          )}
          {done.mode === "case" && (
            <p className="mt-2 text-emerald-700/80 dark:text-emerald-300/70">
              打开对应案件即可看到新文书与抽取后的内容(开庭传票/判决书会更新到看板)。
            </p>
          )}
        </div>
      )}
    </div>
  );
}

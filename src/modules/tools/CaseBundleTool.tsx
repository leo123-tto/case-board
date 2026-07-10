/**
 * 案件资料包(双人办案材料合并)· 工具页。
 *
 * 复杂案件双人办案:两位律师各自电脑里有同一案件的材料,可能重复、也可能各有不同。
 * 律师 A「导出资料包」(.zip),律师 B「导入合并」进自己的同一案件 —— 取材料并集、
 * 按内容 SHA-256 去重、永不覆盖对方已有数据、永不删除,天然无冲突。
 *
 * 后端命令 export_case_bundle / preview_case_bundle / merge_case_bundle(case_bundle 模块),
 * 前端只负责选案件 / 选文件 / 调命令 / 展示结果。错误真错透传。
 */

import { useEffect, useMemo, useState } from "react";
import { open as dialogOpen, save as dialogSave } from "@tauri-apps/plugin-dialog";
import { Combine, Download, Loader2, Upload } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  exportCaseBundle,
  listCases,
  mergeCaseBundle,
  previewCaseBundle,
  type CaseBundlePreview,
  type CaseMergeReport,
} from "@/lib/api";
import type { Case } from "@/lib/types";

function formatError(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

const NEW_CASE = "__new__";

function caseLabel(c: Case): string {
  const no = c.agg_case_no || c.case_no;
  return no ? `${c.name}(${no})` : c.name;
}

export function CaseBundleTool() {
  const [cases, setCases] = useState<Case[]>([]);
  const [exportCaseId, setExportCaseId] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [busyMsg, setBusyMsg] = useState("");
  const [error, setError] = useState<string | null>(null);

  // 导入合并:选好文件后先 preview,确认目标再合并。
  const [pendingZip, setPendingZip] = useState<string | null>(null);
  const [preview, setPreview] = useState<CaseBundlePreview | null>(null);
  const [mergeTarget, setMergeTarget] = useState<string>(NEW_CASE);
  const [report, setReport] = useState<CaseMergeReport | null>(null);

  async function refreshCases() {
    try {
      const list = await listCases();
      setCases(list);
      if (!exportCaseId && list.length > 0) setExportCaseId(list[0].id);
    } catch (e) {
      setError(formatError(e));
    }
  }

  useEffect(() => {
    void refreshCases();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const exportCase = useMemo(
    () => cases.find((c) => c.id === exportCaseId) ?? null,
    [cases, exportCaseId],
  );

  async function handleExport() {
    if (!exportCase) return;
    setError(null);
    try {
      const safeName = exportCase.name.replace(/[\\/:*?"<>|]/g, "_").slice(0, 40);
      const picked = await dialogSave({
        defaultPath: `案件资料包_${safeName}.zip`,
        filters: [{ name: "案件资料包", extensions: ["zip"] }],
      });
      if (typeof picked !== "string" || !picked.trim()) return;
      setBusy(true);
      setBusyMsg("导出中…");
      const count = await exportCaseBundle(exportCase.id, picked);
      setBusyMsg(`✓ 已导出 ${count} 个材料文件,发给同事让他「导入合并」即可。`);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
      window.setTimeout(() => setBusyMsg(""), 8000);
    }
  }

  async function handlePickBundle() {
    setError(null);
    setReport(null);
    setPreview(null);
    setPendingZip(null);
    try {
      const picked = await dialogOpen({
        directory: false,
        multiple: false,
        filters: [{ name: "案件资料包", extensions: ["zip"] }],
      });
      if (typeof picked !== "string" || !picked.trim()) return;
      setBusy(true);
      setBusyMsg("读取资料包…");
      const p = await previewCaseBundle(picked);
      setPendingZip(picked);
      setPreview(p);
      // 后端按案号匹配到同一案件 → 默认合并进它;否则默认新建。
      setMergeTarget(p.suggestedCaseId ?? NEW_CASE);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
      setBusyMsg("");
    }
  }

  async function handleMerge() {
    if (!pendingZip) return;
    setError(null);
    try {
      setBusy(true);
      setBusyMsg("合并中…");
      const r = await mergeCaseBundle(
        pendingZip,
        mergeTarget === NEW_CASE ? null : mergeTarget,
      );
      setReport(r);
      setPreview(null);
      setPendingZip(null);
      setBusyMsg(
        `✓ 合并完成:新增 ${r.added} · 去重 ${r.deduped}${r.skipped ? ` · 跳过 ${r.skipped}` : ""}。合并的材料正在后台抽取,稍后建议在案件详情点「重新分析」。`,
      );
      await refreshCases();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
      window.setTimeout(() => setBusyMsg(""), 10000);
    }
  }

  const selectCls =
    "w-full rounded-md border border-border bg-background px-3 py-1.5 text-sm outline-none focus:border-sky-400";

  return (
    <div className="space-y-5">
      <div className="rounded-lg border border-border bg-card/50 p-4">
        <p className="text-sm leading-relaxed text-foreground">
          双人办案时,你和合办律师电脑里各有同一案件的部分材料。把案件「导出资料包」发给对方,
          对方「导入合并」进同一案件 —— 系统按文件内容自动去重、把双方材料取并集。
        </p>
        <p className="mt-2 text-xs text-muted-foreground">
          <strong className="text-foreground/80">不冲突保证</strong>:合并只新增、不删除、
          不覆盖你已有的数据(案号/概括等空白字段才会补)。重复材料按内容自动去重。合并后的
          材料会自动排队抽取,建议在案件详情点「重新分析」生成统一画像。
        </p>
      </div>

      {/* 导出 */}
      <section className="space-y-2 rounded-lg border border-border bg-background p-4">
        <div className="flex items-center gap-2">
          <Download className="size-4 text-foreground/70" />
          <h3 className="text-sm font-medium text-foreground">导出案件资料包</h3>
        </div>
        <p className="text-xs text-muted-foreground">选一个案件,打包它的材料发给合办律师。</p>
        <select
          value={exportCaseId}
          onChange={(e) => setExportCaseId(e.target.value)}
          disabled={busy || cases.length === 0}
          className={selectCls}
        >
          {cases.length === 0 && <option value="">(暂无案件)</option>}
          {cases.map((c) => (
            <option key={c.id} value={c.id}>
              {caseLabel(c)}
            </option>
          ))}
        </select>
        <Button size="sm" onClick={handleExport} disabled={!exportCase || busy}>
          {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Download className="size-3.5" />}
          导出资料包(.zip)
        </Button>
      </section>

      {/* 导入合并 */}
      <section className="space-y-3 rounded-lg border border-border bg-background p-4">
        <div className="flex items-center gap-2">
          <Upload className="size-4 text-foreground/70" />
          <h3 className="text-sm font-medium text-foreground">导入合并资料包</h3>
        </div>
        <p className="text-xs text-muted-foreground">
          选合办律师发来的 zip,合并进你的同一案件(系统按案号自动建议目标),或新建。
        </p>
        <Button size="sm" variant="outline" onClick={handlePickBundle} disabled={busy}>
          {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Upload className="size-3.5" />}
          选择资料包…
        </Button>

        {preview && (
          <div className="space-y-3 rounded-md border border-sky-200 bg-sky-50/60 p-3 dark:border-sky-900/50 dark:bg-sky-950/20">
            <div className="text-xs text-foreground">
              <div>
                <span className="text-muted-foreground">案件:</span>
                <strong>{preview.name}</strong>
                {preview.caseNo && <span className="ml-1 text-muted-foreground">· {preview.caseNo}</span>}
              </div>
              {preview.parties && (
                <div className="mt-0.5 text-muted-foreground">{preview.parties}</div>
              )}
              <div className="mt-0.5 text-muted-foreground">包内 {preview.fileCount} 个材料文件</div>
            </div>

            <div className="space-y-1">
              <label className="text-xs text-muted-foreground">合并到</label>
              <select
                value={mergeTarget}
                onChange={(e) => setMergeTarget(e.target.value)}
                disabled={busy}
                className={selectCls}
              >
                <option value={NEW_CASE}>➕ 新建为一个独立案件</option>
                {cases.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.id === preview.suggestedCaseId ? "✓ 建议 · " : ""}
                    {caseLabel(c)}
                  </option>
                ))}
              </select>
              {preview.suggestedCaseId && mergeTarget === preview.suggestedCaseId && (
                <p className="text-caption text-sky-700 dark:text-sky-400">
                  按案号匹配到本地同一案件,默认合并进它。
                </p>
              )}
            </div>

            <Button size="sm" onClick={handleMerge} disabled={busy}>
              {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Combine className="size-3.5" />}
              合并
            </Button>
          </div>
        )}

        {report && (
          <div className="space-y-1 rounded-md border border-emerald-200 bg-emerald-50/60 p-3 text-xs dark:border-emerald-900/50 dark:bg-emerald-950/20">
            <p className="text-foreground">
              {report.createdNew ? "已新建案件「" : "已合并进「"}
              <strong>{report.targetCaseName}</strong>」:新增{" "}
              <strong className="text-emerald-700 dark:text-emerald-400">{report.added}</strong> · 去重{" "}
              {report.deduped}
              {report.skipped > 0 && <span> · 跳过 {report.skipped}</span>}
            </p>
            {report.filledFields.length > 0 && (
              <p className="text-muted-foreground">补充字段:{report.filledFields.join("、")}</p>
            )}
            {report.skipReasons.length > 0 && (
              <details className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-amber-900">
                <summary className="cursor-pointer font-medium">
                  查看 {report.skipReasons.length} 条跳过原因
                </summary>
                <ul className="mt-1 list-disc space-y-1 pl-5">
                  {report.skipReasons.map((reason, index) => (
                    <li key={`${index}:${reason}`}>{reason}</li>
                  ))}
                </ul>
              </details>
            )}
            <p className="text-muted-foreground">
              合并的材料正在后台抽取,完成后建议在案件详情点「重新分析」生成统一画像。
            </p>
          </div>
        )}
      </section>

      {busyMsg && <p className="text-xs text-muted-foreground">{busyMsg}</p>}
      {error && (
        <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
          <p className="font-medium">出错了</p>
          <p className="mt-0.5 break-all font-mono">{error}</p>
        </div>
      )}
    </div>
  );
}

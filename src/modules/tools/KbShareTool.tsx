/**
 * 本地知识库共享工具(工具页 · 团队协作 / 省积分)。
 *
 * 复用「设置 → 本地法律知识库」的同一套后端命令(export_kb_to_zip / import_kb_from_zip),
 * 但在工具页做成更直白的「导出资料包 / 导入资料包」入口,面向团队互通。
 *
 * 共享范围(第一版):**只打包元典缓存**(raw/yuandian-cache/ —— 花积分查过的公司/法规/案例)。
 * 这正是省积分核心:同事导入后不用重复花积分查同样的东西。
 *
 * 注意:导出/导入的实际逻辑、查重、冲突合并都在后端 local_kb::share,前端只负责
 * 选文件路径 + 调命令 + 展示结果。后端未绑定 KB 时命令会真错透传。
 */

import { useEffect, useState } from "react";
import { open as dialogOpen, save as dialogSave } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  Database,
  Download,
  Loader2,
  Upload,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  detectKbStatus,
  exportKbToZip,
  importKbFromZip,
  type KbConflictStrategy,
  type KbExportResult,
  type KbImportResult,
  type KbStatus,
} from "@/lib/api";
import { todayIsoLocal } from "@/lib/date";
import { formatBytes } from "@/lib/format";

/** 错误透传(跟 CaseChatPanel/Settings 一致:真错原文,不替换固定文案)。 */
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

/** 三种冲突策略的人话说明(给律师看,不暴露 skip/overwrite_older 等英文枚举)。 */
const STRATEGIES: {
  value: KbConflictStrategy;
  label: string;
  hint: string;
}[] = [
  {
    value: "overwrite_older",
    label: "智能合并(推荐)",
    hint: "对方更新的覆盖我方旧的,我方更新的保留",
  },
  {
    value: "skip",
    label: "只新增",
    hint: "遇到我方已有的一律跳过,只补缺",
  },
  {
    value: "always_overwrite",
    label: "全部覆盖",
    hint: "一律用对方的覆盖我方同名文件",
  },
];

export function KbShareTool() {
  const [status, setStatus] = useState<KbStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [busyMsg, setBusyMsg] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [strategy, setStrategy] = useState<KbConflictStrategy>("overwrite_older");
  const [exportResult, setExportResult] = useState<KbExportResult | null>(null);
  const [importResult, setImportResult] = useState<KbImportResult | null>(null);

  async function refreshStatus() {
    try {
      setStatus(await detectKbStatus());
    } catch (e) {
      setError(formatError(e));
    }
  }

  useEffect(() => {
    void refreshStatus();
  }, []);

  const bound = status?.state === "bound";

  async function handleExport() {
    setError(null);
    setExportResult(null);
    try {
      const today = todayIsoLocal();
      const picked = await dialogSave({
        defaultPath: `caseboard-kb-share-${today}.zip`,
        filters: [{ name: "知识库资料包", extensions: ["zip"] }],
      });
      if (typeof picked !== "string" || !picked.trim()) return;
      setBusy(true);
      setBusyMsg("导出中…");
      const r = await exportKbToZip(picked);
      setExportResult(r);
      setBusyMsg(`导出完成 · ${r.total_items} 条 · ${formatBytes(r.total_size_bytes)}`);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
      window.setTimeout(() => setBusyMsg(""), 6000);
    }
  }

  async function handleImport() {
    setError(null);
    setImportResult(null);
    try {
      const picked = await dialogOpen({
        directory: false,
        multiple: false,
        filters: [{ name: "知识库资料包", extensions: ["zip"] }],
      });
      if (typeof picked !== "string" || !picked.trim()) return;
      setBusy(true);
      setBusyMsg("导入中…");
      const r = await importKbFromZip(picked, strategy);
      setImportResult(r);
      setBusyMsg(
        `导入完成:新增 ${r.added} / 跳过 ${r.skipped} / 覆盖 ${r.overwritten}${r.failed ? ` / 失败 ${r.failed}` : ""}`,
      );
      await refreshStatus();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
      window.setTimeout(() => setBusyMsg(""), 8000);
    }
  }

  return (
    <div className="space-y-5">
      {/* 说明 */}
      <div className="rounded-lg border border-border bg-card/50 p-4">
        <p className="text-sm leading-relaxed text-foreground">
          把你花积分查过的元典结果(公司 / 法规 / 案例)打包成 zip 发给同事,或导入同事的包。
          大家就不用重复花积分查同样的东西 —— 这是团队互相省积分的方式。
        </p>
        <p className="mt-2 text-xs text-muted-foreground">
          本版只共享<strong className="text-foreground/80">元典查询缓存</strong>
          (raw/yuandian-cache/),不含你的笔记 / 专题 / 报告;导入会自动查重,不会重复堆叠。
        </p>
      </div>

      {/* KB 状态 */}
      {status === null ? (
        <p className="text-xs text-muted-foreground">
          <Loader2 className="mr-1 inline size-3 animate-spin" />
          检测知识库状态…
        </p>
      ) : bound ? (
        <div className="flex items-center gap-2 rounded-md border border-border bg-background p-3 text-xs">
          <Database className="size-4 shrink-0 text-emerald-600" />
          <span className="truncate">
            已绑定 <span className="font-mono">{status.root}</span> · 当前缓存{" "}
            <strong className="text-foreground">{status.cache_count}</strong> 条
          </span>
        </div>
      ) : (
        <div className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/5 p-3 text-xs">
          <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-600" />
          <span className="text-foreground">
            还没绑定本地知识库,无法导入导出。请先到
            <strong>【设置 → 本地法律知识库】</strong>新建或绑定一个知识库目录。
          </span>
        </div>
      )}

      {/* 导出 */}
      <section className="space-y-2 rounded-lg border border-border bg-background p-4">
        <div className="flex items-center gap-2">
          <Download className="size-4 text-foreground/70" />
          <h3 className="text-sm font-medium text-foreground">导出资料包</h3>
        </div>
        <p className="text-xs text-muted-foreground">
          把当前知识库的元典缓存打成一个 zip,发给同事即可。
        </p>
        <Button size="sm" onClick={handleExport} disabled={!bound || busy}>
          {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Download className="size-3.5" />}
          导出资料包(.zip)
        </Button>
        {exportResult && (
          <p className="text-xs text-emerald-700 dark:text-emerald-400">
            ✓ 已导出 {exportResult.total_items} 条 ·{" "}
            {formatBytes(exportResult.total_size_bytes)}
            <br />
            <span className="font-mono text-muted-foreground">
              {exportResult.output_path}
            </span>
          </p>
        )}
      </section>

      {/* 导入 */}
      <section className="space-y-3 rounded-lg border border-border bg-background p-4">
        <div className="flex items-center gap-2">
          <Upload className="size-4 text-foreground/70" />
          <h3 className="text-sm font-medium text-foreground">导入资料包</h3>
        </div>
        <p className="text-xs text-muted-foreground">
          选同事发来的 zip 合并进你的知识库。遇到重复内容时如何处理:
        </p>
        {/* 冲突策略 */}
        <div className="flex flex-wrap gap-1.5">
          {STRATEGIES.map((s) => (
            <button
              key={s.value}
              type="button"
              onClick={() => setStrategy(s.value)}
              disabled={busy}
              title={s.hint}
              className={
                strategy === s.value
                  ? "rounded-md border border-foreground bg-foreground px-2.5 py-1 text-xs text-background transition-colors disabled:opacity-40"
                  : "rounded-md border border-border bg-background px-2.5 py-1 text-xs text-foreground transition-colors hover:bg-accent disabled:opacity-40"
              }
            >
              {s.label}
            </button>
          ))}
        </div>
        <p className="text-label text-muted-foreground">
          {STRATEGIES.find((s) => s.value === strategy)?.hint}
        </p>
        <Button
          size="sm"
          variant="outline"
          onClick={handleImport}
          disabled={!bound || busy}
        >
          {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Upload className="size-3.5" />}
          选择资料包导入…
        </Button>
        {importResult && (
          <div className="space-y-1 text-xs">
            <p className="text-foreground">
              新增 <strong className="text-emerald-700 dark:text-emerald-400">{importResult.added}</strong> · 跳过{" "}
              {importResult.skipped} · 覆盖 {importResult.overwritten}
              {importResult.failed > 0 && (
                <span className="text-destructive"> · 失败 {importResult.failed}</span>
              )}
              <span className="text-muted-foreground"> (包内共 {importResult.total_in_zip} 条)</span>
            </p>
            {importResult.conflicts.length > 0 && (
              <details className="text-muted-foreground">
                <summary className="cursor-pointer">
                  查看 {importResult.conflicts.length} 条明细
                </summary>
                <ul className="mt-1 space-y-0.5 pl-2">
                  {importResult.conflicts.slice(0, 50).map((c, i) => (
                    <li key={i} className="font-mono text-caption">
                      [{c.action}] {c.path} — {c.reason}
                    </li>
                  ))}
                </ul>
              </details>
            )}
          </div>
        )}
      </section>

      {/* 进度 / 错误 */}
      {busyMsg && (
        <p className="text-xs text-muted-foreground">{busyMsg}</p>
      )}
      {error && (
        <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
          <p className="font-medium">出错了</p>
          <p className="mt-0.5 break-all font-mono">{error}</p>
        </div>
      )}
    </div>
  );
}

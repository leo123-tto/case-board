/**
 * 多案件拆分确认弹窗(2026-06-04 · Phase 1;2026-06-19 加"展开看文件 + 杂项可勾回")。
 *
 * 拖入文件夹后,后端 `plan_import_folder` 检测到 ≥2 个候选案件时弹出本框。
 * 检测只给**建议**,最终以用户确认为准 —— 可勾选/取消、改案件名、展开看里面有哪些材料,
 * 或一键「合并成 1 个案件」(保底)。
 * 「已忽略·杂项」(后续/待整理…)默认不导入,但可展开核对 + 手动勾回作为案件导入;
 * 「已忽略·空目录」无文档,只列出不可勾。
 * 详见 docs/导入拆案体验-执行清单.md、docs/提案-多案件文件夹识别-2026-06-04.md。
 */
import { useState } from "react";
import {
  Layers,
  FileText,
  FolderOpen,
  AlertTriangle,
  ChevronRight,
  ChevronDown,
} from "lucide-react";
import type { ImportPlan } from "../lib/types";

/** 一次拆分导入的案件数上限(对齐后端 `MAX_SPLIT_IMPORT_CASES`)。 */
const MAX_CASES = 8;

/** 勾选材料总数超过这个数就提醒免费 OCR 额度风险(经验值:几百份起就容易额度不够)。 */
const DOC_WARN_THRESHOLD = 300;

function basename(p: string): string {
  const parts = p.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

/** 剥目录名开头的 `NN_` / `NN-` / `NN.` / `NN ` 序号前缀(对齐后端 strip_num_prefix)。 */
function stripNumPrefix(name: string): string {
  const trimmed = name.replace(/^[\d_\-.、 ]+/, "");
  return trimmed || name;
}

/** 展开后的文件清单(相对路径;超上限时尾部提示还有多少)。 */
function FileList({ files, docCount }: { files: string[]; docCount: number }) {
  if (files.length === 0) {
    return <p className="px-2 py-1.5 text-xs text-stone-400">(无可识别的材料)</p>;
  }
  const more = docCount - files.length;
  return (
    <ul className="max-h-44 space-y-0.5 overflow-y-auto px-2 py-1.5">
      {files.map((f) => (
        <li key={f} className="flex items-center gap-1.5 truncate text-xs text-stone-500">
          <FileText className="size-3 shrink-0 text-stone-300" />
          <span className="truncate">{f}</span>
        </li>
      ))}
      {more > 0 && (
        <li className="pl-5 text-xs text-stone-400">… 还有 {more} 个文件</li>
      )}
    </ul>
  );
}

export function SplitImportDialog({
  plan,
  busy,
  onConfirm,
  onMergeAll,
  onCancel,
}: {
  plan: ImportPlan;
  busy: boolean;
  onConfirm: (cases: { dir: string; name: string }[], sharedDirs: string[]) => void;
  onMergeAll: () => void;
  onCancel: () => void;
}) {
  const [rows, setRows] = useState(
    plan.cases.map((c) => ({
      dir: c.dir,
      name: c.suggested_name,
      // 后端按目录名给默认勾选:命中「证件/宣传/模板…」等非案件资料词的默认不勾选(仍可手动勾上)
      include: c.default_selected,
      docCount: c.doc_count,
      hasStage: c.has_stage_subdirs,
      files: c.files,
      // 疑似非案件资料(默认未勾选的原因),用于行内提示标记
      suspected: !c.default_selected,
    })),
  );

  // 「已忽略·杂项」(有文档的忽略目录)→ 可勾回作为案件导入,默认不勾。
  const miscIgnored = plan.ignored.filter((g) => g.files.length > 0);
  // 「已忽略·空目录」(无文档)→ 只列出,不可勾。
  const emptyIgnored = plan.ignored.filter((g) => g.files.length === 0);
  const [misc, setMisc] = useState(
    miscIgnored.map((g) => ({
      dir: g.path,
      name: stripNumPrefix(basename(g.path)),
      include: false,
      docCount: g.files.length, // 杂项目录的清单已是其全部文档(同口径),够准
      files: g.files,
    })),
  );

  // 展开看文件:记当前展开的目录 dir 集合(候选 + 杂项共用)。
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const toggleExpand = (dir: string) =>
    setExpanded((s) => {
      const next = new Set(s);
      if (next.has(dir)) next.delete(dir);
      else next.add(dir);
      return next;
    });

  // 已选 = 勾选的候选案件 + 勾回的杂项目录(共同受 MAX_CASES 约束)。
  const selectedCases = rows.filter((r) => r.include);
  const selectedMisc = misc.filter((m) => m.include);
  const selectedTotal = selectedCases.length + selectedMisc.length;
  // 免费 OCR 额度是按量计的总量,真正决定跑不跑得完的是材料份数,不是案件个数。
  const selectedDocTotal =
    selectedCases.reduce((sum, r) => sum + r.docCount, 0) +
    selectedMisc.reduce((sum, m) => sum + m.docCount, 0);

  const setName = (i: number, name: string) =>
    setRows((rs) => rs.map((r, j) => (j === i ? { ...r, name } : r)));
  const toggle = (i: number) =>
    setRows((rs) => rs.map((r, j) => (j === i ? { ...r, include: !r.include } : r)));
  const toggleMisc = (i: number) =>
    setMisc((ms) => ms.map((m, j) => (j === i ? { ...m, include: !m.include } : m)));

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/40 p-4 backdrop-blur-sm">
      <div className="flex max-h-[88vh] w-full max-w-2xl flex-col overflow-hidden rounded-xl bg-white shadow-2xl">
        {/* 头 */}
        <div className="flex items-start gap-3 border-b border-stone-200 px-6 py-4">
          <Layers className="mt-0.5 size-6 shrink-0 text-sky-500" />
          <div className="min-w-0">
            <h2 className="text-lg font-semibold text-stone-800">
              检测到 {plan.cases.length} 个目录 · 已勾选 {selectedTotal} 个案件
            </h2>
            <p className="mt-0.5 text-sm text-stone-500">
              这个文件夹像是多个案件混在一起。点目录名前的箭头可
              <span className="text-stone-600">展开看里面有哪些材料</span>;
              <span className="text-stone-600">证件 / 宣传 / 模板</span>
              等疑似资料已默认不选,需要的话手动勾上;或合并成一个案件。
            </p>
          </div>
        </div>

        {/* 体 */}
        <div className="flex-1 overflow-y-auto px-6 py-4">
          {/* 案件多 / 材料多 → 提醒免费 OCR 额度(总量)与全案分析花费,建议分批或先合并 */}
          {(selectedTotal > 3 || selectedDocTotal > DOC_WARN_THRESHOLD) && (
            <div className="mb-4 flex items-start gap-2 rounded-lg border border-amber-300 bg-amber-50 px-3 py-2.5 text-sm text-amber-800">
              <AlertTriangle className="mt-0.5 size-4 shrink-0" />
              <span>
                <strong>
                  已选 {selectedTotal} 个案件、共 {selectedDocTotal} 份材料。
                </strong>
                案件会排队逐个处理(不会并发打爆识别服务),但<strong>云端 OCR
                免费额度是按量算的</strong>:材料一多,额度当天就可能用光,后面的材料会大批识别失败。
                每个案件还会各跑一次全案 AI 分析(约 1-3 分钟,消耗 DeepSeek 余额)。
                建议分批导入,或点「合并成 1 个案件」先入库、之后按需重识别。
              </span>
            </div>
          )}

          {plan.root_already_imported && (
            <div className="mb-4 flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-700">
              <AlertTriangle className="mt-0.5 size-4 shrink-0" />
              <span>
                这个文件夹之前已作为「一个案件」导入过。点「拆成 N 个案件导入」会用拆分结果**替换掉**那个旧的整体案件(旧案及其文档会被删除)。
              </span>
            </div>
          )}

          {/* 候选案件 */}
          <div className="space-y-2">
            {rows.map((r, i) => {
              const open = expanded.has(r.dir);
              return (
                <div
                  key={r.dir}
                  className={`rounded-lg border transition ${
                    r.include
                      ? "border-sky-200 bg-sky-50/60"
                      : "border-stone-200 bg-stone-50 opacity-70"
                  }`}
                >
                  <div className="flex items-center gap-2 px-3 py-2.5">
                    <button
                      type="button"
                      onClick={() => toggleExpand(r.dir)}
                      className="shrink-0 rounded p-0.5 text-stone-400 hover:bg-stone-200 hover:text-stone-600"
                      title={open ? "收起" : "展开看文件"}
                    >
                      {open ? (
                        <ChevronDown className="size-4" />
                      ) : (
                        <ChevronRight className="size-4" />
                      )}
                    </button>
                    <input
                      type="checkbox"
                      checked={r.include}
                      onChange={() => toggle(i)}
                      className="size-4 accent-sky-500"
                    />
                    <FolderOpen className="size-4 shrink-0 text-stone-400" />
                    <input
                      value={r.name}
                      onChange={(e) => setName(i, e.target.value)}
                      className="min-w-0 flex-1 rounded border border-stone-200 bg-white px-2 py-1 text-sm text-stone-800 focus:border-sky-400 focus:outline-none"
                      placeholder="案件名"
                    />
                    <span className="flex shrink-0 items-center gap-1 text-xs text-stone-500">
                      <FileText className="size-3.5" />
                      {r.docCount}
                    </span>
                    {r.hasStage && (
                      <span className="shrink-0 rounded bg-sky-100 px-1.5 py-0.5 text-[11px] font-medium text-sky-600">
                        已分阶段
                      </span>
                    )}
                    {r.suspected && (
                      <span
                        className="shrink-0 rounded bg-amber-100 px-1.5 py-0.5 text-[11px] font-medium text-amber-600"
                        title="目录名像是证件/宣传/模板等非案件资料,已默认不选;确认是案件可手动勾上"
                      >
                        疑似资料
                      </span>
                    )}
                  </div>
                  {open && (
                    <div className="border-t border-stone-200/70 bg-white/60">
                      <FileList files={r.files} docCount={r.docCount} />
                    </div>
                  )}
                </div>
              );
            })}
          </div>

          {/* 共用材料 */}
          {plan.shared_dirs.length > 0 && (
            <div className="mt-4 rounded-lg border border-stone-200 bg-stone-50 px-3 py-2.5">
              <p className="text-xs font-medium text-stone-600">
                共用材料(挂到每个案件)
              </p>
              <ul className="mt-1 space-y-0.5">
                {plan.shared_dirs.map((d) => (
                  <li key={d} className="truncate text-sm text-stone-500">
                    · {basename(d)}
                  </li>
                ))}
              </ul>
              <p className="mt-1.5 text-[11px] text-stone-400">
                这些材料会同时挂到上面每一个案件,各案分析时都能用到。
              </p>
            </div>
          )}

          {/* 已忽略·杂项:默认不导入,可展开核对 + 勾回作为案件 */}
          {misc.length > 0 && (
            <div className="mt-4">
              <p className="mb-1.5 text-xs font-medium text-stone-500">
                已忽略 · 杂项 / 补充目录(默认不导入,确认要的话勾上即作为案件导入)
              </p>
              <div className="space-y-2">
                {misc.map((m, i) => {
                  const open = expanded.has(m.dir);
                  return (
                    <div
                      key={m.dir}
                      className={`rounded-lg border transition ${
                        m.include
                          ? "border-sky-200 bg-sky-50/60"
                          : "border-stone-200 bg-stone-50 opacity-70"
                      }`}
                    >
                      <div className="flex items-center gap-2 px-3 py-2.5">
                        <button
                          type="button"
                          onClick={() => toggleExpand(m.dir)}
                          className="shrink-0 rounded p-0.5 text-stone-400 hover:bg-stone-200 hover:text-stone-600"
                          title={open ? "收起" : "展开看文件"}
                        >
                          {open ? (
                            <ChevronDown className="size-4" />
                          ) : (
                            <ChevronRight className="size-4" />
                          )}
                        </button>
                        <input
                          type="checkbox"
                          checked={m.include}
                          onChange={() => toggleMisc(i)}
                          className="size-4 accent-sky-500"
                        />
                        <FolderOpen className="size-4 shrink-0 text-stone-400" />
                        <span className="min-w-0 flex-1 truncate text-sm text-stone-700">
                          {m.name}
                        </span>
                        <span className="flex shrink-0 items-center gap-1 text-xs text-stone-500">
                          <FileText className="size-3.5" />
                          {m.docCount}
                        </span>
                        <span className="shrink-0 rounded bg-stone-200 px-1.5 py-0.5 text-[11px] font-medium text-stone-500">
                          杂项
                        </span>
                      </div>
                      {open && (
                        <div className="border-t border-stone-200/70 bg-white/60">
                          <FileList files={m.files} docCount={m.docCount} />
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          {/* 已忽略·空目录:无文档,只列出 */}
          {emptyIgnored.length > 0 && (
            <p className="mt-3 text-xs text-stone-400">
              已忽略 · 空目录(无文档):
              {emptyIgnored.map((g) => basename(g.path)).join("、")}
            </p>
          )}
        </div>

        {/* 脚 */}
        <div className="flex items-center justify-between gap-2 border-t border-stone-200 bg-stone-50 px-6 py-3">
          <button
            onClick={onCancel}
            disabled={busy}
            className="rounded-lg px-3 py-2 text-sm text-stone-500 hover:bg-stone-200 disabled:opacity-50"
          >
            取消
          </button>
          <div className="flex gap-2">
            <button
              onClick={onMergeAll}
              disabled={busy}
              className="rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm text-stone-600 hover:bg-stone-100 disabled:opacity-50"
            >
              合并成 1 个案件
            </button>
            <button
              onClick={() =>
                onConfirm(
                  [
                    ...selectedCases.map((r) => ({ dir: r.dir, name: r.name })),
                    ...selectedMisc.map((m) => ({ dir: m.dir, name: m.name })),
                  ],
                  plan.shared_dirs,
                )
              }
              disabled={busy || selectedTotal === 0 || selectedTotal > MAX_CASES}
              title={
                selectedTotal > MAX_CASES
                  ? `一次最多导入 ${MAX_CASES} 个案件,请取消勾选到 ${MAX_CASES} 个以内(或点「合并成 1 个案件」)`
                  : undefined
              }
              className="rounded-lg bg-sky-500 px-4 py-2 text-sm font-medium text-white hover:bg-sky-600 disabled:opacity-50"
            >
              {busy
                ? "导入中…"
                : selectedTotal > MAX_CASES
                  ? `最多 ${MAX_CASES} 个(已选 ${selectedTotal})`
                  : `拆成 ${selectedTotal} 个案件导入`}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

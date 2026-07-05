/**
 * 反馈按钮(右下角悬浮)+ 弹窗(2026-05-24 e · 作者拍板 MD 文件方案)。
 *
 * 流程:
 *   1. 按钮悬浮在右下,常驻不打扰
 *   2. 点开 → 弹窗显示「自动收集的诊断信息(可折叠)+ 输入框」
 *   3. 用户填描述 → 点「生成反馈文件」
 *   4. Rust 写文件到桌面或 Documents/CaseBoard/feedback 兜底目录
 *   5. Toast 提示路径 + 「在文件管理器中显示」按钮
 *
 * 隐私铁律:
 *   - 自动信息**永不含**案件名 / 当事人 / 文档内容
 *   - 匿名 client_id(UUID 前 8 位)用于关联同人多次反馈,无标识用户身份
 */

import { useEffect, useState } from "react";
import {
  Check,
  ChevronDown,
  ChevronUp,
  Loader2,
  MessageCircle,
  Send,
  X,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { Chip } from "@/components/ui/chip";
import {
  collectFeedbackDiagnostic,
  type FeedbackDiagnostic,
  revealInFinder,
  saveFeedbackMd,
  uploadFeedbackReport,
} from "@/lib/api";
import { snapshotConsoleErrors } from "@/lib/console-tap";

/**
 * 反馈入口按钮。
 *
 * 2026-05-27 老板手测反馈:从右下角悬浮挪到右上角 ModuleTabs 行尾,跟
 * DeepSeekBalanceChip 同一排。这样:
 *   - 不会挡 CaseChatPanel 底部的「发送」按钮
 *   - 不需要监听 chat panel 折叠状态做避让(简化逻辑)
 *   - 位置稳定,跟其它顶部 chip 一致的设计语言
 *
 * 按钮本身是 inline 样式(不再 fixed),由 ModuleTabs rightSlot 渲染。
 * 弹窗仍是全屏 modal,跟以前一样。
 */
export function FeedbackButton() {
  const [open, setOpen] = useState(false);

  return (
    <>
      <Chip asChild size="lg" className="gap-1.5">
        <button
          type="button"
          onClick={() => setOpen(true)}
          title="反馈一个问题"
          aria-label="反馈"
        >
          <MessageCircle className="size-3.5" />
          反馈
        </button>
      </Chip>

      {open && <FeedbackModal onClose={() => setOpen(false)} />}
    </>
  );
}

/* ============================ 弹窗主体 ============================ */
function FeedbackModal({ onClose }: { onClose: () => void }) {
  const [diag, setDiag] = useState<FeedbackDiagnostic | null>(null);
  const [diagErr, setDiagErr] = useState<string | null>(null);
  const [description, setDescription] = useState("");
  const [showDiag, setShowDiag] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [uploaded, setUploaded] = useState(false);
  const [uploadErr, setUploadErr] = useState<string | null>(null);
  const [savedPath, setSavedPath] = useState<string | null>(null);

  // 启动时拉诊断(把已累积的 console 错误一起带过去)
  useEffect(() => {
    collectFeedbackDiagnostic(snapshotConsoleErrors())
      .then(setDiag)
      .catch((e) => setDiagErr(String(e)));
  }, []);

  // Esc 关闭
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !submitting) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, submitting]);

  const handleSaveLocal = async () => {
    if (!diag) return;
    setSubmitting(true);
    try {
      const path = await saveFeedbackMd(diag, description);
      setSavedPath(path);
    } catch (e) {
      alert(`生成反馈文件失败:${e}`);
    } finally {
      setSubmitting(false);
    }
  };

  const handleUpload = async () => {
    if (!diag) return;
    setUploading(true);
    setUploadErr(null);
    try {
      await uploadFeedbackReport(diag, description);
      setUploaded(true);
    } catch (e) {
      setUploadErr(String(e));
    } finally {
      setUploading(false);
    }
  };

  // ── 成功态:显示文件位置 + 文件管理器打开按钮 ──
  if (savedPath) {
    return (
      <SavedFeedbackPanel savedPath={savedPath} onClose={onClose} />
    );
  }

  // ── 主弹窗:诊断预览 + 输入框 + 提交 ──
  return (
    <ModalShell onClose={onClose} title="反馈一个问题">
      <div className="space-y-3">
        {/* 自动信息折叠区 */}
        {diag ? (
          <div className="rounded-md border border-border bg-muted/30">
            <button
              type="button"
              onClick={() => setShowDiag((v) => !v)}
              className="flex w-full items-center justify-between px-3 py-2 text-left text-xs text-muted-foreground hover:bg-muted/50"
            >
              <span>
                ▾ 自动收集的信息({Object.keys(diag).length} 项,不含案件名 /
                当事人 / 文档内容)
              </span>
              {showDiag ? (
                <ChevronUp className="size-3.5" />
              ) : (
                <ChevronDown className="size-3.5" />
              )}
            </button>
            {showDiag && <DiagnosticPreview diag={diag} />}
          </div>
        ) : diagErr ? (
          <div className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900">
            诊断信息收集失败:{diagErr}(不影响反馈,继续即可)
          </div>
        ) : (
          <div className="flex items-center gap-2 rounded-md border border-border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
            <Loader2 className="size-3 animate-spin" />
            正在收集诊断信息…
          </div>
        )}

        {/* 输入框 */}
        <div className="space-y-1.5">
          <label
            htmlFor="feedback-desc"
            className="block text-xs font-medium text-muted-foreground"
          >
            你想反馈什么?
          </label>
          <textarea
            id="feedback-desc"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="描述遇到的问题、bug、建议…(可以贴报错信息;或截图后单独发)"
            rows={6}
            className="w-full resize-none rounded-md border border-border bg-card px-3 py-2 text-sm outline-none focus:border-foreground focus:ring-1 focus:ring-foreground/20"
            autoFocus
          />
        </div>

        {/* 按钮行 */}
        {uploaded && (
          <div className="rounded-md border border-emerald-200 bg-emerald-50 px-3 py-2 text-xs text-emerald-900">
            已上传给维护者。维护者会在反馈后台查看和处理。
          </div>
        )}
        {uploadErr && (
          <div className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900">
            上传失败:{uploadErr}。可以先保存到本地后手工发送。
          </div>
        )}
        <div className="flex flex-wrap items-center justify-between gap-2 pt-2">
          <div className="flex flex-col gap-1">
            <span className="text-caption text-muted-foreground">
              ID:{diag?.client_id_short ?? "—"} · 匿名
            </span>
            <button
              type="button"
              onClick={handleSaveLocal}
              disabled={submitting || uploading || !diag}
              className="inline-flex items-center gap-1 text-left text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline disabled:cursor-not-allowed disabled:opacity-50"
            >
              {submitting && <Loader2 className="size-3 animate-spin" />}
              仅生成本地 MD
            </button>
          </div>
          <div className="flex flex-wrap justify-end gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={onClose}
              disabled={submitting || uploading}
            >
              {uploaded ? "完成" : "取消"}
            </Button>
            <Button
              size="sm"
              onClick={handleUpload}
              disabled={uploading || submitting || !diag || uploaded}
              className="bg-foreground text-background hover:bg-foreground/90"
            >
              {uploading ? (
                <Loader2 className="size-3 animate-spin" />
              ) : (
                <Send className="size-3.5" />
              )}
              {uploaded ? "已上传" : "上传给维护者"}
            </Button>
          </div>
        </div>
      </div>
    </ModalShell>
  );
}

/* ============================ 反馈生成后面板(含邮件发送) ============================ */
function SavedFeedbackPanel({
  savedPath,
  onClose,
}: {
  savedPath: string;
  onClose: () => void;
}) {
  return (
    <ModalShell onClose={onClose} title="反馈文件已生成">
      <div className="space-y-3">
        <div className="flex items-start gap-3 rounded-md border border-emerald-200 bg-emerald-50 px-4 py-3">
          <Check className="mt-0.5 size-4 shrink-0 text-emerald-700" />
          <div className="min-w-0 flex-1 text-sm">
            <p className="font-medium text-emerald-900">反馈文件已保存</p>
            <p className="mt-0.5 break-all font-mono text-label text-emerald-800/80">
              {savedPath}
            </p>
          </div>
        </div>

        <p className="text-xs text-muted-foreground">
          可以在文件管理器中显示后,通过你习惯的方式把 MD 发给维护者。
        </p>

        <div className="flex flex-wrap justify-end gap-2 pt-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => revealInFinder(savedPath)}
          >
            在文件管理器中显示
          </Button>
          <Button size="sm" variant="outline" onClick={onClose}>
            完成
          </Button>
        </div>
      </div>
    </ModalShell>
  );
}

/* ============================ 诊断预览 ============================ */
function DiagnosticPreview({ diag }: { diag: FeedbackDiagnostic }) {
  const s = diag.settings_snapshot;
  const sys = diag.system_info;
  const km = diag.kb_memory;
  return (
    <div className="space-y-1 border-t border-border bg-card/50 px-3 py-2.5 font-mono text-label text-foreground">
      <Row k="App 版本" v={diag.app_version} />
      <Row k="操作系统" v={diag.os_version} />
      <Row k="语言" v={diag.language} />
      <div className="my-1 h-px bg-border/50" />
      <Row k="LLM 后端" v={diag.llm_provider} />
      <Row k="OCR 后端" v={diag.ocr_provider} />
      <Row k="本机服务" v={diag.local_server_status} />
      {diag.deepseek_balance != null && (
        <Row k="DeepSeek 余额" v={`¥${diag.deepseek_balance.toFixed(2)}`} />
      )}
      <div className="my-1 h-px bg-border/50" />
      <Row k="案件数" v={String(diag.stats.cases_total)} />
      <Row
        k="文档数"
        v={`${diag.stats.documents_total}(done ${diag.stats.documents_done} / failed ${diag.stats.documents_failed} / pending ${diag.stats.documents_pending} / skipped ${diag.stats.documents_skipped})`}
      />
      <div className="my-1 h-px bg-border/50" />
      {/* Settings 脱敏快照(三态:已验证 ✓ / 未验证 ⚠ / 未填)
          2026-05-26 V0.1.11:老板补强反馈通道——key 状态要一眼能看出,
          避免出现"key 填了但没验证通过却以为没问题"的盲区 */}
      <KeyRow label="MinerU key" filled={s.mineru_api_key} verified={s.mineru_verified} />
      <KeyRow label="DeepSeek key" filled={s.deepseek_api_key} verified={s.deepseek_verified} />
      <KeyRow label="元典 key" filled={s.yuandian_api_key} verified={s.yuandian_verified} />
      <div className="my-1 h-px bg-border/50" />
      {/* 系统级 */}
      <Row
        k="数据目录"
        v={sys.data_dir_writable ? "可写" : "**只读!**"}
      />
      {sys.db_size_mb != null && (
        <Row k="DB 大小" v={`${sys.db_size_mb.toFixed(2)} MB`} />
      )}
      {sys.disk_free_gb != null && (
        <Row k="磁盘剩余" v={`${sys.disk_free_gb.toFixed(2)} GB`} />
      )}
      <Row
        k="pdftotext"
        v={sys.pdftotext_available ? "可用" : "未装(走 OCR 兜底)"}
      />
      <Row
        k="记忆目录"
        v={`${km.memory_root_exists ? "存在" : "未建"} / ${
          km.memory_root_writable ? "可写" : "不可写"
        }`}
      />
      {km.memory_error && <Row k="记忆错误" v={km.memory_error} />}
      {diag.recent_failures.length > 0 && (
        <>
          <div className="my-1 h-px bg-border/50" />
          <p className="text-muted-foreground">
            最近抽取失败({diag.recent_failures.length} 条,带 last_error):
          </p>
          {diag.recent_failures.slice(0, 5).map((f, i) => (
            <p key={i} className="pl-2 truncate">
              · [{f.category ?? "未分类"}] {f.filename}
            </p>
          ))}
          {diag.recent_failures.length > 5 && (
            <p className="pl-2 text-muted-foreground">
              … 还有 {diag.recent_failures.length - 5} 条,全在 MD 里
            </p>
          )}
        </>
      )}
      {diag.stderr_tail.length > 0 && (
        <>
          <div className="my-1 h-px bg-border/50" />
          <Row k="App 日志" v={`${diag.stderr_tail.length} 行(写进 MD)`} />
        </>
      )}
      {diag.console_errors.length > 0 && (
        <Row
          k="前端报错"
          v={`${diag.console_errors.length} 条(写进 MD)`}
        />
      )}
      {/* 2026-05-26 V0.1.12:抽取性能 A/B(给作者看本地 vs 云端 + 模型切换证据) */}
      {diag.metrics_summary.length > 0 && (
        <>
          <div className="my-1 h-px bg-border/50" />
          <p className="text-muted-foreground">
            抽取性能(最近 {diag.metrics_tail.length} 条样本):
          </p>
          {diag.metrics_summary.map((s, i) => (
            <p key={i} className="pl-2 truncate">
              · [{s.stage}] {s.backend} — 样本 {s.samples}(成功 {s.ok_samples})· avg {s.avg_ms}ms
              {s.avg_chars != null && ` · ${s.avg_chars} 字`}
            </p>
          ))}
        </>
      )}
      {/* 2026-05-27 V0.1.13+:功能模块用量(给作者一眼判断同事用了哪些功能) */}
      {diag.feature_usage && (
        <>
          <div className="my-1 h-px bg-border/50" />
          <Row
            k="chat 用量"
            v={`${diag.feature_usage.chat_messages_total} 条消息 / ${diag.feature_usage.chat_artifacts_total} artifact`}
          />
          <Row
            k="LLM 报告"
            v={`分析 ${diag.feature_usage.cases_with_analysis_report} · 风险 ${diag.feature_usage.cases_with_risk_report} · 深挖 ${diag.feature_usage.cases_with_deep_dive_report} · 完整 ${diag.feature_usage.cases_with_full_report}`}
          />
          {diag.feature_usage.document_sources.length > 0 && (
            <Row
              k="document.source"
              v={diag.feature_usage.document_sources
                .map((x) => `${x.key}×${x.count}`)
                .join(" / ")}
            />
          )}
        </>
      )}
      {/* 2026-05-27 V0.1.13+:chat 用量(按 model + task_type) */}
      {diag.chat_usage && diag.chat_usage.length > 0 && (
        <>
          <div className="my-1 h-px bg-border/50" />
          <p className="text-muted-foreground">
            chat 模型用量(近 200 条 assistant):
          </p>
          {diag.chat_usage.map((c, i) => (
            <p key={i} className="pl-2 truncate">
              · {c.model} / {c.task_type} — {c.samples} 样本(ok {c.ok_samples})·
              avg p{c.avg_prompt}/c{c.avg_completion} · {c.avg_latency_ms}ms
            </p>
          ))}
        </>
      )}
      <div className="my-1 h-px bg-border/50" />
      <Row k="反馈方 ID" v={diag.client_id_short} />
    </div>
  );
}

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between gap-2">
      <span className="shrink-0 text-muted-foreground">{k}</span>
      <span className="truncate">{v}</span>
    </div>
  );
}

/** 三态展示 key:已填 + 已验证 ✓ / 已填 + 未验证 ⚠(明显警告) / 未填 */
function KeyRow({
  label,
  filled,
  verified,
}: {
  label: string;
  filled: string; // "[SET]" | "[EMPTY]"
  verified: boolean;
}) {
  const isSet = filled === "[SET]";
  if (!isSet) {
    return (
      <div className="flex justify-between gap-2">
        <span className="shrink-0 text-muted-foreground">{label}</span>
        <span className="truncate text-muted-foreground">未填</span>
      </div>
    );
  }
  if (verified) {
    return (
      <div className="flex justify-between gap-2">
        <span className="shrink-0 text-muted-foreground">{label}</span>
        <span className="truncate text-emerald-700">已填 · 已验证 ✓</span>
      </div>
    );
  }
  // 关键警告:填了但没通过验证
  return (
    <div className="flex justify-between gap-2">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span className="truncate font-semibold text-amber-700">
        已填 · ⚠ 未通过验证(可能是无效 key)
      </span>
    </div>
  );
}

/* ============================ 通用 Modal 壳 ============================ */
function ModalShell({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: React.ReactNode;
}) {
  return (
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-black/40 px-4 py-8 backdrop-blur-sm animate-in fade-in-0 duration-200"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      role="dialog"
      aria-modal="true"
    >
      <div className="flex max-h-full w-full max-w-xl flex-col overflow-hidden rounded-lg border border-border bg-card shadow-2xl animate-in zoom-in-95 fade-in-0 duration-300">
        <header className="flex shrink-0 items-center justify-between border-b border-border bg-card/80 px-5 py-3">
          <h2 className="text-sm font-semibold text-foreground">{title}</h2>
          <button
            type="button"
            onClick={onClose}
            className="rounded p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            aria-label="关闭"
            title="关闭(Esc)"
          >
            <X className="size-4" />
          </button>
        </header>
        <div className="min-h-0 flex-1 overflow-auto px-5 py-4">{children}</div>
      </div>
    </div>
  );
}

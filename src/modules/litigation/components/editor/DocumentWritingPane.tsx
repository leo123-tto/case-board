/**
 * DocumentWritingPane —— 写作模式中栏容器。
 *
 * 职责(把 Milkdown 边界 `<MilkdownEditor>` 包成一个完整的「编辑一份文书」面板):
 *   1. 按 doc.source_path 读 .md → 剥 filing 注释头 → 得正文 body + title。
 *   2. 顶部:返回看板 / 可编辑标题 / 未保存指示 / 保存 / 导出。
 *   3. 显式保存(Cmd+S / 保存按钮);**关闭即存**(返回看板时 dirty 先存再退,advisor 定)。
 *   4. 导出复用两个编辑工作区共享的 Word / HTML 菜单；**导出前先存**。
 *
 * 安全:MVP 不上 autosave(序列化若规范化 MD 会静默改文书,autosave 会覆盖唯一副本)。
 * 详见 docs/V0.3-Milkdown编辑器-实施落地.md §1.5。
 */
import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from "react";
import { ArrowLeft, Loader2, Save } from "lucide-react";

import { Button } from "@/components/ui/button";
import { readTextFile, saveEditorDoc } from "@/lib/api";
import { stripFilingHeader, titleFromFilename } from "@/lib/filing";
import { countChanges, diffParts, type DiffPart } from "@/lib/textDiff";
import type { Document } from "@/lib/types";
import { DiffReview } from "@/components/editor/DiffReview";
import { EditorExportMenu } from "@/components/editor/EditorExportMenu";
import { MilkdownEditor } from "@/components/editor/MilkdownEditor";


interface Props {
  doc: Document;
  /** 返回看板模式(关闭编辑器);若有未保存改动会先存 */
  onClose: () => void;
  /** 保存成功后回调(让 App 轻量 reload 案件,刷新文档列表的大小/时间) */
  onSaved: () => void;
}

/** 暴露给 CaseView 的命令式句柄(chat 改文书的 flush/审阅握手用)。 */
export interface DocumentWritingPaneHandle {
  /** 有未保存改动则先存到磁盘(让 AI 的 edit_artifact 在最新内容上操作)。返回是否「磁盘已是最新」。 */
  flushIfDirty: () => Promise<boolean>;
  /** 当前是否有未保存改动(父组件进审阅前先问,dirty 时不进、不静默覆盖)。 */
  isDirty: () => boolean;
  /** ADR-0003 Phase 2 · AI 用 edit_artifact 改了磁盘后,进 diff 审阅:
   *  读磁盘(改后)对比基线(改前)→ **先把磁盘 revert 成改前**(接受才落盘)→ 渲染 diff。 */
  enterReview: () => Promise<void>;
}

export const DocumentWritingPane = forwardRef<DocumentWritingPaneHandle, Props>(
  function DocumentWritingPane({ doc, onClose, onSaved }, ref) {
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  /** 载入时剥头得到的正文(MilkdownEditor 初值,只在 mount 用一次) */
  const [body, setBody] = useState<string>("");
  const [title, setTitle] = useState<string>("");
  /** 编辑器吐出的最新 MD(onChange 持续更新) */
  const [currentMd, setCurrentMd] = useState<string>("");
  /** 已保存的基线(算 dirty);初始 = body */
  const savedRef = useRef<{ md: string; title: string }>({ md: "", title: "" });
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  /** ADR-0003 Phase 2 · 非 null 时进 diff 审阅视图(AI 改动待用户接受/拒绝) */
  const [review, setReview] = useState<DiffPart[] | null>(null);
  /** 编辑器重挂代次:换它强制 MilkdownEditor remount 显示最新 body(审阅应用后用) */
  const [editorEpoch, setEditorEpoch] = useState(0);

  // ── 载入文书 ────────────────────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    readTextFile(doc.source_path)
      .then((raw) => {
        if (cancelled) return;
        const { meta, body: b } = stripFilingHeader(raw);
        const t = meta.title || titleFromFilename(doc.filename);
        setBody(b);
        setCurrentMd(b);
        setTitle(t);
        savedRef.current = { md: b, title: t };
        setDirty(false);
        setReview(null); // 切文档清掉残留审阅态
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [doc.id, doc.source_path, doc.filename]);

  // dirty 计算:正文或标题任一变了
  useEffect(() => {
    setDirty(
      currentMd !== savedRef.current.md || title !== savedRef.current.title,
    );
  }, [currentMd, title]);

  // ── 保存 ────────────────────────────────────────────────────
  // 返回保存是否成功(关闭即存要用)
  const doSave = useCallback(async (): Promise<boolean> => {
    if (saving) return false;
    setSaving(true);
    setError(null);
    try {
      await saveEditorDoc(doc.id, title.trim() || titleFromFilename(doc.filename), currentMd);
      savedRef.current = { md: currentMd, title };
      setDirty(false);
      onSaved();
      return true;
    } catch (e) {
      setError(`保存失败:${e}`);
      return false;
    } finally {
      setSaving(false);
    }
  }, [saving, doc.id, doc.filename, title, currentMd, onSaved]);

  // ADR-0003 Phase 2 · 进 diff 审阅:读磁盘(AI 改后)对比基线(改前),先 revert 磁盘成改前
  //(接受才落盘,守「防静默覆盖」铁律),再渲染 diff 等用户接受/拒绝。
  const enterReview = useCallback(async () => {
    setError(null);
    try {
      const raw = await readTextFile(doc.source_path);
      const { body: newBody } = stripFilingHeader(raw);
      const oldBody = savedRef.current.md; // 改前基线(发送前已 flush,= AI 改之前的磁盘内容)
      const t = savedRef.current.title || titleFromFilename(doc.filename);
      if (newBody === oldBody) return; // AI 没真正改动 → 不进审阅
      const parts = diffParts(oldBody, newBody);
      if (countChanges(parts) === 0) return;
      // revert:把磁盘还原成改前(pending until accept);newBody 已在 parts 里
      await saveEditorDoc(doc.id, t, oldBody);
      setReview(parts);
    } catch (e) {
      setError(`载入 AI 改动失败:${e}`);
    }
  }, [doc.id, doc.source_path, doc.filename]);

  // 审阅「应用」:按各处接受/拒绝拼回的最终正文落盘 + 重挂编辑器显示
  const applyReview = useCallback(
    async (finalBody: string) => {
      const t = savedRef.current.title || titleFromFilename(doc.filename);
      setSaving(true);
      setError(null);
      try {
        await saveEditorDoc(doc.id, t, finalBody);
        setBody(finalBody);
        setCurrentMd(finalBody);
        savedRef.current = { md: finalBody, title: t };
        setDirty(false);
        setEditorEpoch((e) => e + 1); // 重挂编辑器显示最终版
        setReview(null);
        onSaved(); // reload 案件列表(大小/时间)
      } catch (e) {
        setError(`应用修改失败:${e}`);
      } finally {
        setSaving(false);
      }
    },
    [doc.id, doc.filename, onSaved],
  );

  // 审阅「取消」:放弃 AI 改动(磁盘已 revert 成改前,编辑器仍是改前内容)
  const cancelReview = useCallback(() => setReview(null), []);

  // 暴露 flush/isDirty/enterReview 给 CaseView(chat 改文书的握手)
  useImperativeHandle(
    ref,
    () => ({
      flushIfDirty: async () => (dirty ? await doSave() : true),
      isDirty: () => dirty,
      enterReview,
    }),
    [dirty, doSave, enterReview],
  );

  // 关闭:有改动先存(存失败则不退、留在编辑器看错误)
  const handleClose = useCallback(async () => {
    if (dirty) {
      const ok = await doSave();
      if (!ok) return;
    }
    onClose();
  }, [dirty, doSave, onClose]);

  // Cmd+S 保存
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && (e.key === "s" || e.key === "S")) {
        e.preventDefault();
        if (dirty) void doSave();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [dirty, doSave]);

  // ── 导出(先存再导,因为导出从磁盘读) ──────────────────────
  const prepareExport = useCallback(
    async (): Promise<boolean> => (dirty ? await doSave() : true),
    [dirty, doSave],
  );

  // ADR-0003 Phase 2 · 审阅模式:占满中栏,接受/拒绝 AI 改动(替代直接生效)
  if (review) {
    return (
      <div className="flex h-full min-h-0 flex-1 flex-col bg-background">
        <DiffReview
          parts={review}
          onApply={(finalBody) => void applyReview(finalBody)}
          onCancel={cancelReview}
        />
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-1 flex-col bg-background">
      {/* 顶部工具栏 */}
      <header className="flex shrink-0 items-center gap-2 border-b border-border bg-card/50 px-4 py-2.5">
        <Button
          variant="ghost"
          size="sm"
          onClick={handleClose}
          disabled={saving}
          title="退出编辑器,返回案件看板(有改动会先保存)"
        >
          <ArrowLeft className="size-3.5" />
          退出编辑
        </Button>

        <div className="flex min-w-0 flex-1 items-center gap-2">
          {/* 标题在下方文档区顶部居中显示+可编辑(见 body);这里只读显一份方便折叠/滚动时看 */}
          <span
            className="min-w-0 flex-1 truncate px-2 text-sm font-medium text-muted-foreground"
            title={title}
          >
            {title || "文书"}
          </span>
          {dirty && (
            <span
              className="shrink-0 text-label text-amber-600 animate-in fade-in-0 duration-200"
              title="有未保存的改动"
            >
              ● 未保存
            </span>
          )}
        </div>

        <div className="flex shrink-0 items-center gap-1.5">
          <Button
            size="sm"
            onClick={() => void doSave()}
            disabled={!dirty || saving}
            title="保存(Cmd+S)"
          >
            {saving ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <Save className="size-3.5" />
            )}
            保存
          </Button>
          <EditorExportMenu
            title={title.trim() || titleFromFilename(doc.filename)}
            mdPath={doc.source_path}
            beforeExport={prepareExport}
            onError={(message) => setError(message || null)}
            disabled={saving || loading}
          />
        </div>
      </header>

      {/* 错误条 */}
      {error && (
        <div className="shrink-0 border-b border-destructive/30 bg-destructive/5 px-4 py-1.5 text-xs text-destructive">
          {error}
        </div>
      )}

      {/* 编辑区 */}
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
        {loading ? (
          <div className="flex h-full items-center justify-center">
            <Loader2 className="size-5 animate-spin text-muted-foreground" />
          </div>
        ) : (
          <>
            {/* NEW1 · 文书标题居中大字显示在文档顶部(像 Word 的居中标题),可直接编辑。
                导出 Word 用的就是这个 title;正文(content_md)不含标题。 */}
            <input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="文书标题(如 民事起诉状)"
              className="shrink-0 border-b border-border/40 bg-transparent px-6 pb-3 pt-4 text-center text-lg font-semibold tracking-[0.3em] text-foreground placeholder:tracking-normal placeholder:text-muted-foreground/50 focus:outline-none"
              title="文书标题(导出 Word 的居中大标题)"
            />
            <div className="min-h-0 flex-1 overflow-hidden">
              {/* key 含 editorEpoch:审阅应用后 bump → 重挂编辑器显示最终 body */}
              <MilkdownEditor
                key={`${doc.id}-${editorEpoch}`}
                value={body}
                onChange={setCurrentMd}
              />
            </div>
          </>
        )}
      </div>
    </div>
  );
});

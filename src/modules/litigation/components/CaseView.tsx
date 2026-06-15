import { useRef, useState } from "react";
import {
  BookMarked,
  BookOpen,
  FolderSearch,
  Loader2,
  Pencil,
  RefreshCw,
  Trash2,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { toast } from "@/components/ui/toast";
import {
  deleteDocument,
  distillCaseExperience,
  globalExtractCase,
  reextractDocument,
  reextractDocumentDewatermark,
} from "@/lib/api";
import { confirmDialog } from "@/lib/dialog";
import { type Case, type Document } from "@/lib/types";
import { formatRelativeTime, shortenPath } from "@/lib/format";
import { cn } from "@/lib/utils";

import { groupByStage } from "../lib/groupByStage";
import { CaseChatPanel } from "./chat/CaseChatPanel";
import { CaseSnapshotView } from "./snapshot/CaseSnapshotView";
import { CaseSwitcher } from "./CaseSwitcher";
import {
  type DocumentWritingPaneHandle,
  DocumentWritingPane,
} from "./editor/DocumentWritingPane";
import { ErrorState, LoadingState, NoDocsHint } from "./StatusViews";
import { SourceFilesSection } from "./SourceFilesSection";

/* ------------------------------------------------------------------ */
/* 案件视图                                                            */
/* ------------------------------------------------------------------ */

export function CaseView({
  cases,
  selectedCase,
  documents,
  loading,
  error,
  onSwitchCase,
  onGoHome,
  onOpenDoc,
  onRevealDoc,
  onRevealCase,
  isEditMode,
  onToggleEditMode,
  onDeleteCase,
  onRefreshFiles,
  refreshingFiles,
  onOpenReport,
  reportLoading,
  onReloadCase,
  editingDoc,
  onCloseEditor,
  onArtifactCreated,
}: {
  cases: Case[];
  selectedCase: Case | null;
  documents: Document[];
  loading: boolean;
  error: string | null;
  onSwitchCase: (id: string) => void;
  onGoHome: () => void;
  onOpenDoc: (doc: Document) => void;
  onRevealDoc: (doc: Document) => void;
  onRevealCase: () => void;
  /** 编辑模式开关 — 右上角铅笔按钮控制,P3 接 inline 编辑 / 拖卡片 / 删行 */
  isEditMode: boolean;
  onToggleEditMode: () => void;
  onDeleteCase: () => void;
  onRefreshFiles: () => void;
  refreshingFiles: boolean;
  onOpenReport: () => void;
  reportLoading: boolean;
  /** 2026-05-27 V0.1.13+ chat artifact 完成后的轻量 reload(只重读 DB,不 sync 源文件夹) */
  onReloadCase: () => void;
  /** V0.3 D1+D2 · 写作模式:当前在编辑器里打开的文书(null = 看板模式) */
  editingDoc: Document | null;
  /** V0.3 D1+D2 · 关闭编辑器,回看板模式 */
  onCloseEditor: () => void;
  /** V0.3 D2 · chat 落了 save_artifact 文书后的回调:reload + 自动进编辑器打开(docId 空=仅 reload) */
  onArtifactCreated: (docId: string) => void;
}) {
  const groups = groupByStage(documents);
  const aiArtifacts = documents.filter((d) => d.is_ai_artifact);

  // V0.3 ADR-0003 Phase 1B+2 · chat 改文书的 flush/审阅握手(编辑器磁盘冲突防护)。
  const editorRef = useRef<DocumentWritingPaneHandle>(null);
  // 发送前:编辑器有未保存改动先 flush 到磁盘,让 AI 的 edit_artifact 在最新内容上操作
  //(同时让审阅的「改前基线」= flush 后的内容)。
  const flushEditorBeforeSend = async () => {
    await editorRef.current?.flushIfDirty();
  };
  // AI 这轮调了 edit_artifact 改磁盘后:编辑器若打开 → 进 diff 审阅(接受/拒绝);
  // 编辑器有未保存改动则不进审阅(警告,避免基线错乱);没开编辑器则只刷新列表。
  const handleArtifactEdited = () => {
    if (!editingDoc) {
      onReloadCase();
      return;
    }
    if (editorRef.current?.isDirty()) {
      toast(
        "AI 改了这份文书,但你编辑器里有未保存改动,未进入审阅。先保存或退出,再让 AI 改。",
        "info",
      );
      return;
    }
    void editorRef.current?.enterReview();
  };

  // V0.2.2 · 删除一条 AI 摘要 artifact(软删 + 重读案件)。关键决策点用 confirm 拦一下。
  const handleDeleteDoc = async (doc: Document) => {
    if (
      !(await confirmDialog(
        `删除「${doc.filename}」?会从材料列表移除(软删,不影响磁盘原文件)。`,
        { danger: true, okLabel: "删除" },
      ))
    )
      return;
    try {
      await deleteDocument(doc.id);
      onReloadCase();
    } catch (e) {
      toast(`删除失败:${e}`, "error");
    }
  };

  // V0.3 · 强制重抽单个源文档(抽取失败/想重抽)。立刻 reload 看到「抽取中」,
  // 完成后 App 订阅的 extraction_progress 会再自动刷新。
  const handleReextract = async (doc: Document) => {
    try {
      await reextractDocument(doc.id);
      // 2026-06-13(胡彬律师反馈):重识别不再自动跑全案分析(省钱)。
      // 多个文档逐个重识别完后,手动点一次「重新分析」更新画像即可。
      toast(
        `已开始重新识别「${doc.filename}」· 识别完(可多个一起)后点「重新分析」更新画像`,
        "success",
      );
      onReloadCase();
    } catch (e) {
      toast(`重新抽取失败:${e}`, "error");
    }
  };

  // 2026-06-13(胡彬律师反馈)· 去水印重识别:带大幅水印的工商调档件强制 PP-OCRv6+去水印。
  const handleReextractDewatermark = async (doc: Document) => {
    try {
      await reextractDocumentDewatermark(doc.id);
      toast(
        `已对「${doc.filename}」去水印重新识别(PP-OCRv6)· 识别完点「重新分析」更新画像`,
        "success",
      );
      onReloadCase();
    } catch (e) {
      toast(`去水印重识别失败:${e}`, "error");
    }
  };

  // 2026-06-11 · 重新分析(作者反馈:全案分析失败/没跑完后无干净重试入口)。
  // 只重跑全案 LLM 分析(不重跑 OCR、不烧积分),完成后刷新案件数据。
  const [reanalyzing, setReanalyzing] = useState(false);
  const handleReanalyze = async () => {
    if (!selectedCase || reanalyzing) return;
    setReanalyzing(true);
    toast("已开始重新分析全案(通常 1~3 分钟),期间可继续其他操作", "info");
    try {
      const r = await globalExtractCase(selectedCase.id);
      if (r.table_ok) {
        toast("✓ 全案分析完成,画像已更新", "success");
        onReloadCase();
      } else {
        toast(`全案分析失败:${r.error ?? "未知原因"}`, "error");
      }
    } catch (e) {
      toast(`全案分析失败:${e}`, "error");
    } finally {
      setReanalyzing(false);
    }
  };

  // 项目1:把(已结案/判决)案件提炼成办案经验卡片,写入本地知识库供同类案检索复用(不脱敏)。
  const [distilling, setDistilling] = useState(false);
  const handleDistill = async () => {
    if (!selectedCase || distilling) return;
    setDistilling(true);
    toast("正在提炼办案经验卡片入知识库(约 10-30 秒)…", "info");
    try {
      const path = await distillCaseExperience(selectedCase.id);
      toast("✓ 已沉淀为办案经验,存入本地知识库,日后同类案可检索复用", "success");
      console.info("experience card saved:", path);
    } catch (e) {
      toast(`沉淀失败:${e}`, "error");
    } finally {
      setDistilling(false);
    }
  };

  return (
    <main className="flex h-full w-full flex-col bg-background">
      {/* Header */}
      <header className="border-b border-border bg-card/50 px-8 py-5">
        <div className="mx-auto flex max-w-6xl items-start justify-between gap-4">
          <div className="min-w-0 flex-1">
            <button
              type="button"
              onClick={onGoHome}
              className="mb-2 inline-flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
            >
              ← 返回看板
            </button>
            <div className="flex items-baseline gap-2">
              {cases.length > 1 ? (
                <CaseSwitcher
                  cases={cases}
                  selectedId={selectedCase?.id ?? null}
                  onSwitch={onSwitchCase}
                />
              ) : (
                <h1 className="text-xl font-semibold tracking-tight text-foreground">
                  {selectedCase?.name ?? "—"}
                </h1>
              )}
              <span className="text-xs text-muted-foreground">
                {selectedCase?.case_type}
              </span>
            </div>
            {selectedCase && (
              <button
                type="button"
                onClick={onRevealCase}
                className="mt-1 inline-flex items-center gap-1.5 truncate font-mono text-xs text-muted-foreground transition-colors hover:text-foreground"
                title="在 Finder 中打开案件文件夹"
              >
                <FolderSearch className="size-3 shrink-0" />
                <span className="truncate">
                  {shortenPath(selectedCase.source_folder, 3)}
                </span>
              </button>
            )}
            {!loading && !error && documents.length > 0 && (
              <p className="mt-2 text-xs text-muted-foreground">
                共{" "}
                <span className="font-medium text-foreground">
                  {documents.length}
                </span>{" "}
                份文档
                {aiArtifacts.length > 0 && (
                  <>
                    {" · "}
                    <span className="text-foreground">{aiArtifacts.length}</span>{" "}
                    份 AI 产物
                  </>
                )}
                {selectedCase?.last_scanned_at && (
                  <>
                    {" · 上次扫描 "}
                    <span
                      className="text-foreground"
                      title={selectedCase.last_scanned_at}
                    >
                      {formatRelativeTime(selectedCase.last_scanned_at)}
                    </span>
                  </>
                )}
              </p>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            {/* 「📖 案件分析报告」醒目主按钮 — 没报告也能点(点击触发抽取 + 完成后自动弹) */}
            <Button
              size="sm"
              onClick={onOpenReport}
              disabled={!selectedCase || reportLoading}
              className="bg-foreground text-background hover:bg-foreground/90"
              title={
                selectedCase?.case_report_path
                  ? "查看 LLM 案件分析报告"
                  : "立刻生成案件分析报告(~ 10-30 秒)"
              }
            >
              {reportLoading ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <BookOpen className="size-3.5" />
              )}
              {reportLoading ? "生成中…" : "案件报告"}
            </Button>
            <button
              type="button"
              onClick={handleDistill}
              disabled={
                !selectedCase || distilling || !selectedCase?.case_report_path
              }
              className="rounded p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-30"
              title={
                selectedCase?.case_report_path
                  ? "把本案提炼成办案经验卡片,存入本地知识库,日后同类案可检索复用"
                  : "需先生成案件报告,才能沉淀办案经验"
              }
              aria-label="沉淀为办案经验"
            >
              {distilling ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <BookMarked className="size-4" />
              )}
            </button>
            <button
              type="button"
              onClick={onRefreshFiles}
              disabled={
                !selectedCase ||
                refreshingFiles ||
                selectedCase?.source_folder === "__DEMO__"
              }
              className="rounded p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-30"
              title={
                selectedCase?.source_folder === "__DEMO__"
                  ? "示例案件没有源文件夹,无法更新"
                  : "检测源文件夹有没有新增 / 修改 / 删除的文件,有变动会自动抽取"
              }
              aria-label="更新源文件"
            >
              <RefreshCw
                className={cn("size-4", refreshingFiles && "animate-spin")}
              />
            </button>
            <button
              type="button"
              onClick={onDeleteCase}
              disabled={!selectedCase}
              className="rounded p-1.5 text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive disabled:cursor-not-allowed disabled:opacity-30"
              title="从看板删除当前案件(不动原始文件夹)"
              aria-label="删除当前案件"
            >
              <Trash2 className="size-4" />
            </button>
            <button
              type="button"
              onClick={onToggleEditMode}
              disabled={!selectedCase}
              className={cn(
                "rounded p-1.5 transition-colors disabled:cursor-not-allowed disabled:opacity-30",
                isEditMode
                  ? "bg-foreground text-background hover:bg-foreground/90"
                  : "text-muted-foreground hover:bg-accent hover:text-foreground",
              )}
              title={
                isEditMode
                  ? "退出编辑模式(改动已自动保存)"
                  : "编辑模式 — 改字段 / 删词条 / 拖卡片"
              }
              aria-label={isEditMode ? "退出编辑" : "进入编辑模式"}
              aria-pressed={isEditMode}
            >
              <Pencil className="size-4" />
            </button>
          </div>
        </div>
      </header>

      {/*
        Body + 右侧 AI 助手(2026-05-27 V0.1.13+)。
        V0.3 D1+D2:左侧主区按 mode 二选一(看板 / 写作模式编辑器);
        **CaseChatPanel 永远是本 flex row 的稳定末位子节点**(固定 key,只换它前面的兄弟),
        切换模式不卸载它 —— 否则会丢正在输入的内容/引用 + history 闪烁重拉
        (chatRunRegistry 只保 streaming,不保面板本地状态)。见 docs/V0.3-Milkdown编辑器-实施落地.md §1.3
      */}
      <div className="flex min-h-0 flex-1">
        {editingDoc ? (
          <DocumentWritingPane
            ref={editorRef}
            doc={editingDoc}
            onClose={onCloseEditor}
            onSaved={onReloadCase}
          />
        ) : (
          <div className="flex-1 overflow-auto animate-in fade-in-0 duration-200 ease-out">
            <div className="mx-auto max-w-6xl px-8 py-6">
              {loading && <LoadingState />}
              {error && !loading && <ErrorState message={error} />}
              {!loading && !error && documents.length === 0 && <NoDocsHint />}
              {!loading && !error && documents.length > 0 && selectedCase && (
                <div className="space-y-5">
                  {/* 整套案件信息(框架永远显示,字段空就 "—",作者 2026-05-23 晚十四) */}
                  <CaseSnapshotView
                    caseData={selectedCase}
                    documents={documents}
                    isEditMode={isEditMode}
                  />

                  {/* 原文件(默认折叠) */}
                  <SourceFilesSection
                    total={documents.length}
                    aiArtifacts={aiArtifacts}
                    groups={groups}
                    onOpenDoc={onOpenDoc}
                    onRevealDoc={onRevealDoc}
                    onDeleteDoc={handleDeleteDoc}
                    onReextract={handleReextract}
                    onReextractDewatermark={handleReextractDewatermark}
                    onRefresh={onRefreshFiles}
                    refreshing={refreshingFiles}
                    onReanalyze={handleReanalyze}
                    reanalyzing={reanalyzing}
                  />

                </div>
              )}
            </div>
          </div>
        )}

        {/* 案件 AI 助手 — 默认展开 420px,可折叠到 32px sliver。稳定末位,勿被 mode 分支包裹。 */}
        <CaseChatPanel
          key="case-chat"
          caseId={selectedCase?.id ?? null}
          caseName={selectedCase?.name ?? null}
          onArtifactCreated={onArtifactCreated}
          editingDocId={editingDoc?.id ?? null}
          onBeforeSend={flushEditorBeforeSend}
          onArtifactEdited={handleArtifactEdited}
        />
      </div>
    </main>
  );
}

/* 2026-05-23 晚十:删 ExtractingHint — 抽取中详情页主区空白,进度看顶部 banner */

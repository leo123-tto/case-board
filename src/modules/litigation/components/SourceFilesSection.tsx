import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  Droplets,
  FileText,
  Folder,
  FolderSearch,
  Image as ImageIcon,
  Loader2,
  Pencil,
  RefreshCw,
  Sparkles,
  Trash2,
} from "lucide-react";

import { type Document, STAGE_ORDER } from "@/lib/types";
import { formatBytes } from "@/lib/format";
import { cn, docDisplayName } from "@/lib/utils";
import { useFeatureFlag } from "@/lib/featureFlags";

import { type GroupKey } from "../lib/groupByStage";
import {
  buildFileTree,
  collectDocs,
  countFiles,
  type FileTreeNode,
} from "../lib/fileTree";
import {
  CATEGORIES,
  EMPTY_MARK,
  EVIDENCE_ATTITUDES,
  sortByImportance,
  SUBMISSION_STAGES,
  UNCATEGORIZED,
  type DocMark,
  type DocMarkMap,
  type EvidenceAttitude,
  type Importance,
  type SubmissionStage,
} from "../lib/docMarks";

/** 源文件区视图模式:AI 视图 / 原始视图 / 源结构视图 */
type ViewMode = "folder" | "stage" | "organize";

const PARTY_SIDES = ["原告", "被告", "第三人"] as const;

/** 标记回调签名(文件级单个 / 文件夹级批量都用 docIds 数组) */
export interface MarkHandlers {
  markMap: DocMarkMap;
  onMarkImportance: (docIds: string[], value: Importance | null) => void;
  onMarkPartySide: (docIds: string[], value: string, enabled: boolean) => void;
  /** 分类(单值,单文档;null=清空) */
  onMarkCategory: (docId: string, value: string | null) => void;
  onMarkEvidenceAttitude: (docIds: string[], value: EvidenceAttitude | null) => void;
  onMarkSubmissionStage: (docIds: string[], value: SubmissionStage | null) => void;
  /** 重命名(板内显示名,单文档;name=null/空=清回原文件名) */
  onRename: (docId: string, name: string | null) => void;
}

/* ------------------------------------------------------------------ */
/* 原文件区(默认折叠,展开看分组文件列表 + 统计)                       */
/* ------------------------------------------------------------------ */

export function SourceFilesSection({
  total,
  aiArtifacts,
  groups,
  documents,
  sourceFolder,
  markMap,
  onMarkImportance,
  onMarkPartySide,
  onMarkCategory,
  onMarkEvidenceAttitude,
  onMarkSubmissionStage,
  onRename,
  onAiOrganize,
  organizing,
  onOpenDoc,
  onRevealDoc,
  onDeleteDoc,
  onReextract,
  onReextractDewatermark,
  onRefresh,
  refreshing,
  onReanalyze,
  reanalyzing,
}: {
  total: number;
  aiArtifacts: Document[];
  groups: Record<GroupKey, Document[]>;
  /** 全部文档(含 AI 产物);原结构视图用非产物的源文件派生文件夹树 */
  documents: Document[];
  /** 案件源文件夹绝对路径,派生原结构相对路径用 */
  sourceFolder: string;
  /** Phase 3:每个文档的标记(重要/忽略 + 原被告 + 分类) */
  markMap: DocMarkMap;
  onMarkImportance: (docIds: string[], value: Importance | null) => void;
  onMarkPartySide: (docIds: string[], value: string, enabled: boolean) => void;
  onMarkCategory: (docId: string, value: string | null) => void;
  onMarkEvidenceAttitude: (docIds: string[], value: EvidenceAttitude | null) => void;
  onMarkSubmissionStage: (docIds: string[], value: SubmissionStage | null) => void;
  /** 重命名板内显示名(单文档;null/空=清回原文件名) */
  onRename: (docId: string, name: string | null) => void;
  /** 🪄 AI 自动整理(整案分类) */
  onAiOrganize: () => void;
  organizing: boolean;
  onOpenDoc: (doc: Document) => void;
  onRevealDoc: (doc: Document) => void;
  onDeleteDoc: (doc: Document) => void;
  /** V0.3 · 强制重抽单个源文档(抽取失败/想重抽时) */
  onReextract: (doc: Document) => void;
  /** 2026-06-13 · 去水印重识别(带水印工商调档件,强制 PP-OCRv6+去水印) */
  onReextractDewatermark: (doc: Document) => void;
  onRefresh: () => void;
  refreshing: boolean;
  /** 2026-06-11 · 重新分析:只重跑全案 LLM 分析(不重跑 OCR),分析失败后的重试入口 */
  onReanalyze: () => void;
  reanalyzing: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const toggle = () => setExpanded((v) => !v);
  // 默认打开 AI 视图,让用户一进案件就能看到 AI 整理入口与分类结果。
  const [viewMode, setViewMode] = useState<ViewMode>("organize");

  // 非 AI 产物的源文件,派生原始文件夹树(只读派生,跟着 source_path 走)
  const sourceDocs = useMemo(
    () => documents.filter((d) => !d.is_ai_artifact),
    [documents],
  );
  const tree = useMemo(
    () => buildFileTree(sourceDocs, sourceFolder),
    [sourceDocs, sourceFolder],
  );
  const marks: MarkHandlers = {
    markMap,
    onMarkImportance,
    onMarkPartySide,
    onMarkCategory,
    onMarkEvidenceAttitude,
    onMarkSubmissionStage,
    onRename,
  };

  return (
    <section className="rounded-lg border border-border bg-card shadow-sm">
      {/* 2026-05-25 V0.1.5:外层从 button 改成 div + role=button,允许内部嵌套真正的「刷新」button */}
      <div
        role="button"
        tabIndex={0}
        onClick={toggle}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            toggle();
          }
        }}
        className="flex w-full cursor-pointer items-center justify-between px-5 py-3 text-left transition-colors hover:bg-muted/30"
      >
        <div className="flex items-center gap-2">
          <ChevronDown
            className={cn(
              "size-4 text-muted-foreground transition-transform",
              expanded ? "rotate-0" : "-rotate-90",
            )}
          />
          <h2 className="text-sm font-semibold text-foreground">
            原文件
          </h2>
          <span className="text-xs text-muted-foreground">
            {total} 份{aiArtifacts.length > 0 && ` · ${aiArtifacts.length} 份 AI 摘要`}
          </span>
        </div>
        <div className="flex items-center gap-3">
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onReanalyze();
            }}
            disabled={reanalyzing}
            title="重新跑全案 AI 分析(更新画像/报告;不重跑 OCR、不烧积分)— 分析失败或没更新时用这个"
            className={cn(
              "inline-flex items-center gap-1 rounded-md border border-border bg-background px-2.5 py-1 text-xs text-foreground transition-colors hover:bg-muted",
              reanalyzing && "cursor-wait opacity-60",
            )}
          >
            <Sparkles className={cn("size-3", reanalyzing && "animate-pulse")} />
            {reanalyzing ? "分析中…" : "重新分析"}
          </button>
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onRefresh();
            }}
            disabled={refreshing}
            title="重扫源文件夹,增量抽取新增/修改的文件"
            className={cn(
              "inline-flex items-center gap-1 rounded-md border border-border bg-background px-2.5 py-1 text-xs text-foreground transition-colors hover:bg-muted",
              refreshing && "cursor-wait opacity-60",
            )}
          >
            <RefreshCw
              className={cn("size-3", refreshing && "animate-spin")}
            />
            {refreshing ? "同步中…" : "刷新源文件"}
          </button>
          <span className="text-xs text-muted-foreground">
            {expanded ? "点击折叠" : "点击展开"}
          </span>
        </div>
      </div>

      {expanded && (
        <div className="space-y-6 border-t border-border px-5 py-5">
          {/* 视图切换 */}
          <div className="flex items-center gap-1 rounded-lg bg-muted/40 p-1 text-xs">
            <ViewTab active={viewMode === "organize"} onClick={() => setViewMode("organize")}>
              AI视图
            </ViewTab>
            <ViewTab active={viewMode === "stage"} onClick={() => setViewMode("stage")}>
              原始视图
            </ViewTab>
            <ViewTab active={viewMode === "folder"} onClick={() => setViewMode("folder")}>
              源结构视图
            </ViewTab>
          </div>

          {/* AI 摘要(产物)始终单列在最前 */}
          {aiArtifacts.length > 0 && (
            <StageSection
              title="AI 摘要"
              count={aiArtifacts.length}
              docs={aiArtifacts}
              highlight
              onOpenDoc={onOpenDoc}
              onRevealDoc={onRevealDoc}
              onDeleteDoc={onDeleteDoc}
            />
          )}

          {viewMode === "folder" && (
            <FolderTreeView
              tree={tree}
              marks={marks}
              onOpenDoc={onOpenDoc}
            />
          )}

          {viewMode === "stage" && (
            <>
              {STAGE_ORDER.map((stage) =>
                groups[stage].length > 0 ? (
                  <StageSection
                    key={stage}
                    title={stage}
                    count={groups[stage].length}
                    docs={groups[stage]}
                    marks={marks}
                    onOpenDoc={onOpenDoc}
                    onRevealDoc={onRevealDoc}
                    onReextract={onReextract}
                    onReextractDewatermark={onReextractDewatermark}
                  />
                ) : null,
              )}
              {groups.其他.length > 0 && (
                <StageSection
                  title="其他"
                  count={groups.其他.length}
                  docs={groups.其他}
                  dim
                  marks={marks}
                  onOpenDoc={onOpenDoc}
                  onRevealDoc={onRevealDoc}
                  onReextract={onReextract}
                  onReextractDewatermark={onReextractDewatermark}
                />
              )}
            </>
          )}

          {viewMode === "organize" && (
            <OrganizeView
              docs={sourceDocs}
              marks={marks}
              onAiOrganize={onAiOrganize}
              organizing={organizing}
              onOpenDoc={onOpenDoc}
            />
          )}
        </div>
      )}
    </section>
  );
}

function StageSection({
  title,
  count,
  docs,
  highlight = false,
  dim = false,
  marks,
  onOpenDoc,
  onRevealDoc,
  onDeleteDoc,
  onReextract,
  onReextractDewatermark,
}: {
  title: string;
  count: number;
  docs: Document[];
  highlight?: boolean;
  dim?: boolean;
  /** 标记句柄(AI 摘要分组不传 → 不显示标记控件) */
  marks?: MarkHandlers;
  onOpenDoc: (doc: Document) => void;
  onRevealDoc: (doc: Document) => void;
  onDeleteDoc?: (doc: Document) => void;
  onReextract?: (doc: Document) => void;
  onReextractDewatermark?: (doc: Document) => void;
}) {
  // 重要置顶、忽略沉底(与原文件夹结构视图一致);无标记句柄(AI 摘要组)保持原序
  const orderedDocs = marks ? sortByImportance(docs, marks.markMap) : docs;
  return (
    <section>
      <div className="mb-3 flex items-baseline gap-2">
        <h2
          className={cn(
            "text-sm font-semibold",
            dim ? "text-muted-foreground" : "text-foreground",
          )}
        >
          {title}
        </h2>
        <span className="text-xs text-muted-foreground">{count}</span>
      </div>
      <ul
        className={cn(
          "divide-y divide-border rounded-lg border",
          highlight
            ? "border-foreground/15 bg-muted/30"
            : "border-border bg-card",
        )}
      >
        {orderedDocs.map((doc) => (
          <DocRow
            key={doc.id}
            doc={doc}
            highlight={highlight}
            marks={marks}
            onOpen={() => onOpenDoc(doc)}
            onReveal={() => onRevealDoc(doc)}
            onDelete={onDeleteDoc ? () => onDeleteDoc(doc) : undefined}
            onReextract={onReextract ? () => onReextract(doc) : undefined}
            onReextractDewatermark={
              onReextractDewatermark
                ? () => onReextractDewatermark(doc)
                : undefined
            }
          />
        ))}
      </ul>
    </section>
  );
}

function ViewTab({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "rounded-md px-3 py-1.5 font-medium transition-colors",
        active
          ? "bg-card text-foreground shadow-sm"
          : "text-muted-foreground hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}

/**
 * 原文件夹结构视图:模拟 Finder 图标视图 —— 文件夹/文件都是方块卡片、网格排列;
 * 点文件夹卡片钻进去(面包屑返回),点文件卡片进板内查看器。
 */
function FolderTreeView({
  tree,
  marks,
  onOpenDoc,
}: {
  tree: FileTreeNode;
  marks: MarkHandlers;
  onOpenDoc: (doc: Document) => void;
}) {
  // 导航路径(从根到当前文件夹的 name 序列)
  const [path, setPath] = useState<string[]>([]);
  // 右键标记菜单(单文件 or 整文件夹)
  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    label: string;
    docIds: string[];
    mark?: DocMark;
    /** 单文件时带上文档,供「重命名」用(文件夹批量不带) */
    doc?: Document;
  } | null>(null);
  // 重命名弹窗(单文件)
  const [renaming, setRenaming] = useState<Document | null>(null);
  const openMenuFor = (
    e: React.MouseEvent,
    label: string,
    docIds: string[],
    mark?: DocMark,
    doc?: Document,
  ) => {
    e.preventDefault();
    if (docIds.length === 0) return;
    setMenu({ x: e.clientX, y: e.clientY, label, docIds, mark, doc });
  };

  // 沿 path 解析当前节点 + 面包屑(路径失效则回退到能走到的最深处)
  let current = tree;
  const crumbs: string[] = [];
  for (const seg of path) {
    const next = current.folders.find((f) => f.name === seg);
    if (!next) break;
    current = next;
    crumbs.push(seg);
  }

  if (tree.folders.length === 0 && tree.files.length === 0) {
    return <p className="text-sm text-muted-foreground">没有源文件。</p>;
  }

  // 当前文件夹(含子文件夹)下全部文档 → 文件夹级整批标记
  const docsHere = collectDocs(current);
  const docIds = docsHere.map((d) => d.id);

  return (
    <div>
      {/* 面包屑 */}
      <div className="mb-3 flex flex-wrap items-center gap-1 text-xs">
        <button
          type="button"
          onClick={() => setPath([])}
          className={cn(
            "rounded px-1.5 py-0.5 transition-colors hover:bg-muted",
            crumbs.length === 0 ? "font-medium text-foreground" : "text-muted-foreground",
          )}
        >
          全部材料
        </button>
        {crumbs.map((name, i) => (
          <Fragment key={i}>
            <ChevronRight className="size-3 text-muted-foreground/50" />
            <button
              type="button"
              onClick={() => setPath(crumbs.slice(0, i + 1))}
              className={cn(
                "max-w-[180px] truncate rounded px-1.5 py-0.5 transition-colors hover:bg-muted",
                i === crumbs.length - 1
                  ? "font-medium text-foreground"
                  : "text-muted-foreground",
              )}
            >
              {name}
            </button>
          </Fragment>
        ))}
      </div>

      {/* 文件夹级整批标记(把当前文件夹下全部 docIds 一次性标) */}
      {docIds.length > 0 && (
        <div className="mb-3 flex flex-wrap items-center gap-2 rounded-lg bg-muted/30 px-3 py-2 text-xs">
          <span className="text-muted-foreground">
            整批标记本文件夹({docIds.length} 份):
          </span>
          <button
            type="button"
            onClick={() => marks.onMarkImportance(docIds, "重要")}
            className="rounded-md border border-amber-300 bg-amber-50 px-2 py-0.5 font-medium text-amber-700 hover:bg-amber-100"
          >
            ★ 重要
          </button>
          <button
            type="button"
            onClick={() => marks.onMarkImportance(docIds, "忽略")}
            className="rounded-md border border-stone-300 bg-stone-50 px-2 py-0.5 text-stone-500 hover:bg-stone-100"
          >
            忽略
          </button>
          <button
            type="button"
            onClick={() => marks.onMarkImportance(docIds, null)}
            className="rounded-md px-2 py-0.5 text-muted-foreground hover:bg-muted"
          >
            清除重要度
          </button>
          <span className="mx-1 text-border">|</span>
          {PARTY_SIDES.map((p) => (
            <button
              key={p}
              type="button"
              onClick={() => marks.onMarkPartySide(docIds, p, true)}
              className="rounded-md border border-sky-200 bg-sky-50 px-2 py-0.5 text-sky-700 hover:bg-sky-100"
            >
              +{p}
            </button>
          ))}
          <span className="mx-1 text-border">|</span>
          {EVIDENCE_ATTITUDES.map((a) => (
            <button
              key={a}
              type="button"
              onClick={() => marks.onMarkEvidenceAttitude(docIds, a)}
              className="rounded-md border border-emerald-200 bg-emerald-50 px-2 py-0.5 text-emerald-700 hover:bg-emerald-100"
            >
              {a}
            </button>
          ))}
        </div>
      )}

      {/* 卡片网格(Finder 图标视图风) */}
      {current.folders.length === 0 && current.files.length === 0 ? (
        <p className="py-6 text-center text-sm text-muted-foreground">这个文件夹是空的。</p>
      ) : (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(108px,1fr))] gap-3">
          {current.folders.map((f) => (
            <FolderTile
              key={f.name}
              node={f}
              onOpen={() => setPath([...crumbs, f.name])}
              onContextMenu={(e) =>
                openMenuFor(
                  e,
                  `文件夹「${f.name}」(${countFiles(f)} 份)`,
                  collectDocs(f).map((d) => d.id),
                )
              }
            />
          ))}
          {sortByImportance(current.files, marks.markMap).map((doc) => {
            const m = marks.markMap.get(doc.id) ?? EMPTY_MARK;
            return (
              <FileTile
                key={doc.id}
                doc={doc}
                mark={m}
                onOpen={() => onOpenDoc(doc)}
                onContextMenu={(e) =>
                  openMenuFor(e, docDisplayName(doc), [doc.id], m, doc)
                }
              />
            );
          })}
        </div>
      )}

      {menu && (
        <MarkContextMenu
          x={menu.x}
          y={menu.y}
          label={menu.label}
          mark={menu.mark}
          onImportance={(v) => marks.onMarkImportance(menu.docIds, v)}
          onParty={(v, en) => marks.onMarkPartySide(menu.docIds, v, en)}
          onEvidenceAttitude={(v) => marks.onMarkEvidenceAttitude(menu.docIds, v)}
          onSubmissionStage={(v) => marks.onMarkSubmissionStage(menu.docIds, v)}
          onCategory={
            menu.docIds.length === 1
              ? (v) => marks.onMarkCategory(menu.docIds[0], v)
              : undefined
          }
          onRename={menu.doc ? () => setRenaming(menu.doc!) : undefined}
          onClose={() => setMenu(null)}
        />
      )}

      {renaming && (
        <RenameDialog
          doc={renaming}
          onSubmit={(name) => marks.onRename(renaming.id, name)}
          onClose={() => setRenaming(null)}
        />
      )}
    </div>
  );
}

/** 文件夹卡片(方块):大文件夹图标 + 名 + 文件数,点击钻进去,右键整批标记。 */
function FolderTile({
  node,
  onOpen,
  onContextMenu,
}: {
  node: FileTreeNode;
  onOpen: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
}) {
  const count = countFiles(node);
  return (
    <button
      type="button"
      onClick={onOpen}
      onContextMenu={onContextMenu}
      title={`${node.name}(右键标记整个文件夹)`}
      className="group flex aspect-square flex-col items-center justify-center gap-1.5 rounded-xl border border-border bg-card p-2 text-center transition hover:border-sky-300 hover:bg-sky-50/50"
    >
      <Folder className="size-11 text-sky-500/80 group-hover:text-sky-500" />
      <span className="line-clamp-2 break-all text-xs font-medium text-foreground">
        {node.name}
      </span>
      <span className="text-[11px] text-muted-foreground">{count} 个文件</span>
    </button>
  );
}

/** 文件卡片(方块):类型图标 + 名 + 抽取状态点 + 标记角标,点击进板内查看器。 */
function FileTile({
  doc,
  mark,
  onOpen,
  onContextMenu,
}: {
  doc: Document;
  mark: DocMark;
  onOpen: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
}) {
  const isImg = /\.(png|jpe?g|webp|tiff?|bmp|gif|jp2)$/i.test(doc.filename);
  const Icon = doc.is_ai_artifact ? Sparkles : isImg ? ImageIcon : FileText;
  const status = doc.extraction_status;
  const ignored = mark.importance === "忽略";
  const important = mark.importance === "重要";
  const renamed = !!doc.display_name?.trim();
  // 有任一轴是 AI 建议(未被人工确认)→ 角标提示,引导右键确认/修改
  const aiSuggested =
    mark.importanceSource === "ai_suggest" ||
    mark.categorySource === "ai_suggest" ||
    mark.evidenceAttitudeSource === "ai_suggest" ||
    mark.submissionStageSource === "ai_suggest" ||
    doc.display_name_source === "ai_suggest";
  const bottomTags = [
    ...mark.parties,
    mark.evidenceAttitude,
    mark.submissionStage,
  ].filter(Boolean);
  return (
    <button
      type="button"
      onClick={onOpen}
      onContextMenu={onContextMenu}
      title={
        renamed
          ? `${docDisplayName(doc)}\n原文件名:${doc.filename}(右键可重命名/标记)`
          : `${doc.filename}(右键可重命名/标记)`
      }
      className={cn(
        "group relative flex aspect-square flex-col items-center justify-center gap-1.5 rounded-xl border p-2 text-center transition",
        important
          ? "border-amber-300 bg-amber-50/40"
          : "border-border bg-card hover:border-foreground/20 hover:bg-accent/40",
        ignored && "opacity-45",
      )}
    >
      {/* 左上角:重要星 */}
      {important && (
        <span className="absolute left-1.5 top-1.5 text-amber-500" title="重要">
          ★
        </span>
      )}
      {/* 右上角:抽取状态点 */}
      <span className="absolute right-1.5 top-1.5">
        {status === "done" ? (
          <CheckCircle2 className="size-3.5 text-emerald-600" />
        ) : status === "failed" ? (
          <CircleAlert className="size-3.5 text-destructive" />
        ) : status === "pending" || status === "processing" ? (
          <Loader2 className="size-3.5 animate-spin text-muted-foreground" />
        ) : null}
      </span>
      {/* AI 建议角标(右下) */}
      {aiSuggested && (
        <span
          className="absolute bottom-1.5 right-1.5 rounded bg-violet-100 px-1 text-[9px] font-medium text-violet-600"
          title="AI 建议,右键可确认或修改"
        >
          AI
        </span>
      )}
      <Icon className="size-10 text-muted-foreground group-hover:text-foreground" />
      <span className="line-clamp-2 break-all text-xs text-foreground">
        {docDisplayName(doc)}
      </span>
      {/* 底部:材料标签角标 */}
      {bottomTags.length > 0 && (
        <span className="line-clamp-1 text-[10px] text-sky-600">
          {bottomTags.join("·")}
        </span>
      )}
    </button>
  );
}

function DocRow({
  doc,
  highlight = false,
  marks,
  onOpen,
  onReveal,
  onDelete,
  onReextract,
  onReextractDewatermark,
}: {
  doc: Document;
  highlight?: boolean;
  marks?: MarkHandlers;
  onOpen: () => void;
  onReveal: () => void;
  onDelete?: () => void;
  onReextract?: () => void;
  onReextractDewatermark?: () => void;
}) {
  const Icon = doc.is_ai_artifact ? Sparkles : FileText;
  // 标记仅对源文件(非 AI 产物)显示
  const mark = marks && !doc.is_ai_artifact ? (marks.markMap.get(doc.id) ?? EMPTY_MARK) : null;

  return (
    <li
      className={cn(
        "group flex items-center gap-3 px-4 py-2.5 text-sm transition-colors hover:bg-accent/50",
        mark?.importance === "忽略" && "opacity-50",
      )}
    >
      <button
        type="button"
        onClick={onOpen}
        className="flex min-w-0 flex-1 items-center gap-3 text-left"
        title="文本类 → 在 App 内渲染;其他类型 → 用系统默认应用打开"
      >
        <Icon
          className={cn(
            "size-4 shrink-0",
            highlight ? "text-foreground" : "text-muted-foreground",
          )}
        />
        <div className="min-w-0 flex-1">
          <p
            className="truncate font-medium text-foreground"
            title={doc.display_name?.trim() ? `原文件名:${doc.filename}` : undefined}
          >
            {mark?.importance === "重要" && (
              <span className="mr-1 text-amber-500" title="重要">
                ★
              </span>
            )}
            {docDisplayName(doc)}
          </p>
          {doc.category && (
            <p className="mt-0.5 text-xs text-muted-foreground">{doc.category}</p>
          )}
        </div>
      </button>

      {/* Phase 3:标记控件(重要/忽略 + 原被告),hover 显示 */}
      {mark && marks && (
        <MarkControls
          mark={mark}
          onImportance={(v) => marks.onMarkImportance([doc.id], v)}
          onParty={(v, en) => marks.onMarkPartySide([doc.id], v, en)}
          onEvidenceAttitude={(v) => marks.onMarkEvidenceAttitude([doc.id], v)}
        />
      )}

      {/* V0.3 · 抽取状态(只对源文件,AI 摘要是产物不显) */}
      {!doc.is_ai_artifact && (
        <ExtractStatus
          doc={doc}
          onReextract={onReextract}
          onReextractDewatermark={onReextractDewatermark}
        />
      )}

      <span className="shrink-0 font-mono text-xs text-muted-foreground/70">
        {formatBytes(doc.size_bytes)}
      </span>

      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onReveal();
        }}
        className="shrink-0 rounded p-1 text-muted-foreground/60 opacity-0 transition-all hover:bg-accent hover:text-foreground group-hover:opacity-100"
        title="在 Finder 中显示"
        aria-label="在 Finder 中显示"
      >
        <FolderSearch className="size-3.5" />
      </button>

      {onDelete && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onDelete();
          }}
          className="shrink-0 rounded p-1 text-muted-foreground/60 opacity-0 transition-all hover:bg-destructive/10 hover:text-destructive group-hover:opacity-100"
          title="删除这条 AI 摘要(从材料列表移除)"
          aria-label="删除这条 AI 摘要"
        >
          <Trash2 className="size-3.5" />
        </button>
      )}
    </li>
  );
}

/**
 * Phase 3b · 整理视图:按 AI 归类分组的卡片板 + 「AI 自动整理」。
 * 每个分类一组方块卡片(组内重要置顶、忽略沉底);右键卡片标记/确认 AI 建议。
 */
function OrganizeView({
  docs,
  marks,
  onAiOrganize,
  organizing,
  onOpenDoc,
}: {
  docs: Document[];
  marks: MarkHandlers;
  onAiOrganize: () => void;
  organizing: boolean;
  onOpenDoc: (doc: Document) => void;
}) {
  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    label: string;
    doc: Document;
    mark: DocMark;
  } | null>(null);
  const [renaming, setRenaming] = useState<Document | null>(null);

  // 按分类分组(未分类垫底)
  const groups = useMemo(() => {
    const map = new Map<string, Document[]>();
    for (const d of docs) {
      const cat = marks.markMap.get(d.id)?.category ?? UNCATEGORIZED;
      const arr = map.get(cat);
      if (arr) arr.push(d);
      else map.set(cat, [d]);
    }
    return map;
  }, [docs, marks.markMap]);

  const order = [...CATEGORIES, UNCATEGORIZED].filter((c) => groups.has(c));
  const suggestedCount = docs.filter((d) => {
    const m = marks.markMap.get(d.id);
    return m?.importanceSource === "ai_suggest" || m?.categorySource === "ai_suggest";
  }).length;

  return (
    <div>
      {/* 头:AI 自动整理 */}
      <div className="mb-4 flex flex-wrap items-center gap-3 rounded-lg border border-violet-200 bg-violet-50/50 px-4 py-3">
        <button
          type="button"
          onClick={onAiOrganize}
          disabled={organizing}
          className={cn(
            "inline-flex shrink-0 items-center justify-center gap-1.5 overflow-hidden rounded-lg bg-violet-500 px-3 py-1.5 text-sm font-medium leading-none text-white transition hover:bg-violet-600",
            organizing && "cursor-wait opacity-60",
          )}
        >
          <span className="flex size-4 items-center justify-center">
            {organizing ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <span aria-hidden>🪄</span>
            )}
          </span>
          <span>{organizing ? "AI 整理中…" : "AI 自动整理"}</span>
        </button>
        <span className="text-xs text-muted-foreground">
          AI 通读材料给出「重要度 + 归类」建议(紫色 AI 角标),
          <b>右键卡片</b>可确认或改;你手动标的永远优先。
          {suggestedCount > 0 && ` 当前 ${suggestedCount} 份有 AI 建议待确认。`}
        </span>
      </div>

      {docs.length === 0 ? (
        <p className="text-sm text-muted-foreground">没有源文件。</p>
      ) : (
        <div className="space-y-5">
          {order.map((cat) => {
            const list = sortByImportance(groups.get(cat) ?? [], marks.markMap);
            return (
              <section key={cat}>
                <div className="mb-2 flex items-baseline gap-2">
                  <h3
                    className={cn(
                      "text-sm font-semibold",
                      cat === UNCATEGORIZED ? "text-muted-foreground" : "text-foreground",
                    )}
                  >
                    {cat}
                  </h3>
                  <span className="text-xs text-muted-foreground">{list.length}</span>
                </div>
                <div className="grid grid-cols-[repeat(auto-fill,minmax(108px,1fr))] gap-3">
                  {list.map((doc) => {
                    const m = marks.markMap.get(doc.id) ?? EMPTY_MARK;
                    return (
                      <FileTile
                        key={doc.id}
                        doc={doc}
                        mark={m}
                        onOpen={() => onOpenDoc(doc)}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          setMenu({
                            x: e.clientX,
                            y: e.clientY,
                            label: docDisplayName(doc),
                            doc,
                            mark: m,
                          });
                        }}
                      />
                    );
                  })}
                </div>
              </section>
            );
          })}
        </div>
      )}

      {menu && (
        <MarkContextMenu
          x={menu.x}
          y={menu.y}
          label={menu.label}
          mark={menu.mark}
          onImportance={(v) => marks.onMarkImportance([menu.doc.id], v)}
          onParty={(v, en) => marks.onMarkPartySide([menu.doc.id], v, en)}
          onCategory={(v) => marks.onMarkCategory(menu.doc.id, v)}
          onEvidenceAttitude={(v) => marks.onMarkEvidenceAttitude([menu.doc.id], v)}
          onSubmissionStage={(v) => marks.onMarkSubmissionStage([menu.doc.id], v)}
          onRename={() => setRenaming(menu.doc)}
          onClose={() => setMenu(null)}
        />
      )}

      {renaming && (
        <RenameDialog
          doc={renaming}
          onSubmit={(name) => marks.onRename(renaming.id, name)}
          onClose={() => setRenaming(null)}
        />
      )}
    </div>
  );
}

/** Phase 3:右键标记菜单(单文件 / 整文件夹)。固定定位在光标处,点别处/Esc/滚动关闭。 */
function MarkContextMenu({
  x,
  y,
  label,
  mark,
  onImportance,
  onParty,
  onCategory,
  onEvidenceAttitude,
  onSubmissionStage,
  onRename,
  onClose,
}: {
  x: number;
  y: number;
  label: string;
  /** 单文件传入其当前标记(显示勾选态);文件夹不传 */
  mark?: DocMark;
  onImportance: (v: Importance | null) => void;
  onParty: (v: string, enabled: boolean) => void;
  onEvidenceAttitude: (v: EvidenceAttitude | null) => void;
  onSubmissionStage: (v: SubmissionStage | null) => void;
  /** 仅单文件提供 → 显示「归类」分区(批量不支持改分类) */
  onCategory?: (v: string | null) => void;
  /** 仅单文件提供 → 显示「重命名」(打开重命名弹窗) */
  onRename?: () => void;
  onClose: () => void;
}) {
  const [referenceMaterialsEnabled] = useFeatureFlag("reference_materials");
  const visibleCategories = referenceMaterialsEnabled
    ? CATEGORIES
    : CATEGORIES.filter((category) => category !== "参考材料");
  const menuRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ left: x, top: y });

  useLayoutEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const padding = 8;
    const rect = menu.getBoundingClientRect();
    const left = Math.max(
      padding,
      Math.min(x, window.innerWidth - rect.width - padding),
    );
    const top = Math.max(
      padding,
      Math.min(y, window.innerHeight - rect.height - padding),
    );
    setPosition((current) =>
      current.left === left && current.top === top ? current : { left, top },
    );
  }, [x, y, onCategory, onEvidenceAttitude, onRename, onSubmissionStage]);

  useEffect(() => {
    const close = () => onClose();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    // 延一帧再挂,避免触发本次右键的 click 立刻关掉
    const t = window.setTimeout(() => {
      window.addEventListener("click", close);
      window.addEventListener("contextmenu", close);
      window.addEventListener("resize", close);
      window.addEventListener("keydown", onKey);
    }, 0);
    return () => {
      window.clearTimeout(t);
      window.removeEventListener("click", close);
      window.removeEventListener("contextmenu", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  const item =
    "flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs hover:bg-accent";

  return createPortal(
    <div
      ref={menuRef}
      style={position}
      className="fixed z-[120] max-h-[calc(100vh-16px)] w-48 overflow-y-auto rounded-lg border border-border bg-popover p-1 shadow-xl"
      onClick={(e) => e.stopPropagation()}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div className="truncate px-2 py-1 text-[11px] text-muted-foreground" title={label}>
        {label}
      </div>
      {onRename && (
        <>
          <button
            type="button"
            className={item}
            onClick={() => {
              onRename();
              onClose();
            }}
          >
            <Pencil className="size-3 text-sky-600" />
            重命名
          </button>
          <div className="my-1 border-t border-border" />
        </>
      )}
      <button
        type="button"
        className={item}
        onClick={() => {
          onImportance(mark?.importance === "重要" ? null : "重要");
          onClose();
        }}
      >
        <span className="text-amber-500">★</span>
        {mark?.importance === "重要" ? "取消重要" : "标为重要"}
      </button>
      <button
        type="button"
        className={item}
        onClick={() => {
          onImportance(mark?.importance === "忽略" ? null : "忽略");
          onClose();
        }}
      >
        <span className="text-stone-400">⊘</span>
        {mark?.importance === "忽略" ? "取消忽略" : "标为忽略"}
      </button>
      <div className="my-1 border-t border-border" />
      <div className="px-2 py-0.5 text-[11px] text-muted-foreground">当事人侧</div>
      {PARTY_SIDES.map((p) => {
        const on = mark?.parties.includes(p);
        return (
          <button
            key={p}
            type="button"
            className={item}
            onClick={() => {
              onParty(p, !on);
              onClose();
            }}
          >
            <span className="w-3 text-sky-600">{on ? "✓" : ""}</span>
            {p}
          </button>
        );
      })}
      {onCategory && (
        <>
          <div className="my-1 border-t border-border" />
          <div className="px-2 py-0.5 text-[11px] text-muted-foreground">归类</div>
          {visibleCategories.map((c) => {
            const on = mark?.category === c;
            return (
              <button
                key={c}
                type="button"
                className={item}
                onClick={() => {
                  onCategory(on ? null : c);
                  onClose();
                }}
              >
                <span className="w-3 text-emerald-600">{on ? "✓" : ""}</span>
                {c}
              </button>
            );
          })}
        </>
      )}
      <div className="my-1 border-t border-border" />
      <div className="px-2 py-0.5 text-[11px] text-muted-foreground">证据倾向</div>
      {EVIDENCE_ATTITUDES.map((a) => {
        const on = mark?.evidenceAttitude === a;
        return (
          <button
            key={a}
            type="button"
            className={item}
            onClick={() => {
              onEvidenceAttitude(on ? null : a);
              onClose();
            }}
          >
            <span className="w-3 text-emerald-600">{on ? "✓" : ""}</span>
            {a}
          </button>
        );
      })}
      <div className="my-1 border-t border-border" />
      <div className="px-2 py-0.5 text-[11px] text-muted-foreground">提交阶段</div>
      {SUBMISSION_STAGES.map((stage) => {
        const on = mark?.submissionStage === stage;
        return (
          <button
            key={stage}
            type="button"
            className={item}
            onClick={() => {
              onSubmissionStage(on ? null : stage);
              onClose();
            }}
          >
            <span className="w-3 text-indigo-600">{on ? "✓" : ""}</span>
            {stage}
          </button>
        );
      })}
    </div>,
    document.body,
  );
}

/**
 * 重命名弹窗:给文档起一个干净的板内显示名(纯元数据,**不动磁盘原件**)。
 * 预填当前显示名;Enter 提交,Esc / 点遮罩取消;「恢复原文件名」清回原始 filename。
 */
function RenameDialog({
  doc,
  onSubmit,
  onClose,
}: {
  doc: Document;
  onSubmit: (name: string | null) => void;
  onClose: () => void;
}) {
  const [value, setValue] = useState(docDisplayName(doc));
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const t = window.setTimeout(() => {
      inputRef.current?.focus();
      inputRef.current?.select();
    }, 0);
    return () => window.clearTimeout(t);
  }, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const submit = () => {
    const trimmed = value.trim();
    onSubmit(trimmed.length > 0 ? trimmed : null); // 空 → 清回原文件名
    onClose();
  };

  return createPortal(
    <div
      className="fixed inset-0 z-[130] flex items-center justify-center bg-black/40 px-4 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="w-full max-w-md rounded-xl border border-border bg-card p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="mb-1 flex items-center gap-2 text-sm font-semibold text-foreground">
          <Pencil className="size-4 text-sky-600" />
          重命名(板内显示名)
        </h3>
        <p className="mb-3 text-[11px] text-muted-foreground">
          只改看板里的显示名,<b className="text-foreground">不动磁盘上的原文件</b>
          。建议带类型前缀,如「证据-微信聊天记录」。
        </p>
        <input
          ref={inputRef}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              submit();
            }
          }}
          className="w-full rounded-lg border border-border bg-background px-3 py-2 text-sm text-foreground outline-none focus:border-sky-400"
          placeholder="如:证据-XX买卖合同"
        />
        <p className="mt-2 truncate text-[11px] text-muted-foreground" title={doc.filename}>
          原文件名:{doc.filename}
        </p>
        <div className="mt-4 flex items-center justify-between gap-2">
          <button
            type="button"
            onClick={() => {
              onSubmit(null);
              onClose();
            }}
            className="rounded-lg px-2 py-1.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            恢复原文件名
          </button>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={onClose}
              className="rounded-lg border border-border bg-background px-3 py-1.5 text-sm text-foreground hover:bg-muted"
            >
              取消
            </button>
            <button
              type="button"
              onClick={submit}
              className="rounded-lg bg-sky-500 px-4 py-1.5 text-sm font-medium text-white hover:bg-sky-600"
            >
              确定
            </button>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}

/** Phase 3:文件行内标记控件 —— 重要/忽略(单选,再点取消) + 原/被/三(多选)。 */
function MarkControls({
  mark,
  onImportance,
  onParty,
  onEvidenceAttitude,
}: {
  mark: DocMark;
  onImportance: (v: Importance | null) => void;
  onParty: (v: string, enabled: boolean) => void;
  onEvidenceAttitude: (v: EvidenceAttitude | null) => void;
}) {
  return (
    <div
      className="flex shrink-0 items-center gap-0.5"
      onClick={(e) => e.stopPropagation()}
    >
      <button
        type="button"
        title="标为重要(再点取消)"
        onClick={() => onImportance(mark.importance === "重要" ? null : "重要")}
        className={cn(
          "rounded px-1 text-sm leading-none transition-colors",
          mark.importance === "重要"
            ? "text-amber-500"
            : "text-muted-foreground/40 hover:text-amber-500",
        )}
      >
        ★
      </button>
      <button
        type="button"
        title="标为忽略(再点取消)"
        onClick={() => onImportance(mark.importance === "忽略" ? null : "忽略")}
        className={cn(
          "rounded px-1 text-[11px] leading-none transition-colors",
          mark.importance === "忽略"
            ? "font-medium text-stone-600"
            : "text-muted-foreground/40 hover:text-stone-600",
        )}
      >
        忽略
      </button>
      <span className="mx-0.5 text-border">·</span>
      {PARTY_SIDES.map((p) => {
        const on = mark.parties.includes(p);
        return (
          <button
            key={p}
            type="button"
            title={`标 ${p}(再点取消)`}
            onClick={() => onParty(p, !on)}
            className={cn(
              "rounded px-1 text-[11px] leading-none transition-colors",
              on
                ? "bg-sky-100 font-medium text-sky-700"
                : "text-muted-foreground/40 hover:text-sky-600",
            )}
          >
            {p[0]}
          </button>
        );
      })}
      <span className="mx-0.5 text-border">·</span>
      {EVIDENCE_ATTITUDES.map((a) => {
        const on = mark.evidenceAttitude === a;
        return (
          <button
            key={a}
            type="button"
            title={`证据倾向:${a}(再点取消)`}
            onClick={() => onEvidenceAttitude(on ? null : a)}
            className={cn(
              "rounded px-1 text-[11px] leading-none transition-colors",
              on
                ? "bg-emerald-100 font-medium text-emerald-700"
                : "text-muted-foreground/40 hover:text-emerald-600",
            )}
          >
            {a[0]}
          </button>
        );
      })}
    </div>
  );
}

/**
 * V0.3 · 源文件抽取状态指示 + 重抽按钮。
 *   done → 绿勾;failed → 红「抽取失败」+ 重抽;pending/processing → 抽取中;skipped → 跳过。
 * 重抽按钮:失败时常显,其余 hover 显示(允许手动强制重抽)。
 */
function ExtractStatus({
  doc,
  onReextract,
  onReextractDewatermark,
}: {
  doc: Document;
  onReextract?: () => void;
  onReextractDewatermark?: () => void;
}) {
  const status = doc.extraction_status;
  const isPdf = /\.pdf$/i.test(doc.filename);
  const onDw = doc.is_ai_artifact ? undefined : onReextractDewatermark;
  const reBtn = onReextract ? (
    <button
      type="button"
      onClick={(e) => {
        e.stopPropagation();
        onReextract();
      }}
      title="重新抽取这份文档(重跑 OCR/识别;PDF 会再用云端 OCR 积分)"
      aria-label="重新抽取"
      className={cn(
        "shrink-0 rounded p-1 transition-all hover:bg-accent hover:text-foreground",
        status === "failed"
          ? "text-destructive/80 opacity-100"
          : "text-muted-foreground/60 opacity-0 group-hover:opacity-100",
      )}
    >
      <RefreshCw className="size-3.5" />
    </button>
  ) : null;
  // 去水印重识别:仅 PDF 显示(带水印的工商调档件常见;图片用普通重识别即可)
  const dwBtn =
    onDw && isPdf ? (
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onDw();
        }}
        title="去水印重新识别:带大幅水印的工商调档件/章程改用 PP-OCRv6(纯文字)+ 去水印,关键字段更准。已被 ocr_backend_override 标记的会一直走去水印,普通「重新识别」可恢复"
        aria-label="去水印重新识别"
        className={cn(
          "shrink-0 rounded p-1 transition-all hover:bg-accent hover:text-foreground",
          doc.ocr_backend_override
            ? "text-sky-600 opacity-100"
            : "text-muted-foreground/60 opacity-0 group-hover:opacity-100",
        )}
      >
        <Droplets className="size-3.5" />
      </button>
    ) : null;

  if (status === "done") {
    return (
      <span className="flex shrink-0 items-center gap-0.5">
        <CheckCircle2 className="size-4 text-emerald-600" aria-label="已抽取" />
        {dwBtn}
        {reBtn}
      </span>
    );
  }
  if (status === "failed") {
    return (
      <span className="flex shrink-0 items-center gap-1">
        <span
          className="inline-flex items-center gap-1 rounded bg-destructive/10 px-1.5 py-0.5 text-label font-medium text-destructive"
          title="这份文档抽取失败(OCR 或字段识别出错),点右边按钮重抽"
        >
          <CircleAlert className="size-3" />
          抽取失败
        </span>
        {dwBtn}
        {reBtn}
      </span>
    );
  }
  if (status === "pending" || status === "processing") {
    return (
      <span className="flex shrink-0 items-center gap-1 text-label text-muted-foreground">
        <Loader2 className="size-3.5 animate-spin" />
        抽取中
      </span>
    );
  }
  // skipped 及其他
  return (
    <span className="flex shrink-0 items-center gap-0.5">
      <span
        className="text-label text-muted-foreground/60"
        title="律所规范/程序材料,按设计不抽取正文(仍可在 chat 里读全文);如需可手动重抽"
      >
        跳过
      </span>
      {dwBtn}
      {reBtn}
    </span>
  );
}

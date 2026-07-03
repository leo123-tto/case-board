/**
 * V0.2 D6 · `<CITATIONS>` 引用卡 — 按 type 分组渲染。
 *
 * 用户视角:AI 回答末尾收起一张"引用 (N) ▾",点开看法条/案例/文档/KB 来源清单,
 *           点条目可以跳到原文(文档 → Preview/Word,法条/案例 → 浏览器搜索)。
 * 系统视角:后端 `chat::citations::parse` 解析 `<CITATIONS>` JSON block 后,
 *           前端把这一段从 markdown 正文里替换为本组件渲染。
 *
 * 设计要点(详 § 20 D6 acceptance):
 *   - 按 type 分四组:📜 法条 / ⚖️ 案例 / 📄 文档 / 📚 本地 KB
 *   - 同组内按 ref 升序
 *   - `verified=false` (LLM 编造的 doc 引用) 单独标 ⚠️ + 提示
 *   - 跳转:
 *     - `doc` / `kb_local` → `open_in_default_app(source)`(source 是绝对路径)
 *     - `law` / `case` → 不直接跳(没有标准 URL),复制到剪贴板让用户自己去搜
 *   - 整卡默认**折叠**,点击 header 展开;给一个 ref 数量 badge
 *   - 空 citations → 返回 null
 */

import { useMemo, useState } from "react";
import {
  AlertTriangle,
  BookOpen,
  ChevronDown,
  ChevronRight,
  Copy,
  ExternalLink,
  FileText,
  Gavel,
  Scale,
} from "lucide-react";

import { openInDefaultApp, openUrl } from "@/lib/api";
import type { Citation } from "@/lib/types";
import { cn } from "@/lib/utils";

interface Props {
  citations: Citation[];
  /** 默认是否展开;一般 LLM 回答完后默认折叠,用户主动点开 */
  defaultOpen?: boolean;
}

interface Group {
  key: string;
  label: string;
  icon: React.ReactNode;
  items: Citation[];
}

export function CitationsCard({ citations, defaultOpen = false }: Props) {
  const [open, setOpen] = useState(defaultOpen);
  const [copiedRef, setCopiedRef] = useState<number | null>(null);

  const groups = useMemo<Group[]>(() => {
    const byKind = new Map<string, Citation[]>();
    for (const c of citations) {
      const k = c.type || "other";
      const arr = byKind.get(k) ?? [];
      arr.push(c);
      byKind.set(k, arr);
    }
    // 同组内按 ref 升序
    for (const arr of byKind.values()) {
      arr.sort((a, b) => a.ref - b.ref);
    }
    const ordered: Group[] = [];
    pushGroup(ordered, byKind, "law", "法条", <Scale className="size-3.5" />);
    pushGroup(ordered, byKind, "case", "案例", <Gavel className="size-3.5" />);
    pushGroup(ordered, byKind, "doc", "本案文档", <FileText className="size-3.5" />);
    pushGroup(ordered, byKind, "kb_local", "本地知识库", <BookOpen className="size-3.5" />);
    pushGroup(ordered, byKind, "web", "网页", <ExternalLink className="size-3.5" />);
    // 兜底:其他未识别 type
    for (const [k, arr] of byKind) {
      if (!["law", "case", "doc", "kb_local", "web"].includes(k)) {
        ordered.push({ key: k, label: k, icon: <BookOpen className="size-3.5" />, items: arr });
      }
    }
    return ordered;
  }, [citations]);

  if (citations.length === 0) return null;

  const unverifiedCount = citations.filter((c) => !c.verified).length;

  const handleCopy = async (c: Citation) => {
    try {
      await navigator.clipboard.writeText(c.source);
      setCopiedRef(c.ref);
      window.setTimeout(() => setCopiedRef((v) => (v === c.ref ? null : v)), 1500);
    } catch {
      /* 复制失败静默,Tauri WebView 通常 clipboard 可用 */
    }
  };

  const handleOpenPath = async (c: Citation) => {
    try {
      await openInDefaultApp(c.source);
    } catch (e) {
      console.error("[CitationsCard] open_in_default_app failed", e);
    }
  };

  const handleOpenUrl = async (c: Citation) => {
    const url = c.url || (c.source.startsWith("http") ? c.source : "");
    if (!url) return;
    try {
      await openUrl(url);
    } catch (e) {
      console.error("[CitationsCard] open_url failed", e);
    }
  };

  return (
    <div className="my-2 rounded-md border border-border/60 bg-muted/20 text-xs">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-accent/40"
      >
        {open ? (
          <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="size-3.5 shrink-0 text-muted-foreground" />
        )}
        <span className="font-medium">📑 引用 ({citations.length})</span>
        {unverifiedCount > 0 && (
          <span className="inline-flex items-center gap-1 rounded-sm bg-amber-500/15 px-1.5 py-0.5 text-caption font-medium text-amber-700 dark:text-amber-300">
            <AlertTriangle className="size-3" />
            {unverifiedCount} 条待核实
          </span>
        )}
        <span className="ml-auto text-caption text-muted-foreground">
          {groups.map((g) => `${g.label} ${g.items.length}`).join(" / ")}
        </span>
      </button>

      {open && (
        <div className="border-t border-border/40">
          {groups.map((g) => (
            <div key={g.key} className="px-3 py-2">
              <div className="mb-1 flex items-center gap-1.5 text-label font-medium text-muted-foreground">
                {g.icon}
                <span>
                  {g.label} ({g.items.length})
                </span>
              </div>
              <ul className="space-y-1">
                {g.items.map((c) => (
                  <CitationRow
                    key={c.ref}
                    citation={c}
                    copied={copiedRef === c.ref}
                    onCopy={() => handleCopy(c)}
                    onOpenPath={() => handleOpenPath(c)}
                    onOpenUrl={() => handleOpenUrl(c)}
                  />
                ))}
              </ul>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

interface RowProps {
  citation: Citation;
  copied: boolean;
  onCopy: () => void;
  onOpenPath: () => void;
  onOpenUrl: () => void;
}

function CitationRow({ citation: c, copied, onCopy, onOpenPath, onOpenUrl }: RowProps) {
  // doc / kb_local 才有"开原文"按钮(source 是绝对路径)
  const canOpenPath = c.type === "doc" || c.type === "kb_local";
  const canOpenUrl = c.type === "web" && Boolean(c.url || c.source.startsWith("http"));

  return (
    <li
      className={cn(
        "rounded border border-border/40 bg-background/60 px-2 py-1.5",
        !c.verified && "border-amber-500/40 bg-amber-500/5",
      )}
    >
      <div className="flex items-start gap-2">
        <span className="shrink-0 rounded bg-accent px-1.5 py-0.5 text-caption font-mono">
          [{c.ref}]
        </span>
        <div className="min-w-0 flex-1">
          <div className="break-all font-medium">{c.source}</div>
          {c.court && (
            <div className="text-caption text-muted-foreground">{c.court}</div>
          )}
          {c.url && (
            <div className="break-all text-caption text-muted-foreground">{c.url}</div>
          )}
          {c.quote && (
            <blockquote className="mt-1 border-l-2 border-border/60 pl-2 text-label text-muted-foreground">
              "{c.quote}"
            </blockquote>
          )}
          {!c.verified && (
            <div className="mt-1 flex items-center gap-1 text-caption text-amber-700 dark:text-amber-300">
              <AlertTriangle className="size-3" />
              <span>原文中未找到引述句,可能为 LLM 编造,请核实</span>
            </div>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {canOpenPath || canOpenUrl ? (
            <button
              type="button"
              onClick={canOpenUrl ? onOpenUrl : onOpenPath}
              title={canOpenUrl ? "用系统浏览器打开网页" : "用系统默认应用打开原文"}
              className="grid place-items-center rounded-sm p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
            >
              <ExternalLink className="size-3" />
            </button>
          ) : (
            <button
              type="button"
              onClick={onCopy}
              title="复制来源,自行去检索"
              className="grid place-items-center rounded-sm p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
            >
              <Copy className="size-3" />
            </button>
          )}
        </div>
      </div>
      {copied && (
        <div className="mt-1 text-caption text-emerald-600">已复制到剪贴板</div>
      )}
    </li>
  );
}

function pushGroup(
  out: Group[],
  bag: Map<string, Citation[]>,
  key: string,
  label: string,
  icon: React.ReactNode,
) {
  const items = bag.get(key);
  if (items && items.length > 0) {
    out.push({ key, label, icon, items });
    bag.delete(key);
  }
}

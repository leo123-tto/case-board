/**
 * DiffReview —— AI 改文书的「diff 审阅」视图(ADR-0003 Phase 2)。
 *
 * AI 用 edit_artifact 改了文书后,不直接生效:进本视图,把「改前 vs 改后」按 token 级 diff
 * 渲染出来 —— 新增=浅绿底、删除=灰色删除线,每一处可单独「接受/拒绝」(拒绝=还原旧文本)。
 * 点「应用」才把(按各处选择拼回的)最终正文落盘;「取消」则全部还原(磁盘已 revert 成旧版)。
 *
 * 纯展示 + 选择,不碰 Milkdown/ProseMirror 内部(编辑器是封死边界);最终正文交回父组件落盘。
 */
import { useMemo, useState } from "react";
import { Check, RotateCcw, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { countChanges, reconstruct, type DiffPart } from "@/lib/textDiff";

interface Props {
  /** 改前→改后 的 diff 段 */
  parts: DiffPart[];
  /** 点「应用」:把按各处选择拼回的最终正文交回父组件落盘。rejected=被拒绝的处数 */
  onApply: (finalBody: string, rejected: number) => void;
  /** 点「取消」:放弃 AI 这次改动(磁盘已还原成旧版) */
  onCancel: () => void;
}

export function DiffReview({ parts, onApply, onCancel }: Props) {
  const total = countChanges(parts);
  // 每处改动的接受状态,默认全接受(AI 提议的,用户拒掉不想要的)
  const [accepts, setAccepts] = useState<boolean[]>(() =>
    Array.from({ length: total }, () => true),
  );

  // part 下标 → 第几个 change(渲染时取 accepts[ord])
  const changeOrd = useMemo(() => {
    let c = -1;
    return parts.map((p) => (p.kind === "change" ? ++c : -1));
  }, [parts]);

  const acceptedCount = accepts.filter(Boolean).length;
  const rejectedCount = total - acceptedCount;

  const toggle = (ci: number) =>
    setAccepts((cur) => cur.map((v, i) => (i === ci ? !v : v)));
  const setAll = (v: boolean) => setAccepts(accepts.map(() => v));

  return (
    <div className="flex h-full min-h-0 flex-col bg-background">
      {/* 审阅工具栏 */}
      <header className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border bg-sky-50 px-4 py-2.5 dark:bg-sky-950/30">
        <span className="text-sm font-medium text-foreground">
          审阅 AI 修改 · 共 {total} 处
        </span>
        <span className="text-label text-muted-foreground">
          (接受 {acceptedCount} / 拒绝 {rejectedCount})
        </span>
        <span className="flex-1" />
        <Button
          size="sm"
          variant="ghost"
          onClick={() => setAll(true)}
          disabled={total === 0 || acceptedCount === total}
          title="全部接受"
        >
          全部接受
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => setAll(false)}
          disabled={total === 0 || rejectedCount === total}
          title="全部拒绝(还原)"
        >
          全部拒绝
        </Button>
        <Button size="sm" variant="outline" onClick={onCancel} title="放弃 AI 这次改动">
          <X className="size-3.5" />
          取消
        </Button>
        <Button
          size="sm"
          onClick={() => onApply(reconstruct(parts, accepts), rejectedCount)}
          title="应用:接受的改动落盘,拒绝的还原"
        >
          <Check className="size-3.5" />
          应用
        </Button>
      </header>

      {/* 提示条 */}
      <div className="shrink-0 border-b border-border bg-background px-4 py-1.5 text-label text-muted-foreground">
        <span className="rounded bg-green-100 px-1 text-green-900 dark:bg-green-900/40 dark:text-green-200">
          浅绿
        </span>{" "}
        = AI 新增 ·{" "}
        <span className="rounded bg-gray-100 px-1 text-gray-400 line-through dark:bg-gray-800">
          删除线
        </span>{" "}
        = AI 删除。点每处的 ✓/↩ 单独接受或拒绝;拒绝即还原该处旧文本。
      </div>

      {/* diff 正文(像文档一样阅读,改动处内联高亮 + 逐处开关) */}
      <div className="min-h-0 flex-1 overflow-auto px-6 py-5">
        <div className="mx-auto max-w-3xl whitespace-pre-wrap break-words font-serif text-[15px] leading-loose text-foreground">
          {parts.map((part, idx) => {
            if (part.kind === "equal") {
              return <span key={idx}>{part.text}</span>;
            }
            const ci = changeOrd[idx];
            const accepted = accepts[ci];
            return (
              <span
                key={idx}
                className="relative mx-0.5 inline rounded-sm ring-1 ring-inset ring-border/60"
              >
                {part.del ? (
                  <span
                    className={cn(
                      "rounded-sm px-0.5",
                      accepted
                        ? "bg-gray-100 text-gray-400 line-through dark:bg-gray-800"
                        : "bg-amber-50 text-foreground dark:bg-amber-950/30",
                    )}
                  >
                    {part.del}
                  </span>
                ) : null}
                {part.add ? (
                  <span
                    className={cn(
                      "rounded-sm px-0.5",
                      accepted
                        ? "bg-green-100 text-green-900 dark:bg-green-900/40 dark:text-green-200"
                        : "bg-gray-50 text-gray-400 line-through dark:bg-gray-900",
                    )}
                  >
                    {part.add}
                  </span>
                ) : null}
                {/* 逐处接受/拒绝开关 */}
                <button
                  type="button"
                  onClick={() => toggle(ci)}
                  title={
                    accepted ? "点此拒绝(还原旧文本)" : "点此接受(采用 AI 修改)"
                  }
                  className={cn(
                    "ml-0.5 inline-flex size-4 translate-y-0.5 items-center justify-center rounded align-middle text-background transition-colors",
                    accepted
                      ? "bg-green-600 hover:bg-green-700"
                      : "bg-gray-400 hover:bg-gray-500",
                  )}
                >
                  {accepted ? (
                    <Check className="size-3" />
                  ) : (
                    <RotateCcw className="size-3" />
                  )}
                </button>
              </span>
            );
          })}
        </div>
      </div>
    </div>
  );
}

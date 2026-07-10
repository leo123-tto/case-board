/**
 * 非诉模块入口(2026-06-18 卡片化重构)。
 *
 * 非诉 tab 从「直接是合同审查」改成**卡片式功能入口**(照搬 `法律工具` 页 ToolsModule 模式):
 *   - 卡片网格(grid)→ 点一张卡进入该工具的详情视图 → 顶部「返回」回网格。
 *   - 当前:合同审查 + 合同起草。给后续非诉功能留位。
 *
 * 本模块完全独立 —— 不依赖诉讼模块的任何 state、组件、IPC。
 */

import { useState } from "react";
import { ArrowLeft, FileSignature, ShieldCheck } from "lucide-react";

import { BetaBadge } from "@/components/BetaBadge";
import { LegalToolCard } from "@/modules/tools/components/LegalToolCard";
import { ContractReviewTool } from "./ContractReviewTool";
import { ContractDraftTool } from "./ContractDraftTool";

type TransactionToolId = "contract_review" | "contract_draft";

export function TransactionModule() {
  const [activeTool, setActiveTool] = useState<TransactionToolId | null>(null);

  // 详情视图:合同审查
  if (activeTool === "contract_review") {
    return (
      <main className="app-shell flex h-full w-full flex-col">
        <header className="app-subheader flex shrink-0 items-center gap-3 border-b px-4 py-2.5 sm:px-6">
          <button
            type="button"
            onClick={() => setActiveTool(null)}
            className="flex items-center gap-1 rounded-md px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" />
            返回非诉
          </button>
          <h2 className="flex items-center gap-2 text-sm font-medium text-foreground">
            <ShieldCheck className="size-4 text-sky-600 dark:text-sky-400" />
            合同审查
            <BetaBadge />
          </h2>
        </header>
        <div className="flex-1 overflow-auto">
          <div className="app-page-enter mx-auto max-w-4xl space-y-5 px-4 py-5 sm:px-6 xl:px-8 xl:py-6">
            <p className="text-[11px] text-muted-foreground/70">
              审查方法论参考杨卫薪律师 contract-copilot(CC BY-NC),prompt / 引擎 /
              意见书均由本系统自建。
            </p>
            <ContractReviewTool />
          </div>
        </div>
      </main>
    );
  }

  // 详情视图:合同起草
  if (activeTool === "contract_draft") {
    return (
      <main className="app-shell flex h-full w-full flex-col">
        <header className="app-subheader flex shrink-0 items-center gap-3 border-b px-4 py-2.5 sm:px-6">
          <button
            type="button"
            onClick={() => setActiveTool(null)}
            className="flex items-center gap-1 rounded-md px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" />
            返回非诉
          </button>
          <h2 className="flex items-center gap-2 text-sm font-medium text-foreground">
            <FileSignature className="size-4 text-sky-600 dark:text-sky-400" />
            合同起草
            <BetaBadge />
          </h2>
        </header>
        <div className="flex-1 overflow-auto">
          <div className="app-page-enter mx-auto max-w-4xl px-4 py-5 sm:px-6 xl:px-8 xl:py-6">
            <ContractDraftTool />
          </div>
        </div>
      </main>
    );
  }

  // 卡片网格(默认)
  return (
    <main className="app-shell flex h-full w-full flex-col">
      <div className="flex-1 overflow-auto">
        <div className="app-page-enter mx-auto max-w-4xl space-y-6 px-4 py-6 sm:px-6 xl:px-8 xl:py-8">
          <header>
            <h1 className="text-lg font-semibold tracking-tight text-foreground">
              非诉
            </h1>
            <p className="mt-1 text-xs text-muted-foreground">
              合同审查与起草。
            </p>
          </header>

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <LegalToolCard
              icon={ShieldCheck}
              title="合同审查"
              desc="扫描合同风险，生成意见书与修订版 Word。"
              onClick={() => setActiveTool("contract_review")}
            />
            <LegalToolCard
              icon={FileSignature}
              title="合同起草"
              desc="补全交易要素，生成可导出的合同草案。"
              badge="Beta"
              onClick={() => setActiveTool("contract_draft")}
            />
          </div>
        </div>
      </div>
    </main>
  );
}

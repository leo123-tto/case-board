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
      <main className="flex h-full w-full flex-col bg-background">
        <header className="flex shrink-0 items-center gap-3 border-b border-border bg-card/50 px-6 py-2.5">
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
          <div className="mx-auto max-w-4xl space-y-5 px-8 py-6">
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
      <main className="flex h-full w-full flex-col bg-background">
        <header className="flex shrink-0 items-center gap-3 border-b border-border bg-card/50 px-6 py-2.5">
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
          <div className="mx-auto max-w-4xl px-8 py-6">
            <ContractDraftTool />
          </div>
        </div>
      </main>
    );
  }

  // 卡片网格(默认)
  return (
    <main className="flex h-full w-full flex-col bg-background">
      <div className="flex-1 overflow-auto">
        <div className="mx-auto max-w-4xl space-y-5 px-8 py-6">
          <header>
            <h1 className="text-lg font-semibold tracking-tight text-foreground">
              非诉
            </h1>
            <p className="mt-1 text-xs text-muted-foreground">
              合同与非诉业务工具。点卡片进入对应功能。
            </p>
          </header>

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <LegalToolCard
              icon={ShieldCheck}
              title="合同审查"
              desc="上传合同 .docx → AI 三层扫描 → 分级风险清单 + 审查意见书 / 修订批注版 Word。"
              onClick={() => setActiveTool("contract_review")}
            />
            <LegalToolCard
              icon={FileSignature}
              title="合同起草"
              desc="描述交易需求 → AI 按三观四步法识别类型、引导补全要素、生成合同草案,可导出 Word。"
              badge="Beta"
              onClick={() => setActiveTool("contract_draft")}
            />
          </div>
        </div>
      </div>
    </main>
  );
}

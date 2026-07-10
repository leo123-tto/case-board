/**
 * 工具模块入口。
 *
 * 三态:
 *   - `activeTool === null`:工具列表态
 *   - `activeTool === <id>` 且对应工具有 React 组件:React 原生视图
 *   - `activeTool === <automation id>`:知识共享 / 日程同步 / 法院立案等独立工具视图
 *
 * 法律计算器已统一为 React 原生 UI:
 *   - 数字大写转换器(NumberConverter)
 *   - 天数计算器(DateCalculator)
 *   - 律师费计算器(LawyerFeeCalculator)
 *   - 诉讼费计算器(LitigationFeeCalculator)
 *   - 利息 / 执行款计算器(InterestCalculator)
 *   - 交通事故赔偿计算器(TrafficAccidentCompensationCalculator)
 *   - 劳动解除赔偿计算器(LaborSeveranceCalculator)
 */

import { useEffect, useState } from "react";

import type { InterestPrefill } from "./calculators/InterestCalculator";
import {
  ArrowLeft,
  BarChart3,
  Briefcase,
  Calculator,
  Calendar,
  CalendarClock,
  Car,
  Combine,
  FileOutput,
  Gavel,
  Hash,
  ListChecks,
  Scale,
  Share2,
  TrendingUp,
  Truck,
} from "lucide-react";

import { DateCalculator } from "./calculators/DateCalculator";
import { InterestCalculator } from "./calculators/InterestCalculator";
import { LaborSeveranceCalculator } from "./calculators/LaborSeveranceCalculator";
import { LawyerFeeCalculator } from "./calculators/LawyerFeeCalculator";
import { LitigationFeeCalculator } from "./calculators/LitigationFeeCalculator";
import { NumberConverter } from "./calculators/NumberConverter";
import { TrafficAccidentCompensationCalculator } from "./calculators/TrafficAccidentCompensationCalculator";
import { KbShareTool } from "./KbShareTool";
import { CaseBundleTool } from "./CaseBundleTool";
import { CourtSmsTool } from "./CourtSmsTool";
import { CourierTool } from "./CourierTool";
import { FeishuCalendarTool } from "./FeishuCalendarTool";
import { CourtFilingTool } from "./CourtFilingTool";
import { ElementConvertWorkbench } from "./ElementConvertWorkbench";
import { LawyerInsightsTool } from "./LawyerInsightsTool";
import { TickTickPanel } from "@/components/TickTickPanel";
import { LegalToolCard } from "./components/LegalToolCard";

type LegalToolId =
  | "number"
  | "daycal"
  | "fee"
  | "legalfee"
  | "interest"
  | "traffic"
  | "labor"
  | "kbshare"
  | "casebundle"
  | "lawyerinsights"
  | "courtsms"
  | "courier"
  | "ticktick"
  | "feishu"
  | "courtfiling"
  | "elementconvert";

interface LegalTool {
  id: LegalToolId;
  title: string;
  desc: string;
  icon: typeof Calculator;
}

const LEGAL_TOOLS: LegalTool[] = [
  {
    id: "number",
    title: "数字大写转换器",
    desc: "数字转中文大写金额，支持角分。",
    icon: Hash,
  },
  {
    id: "daycal",
    title: "天数计算器",
    desc: "计算日期间隔，或按天数推算日期。",
    icon: Calendar,
  },
  {
    id: "fee",
    title: "律师费计算器",
    desc: "按案件类型与标的额估算律师费。",
    icon: Calculator,
  },
  {
    id: "legalfee",
    title: "诉讼费计算器",
    desc: "计算诉讼费与财产保全费。",
    icon: Scale,
  },
  {
    id: "interest",
    title: "利息 / 执行款计算器",
    desc: "计算 LPR 利息、执行款、还款抵扣和迟延履行利息。",
    icon: TrendingUp,
  },
  {
    id: "traffic",
    title: "交通事故赔偿计算器",
    desc: "计算伤残、死亡、被扶养人等交通事故赔偿。",
    icon: Car,
  },
  {
    id: "labor",
    title: "劳动解除赔偿计算器",
    desc: "计算 N、N+1、2N、封顶与未休年假。",
    icon: Briefcase,
  },
];

export function ToolsModule({
  initialTool,
  interestPrefill,
  routeNonce,
}: {
  /** 2026-05-25:从执行模块「→ 算剩余执行款」跳过来时,自动打开对应工具 */
  initialTool?: LegalToolId | null;
  /** 给 InterestCalculator 的预填(本金 / 起算日 / 备注)*/
  interestPrefill?: InterestPrefill | null;
  /** 自增 nonce:即使 initialTool 不变也强制重新打开(重复跳转用) */
  routeNonce?: number;
}) {
  const [activeTool, setActiveTool] = useState<LegalToolId | null>(initialTool ?? null);
  // 父组件切换 initialTool(或 routeNonce)时同步
  useEffect(() => {
    if (initialTool) setActiveTool(initialTool);
  }, [initialTool, routeNonce]);
  const tool = activeTool
    ? LEGAL_TOOLS.find((t) => t.id === activeTool) ?? null
    : null;

  // ──────────── 知识库共享(独立于计算器,自带视图) ────────────
  if (activeTool === "kbshare") {
    return (
      <main className="app-shell flex h-full w-full flex-col">
        <header className="app-subheader flex shrink-0 items-center gap-3 border-b px-4 py-2.5 sm:px-6">
          <button
            type="button"
            onClick={() => setActiveTool(null)}
            className="inline-flex items-center gap-1 rounded px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" />
            返回工具列表
          </button>
          <span className="text-muted-foreground/40">·</span>
          <h2 className="text-sm font-medium text-foreground">本地知识库共享</h2>
        </header>
        <div className="min-h-0 flex-1 overflow-auto">
          <div className="app-page-enter mx-auto max-w-3xl px-4 py-5 sm:px-6 xl:py-6">
            <KbShareTool />
          </div>
        </div>
      </main>
    );
  }

  // ──────────── 案件资料包合并(双人办案,自带视图) ────────────
  if (activeTool === "casebundle") {
    return (
      <main className="app-shell flex h-full w-full flex-col">
        <header className="app-subheader flex shrink-0 items-center gap-3 border-b px-4 py-2.5 sm:px-6">
          <button
            type="button"
            onClick={() => setActiveTool(null)}
            className="inline-flex items-center gap-1 rounded px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" />
            返回工具列表
          </button>
          <span className="text-muted-foreground/40">·</span>
          <h2 className="text-sm font-medium text-foreground">案件资料包合并(双人办案)</h2>
        </header>
        <div className="min-h-0 flex-1 overflow-auto">
          <div className="app-page-enter mx-auto max-w-3xl px-4 py-5 sm:px-6 xl:py-6">
            <CaseBundleTool />
          </div>
        </div>
      </main>
    );
  }

  // ──────────── 办案画像(案件数据分析,自带视图) ────────────
  if (activeTool === "lawyerinsights") {
    return (
      <main className="app-shell flex h-full w-full flex-col">
        <header className="app-subheader flex shrink-0 items-center gap-3 border-b px-4 py-2.5 sm:px-6">
          <button
            type="button"
            onClick={() => setActiveTool(null)}
            className="inline-flex items-center gap-1 rounded px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" />
            返回工具列表
          </button>
          <span className="text-muted-foreground/40">·</span>
          <h2 className="text-sm font-medium text-foreground">办案画像</h2>
        </header>
        <div className="min-h-0 flex-1 overflow-auto">
          <div className="app-page-enter mx-auto max-w-5xl px-4 py-5 sm:px-6 xl:py-6">
            <LawyerInsightsTool />
          </div>
        </div>
      </main>
    );
  }

  // ──────────── 法院短信处理(独立于计算器,自带视图) ────────────
  if (activeTool === "courtsms") {
    return (
      <main className="app-shell flex h-full w-full flex-col">
        <header className="app-subheader flex shrink-0 items-center gap-3 border-b px-4 py-2.5 sm:px-6">
          <button
            type="button"
            onClick={() => setActiveTool(null)}
            className="inline-flex items-center gap-1 rounded px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" />
            返回工具列表
          </button>
          <span className="text-muted-foreground/40">·</span>
          <h2 className="text-sm font-medium text-foreground">法院短信处理</h2>
        </header>
        <div className="min-h-0 flex-1 overflow-auto">
          <div className="app-page-enter mx-auto max-w-3xl px-4 py-5 sm:px-6 xl:py-6">
            <CourtSmsTool />
          </div>
        </div>
      </main>
    );
  }

  // ──────────── 快递查询(独立于计算器,自带视图) ────────────
  if (activeTool === "courier") {
    return (
      <main className="app-shell flex h-full w-full flex-col">
        <header className="app-subheader flex shrink-0 items-center gap-3 border-b px-4 py-2.5 sm:px-6">
          <button
            type="button"
            onClick={() => setActiveTool(null)}
            className="inline-flex items-center gap-1 rounded px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" />
            返回工具列表
          </button>
          <span className="text-muted-foreground/40">·</span>
          <h2 className="text-sm font-medium text-foreground">快递查询</h2>
        </header>
        <div className="min-h-0 flex-1 overflow-auto">
          <div className="app-page-enter mx-auto max-w-3xl px-4 py-5 sm:px-6 xl:py-6">
            <CourierTool />
          </div>
        </div>
      </main>
    );
  }

  // ──────────── 滴答清单 ToDo 同步(独立于计算器,自带视图) ────────────
  if (activeTool === "ticktick") {
    return (
      <main className="app-shell flex h-full w-full flex-col">
        <header className="app-subheader flex shrink-0 items-center gap-3 border-b px-4 py-2.5 sm:px-6">
          <button
            type="button"
            onClick={() => setActiveTool(null)}
            className="inline-flex items-center gap-1 rounded px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" />
            返回工具列表
          </button>
          <span className="text-muted-foreground/40">·</span>
          <h2 className="text-sm font-medium text-foreground">滴答清单 ToDo 同步</h2>
        </header>
        <div className="min-h-0 flex-1 overflow-auto">
          <div className="app-page-enter mx-auto max-w-3xl px-4 py-5 sm:px-6 xl:py-6">
            <TickTickPanel />
          </div>
        </div>
      </main>
    );
  }

  // ──────────── 飞书联动（日历 + 手机提醒） ────────────
  if (activeTool === "feishu") {
    return (
      <main className="app-shell flex h-full w-full flex-col">
        <header className="app-subheader flex shrink-0 items-center gap-3 border-b px-4 py-2.5 sm:px-6">
          <button
            type="button"
            onClick={() => setActiveTool(null)}
            className="inline-flex items-center gap-1 rounded px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" />
            返回工具列表
          </button>
          <span className="text-muted-foreground/40">·</span>
          <h2 className="text-sm font-medium text-foreground">飞书联动</h2>
        </header>
        <div className="min-h-0 flex-1 overflow-auto">
          <div className="app-page-enter mx-auto max-w-3xl px-4 py-5 sm:px-6 xl:py-6">
            <FeishuCalendarTool />
          </div>
        </div>
      </main>
    );
  }

  // ──────────── 辅助在线立案(独立于计算器,自带视图) ────────────
  if (activeTool === "courtfiling") {
    return (
      <main className="app-shell flex h-full w-full flex-col">
        <header className="app-subheader flex shrink-0 items-center gap-3 border-b px-4 py-2.5 sm:px-6">
          <button
            type="button"
            onClick={() => setActiveTool(null)}
            className="inline-flex items-center gap-1 rounded px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" />
            返回工具列表
          </button>
          <span className="text-muted-foreground/40">·</span>
          <h2 className="text-sm font-medium text-foreground">辅助在线立案</h2>
        </header>
        <div className="min-h-0 flex-1 overflow-auto">
          <div className="app-page-enter mx-auto max-w-3xl px-4 py-5 sm:px-6 xl:py-6">
            <CourtFilingTool />
          </div>
        </div>
      </main>
    );
  }

  if (activeTool === "elementconvert") {
    return <ElementConvertWorkbench onClose={() => setActiveTool(null)} />;
  }

  // ────────────────────────── 工具视图态 ──────────────────────────
  if (tool) {
    return (
      <main className="app-shell flex h-full w-full flex-col">
        <header className="app-subheader flex shrink-0 items-center gap-3 border-b px-4 py-2.5 sm:px-6">
          <button
            type="button"
            onClick={() => setActiveTool(null)}
            className="inline-flex items-center gap-1 rounded px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" />
            返回工具列表
          </button>
          <span className="text-muted-foreground/40">·</span>
          <h2 className="text-sm font-medium text-foreground">{tool.title}</h2>
        </header>

        <div className="min-h-0 flex-1 overflow-auto">
          <div className="app-page-enter mx-auto max-w-3xl px-4 py-5 sm:px-6 xl:py-6">
            {tool.id === "number" && <NumberConverter />}
            {tool.id === "daycal" && <DateCalculator />}
            {tool.id === "fee" && <LawyerFeeCalculator />}
            {tool.id === "legalfee" && <LitigationFeeCalculator />}
            {tool.id === "traffic" && <TrafficAccidentCompensationCalculator />}
            {tool.id === "labor" && <LaborSeveranceCalculator />}
            {tool.id === "interest" && (
              // key:prefill 变了强制重挂(state 是惰性初始化,不重挂的话
              // "先开过计算器再从执行页跳来"的场景预填不生效)
              <InterestCalculator
                key={
                  interestPrefill
                    ? `${interestPrefill.note ?? ""}|${interestPrefill.principal ?? ""}|${interestPrefill.repayments?.length ?? 0}`
                    : "blank"
                }
                prefill={interestPrefill}
              />
            )}
          </div>
        </div>
      </main>
    );
  }

  // ────────────────────────── 工具列表态 ──────────────────────────
  return (
    <main className="app-shell flex h-full w-full flex-col">
      <div className="flex-1 overflow-auto">
        <div className="app-page-enter mx-auto max-w-6xl space-y-8 px-4 py-6 sm:px-6 xl:px-8 xl:py-8">
          <header className="flex flex-wrap items-end justify-between gap-4">
            <div>
              <p className="font-mono text-caption uppercase tracking-wider text-brand">
                WORKSPACE
              </p>
              <h1 className="mt-1 text-2xl font-semibold tracking-tight text-foreground">
                工具
              </h1>
              <p className="mt-1 text-sm text-muted-foreground">
                常用计算、案件协作与自动化集中在这里。
              </p>
            </div>
            <span className="rounded-md border border-brand/10 bg-brand-soft/60 px-2.5 py-1 font-mono text-caption text-brand">
              {LEGAL_TOOLS.length + 9} 项可用
            </span>
          </header>
          {/* 法律计算工具(可用) */}
          <section className="space-y-3">
            <div className="border-l-2 border-brand/45 pl-3">
              <h2 className="text-sm font-semibold text-foreground">
                法律计算工具
              </h2>
            </div>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              {LEGAL_TOOLS.map((t) => (
                <LegalToolCard
                  key={t.id}
                  icon={t.icon}
                  title={t.title}
                  desc={t.desc}
                  onClick={() => setActiveTool(t.id)}
                />
              ))}
            </div>
          </section>

          {/* 知识库共享(团队协作 · 省积分) */}
          <section className="space-y-3">
            <div className="border-l-2 border-brand/45 pl-3">
              <h2 className="text-sm font-semibold text-foreground">
                知识库共享
              </h2>
              <p className="mt-0.5 text-xs text-muted-foreground">
                导入或导出元典缓存资料包，团队共享查询结果。
              </p>
            </div>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <LegalToolCard
                icon={Share2}
                title="本地知识库共享"
                desc="导入或导出元典缓存资料包。"
                onClick={() => setActiveTool("kbshare")}
              />
              <LegalToolCard
                icon={Combine}
                title="案件资料包(双人办案合并)"
                desc="导出或合并合办律师的案件资料包。"
                onClick={() => setActiveTool("casebundle")}
              />
            </div>
          </section>

          {/* 案件数据分析 */}
          <section className="space-y-3">
            <div className="border-l-2 border-brand/45 pl-3">
              <h2 className="text-sm font-semibold text-foreground">案件数据分析</h2>
              <p className="mt-0.5 text-xs text-muted-foreground">
                统计本机案件数据，不推测缺失指标。
              </p>
            </div>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <LegalToolCard
                icon={BarChart3}
                title="办案画像"
                desc="统计案由、法院、代理立场、状态与标的金额。"
                onClick={() => setActiveTool("lawyerinsights")}
              />
            </div>
          </section>

          {/* 日程 / 待办同步 */}
          <section className="space-y-3">
            <div className="border-l-2 border-brand/45 pl-3">
              <h2 className="text-sm font-semibold text-foreground">日程 / 待办同步</h2>
              <p className="mt-0.5 text-xs text-muted-foreground">
                连接滴答或飞书，把待办与日历带到手机。
              </p>
            </div>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <LegalToolCard
                icon={ListChecks}
                title="滴答清单 ToDo 同步"
                desc="双向同步手机滴答待办。"
                onClick={() => setActiveTool("ticktick")}
              />
              <LegalToolCard
                icon={CalendarClock}
                title="飞书联动"
                desc="读取飞书日历，推送案件待办与关键日期。"
                onClick={() => setActiveTool("feishu")}
              />
            </div>
          </section>

          {/* 案件自动化 */}
          <section className="space-y-3">
            <div className="border-l-2 border-brand/45 pl-3">
              <h2 className="text-sm font-semibold text-foreground">案件自动化</h2>
              <p className="mt-0.5 text-xs text-muted-foreground">
                处理重复的收件、查询与归档工作。
              </p>
            </div>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <LegalToolCard
                icon={Gavel}
                title="法院短信处理"
                desc="从送达短信下载文书并归档到案件。"
                onClick={() => setActiveTool("courtsms")}
              />
              <LegalToolCard
                icon={Truck}
                title="快递查询"
                desc="查询 EMS、顺丰等物流轨迹。"
                onClick={() => setActiveTool("courier")}
              />
              <LegalToolCard
                icon={Gavel}
                title="辅助在线立案(实验)"
                desc="自动填写到预览页，不会提交。"
                onClick={() => setActiveTool("courtfiling")}
              />
              <LegalToolCard
                icon={FileOutput}
                title="要素式文书转换（Beta）"
                desc="抽取要素，人工复核后生成 Word。"
                onClick={() => setActiveTool("elementconvert")}
              />
            </div>
          </section>

        </div>
      </div>
    </main>
  );
}

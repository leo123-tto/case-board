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
    desc: "阿拉伯数字实时转中文大写金额,支持角分",
    icon: Hash,
  },
  {
    id: "daycal",
    title: "天数计算器",
    desc: "算两个日期之间天数,或从某天加减若干天推算",
    icon: Calendar,
  },
  {
    id: "fee",
    title: "律师费计算器",
    desc: "按案件类型 + 标的额计算律师服务费(一口价 / 基础+风险)",
    icon: Calculator,
  },
  {
    id: "legalfee",
    title: "诉讼费计算器",
    desc: "按《诉讼费用交纳办法》算财产 / 离婚案件诉讼费 + 财产保全费",
    icon: Scale,
  },
  {
    id: "interest",
    title: "利息 / 执行款计算器",
    desc: "借款利息(LPR 历史)+ 执行款(多案 / 还款抵扣 / 五阶段清偿 / 迟延履行利息)",
    icon: TrendingUp,
  },
  {
    id: "traffic",
    title: "交通事故赔偿计算器",
    desc: "残疾/死亡赔偿金、被扶养人生活费、各项费用 + 责任比例 + 交强险扣减,带法律依据",
    icon: Car,
  },
  {
    id: "labor",
    title: "劳动解除赔偿计算器",
    desc: "经济补偿 N / 代通知金 N+1 / 违法解除 2N + 3 倍社平封顶 + 未休年假,带法律依据",
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
      <main className="flex h-full w-full flex-col bg-background">
        <header className="flex shrink-0 items-center gap-3 border-b border-border bg-card/50 px-6 py-2.5">
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
          <div className="mx-auto max-w-3xl px-6 py-6">
            <KbShareTool />
          </div>
        </div>
      </main>
    );
  }

  // ──────────── 案件资料包合并(双人办案,自带视图) ────────────
  if (activeTool === "casebundle") {
    return (
      <main className="flex h-full w-full flex-col bg-background">
        <header className="flex shrink-0 items-center gap-3 border-b border-border bg-card/50 px-6 py-2.5">
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
          <div className="mx-auto max-w-3xl px-6 py-6">
            <CaseBundleTool />
          </div>
        </div>
      </main>
    );
  }

  // ──────────── 办案画像(案件数据分析,自带视图) ────────────
  if (activeTool === "lawyerinsights") {
    return (
      <main className="flex h-full w-full flex-col bg-background">
        <header className="flex shrink-0 items-center gap-3 border-b border-border bg-card/50 px-6 py-2.5">
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
          <div className="mx-auto max-w-5xl px-6 py-6">
            <LawyerInsightsTool />
          </div>
        </div>
      </main>
    );
  }

  // ──────────── 法院短信处理(独立于计算器,自带视图) ────────────
  if (activeTool === "courtsms") {
    return (
      <main className="flex h-full w-full flex-col bg-background">
        <header className="flex shrink-0 items-center gap-3 border-b border-border bg-card/50 px-6 py-2.5">
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
          <div className="mx-auto max-w-3xl px-6 py-6">
            <CourtSmsTool />
          </div>
        </div>
      </main>
    );
  }

  // ──────────── 快递查询(独立于计算器,自带视图) ────────────
  if (activeTool === "courier") {
    return (
      <main className="flex h-full w-full flex-col bg-background">
        <header className="flex shrink-0 items-center gap-3 border-b border-border bg-card/50 px-6 py-2.5">
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
          <div className="mx-auto max-w-3xl px-6 py-6">
            <CourierTool />
          </div>
        </div>
      </main>
    );
  }

  // ──────────── 滴答清单 ToDo 同步(独立于计算器,自带视图) ────────────
  if (activeTool === "ticktick") {
    return (
      <main className="flex h-full w-full flex-col bg-background">
        <header className="flex shrink-0 items-center gap-3 border-b border-border bg-card/50 px-6 py-2.5">
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
          <div className="mx-auto max-w-3xl px-6 py-6">
            <TickTickPanel />
          </div>
        </div>
      </main>
    );
  }

  // ──────────── 飞书联动（日历 + 手机提醒） ────────────
  if (activeTool === "feishu") {
    return (
      <main className="flex h-full w-full flex-col bg-background">
        <header className="flex shrink-0 items-center gap-3 border-b border-border bg-card/50 px-6 py-2.5">
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
          <div className="mx-auto max-w-3xl px-6 py-6">
            <FeishuCalendarTool />
          </div>
        </div>
      </main>
    );
  }

  // ──────────── 辅助在线立案(独立于计算器,自带视图) ────────────
  if (activeTool === "courtfiling") {
    return (
      <main className="flex h-full w-full flex-col bg-background">
        <header className="flex shrink-0 items-center gap-3 border-b border-border bg-card/50 px-6 py-2.5">
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
          <div className="mx-auto max-w-3xl px-6 py-6">
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
      <main className="flex h-full w-full flex-col bg-background">
        <header className="flex shrink-0 items-center gap-3 border-b border-border bg-card/50 px-6 py-2.5">
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
          <div className="mx-auto max-w-3xl px-6 py-6">
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
    <main className="flex h-full w-full flex-col bg-background">
      <div className="flex-1 overflow-auto">
        <div className="mx-auto max-w-6xl space-y-6 px-8 py-6">
          {/* 法律计算工具(可用) */}
          <section className="space-y-2">
            <div className="px-1">
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
          <section className="space-y-2">
            <div className="px-1">
              <h2 className="text-sm font-semibold text-foreground">
                知识库共享
              </h2>
              <p className="mt-0.5 text-xs text-muted-foreground">
                把花积分查过的元典结果打包发给同事,或导入同事的包 —— 团队互相省积分
              </p>
            </div>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <LegalToolCard
                icon={Share2}
                title="本地知识库共享"
                desc="导出 / 导入元典缓存资料包(.zip),团队互通、互相省积分"
                onClick={() => setActiveTool("kbshare")}
              />
              <LegalToolCard
                icon={Combine}
                title="案件资料包(双人办案合并)"
                desc="导出某案件给合办律师 / 导入对方资料包合并进同一案件,材料按内容去重、并集、不冲突"
                onClick={() => setActiveTool("casebundle")}
              />
            </div>
          </section>

          {/* 案件数据分析 */}
          <section className="space-y-2">
            <div className="px-1">
              <h2 className="text-sm font-semibold text-foreground">案件数据分析</h2>
              <p className="mt-0.5 text-xs text-muted-foreground">
                基于看板里的本机案件数据做统计和导出,不给不存在的胜诉率、收入、案源渠道硬下结论
              </p>
            </div>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <LegalToolCard
                icon={BarChart3}
                title="办案画像"
                desc="统计高频案由、主要法院、代理立场、案件状态和标的金额,导出 Markdown 给 AI 继续分析"
                onClick={() => setActiveTool("lawyerinsights")}
              />
            </div>
          </section>

          {/* 日程 / 待办同步 */}
          <section className="space-y-2">
            <div className="px-1">
              <h2 className="text-sm font-semibold text-foreground">日程 / 待办同步</h2>
              <p className="mt-0.5 text-xs text-muted-foreground">
                跟手机滴答清单双向同步个人待办,首页展示;用你自己注册的滴答应用连自己账号
              </p>
            </div>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <LegalToolCard
                icon={ListChecks}
                title="滴答清单 ToDo 同步"
                desc="连接手机滴答(收件箱),双向同步待办、勾完成两边同步;每分钟 + 切回 App 自动同步"
                onClick={() => setActiveTool("ticktick")}
              />
              <LegalToolCard
                icon={CalendarClock}
                title="飞书联动"
                desc="读取飞书日历，也可把案件待办和关键日期每天推送到飞书手机；两项均可独立开关"
                onClick={() => setActiveTool("feishu")}
              />
            </div>
          </section>

          {/* 案件自动化 */}
          <section className="space-y-2">
            <div className="px-1">
              <h2 className="text-sm font-semibold text-foreground">案件自动化</h2>
              <p className="mt-0.5 text-xs text-muted-foreground">
                把重复的取件 / 归档动作交给程序
              </p>
            </div>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <LegalToolCard
                icon={Gavel}
                title="法院短信处理"
                desc="粘贴一张网送达短信 → 自动下载文书、归档进对应案件、抽取上看板"
                onClick={() => setActiveTool("courtsms")}
              />
              <LegalToolCard
                icon={Truck}
                title="快递查询"
                desc="查 EMS / 顺丰等物流轨迹(寄送达、材料追踪);需配快递100 key"
                onClick={() => setActiveTool("courier")}
              />
              <LegalToolCard
                icon={Gavel}
                title="辅助在线立案(实验)"
                desc="一张网自动填到预览页停、不自动提交;配置+律师档案在此,发起在案件详情页;需本机 Python 运行时"
                onClick={() => setActiveTool("courtfiling")}
              />
              <LegalToolCard
                icon={FileOutput}
                title="要素式文书转换（Beta）"
                desc="共同测试版：抽取要素、人工复核后生成 Word；结果必须由律师核对"
                onClick={() => setActiveTool("elementconvert")}
              />
            </div>
          </section>

        </div>
      </div>
    </main>
  );
}

/**
 * 「执行」模块(2026-05-24 j 骨架版)。
 *
 * 设计意图(作者拍板):
 *  - 案件 workflow_status='执行中' 的自动出现在这里
 *  - 主要显示执行相关信息:保全 / 解封时间 / 执行法官 / 执行标的 / 执行申请 / 还款记录
 *  - V0.2 接入:① 一键调元典 API 查财产线索 / 失信 / 限高
 *               ② 跟「利息执行款」工具联动(一键填值算剩余执行款)
 *
 * 当前 V0.1 骨架:
 *  - 列出所有 workflow_status='执行中' 的案件卡片
 *  - 每卡显示 case_summary + agg_resolution + key_dates 中的执行节点
 *  - 点卡进入详情(暂时跳回诉讼详情页,复用同套展示)
 */

import { Gavel, Loader2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { formatYuan } from "@/lib/format";
import { normalizeCaseStatusText } from "@/lib/caseSnapshot";
import type { Case, CourtContact, Document } from "@/lib/types";
import { parseJsonArray } from "@/lib/types";
import { getCaseWithDocs, listCases } from "@/lib/api";
import { resolveCaseStatus } from "@/modules/litigation/lib/inferStatus";
import type { InterestPrefill } from "@/modules/tools/calculators/InterestCalculator";
import { ExecutionDetailView } from "./ExecutionDetailView";

interface Props {
  /** 2026-05-25:点案件详情页「算执行款」时把数据(本金/起算日/还款记录)传给工具模块 */
  onCalculateInterest?: (prefill: InterestPrefill) => void;
}

export function ExecutionModule({ onCalculateInterest }: Props) {
  const [selectedCase, setSelectedCase] = useState<Case | null>(null);
  const [cases, setCases] = useState<Case[]>([]);
  const [docsByCase, setDocsByCase] = useState<Record<string, Document[]>>({});
  const [loading, setLoading] = useState(true);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const reloadExecutionCases = useCallback(
    async ({ showSpinner = false }: { showSpinner?: boolean } = {}) => {
      if (showSpinner) setLoading(true);
      try {
        const all = await listCases();
        if (!mountedRef.current) return;
        setCases(all);
        // 每个案件拉 docs 给 inferStatus 用(跟首页 HomeView 同源)
        const pairs = await Promise.all(
          all.map(async (c) => {
            try {
              const r = await getCaseWithDocs(c.id);
              return [c.id, r.documents] as const;
            } catch {
              return [c.id, [] as Document[]] as const;
            }
          }),
        );
        if (!mountedRef.current) return;
        setDocsByCase(Object.fromEntries(pairs));
        setSelectedCase((current) =>
          current ? (all.find((c) => c.id === current.id) ?? current) : null,
        );
      } finally {
        if (mountedRef.current) setLoading(false);
      }
    },
    [],
  );

  useEffect(() => {
    reloadExecutionCases({ showSpinner: true }).catch(() => {
      if (mountedRef.current) setLoading(false);
    });
  }, [reloadExecutionCases]);

  useEffect(() => {
    const refreshOnFocus = () => {
      if (document.visibilityState === "hidden") return;
      void reloadExecutionCases().catch(() => {});
    };
    window.addEventListener("focus", refreshOnFocus);
    document.addEventListener("visibilitychange", refreshOnFocus);
    return () => {
      window.removeEventListener("focus", refreshOnFocus);
      document.removeEventListener("visibilitychange", refreshOnFocus);
    };
  }, [reloadExecutionCases]);

  // 2026-05-24 j-3 修:用 resolveCaseStatus(workflow_status 优先[手工/LLM 推断] > 文档自动推断)— 跟首页 chip 同源
  const executionCases = useMemo(
    () =>
      cases.filter((c) => {
        const status = resolveCaseStatus(c, docsByCase[c.id] ?? []);
        return status.id === "execution";
      }),
    [cases, docsByCase],
  );

  // 详情视图(在执行模块内部,不跳回诉讼)
  if (selectedCase) {
    return (
      <ExecutionDetailView
        caseData={selectedCase}
        documents={docsByCase[selectedCase.id] ?? []}
        onBack={() => setSelectedCase(null)}
        onCalculateInterest={onCalculateInterest}
      />
    );
  }

  return (
    <main className="flex h-full w-full flex-col bg-background">
      {/* 顶部 nav */}
      <header className="border-b border-border bg-card/50 px-8 py-3">
        <div className="mx-auto flex max-w-6xl items-center justify-between">
          <div className="flex items-center gap-2">
            <Gavel className="size-4 text-muted-foreground" />
            <h1 className="text-sm font-semibold tracking-tight text-foreground">
              执行案件
            </h1>
            <span className="rounded bg-muted px-1.5 py-0.5 text-caption font-medium text-muted-foreground">
              {executionCases.length} 件
            </span>
          </div>
        </div>
      </header>

      <div className="flex-1 overflow-auto px-8 py-8">
        <div className="mx-auto max-w-6xl">
          {loading ? (
            <div className="flex h-40 items-center justify-center">
              <Loader2 className="size-5 animate-spin text-muted-foreground" />
            </div>
          ) : executionCases.length === 0 ? (
            <EmptyExecution />
          ) : (
            <ExecutionGrid
              cases={executionCases}
              onOpenCase={(id) => {
                const c = executionCases.find((x) => x.id === id);
                if (c) setSelectedCase(c);
              }}
            />
          )}
        </div>
      </div>
    </main>
  );
}

function EmptyExecution() {
  return (
    <div className="rounded-lg border border-dashed border-border bg-card/30 p-12 text-center">
      <Gavel className="mx-auto size-10 text-muted-foreground/40" />
      <h2 className="mt-4 text-base font-semibold text-foreground">
        当前没有"执行中"案件
      </h2>
      <p className="mt-2 max-w-md mx-auto text-sm text-muted-foreground">
        案件状态切到「执行中」后会自动出现在这里。
        <br />
        诉讼首页案件卡片右上角的状态下拉可以手工切到「执行中」,或者由
        LLM 在全局抽时自动给出。
      </p>
    </div>
  );
}

function ExecutionGrid({
  cases,
  onOpenCase,
}: {
  cases: Case[];
  onOpenCase: (caseId: string) => void;
}) {
  return (
    <div className="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
      {cases.map((c) => (
        <ExecutionCard key={c.id} caseData={c} onOpen={() => onOpenCase(c.id)} />
      ))}
    </div>
  );
}

function ExecutionCard({
  caseData,
  onOpen,
}: {
  caseData: Case;
  onOpen: () => void;
}) {
  const defendants = parseJsonArray(caseData.agg_defendants);
  const statusText = normalizeCaseStatusText(caseData.agg_status_text);
  const keyDates = parseKeyDates(caseData.agg_key_dates);
  // 优先展示"执行立案 / 申请保全 / 续封 / 财产查询" 节点
  const executionDates = keyDates.filter((d) =>
    /执行|保全|续封|查封|查询|还款|付款/.test(d.event),
  );
  // V0.3 · 承办法官 + 电话(执行阶段常要联系法官)。优先 role 含「法官/承办/审判」的法院联系人,
  // 否则取有电话的联系人;名字兜底用 agg_judges。
  const courtContacts = parseCourtContacts(caseData.agg_court_contacts);
  const judgeContact =
    courtContacts.find((c) => c.role && /法官|承办|审判/.test(c.role)) ??
    courtContacts.find((c) => c.phone);
  const judgeName = judgeContact?.name ?? parseJsonArray(caseData.agg_judges)[0] ?? null;
  const judgePhone = judgeContact?.phone ?? null;

  return (
    <button
      type="button"
      onClick={onOpen}
      className="group flex flex-col rounded-lg border border-border bg-card p-5 text-left transition-all hover:shadow-md"
    >
      <h3 className="line-clamp-1 text-sm font-semibold text-foreground">
        {caseData.name}
      </h3>
      {caseData.case_summary && (
        <p className="mt-2 line-clamp-2 text-xs text-muted-foreground">
          {caseData.case_summary}
        </p>
      )}

      <div className="mt-4 space-y-1.5 text-xs">
        {defendants.length > 0 && (
          <div className="flex items-baseline gap-2">
            <span className="shrink-0 text-muted-foreground">被执行人</span>
            <span className="font-medium text-foreground">
              {defendants.join("、")}
            </span>
          </div>
        )}
        {caseData.agg_claim_amount != null && (
          <div className="flex items-baseline gap-2">
            <span className="shrink-0 text-muted-foreground">执行标的</span>
            <span className="font-mono font-medium text-foreground">
              {formatYuan(caseData.agg_claim_amount)}
            </span>
          </div>
        )}
        {caseData.execution_remaining != null && (
          <div className="flex items-baseline gap-2">
            <span className="shrink-0 text-muted-foreground">剩余</span>
            <span className="font-mono font-medium text-foreground">
              {formatYuan(caseData.execution_remaining)}
            </span>
          </div>
        )}
        {(judgeName || judgePhone) && (
          <div className="flex items-baseline gap-2">
            <span className="shrink-0 text-muted-foreground">承办法官</span>
            <span className="font-medium text-foreground">
              {judgeName ?? "—"}
              {judgePhone && (
                <span className="ml-1.5 font-mono text-muted-foreground">
                  {judgePhone}
                </span>
              )}
            </span>
          </div>
        )}
      </div>

      {executionDates.length > 0 && (
        <div className="mt-4 border-t border-border pt-3">
          <div className="text-caption font-medium uppercase tracking-wide text-muted-foreground">
            执行节点
          </div>
          <ul className="mt-2 space-y-1 text-xs">
            {executionDates.slice(0, 3).map((d, i) => (
              <li key={i} className="flex items-baseline gap-2">
                <span className="font-mono text-muted-foreground">
                  {d.date}
                </span>
                <span className="text-foreground">{d.event}</span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {statusText && (
        <p className="mt-3 line-clamp-2 text-xs text-muted-foreground/80">
          {statusText}
        </p>
      )}
    </button>
  );
}

interface KeyDateItem {
  date: string;
  event: string;
  note?: string;
}

function parseKeyDates(json: string | null): KeyDateItem[] {
  if (!json) return [];
  try {
    const parsed = JSON.parse(json);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter((x) => x && typeof x === "object" && typeof x.date === "string" && typeof x.event === "string")
      .sort((a, b) => (a.date < b.date ? 1 : -1));
  } catch {
    return [];
  }
}

/** 解析 agg_court_contacts(CourtContact[] 的 JSON);非数组/坏 JSON 返回 []。 */
function parseCourtContacts(json: string | null): CourtContact[] {
  if (!json) return [];
  try {
    const parsed = JSON.parse(json);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (x): x is CourtContact => x != null && typeof x === "object",
    );
  } catch {
    return [];
  }
}

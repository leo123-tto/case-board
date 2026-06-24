/**
 * 利息 / 执行款计算器 — React 原生实现(2026-05-24 e)。
 *
 * 两个模式:
 *   - 利息计算(interest):多本金 + 各自时间段 + 自定义利率 / LPR;按 LPR 变化点分段
 *   - 执行款(execution):多案件 + 多还款 + 五阶段清偿 + 迟延履行利息
 *
 * 计算逻辑见 ../lib/interestCalc.ts。
 *
 * 法律依据:利息 / 执行款 两套独立弹窗。
 */

import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { DetailRow, TabBtn } from "./ui";

import {
  LegalBasisButton,
  LegalBasisModal,
} from "../components/LegalBasisModal";
import {
  EXECUTION_BASIS,
  INTEREST_BASIS,
} from "../lib/legalBasisData";
import {
  calcFiveStage,
  calculateInterestByPeriod,
  calculateInterestSegments,
  daysBetween,
  type ExecCaseInput,
  formatMoney,
  type InterestPrincipal,
  type InterestSegment,
  normalizeLprMultiplier,
  type RateType,
  type Repayment,
} from "../lib/interestCalc";
import type { LprTerm } from "../lib/lprData";
import { todayIso } from "../lib/dateMath";

type Mode = "interest" | "execution";

export function InterestCalculator({
  prefill,
}: {
  prefill?: InterestPrefill | null;
} = {}) {
  // 2026-06-11:执行模块「算执行款」跳过来时 prefill.mode="execution" → 直接打开执行款 tab
  const [mode, setMode] = useState<Mode>(
    prefill?.mode === "execution" ? "execution" : "interest",
  );
  const [basisOpen, setBasisOpen] = useState<null | "interest" | "execution">(
    null,
  );

  return (
    <div className="space-y-5">
      {prefill && (
        <div className="rounded-md border border-amber-300 bg-amber-50/60 px-3 py-2 text-xs text-amber-900">
          ⓘ 已从案件预填:本金 {prefill.principal ? `¥${prefill.principal}` : "—"} ·
          起算日 {prefill.startDate || "—"}
          {prefill.repayments && prefill.repayments.length > 0 && (
            <> · 还款记录 {prefill.repayments.length} 笔</>
          )}
          {prefill.note && (
            <span className="ml-2 text-amber-800/70">· {prefill.note}</span>
          )}
        </div>
      )}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="inline-flex rounded-md border border-border bg-card p-0.5">
          <TabBtn active={mode === "interest"} onClick={() => setMode("interest")}>
            利息计算
          </TabBtn>
          <TabBtn active={mode === "execution"} onClick={() => setMode("execution")}>
            执行款计算
          </TabBtn>
        </div>
        <LegalBasisButton onClick={() => setBasisOpen(mode)}>
          {mode === "interest" ? "查看利息计算法律依据" : "查看执行款计算法律依据"}
        </LegalBasisButton>
      </div>

      {mode === "interest" ? (
        <InterestPanel prefill={prefill} />
      ) : (
        <ExecutionPanel prefill={prefill} />
      )}

      <LegalBasisModal
        open={basisOpen === "interest"}
        onClose={() => setBasisOpen(null)}
        title="利息计算法律依据"
        sections={INTEREST_BASIS}
      />
      <LegalBasisModal
        open={basisOpen === "execution"}
        onClose={() => setBasisOpen(null)}
        title="执行款计算法律依据"
        sections={EXECUTION_BASIS}
      />
    </div>
  );
}

/* ============================ 利息计算 ============================ */

/** 2026-05-25:从案件详情页跳过来时预填本金 / 起算日 / 备注。
 *  2026-06-11:执行模块「算执行款」联动 — mode 指定打开哪个 tab,
 *  repayments 预填执行款面板的还款记录(来自 case_payments)。 */
export interface InterestPrefill {
  principal?: string;
  startDate?: string;
  endDate?: string;
  note?: string;
  /** 打开哪个 tab(默认 interest) */
  mode?: "interest" | "execution";
  /** 还款记录(执行款 tab 用):日期 + 金额(元) */
  repayments?: Array<{ date: string; amount: number }>;
}

function InterestPanel({ prefill }: { prefill?: InterestPrefill | null } = {}) {
  const [principals, setPrincipals] = useState<InterestPrincipal[]>(() => {
    if (prefill && (prefill.principal || prefill.startDate)) {
      const seed = makeBlankPrincipal();
      return [
        {
          ...seed,
          principal: prefill.principal ?? "",
          startDate: prefill.startDate ?? "",
          endDate: prefill.endDate ?? seed.endDate,
        },
      ];
    }
    return [makeBlankPrincipal()];
  });
  const [showDetail, setShowDetail] = useState(false);

  const updateP = (id: number, patch: Partial<InterestPrincipal>) => {
    setPrincipals((arr) =>
      arr.map((p) => (p.id === id ? { ...p, ...patch } : p)),
    );
  };

  // 计算每笔本金的利息
  const items = principals
    .map((p) => {
      const principal = parseFloat(p.principal);
      if (!principal || principal <= 0) return null;
      const customRate = parseFloat(p.rate) || 0;
      const lprMultiplier = normalizeLprMultiplier(parseFloat(p.lprMultiplier));
      const interest = calculateInterestByPeriod(
        principal,
        p.startDate,
        p.endDate,
        p.rateType,
        customRate,
        p.lprTerm,
        lprMultiplier,
      );
      const days = daysBetween(p.startDate, p.endDate);
      const segments = calculateInterestSegments(
        principal,
        p.startDate,
        p.endDate,
        p.rateType,
        customRate,
        p.lprTerm,
        lprMultiplier,
      );
      return { p, principal, interest, days, lprMultiplier, segments };
    })
    .filter((x): x is NonNullable<typeof x> => x !== null);

  const total = items.reduce((s, x) => s + x.interest, 0);

  return (
    <div className="space-y-4">
      {/* 本金列表 */}
      <div className="space-y-3">
        {principals.map((p, idx) => (
          <PrincipalRow
            key={p.id}
            index={idx}
            data={p}
            canDelete={principals.length > 1}
            onChange={(patch) => updateP(p.id, patch)}
            onDelete={() =>
              setPrincipals((arr) => arr.filter((x) => x.id !== p.id))
            }
          />
        ))}
        <Button
          variant="outline"
          size="sm"
          onClick={() =>
            setPrincipals((arr) => [...arr, makeBlankPrincipal()])
          }
          className="h-8 gap-1 text-xs"
        >
          <Plus className="size-3.5" />
          添加本金
        </Button>
      </div>

      {/* 结果 */}
      {items.length === 0 ? (
        <Placeholder>填写本金后查看利息</Placeholder>
      ) : (
        <div className="space-y-3 rounded-md border border-border bg-card px-5 py-4">
          <div>
            <p className="text-caption uppercase tracking-wider text-muted-foreground">
              利息合计
            </p>
            <p className="mt-1 font-mono text-3xl font-semibold text-foreground">
              {formatMoney(total)}
            </p>
          </div>

          <dl className="border-t border-border/70 pt-3 text-sm">
            {items.map((x, i) => (
              <div
                key={i}
                className="flex flex-wrap items-baseline justify-between gap-2 border-b border-border/40 py-1.5 last:border-0"
              >
                <dt className="text-xs text-muted-foreground">
                  本金 {formatMoney(x.principal)} · {x.p.startDate} 至 {x.p.endDate} · {x.days} 天
                </dt>
                <dd className="font-mono text-sm text-foreground">
                  {formatMoney(x.interest)}
                </dd>
              </div>
            ))}
          </dl>

          <div>
            <button
              type="button"
              onClick={() => setShowDetail((v) => !v)}
              className="text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
            >
              {showDetail ? "收起计算过程" : "查看计算过程(各时段 LPR 分段)"}
            </button>
            {showDetail && (
              <div className="mt-2 space-y-2 rounded-md border border-dashed border-border bg-muted/20 px-3 py-2 text-label text-foreground">
                {items.map((x, i) => (
                  <div key={i}>
                    <p className="font-medium">
                      本金 {i + 1}: {formatMoney(x.principal)} · {x.p.startDate} ~ {x.p.endDate} · 共 {x.days} 天
                      {x.p.rateType === "lpr" && x.lprMultiplier !== 1
                        ? ` · LPR × ${formatMultiplier(x.lprMultiplier)}`
                        : ""}
                    </p>
                    {x.p.rateType === "custom" ? (
                      <p className="pl-3 font-mono text-muted-foreground">
                        {x.principal} × {x.p.rate}% ÷ 365 × {x.days} = {formatMoney(x.interest)}
                      </p>
                    ) : (
                      <div className="space-y-0.5 pl-3 font-mono text-muted-foreground">
                        {x.segments.map((s, si) => (
                          <p key={si}>
                            {s.startDate} ~ {s.endDate}: {formatInterestSegmentFormula(x.principal, s)} = {formatMoney(s.interest)}
                          </p>
                        ))}
                        <p className="font-medium text-foreground">
                          小计: {formatMoney(x.interest)}
                        </p>
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function makeBlankPrincipal(): InterestPrincipal {
  return {
    id: Date.now() + Math.floor(Math.random() * 1000),
    principal: "",
    rateType: "custom",
    rate: "",
    lprTerm: "1y",
    lprMultiplier: "1",
    startDate: "",
    endDate: todayIso(),
  };
}

function formatMultiplier(value: number): string {
  return Number.isInteger(value) ? String(value) : value.toFixed(4).replace(/0+$/, "").replace(/\.$/, "");
}

function formatRate(value: number): string {
  return value.toFixed(4).replace(/0+$/, "").replace(/\.$/, "");
}

function formatInterestSegmentFormula(
  principal: number,
  segment: InterestSegment,
): string {
  if (segment.rateType === "custom") {
    return `${principal} × ${formatRate(segment.rate)}% ÷ 365 × ${segment.days} 天`;
  }
  const rateText =
    segment.multiplier === 1
      ? `LPR ${formatRate(segment.baseRate)}%`
      : `LPR ${formatRate(segment.baseRate)}% × ${formatMultiplier(segment.multiplier)} = ${formatRate(segment.rate)}%`;
  return `${principal} × ${rateText} ÷ 365 × ${segment.days} 天`;
}

function PrincipalRow({
  index,
  data,
  canDelete,
  onChange,
  onDelete,
}: {
  index: number;
  data: InterestPrincipal;
  canDelete: boolean;
  onChange: (patch: Partial<InterestPrincipal>) => void;
  onDelete: () => void;
}) {
  return (
    <div className="rounded-md border border-border bg-card px-4 py-3">
      <div className="mb-2 flex items-center justify-between">
        <span className="text-xs font-medium text-foreground">本金 {index + 1}</span>
        {canDelete && (
          <button
            type="button"
            onClick={onDelete}
            className="rounded p-1 text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
            aria-label="删除"
          >
            <Trash2 className="size-3.5" />
          </button>
        )}
      </div>

      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <SmallField label="本金(元)">
          <input
            type="number"
            min={0}
            step={0.01}
            placeholder="例如:100000"
            value={data.principal}
            onChange={(e) => onChange({ principal: e.target.value })}
            className="w-full rounded border border-border bg-background px-2 py-1.5 font-mono text-sm outline-none focus:border-foreground/50"
          />
        </SmallField>

        <SmallField label="利率类型">
          <div className="flex gap-1.5">
            <select
              value={data.rateType}
              onChange={(e) => onChange({ rateType: e.target.value as RateType })}
              className="flex-1 rounded border border-border bg-background px-2 py-1.5 text-sm outline-none focus:border-foreground/50"
            >
              <option value="custom">约定利率</option>
              <option value="lpr">LPR</option>
            </select>
            {data.rateType === "custom" ? (
              <div className="relative flex-1">
                <input
                  type="number"
                  min={0}
                  step={0.01}
                  placeholder="年利率"
                  value={data.rate}
                  onChange={(e) => onChange({ rate: e.target.value })}
                  className="w-full rounded border border-border bg-background px-2 py-1.5 pr-7 font-mono text-sm outline-none focus:border-foreground/50"
                />
                <span className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 text-xs text-muted-foreground">
                  %
                </span>
              </div>
            ) : (
              <>
                <select
                  value={data.lprTerm}
                  onChange={(e) => onChange({ lprTerm: e.target.value as LprTerm })}
                  className="min-w-0 flex-1 rounded border border-border bg-background px-2 py-1.5 text-sm outline-none focus:border-foreground/50"
                >
                  <option value="1y">1 年期 LPR</option>
                  <option value="5y+">5 年期以上 LPR</option>
                </select>
                <div className="relative w-24">
                  <input
                    type="number"
                    min={0}
                    step={0.01}
                    placeholder="1.5"
                    value={data.lprMultiplier}
                    onChange={(e) => onChange({ lprMultiplier: e.target.value })}
                    className="w-full rounded border border-border bg-background px-2 py-1.5 pr-7 font-mono text-sm outline-none focus:border-foreground/50"
                    aria-label="LPR 倍数"
                  />
                  <span className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 text-xs text-muted-foreground">
                    倍
                  </span>
                </div>
              </>
            )}
          </div>
        </SmallField>

        <SmallField label="起算日">
          <input
            type="date"
            value={data.startDate}
            onChange={(e) => onChange({ startDate: e.target.value })}
            className="w-full rounded border border-border bg-background px-2 py-1.5 font-mono text-sm outline-none focus:border-foreground/50"
          />
        </SmallField>

        <SmallField label="截止日">
          <input
            type="date"
            value={data.endDate}
            onChange={(e) => onChange({ endDate: e.target.value })}
            className="w-full rounded border border-border bg-background px-2 py-1.5 font-mono text-sm outline-none focus:border-foreground/50"
          />
        </SmallField>
      </div>
    </div>
  );
}

/* ============================ 执行款计算 ============================ */
function ExecutionPanel({ prefill }: { prefill?: InterestPrefill | null } = {}) {
  // 2026-06-11:执行模块跳过来时预填首案(本金/起算日/名称)+ 还款记录,能提取到的都填
  const [cases, setCases] = useState<ExecCaseFormData[]>(() => {
    const blank = makeBlankCase();
    if (prefill?.mode === "execution" && (prefill.principal || prefill.startDate)) {
      return [
        {
          ...blank,
          name: prefill.note ?? "",
          principal: prefill.principal ?? "",
          startDate: prefill.startDate ?? "",
        },
      ];
    }
    return [blank];
  });
  const [repayments, setRepayments] = useState<Repayment[]>(() =>
    (prefill?.mode === "execution" ? (prefill.repayments ?? []) : []).map(
      (r, i) => ({ id: Date.now() + i, date: r.date, amount: r.amount }),
    ),
  );
  const [multiCase, setMultiCase] = useState(false);
  const [includeDelayed, setIncludeDelayed] = useState(true);
  const [showDetail, setShowDetail] = useState(false);

  const updateCase = (id: number, patch: Partial<ExecCaseFormData>) => {
    setCases((arr) => arr.map((c) => (c.id === id ? { ...c, ...patch } : c)));
  };
  const updateRep = (id: number, patch: Partial<Repayment>) => {
    setRepayments((arr) => arr.map((r) => (r.id === id ? { ...r, ...patch } : r)));
  };

  // 解析 / 规整案件数据
  const rawCases: ExecCaseInput[] = cases
    .map((c, i) => {
      const principal = parseFloat(c.principal) || 0;
      if (principal <= 0) return null;
      const rate =
        c.rateType === "lpr"
          ? 0
          : parseFloat(c.rate) || 0;
      return {
        id: c.id,
        name: c.name || `案件 ${i + 1}`,
        principal,
        rate,
        rateType: c.rateType,
        lprTerm: c.lprTerm,
        lprMultiplier: normalizeLprMultiplier(parseFloat(c.lprMultiplier)),
        startDate: c.startDate,
        endDate: c.endDate || todayIso(),
        litigationFee: parseFloat(c.litigationFee) || 0,
        lawyerFee: parseFloat(c.lawyerFee) || 0,
        otherFee: parseFloat(c.otherFee) || 0,
      };
    })
    .filter((x): x is ExecCaseInput => x !== null);

  const sortedReps = repayments
    .filter((r) => r.date && r.amount > 0)
    .sort((a, b) => a.date.localeCompare(b.date));

  // 计算
  const computeResults = () => {
    if (rawCases.length === 0) return null;
    if (multiCase) {
      const totalPrincipal = rawCases.reduce((s, c) => s + c.principal, 0);
      const mergedStart =
        rawCases
          .map((c) => c.startDate)
          .filter(Boolean)
          .sort()[0] || "";
      const mergedEnd =
        rawCases
          .map((c) => c.endDate)
          .filter(Boolean)
          .sort()
          .reverse()[0] || todayIso();
      const mergedRate = rawCases[0].rate;
      const merged = calcFiveStage(
        {
          ...rawCases[0],
          principal: totalPrincipal,
          rate: mergedRate,
          startDate: mergedStart,
          endDate: mergedEnd,
          litigationFee: rawCases.reduce((s, c) => s + c.litigationFee, 0),
          lawyerFee: rawCases.reduce((s, c) => s + c.lawyerFee, 0),
          otherFee: rawCases.reduce((s, c) => s + c.otherFee, 0),
        },
        sortedReps,
        includeDelayed,
      );
      return { mode: "multi" as const, merged };
    }
    return {
      mode: "separate" as const,
      perCase: rawCases.map((c) => ({
        info: c,
        result: calcFiveStage(c, sortedReps, includeDelayed),
      })),
    };
  };

  const results = computeResults();

  return (
    <div className="space-y-4">
      {/* 案件列表 */}
      <SectionHeader title="案件">
        <Button
          variant="outline"
          size="sm"
          onClick={() => setCases((arr) => [...arr, makeBlankCase()])}
          className="h-7 gap-1 text-xs"
        >
          <Plus className="size-3.5" />
          添加案件
        </Button>
      </SectionHeader>
      <div className="space-y-3">
        {cases.map((c, idx) => (
          <CaseRow
            key={c.id}
            index={idx}
            data={c}
            canDelete={cases.length > 1}
            onChange={(patch) => updateCase(c.id, patch)}
            onDelete={() =>
              setCases((arr) => arr.filter((x) => x.id !== c.id))
            }
          />
        ))}
      </div>

      {/* 还款列表 */}
      <SectionHeader title="还款记录(按日期早 → 晚抵扣)">
        <Button
          variant="outline"
          size="sm"
          onClick={() =>
            setRepayments((arr) => [
              ...arr,
              { id: Date.now() + Math.floor(Math.random() * 1000), date: "", amount: 0 },
            ])
          }
          className="h-7 gap-1 text-xs"
        >
          <Plus className="size-3.5" />
          添加还款
        </Button>
      </SectionHeader>
      {repayments.length === 0 ? (
        <p className="rounded-md border border-dashed border-border/70 bg-muted/20 px-3 py-3 text-center text-label text-muted-foreground">
          暂无还款记录
        </p>
      ) : (
        <div className="space-y-2">
          {repayments.map((r, idx) => (
            <div
              key={r.id}
              className="flex items-center gap-2 rounded-md border border-border bg-card px-3 py-2"
            >
              <span className="shrink-0 text-xs font-medium text-foreground">
                还款 {idx + 1}
              </span>
              <input
                type="date"
                value={r.date}
                onChange={(e) => updateRep(r.id, { date: e.target.value })}
                className="rounded border border-border bg-background px-2 py-1 font-mono text-xs outline-none focus:border-foreground/50"
              />
              <input
                type="number"
                min={0}
                step={0.01}
                placeholder="还款金额(元)"
                value={r.amount || ""}
                onChange={(e) => updateRep(r.id, { amount: parseFloat(e.target.value) || 0 })}
                className="flex-1 rounded border border-border bg-background px-2 py-1 font-mono text-xs outline-none focus:border-foreground/50"
              />
              <button
                type="button"
                onClick={() =>
                  setRepayments((arr) => arr.filter((x) => x.id !== r.id))
                }
                className="rounded p-1 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                aria-label="删除还款"
              >
                <Trash2 className="size-3.5" />
              </button>
            </div>
          ))}
        </div>
      )}

      {/* 选项 */}
      <div className="space-y-2 rounded-md border border-border bg-muted/20 px-4 py-3">
        <Checkbox
          checked={multiCase}
          onChange={setMultiCase}
          label="多案合并计算"
        />
        <Checkbox
          checked={includeDelayed}
          onChange={setIncludeDelayed}
          label="计算迟延履行利息(加倍部分)"
        />
      </div>

      {/* 结果 */}
      {!results ? (
        <Placeholder>添加案件并填写本金后查看结果</Placeholder>
      ) : results.mode === "multi" ? (
        <ExecResultMerged result={results.merged} />
      ) : (
        <div className="space-y-3">
          {results.perCase.map((x, i) => (
            <ExecResultSingle key={i} caseInfo={x.info} result={x.result} />
          ))}
        </div>
      )}

      {/* 计算过程详情 */}
      {results && (
        <div>
          <button
            type="button"
            onClick={() => setShowDetail((v) => !v)}
            className="text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
          >
            {showDetail ? "收起计算过程" : "查看计算过程(还款抵扣明细)"}
          </button>
          {showDetail && (
            <div className="mt-2 space-y-3 rounded-md border border-dashed border-border bg-muted/20 px-3 py-3 text-label">
              {results.mode === "multi" ? (
                <ExecDetailBlock title="多案合并" result={results.merged} />
              ) : (
                results.perCase.map((x, i) => (
                  <ExecDetailBlock key={i} title={x.info.name} result={x.result} />
                ))
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/* ============================ Execution 子组件 ============================ */

interface ExecCaseFormData {
  id: number;
  name: string;
  principal: string;
  rate: string;
  rateType: RateType;
  lprTerm: LprTerm;
  lprMultiplier: string;
  startDate: string;
  endDate: string;
  litigationFee: string;
  lawyerFee: string;
  otherFee: string;
}

function makeBlankCase(): ExecCaseFormData {
  return {
    id: Date.now() + Math.floor(Math.random() * 1000),
    name: "",
    principal: "",
    rate: "",
    rateType: "custom",
    lprTerm: "1y",
    lprMultiplier: "1",
    startDate: "",
    endDate: todayIso(),
    litigationFee: "",
    lawyerFee: "",
    otherFee: "",
  };
}

function CaseRow({
  index,
  data,
  canDelete,
  onChange,
  onDelete,
}: {
  index: number;
  data: ExecCaseFormData;
  canDelete: boolean;
  onChange: (patch: Partial<ExecCaseFormData>) => void;
  onDelete: () => void;
}) {
  return (
    <div className="space-y-2 rounded-md border border-border bg-card px-4 py-3">
      <div className="flex items-center gap-2">
        <input
          type="text"
          placeholder={`案件 ${index + 1}(如:(2026)苏02民初0001 号)`}
          value={data.name}
          onChange={(e) => onChange({ name: e.target.value })}
          className="flex-1 rounded border border-border bg-background px-2 py-1.5 text-sm font-medium outline-none focus:border-foreground/50"
        />
        {canDelete && (
          <button
            type="button"
            onClick={onDelete}
            className="rounded p-1 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
            aria-label="删除案件"
          >
            <Trash2 className="size-3.5" />
          </button>
        )}
      </div>

      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <SmallField label="判决本金(元)">
          <input
            type="number"
            min={0}
            step={0.01}
            placeholder="判决确定本金"
            value={data.principal}
            onChange={(e) => onChange({ principal: e.target.value })}
            className="w-full rounded border border-border bg-background px-2 py-1.5 font-mono text-sm outline-none focus:border-foreground/50"
          />
        </SmallField>
        <SmallField label="利率">
          <div className="flex gap-1.5">
            <select
              value={data.rateType}
              onChange={(e) => onChange({ rateType: e.target.value as RateType })}
              className="flex-1 rounded border border-border bg-background px-2 py-1.5 text-sm outline-none focus:border-foreground/50"
            >
              <option value="custom">约定利率</option>
              <option value="lpr">LPR</option>
            </select>
            {data.rateType === "custom" ? (
              <div className="relative flex-1">
                <input
                  type="number"
                  min={0}
                  step={0.01}
                  placeholder="年利率"
                  value={data.rate}
                  onChange={(e) => onChange({ rate: e.target.value })}
                  className="w-full rounded border border-border bg-background px-2 py-1.5 pr-7 font-mono text-sm outline-none focus:border-foreground/50"
                />
                <span className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 text-xs text-muted-foreground">
                  %
                </span>
              </div>
            ) : (
              <>
                <select
                  value={data.lprTerm}
                  onChange={(e) => onChange({ lprTerm: e.target.value as LprTerm })}
                  className="min-w-0 flex-1 rounded border border-border bg-background px-2 py-1.5 text-sm outline-none focus:border-foreground/50"
                >
                  <option value="1y">1 年期 LPR</option>
                  <option value="5y+">5 年期以上 LPR</option>
                </select>
                <div className="relative w-24">
                  <input
                    type="number"
                    min={0}
                    step={0.01}
                    placeholder="1.5"
                    value={data.lprMultiplier}
                    onChange={(e) => onChange({ lprMultiplier: e.target.value })}
                    className="w-full rounded border border-border bg-background px-2 py-1.5 pr-7 font-mono text-sm outline-none focus:border-foreground/50"
                    aria-label="LPR 倍数"
                  />
                  <span className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 text-xs text-muted-foreground">
                    倍
                  </span>
                </div>
              </>
            )}
          </div>
        </SmallField>
        <SmallField label="起算日">
          <input
            type="date"
            value={data.startDate}
            onChange={(e) => onChange({ startDate: e.target.value })}
            className="w-full rounded border border-border bg-background px-2 py-1.5 font-mono text-sm outline-none focus:border-foreground/50"
          />
        </SmallField>
        <SmallField label="截止日">
          <input
            type="date"
            value={data.endDate}
            onChange={(e) => onChange({ endDate: e.target.value })}
            className="w-full rounded border border-border bg-background px-2 py-1.5 font-mono text-sm outline-none focus:border-foreground/50"
          />
        </SmallField>
        <SmallField label="诉讼费(元)">
          <input
            type="number"
            min={0}
            step={0.01}
            placeholder="0"
            value={data.litigationFee}
            onChange={(e) => onChange({ litigationFee: e.target.value })}
            className="w-full rounded border border-border bg-background px-2 py-1.5 font-mono text-sm outline-none focus:border-foreground/50"
          />
        </SmallField>
        <SmallField label="律师费(元)">
          <input
            type="number"
            min={0}
            step={0.01}
            placeholder="0"
            value={data.lawyerFee}
            onChange={(e) => onChange({ lawyerFee: e.target.value })}
            className="w-full rounded border border-border bg-background px-2 py-1.5 font-mono text-sm outline-none focus:border-foreground/50"
          />
        </SmallField>
        <SmallField label="其他费用(元)">
          <input
            type="number"
            min={0}
            step={0.01}
            placeholder="0"
            value={data.otherFee}
            onChange={(e) => onChange({ otherFee: e.target.value })}
            className="w-full rounded border border-border bg-background px-2 py-1.5 font-mono text-sm outline-none focus:border-foreground/50"
          />
        </SmallField>
      </div>
    </div>
  );
}

function ExecResultMerged({
  result,
}: {
  result: ReturnType<typeof calcFiveStage>;
}) {
  return (
    <div className="space-y-3 rounded-md border border-border bg-card px-5 py-4">
      <div>
        <p className="text-caption uppercase tracking-wider text-muted-foreground">
          多案合并 · 应付总额
        </p>
        <p className="mt-1 font-mono text-3xl font-semibold text-foreground">
          {formatMoney(result.total)}
        </p>
      </div>
      <BreakdownDl result={result} />
    </div>
  );
}

function ExecResultSingle({
  caseInfo,
  result,
}: {
  caseInfo: ExecCaseInput;
  result: ReturnType<typeof calcFiveStage>;
}) {
  return (
    <div className="space-y-3 rounded-md border border-border bg-card px-5 py-4">
      <div>
        <p className="text-caption uppercase tracking-wider text-muted-foreground">
          {caseInfo.name} · 应付总额
        </p>
        <p className="mt-1 font-mono text-2xl font-semibold text-foreground">
          {formatMoney(result.total)}
        </p>
      </div>
      <BreakdownDl result={result} />
    </div>
  );
}

function BreakdownDl({ result }: { result: ReturnType<typeof calcFiveStage> }) {
  return (
    <dl className="border-t border-border/70 pt-3 text-sm">
      <DetailRow label="剩余本金" value={formatMoney(result.remainingPrincipal)} />
      <DetailRow label="一般债务利息" value={formatMoney(result.accumulatedInterest)} />
      {result.accumulatedDelayed > 0 && (
        <DetailRow
          label="加倍部分债务利息"
          value={formatMoney(result.accumulatedDelayed)}
        />
      )}
      <DetailRow label="未付费用" value={formatMoney(result.remainingFees)} />
      <DetailRow label="合计" value={formatMoney(result.total)} strong />
    </dl>
  );
}

function ExecDetailBlock({
  title,
  result,
}: {
  title: string;
  result: ReturnType<typeof calcFiveStage>;
}) {
  return (
    <div>
      <p className="font-medium text-foreground">{title}</p>
      {result.steps.length > 0 && (
        <div className="mt-1 space-y-1 pl-3">
          {result.steps.map((s, i) => (
            <div key={i} className="text-muted-foreground">
              <p>
                {s.repDate} 还款 {formatMoney(s.repAmount)} · 距上次计息 {s.daysSinceLast} 天 · 新增利息 {formatMoney(s.newInterest)}
              </p>
              {s.interestSegments.length > 0 && (
                <div className="pl-3 font-mono text-muted-foreground/80">
                  {s.interestSegments.map((seg, si) => (
                    <p key={si}>
                      {seg.startDate} ~ {seg.endDate}: {formatInterestSegmentFormula(seg.principal, seg)} = {formatMoney(seg.interest)}
                    </p>
                  ))}
                </div>
              )}
              <p className="pl-3 font-mono">
                抵扣: {s.deductions.map((d) => `${d.type} ${formatMoney(d.amount)}`).join(" / ")}
              </p>
              <p className="pl-3 text-muted-foreground/80">
                余:本金 {formatMoney(s.remainingPrincipalAfter)} / 利息 {formatMoney(s.accumulatedInterestAfter)} / 费用 {formatMoney(s.remainingFeesAfter)}
              </p>
            </div>
          ))}
        </div>
      )}
      {result.finalDays > 0 && (
        <div className="mt-1 pl-3 text-muted-foreground">
          <p>末段利息:</p>
          <div className="pl-3 font-mono text-muted-foreground/80">
            {result.finalInterestSegments.map((seg, i) => (
              <p key={i}>
                {seg.startDate} ~ {seg.endDate}: {formatInterestSegmentFormula(seg.principal, seg)} = {formatMoney(seg.interest)}
              </p>
            ))}
          </div>
          <p className="pl-3">小计: {formatMoney(result.finalInterest)}</p>
        </div>
      )}
      {result.doubleSegments.length > 0 && (
        <div className="mt-1 pl-3">
          <p className="text-muted-foreground">加倍利息分段:</p>
          {result.doubleSegments.map((d, i) => (
            <p key={i} className="pl-3 font-mono text-muted-foreground/80">
              {d.start} ~ {d.end} · 本金 {formatMoney(d.principal)} × 0.0175% × {d.days} 天 = {formatMoney(d.interest)}
            </p>
          ))}
        </div>
      )}
    </div>
  );
}

/* ============================ 共享 UI ============================ */
function SmallField({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-1">
      <label className="block text-caption font-medium uppercase tracking-wider text-muted-foreground">
        {label}
      </label>
      {children}
    </div>
  );
}

function SectionHeader({
  title,
  children,
}: {
  title: string;
  children?: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between px-1">
      <h3 className="text-xs font-semibold text-foreground">{title}</h3>
      {children}
    </div>
  );
}

function Checkbox({
  checked,
  onChange,
  label,
  desc,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  desc?: string;
}) {
  return (
    <label className="flex cursor-pointer items-start gap-2 text-sm text-foreground">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="mt-0.5 size-4 cursor-pointer accent-foreground"
      />
      <span>
        {label}
        {desc && (
          <span className="mt-0.5 block text-label font-normal text-muted-foreground">
            {desc}
          </span>
        )}
      </span>
    </label>
  );
}

function Placeholder({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-md border border-dashed border-border/70 bg-muted/20 px-4 py-8 text-center text-xs text-muted-foreground">
      {children}
    </div>
  );
}

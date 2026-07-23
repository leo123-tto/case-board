/**
 * 利息 / 执行款核心计算 — 由早期网页原型迁移。
 *
 * 含:
 *   - daysBetween: 两日期之间天数(Math.ceil,半天算一天)
 *   - calculateInterestByPeriod: 按 LPR 变化点分段算利息(本金 × 年率 ÷ 360/365 × 天数)
 *   - calcFiveStage: 执行款五阶段清偿(费用 → 一般利息 → 本金,迟延履行利息独立累计)
 *   - calcExecution: 多案合并 / 单独计算的入口
 */

import { getLprForDate, LPR_DATA, type LprTerm } from "./lprData";

/* ============================ 基础工具 ============================ */

/** 两日期天数差(end - start),向上取整,负数返 0。 */
export function daysBetween(startDate: string, endDate: string): number {
  if (!startDate || !endDate) return 0;
  const start = new Date(startDate);
  const end = new Date(endDate);
  const diffMs = end.getTime() - start.getTime();
  const diffDays = Math.ceil(diffMs / (1000 * 60 * 60 * 24));
  return diffDays > 0 ? diffDays : 0;
}

/** Date → "YYYY-MM-DD"(本地时区) */
function isoLocal(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** 金额格式化。法律文书引用优先保留“元”口径,避免复制后再换算万元。 */
export function formatMoney(amount: number): string {
  return (
    amount.toLocaleString("zh-CN", {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    }) + " 元"
  );
}

export function normalizeLprMultiplier(multiplier?: number): number {
  if (multiplier === undefined || !Number.isFinite(multiplier)) return 1;
  if (multiplier < 0) return 1;
  return multiplier;
}

function effectiveLprRate(baseRate: number, multiplier?: number): number {
  return baseRate * normalizeLprMultiplier(multiplier);
}

/* ============================ 利息计算 ============================ */

export const PRIVATE_LENDING_CAP_SWITCH_DATE = "2020-08-20";

export type RateType = "custom" | "lpr" | "hybrid";
export type AnnualDayBasis = 360 | 365;

export interface InterestPrincipal {
  id: number;
  principal: string; // 输入框值(string,parse 时转 number)
  rateType: RateType;
  rate: string; // 自定义利率(%),rateType="custom"/"hybrid" 时用
  lprTerm: LprTerm; // LPR 期限,rateType="lpr"/"hybrid" 时用
  lprMultiplier: string; // LPR 倍数,rateType="lpr"/"hybrid" 时用
  startDate: string;
  endDate: string;
}

function calculateInterestRaw(
  principal: number,
  startDate: string,
  endDate: string,
  rateType: RateType,
  customRate: number,
  lprTerm: LprTerm,
  lprMultiplier = 1,
  annualDayBasis: AnnualDayBasis = 365,
): number {
  if (!principal || principal <= 0 || !startDate || !endDate) return 0;
  const start = new Date(startDate);
  const end = new Date(endDate);
  if (end <= start) return 0;

  if (rateType === "custom") {
    if (!customRate || isNaN(customRate)) return 0;
    const days = daysBetween(startDate, endDate);
    return (principal * customRate) / 100 / annualDayBasis * days;
  }

  if (rateType === "hybrid") {
    let totalInterest = 0;
    const switchDate = new Date(PRIVATE_LENDING_CAP_SWITCH_DATE);

    if (start < switchDate) {
      if (!customRate || isNaN(customRate)) return 0;

      const legacyEndDate =
        end < switchDate ? endDate : PRIVATE_LENDING_CAP_SWITCH_DATE;
      const legacyDays = daysBetween(startDate, legacyEndDate);
      totalInterest +=
        (principal * customRate) / 100 / annualDayBasis * legacyDays;
    }

    const postStartDate =
      start < switchDate ? PRIVATE_LENDING_CAP_SWITCH_DATE : startDate;
    if (new Date(endDate) > new Date(postStartDate)) {
      totalInterest += calculateInterestRaw(
        principal,
        postStartDate,
        endDate,
        "lpr",
        0,
        lprTerm,
        lprMultiplier,
        annualDayBasis,
      );
    }

    return totalInterest;
  }

  let totalInterest = 0;
  let currentDate = new Date(start);
  const initialLpr = getLprForDate(startDate, lprTerm) ?? 3.65;
  let currentRate = effectiveLprRate(initialLpr, lprMultiplier);

  for (let i = 0; i < LPR_DATA.length; i++) {
    const lprDate = new Date(LPR_DATA[i].date);
    if (lprDate <= start) continue;
    if (lprDate > end) break;

    const segStart = isoLocal(currentDate);
    const segEnd = isoLocal(lprDate);
    const segDays = daysBetween(segStart, segEnd);

    if (segDays > 0) {
      totalInterest +=
        (principal * currentRate) / 100 / annualDayBasis * segDays;
    }

    currentDate = lprDate;
    const nextBaseRate = lprTerm === "5y+" ? LPR_DATA[i].lpr5y : LPR_DATA[i].lpr1y;
    currentRate = effectiveLprRate(nextBaseRate, lprMultiplier);
  }

  const lastDays = daysBetween(isoLocal(currentDate), endDate);
  if (lastDays > 0 && currentRate) {
    totalInterest +=
      (principal * currentRate) / 100 / annualDayBasis * lastDays;
  }

  return totalInterest;
}

/**
 * 按 LPR 变化点分段算单笔利息。
 *
 * @returns 利息(元,保留 2 位小数)
 */
export function calculateInterestByPeriod(
  principal: number,
  startDate: string,
  endDate: string,
  rateType: RateType,
  customRate: number,
  lprTerm: LprTerm,
  lprMultiplier = 1,
  annualDayBasis: AnnualDayBasis = 365,
): number {
  const totalInterest = calculateInterestRaw(
    principal,
    startDate,
    endDate,
    rateType,
    customRate,
    lprTerm,
    lprMultiplier,
    annualDayBasis,
  );
  return Math.round(totalInterest * 100) / 100;
}

/* ============================ 利息详细分段(用于"查看计算过程") ============================ */

export interface InterestSegment {
  startDate: string;
  endDate: string;
  days: number;
  principal: number;
  rateType: RateType;
  baseRate: number;
  multiplier: number;
  rate: number;
  interest: number;
}

/** 按 LPR 变化点分段输出明细 — 给 UI 展示"计算过程"用 */
export function calculateInterestSegments(
  principal: number,
  startDate: string,
  endDate: string,
  rateType: RateType,
  customRate: number,
  lprTerm: LprTerm,
  lprMultiplier = 1,
  annualDayBasis: AnnualDayBasis = 365,
): InterestSegment[] {
  if (!principal || principal <= 0 || !startDate || !endDate) return [];
  const start = new Date(startDate);
  const end = new Date(endDate);
  if (end <= start) return [];

  if (rateType === "custom") {
    const days = daysBetween(startDate, endDate);
    return [
      {
        startDate,
        endDate,
        days,
        principal,
        rateType: "custom",
        baseRate: customRate,
        multiplier: 1,
        rate: customRate,
        interest:
          Math.round(
            (principal * customRate / 100 / annualDayBasis * days) * 100,
          ) / 100,
      },
    ];
  }

  if (rateType === "hybrid") {
    const segs: InterestSegment[] = [];
    const switchDate = new Date(PRIVATE_LENDING_CAP_SWITCH_DATE);

    if (start < switchDate) {
      if (!customRate || isNaN(customRate)) return [];

      const legacyEndDate =
        end < switchDate ? endDate : PRIVATE_LENDING_CAP_SWITCH_DATE;
      const legacyDays = daysBetween(startDate, legacyEndDate);
      if (legacyDays > 0) {
        segs.push({
          startDate,
          endDate: legacyEndDate,
          days: legacyDays,
          principal,
          rateType: "hybrid",
          baseRate: customRate,
          multiplier: 1,
          rate: customRate,
          interest:
            Math.round(
              (principal * customRate / 100 / annualDayBasis * legacyDays) *
                100,
            ) / 100,
        });
      }
    }

    const postStartDate =
      start < switchDate ? PRIVATE_LENDING_CAP_SWITCH_DATE : startDate;
    if (end > new Date(postStartDate)) {
      segs.push(
        ...calculateInterestSegments(
          principal,
          postStartDate,
          endDate,
          "lpr",
          0,
          lprTerm,
          lprMultiplier,
          annualDayBasis,
        ).map((seg) => ({
          ...seg,
          rateType: "hybrid" as const,
        })),
      );
    }

    return segs;
  }

  // LPR:按"利率实际变化"的点分段(连续相同利率合并)
  const segs: InterestSegment[] = [];
  let segStartDate = startDate;
  const multiplier = normalizeLprMultiplier(lprMultiplier);
  let currentBaseRate = getLprForDate(startDate, lprTerm) ?? 3.65;
  let currentRate = effectiveLprRate(currentBaseRate, multiplier);

  for (let i = 0; i < LPR_DATA.length; i++) {
    const lprDate = LPR_DATA[i].date;
    if (new Date(lprDate) <= new Date(startDate)) continue;
    if (new Date(lprDate) > new Date(endDate)) break;

    const newBaseRate = lprTerm === "5y+" ? LPR_DATA[i].lpr5y : LPR_DATA[i].lpr1y;
    const newRate = effectiveLprRate(newBaseRate, multiplier);
    if (Math.abs(newRate - currentRate) > 0.001) {
      const segDays = daysBetween(segStartDate, lprDate);
      if (segDays > 0) {
        segs.push({
          startDate: segStartDate,
          endDate: lprDate,
          days: segDays,
          principal,
          rateType: "lpr",
          baseRate: currentBaseRate,
          multiplier,
          rate: currentRate,
          interest:
            Math.round(
              (principal * currentRate / 100 / annualDayBasis * segDays) * 100,
            ) / 100,
        });
      }
      segStartDate = lprDate;
      currentBaseRate = newBaseRate;
      currentRate = newRate;
    }
  }

  const lastDays = daysBetween(segStartDate, endDate);
  if (lastDays > 0) {
    segs.push({
      startDate: segStartDate,
      endDate,
      days: lastDays,
      principal,
      rateType: "lpr",
      baseRate: currentBaseRate,
      multiplier,
      rate: currentRate,
      interest:
        Math.round(
          (principal * currentRate / 100 / annualDayBasis * lastDays) * 100,
        ) / 100,
    });
  }

  return segs;
}

/* ============================ 执行款五阶段清偿 ============================ */

export interface ExecCaseInput {
  id: number;
  name: string;
  principal: number;
  rate: number; // 年利率(%)
  rateType: RateType;
  lprTerm: LprTerm;
  lprMultiplier: number;
  startDate: string; // 计算起始日
  endDate: string;
  litigationFee: number;
  lawyerFee: number;
  otherFee: number;
}

export interface Repayment {
  id: number;
  date: string;
  amount: number;
}

export interface RepaymentStep {
  repDate: string;
  repAmount: number;
  daysSinceLast: number;
  newInterest: number;
  principalBefore: number;
  accumulatedInterestBefore: number;
  interestSegments: InterestSegment[];
  deductions: { type: "费用" | "一般利息" | "本金"; amount: number }[];
  remainingFeesAfter: number;
  accumulatedInterestAfter: number;
  remainingPrincipalAfter: number;
}

export interface DoubleSegment {
  start: string;
  end: string;
  days: number;
  principal: number;
  interest: number;
}

export interface FiveStageResult {
  remainingPrincipal: number;
  accumulatedInterest: number;
  accumulatedDelayed: number;
  remainingFees: number;
  finalDays: number;
  finalInterest: number;
  finalInterestSegments: InterestSegment[];
  total: number;
  steps: RepaymentStep[];
  doubleSegments: DoubleSegment[];
}

/**
 * 五阶段清偿计算(单个案件 / 多案合并后)。
 *
 * 还款按法定顺序抵扣:费用 → 一般利息 → 本金;
 * 迟延履行加倍利息独立累计(基数不含已产生的一般利息)。
 */
export function calcFiveStage(
  caseInfo: ExecCaseInput,
  repayments: Repayment[],
  includeDelayed: boolean,
  annualDayBasis: AnnualDayBasis = 365,
): FiveStageResult {
  const principal0 = caseInfo.principal;
  const startDate = caseInfo.startDate;
  const endDate0 = caseInfo.endDate;
  const totalFees = caseInfo.litigationFee + caseInfo.lawyerFee + caseInfo.otherFee;

  const delayedDailyRate = 0.000175; // 万分之一点七五

  let remainingPrincipal = principal0;
  let accumulatedInterest = 0;
  let remainingFees = totalFees;
  let lastInterestDate = startDate;
  const steps: RepaymentStep[] = [];

  // ── 第三阶段:逐笔还款 ──
  for (const rep of repayments) {
    const daysSinceLast = daysBetween(lastInterestDate, rep.date);
    const newInterest = calculateInterestRaw(
      remainingPrincipal,
      lastInterestDate,
      rep.date,
      caseInfo.rateType,
      caseInfo.rate,
      caseInfo.lprTerm,
      caseInfo.lprMultiplier,
      annualDayBasis,
    );
    const interestSegments = calculateInterestSegments(
      remainingPrincipal,
      lastInterestDate,
      rep.date,
      caseInfo.rateType,
      caseInfo.rate,
      caseInfo.lprTerm,
      caseInfo.lprMultiplier,
      annualDayBasis,
    );
    accumulatedInterest += newInterest;

    const step: RepaymentStep = {
      repDate: rep.date,
      repAmount: rep.amount,
      daysSinceLast,
      newInterest,
      principalBefore: remainingPrincipal,
      accumulatedInterestBefore: accumulatedInterest - newInterest,
      interestSegments,
      deductions: [],
      remainingFeesAfter: 0,
      accumulatedInterestAfter: 0,
      remainingPrincipalAfter: 0,
    };

    let remaining = rep.amount;

    // 费用 → 一般利息 → 本金
    if (remainingFees > 0 && remaining > 0) {
      const used = Math.min(remainingFees, remaining);
      remainingFees -= used;
      remaining -= used;
      step.deductions.push({ type: "费用", amount: used });
    }
    if (accumulatedInterest > 0 && remaining > 0) {
      const used = Math.min(accumulatedInterest, remaining);
      accumulatedInterest -= used;
      remaining -= used;
      step.deductions.push({ type: "一般利息", amount: used });
    }
    if (remainingPrincipal > 0 && remaining > 0) {
      const used = Math.min(remainingPrincipal, remaining);
      remainingPrincipal -= used;
      remaining -= used;
      step.deductions.push({ type: "本金", amount: used });
    }

    lastInterestDate = rep.date;
    step.remainingFeesAfter = remainingFees;
    step.accumulatedInterestAfter = accumulatedInterest;
    step.remainingPrincipalAfter = remainingPrincipal;
    steps.push(step);
  }

  // ── 第四阶段:末段利息 ──
  const finalStartDate =
    repayments.length > 0 ? repayments[repayments.length - 1].date : startDate;
  const finalDays = daysBetween(finalStartDate, endDate0);
  const finalInterest = calculateInterestRaw(
    remainingPrincipal,
    finalStartDate,
    endDate0,
    caseInfo.rateType,
    caseInfo.rate,
    caseInfo.lprTerm,
    caseInfo.lprMultiplier,
    annualDayBasis,
  );
  const finalInterestSegments = calculateInterestSegments(
    remainingPrincipal,
    finalStartDate,
    endDate0,
    caseInfo.rateType,
    caseInfo.rate,
    caseInfo.lprTerm,
    caseInfo.lprMultiplier,
    annualDayBasis,
  );
  accumulatedInterest += finalInterest;

  // ── 第五阶段:迟延履行加倍利息(独立累计) ──
  let accumulatedDelayed = 0;
  const doubleSegments: DoubleSegment[] = [];
  if (includeDelayed) {
    let delayedPrincipal = principal0;
    let lastDelayedDate = startDate;

    for (const rep of repayments) {
      const segStart = lastDelayedDate;
      const segEnd = rep.date;
      const segDays = daysBetween(segStart, segEnd);
      if (segDays > 0) {
        const segInterest = delayedPrincipal * delayedDailyRate * segDays;
        accumulatedDelayed += segInterest;
        doubleSegments.push({
          start: segStart,
          end: segEnd,
          days: segDays,
          principal: delayedPrincipal,
          interest: segInterest,
        });
      }

      // 还款抵扣:简化版(仅用于折减加倍利息计算基数中的本金)
      let remaining = rep.amount;
      const usedFee = Math.min(totalFees, remaining);
      remaining -= usedFee;
      const usedInt = Math.min(accumulatedInterest, remaining);
      remaining -= usedInt;
      const usedPrin = Math.min(delayedPrincipal, remaining);
      delayedPrincipal -= usedPrin;

      lastDelayedDate = rep.date;
    }

    const lastRepDate =
      repayments.length > 0 ? repayments[repayments.length - 1].date : startDate;
    const lastDelayedDays = daysBetween(lastRepDate, endDate0);
    if (lastDelayedDays > 0) {
      const lastSegInterest = delayedPrincipal * delayedDailyRate * lastDelayedDays;
      accumulatedDelayed += lastSegInterest;
      doubleSegments.push({
        start: lastRepDate,
        end: endDate0,
        days: lastDelayedDays,
        principal: delayedPrincipal,
        interest: lastSegInterest,
      });
    }
  }

  const total =
    remainingPrincipal + accumulatedInterest + accumulatedDelayed + remainingFees;

  return {
    remainingPrincipal,
    accumulatedInterest,
    accumulatedDelayed,
    remainingFees,
    finalDays,
    finalInterest,
    finalInterestSegments,
    total,
    steps,
    doubleSegments,
  };
}

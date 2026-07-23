export interface IndexProgressCounts {
  done: number;
  total: number;
}

const countFormatter = new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 });

export function formatIndexProgress({ done, total }: IndexProgressCounts): string {
  const safeTotal = Math.max(0, Math.trunc(total));
  const safeDone = Math.min(safeTotal, Math.max(0, Math.trunc(done)));
  const remaining = safeTotal - safeDone;
  return `总计 ${countFormatter.format(safeTotal)} · 已完成 ${countFormatter.format(safeDone)} · 剩余 ${countFormatter.format(remaining)}`;
}

export function indexProgressPercent({ done, total }: IndexProgressCounts): number {
  if (total <= 0) return 0;
  return Math.max(0, Math.min(100, Math.round((done / total) * 100)));
}

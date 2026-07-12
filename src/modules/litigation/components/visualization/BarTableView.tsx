import { useMemo } from "react";

import type { CaseGraphDataset, CaseGraphView } from "./types";

function configString(view: CaseGraphView, key: string): string | undefined {
  const value = view.config[key];
  return typeof value === "string" ? value : undefined;
}

interface Props {
  dataset: CaseGraphDataset;
  view: CaseGraphView;
}

export default function BarTableView({ dataset, view }: Props) {
  const labelKey = configString(view, "label_key") ?? dataset.columns.find((column) => column.type === "text")?.key;
  const valueKey = configString(view, "value_key") ?? dataset.columns.find((column) => column.type === "number")?.key;
  const rows = useMemo(() => {
    if (!labelKey || !valueKey) return [];
    return dataset.rows
      .map((row) => ({ label: String(row[labelKey] ?? ""), value: Number(row[valueKey] ?? 0) }))
      .filter((row) => row.label && Number.isFinite(row.value))
      .sort((left, right) => right.value - left.value);
  }, [dataset.rows, labelKey, valueKey]);
  const maximum = Math.max(...rows.map((row) => Math.abs(row.value)), 1);

  if (rows.length === 0) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        当前数据集缺少可用于数据条表格的文本列或数值列
      </div>
    );
  }
  return (
    <div data-visual-export-root className="h-full overflow-auto p-6">
      <table className="mx-auto w-full max-w-4xl border-separate border-spacing-0">
        <thead>
          <tr className="text-xs text-muted-foreground">
            <th className="border-b border-border px-3 py-2 text-left font-medium">项目</th>
            <th className="w-[52%] border-b border-border px-3 py-2 text-left font-medium">相对规模</th>
            <th className="border-b border-border px-3 py-2 text-right font-medium">数值</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={`${row.label}-${row.value}`}>
              <td className="border-b border-border/70 px-3 py-3 text-sm font-medium text-foreground">{row.label}</td>
              <td className="border-b border-border/70 px-3 py-3">
                <span
                  className="block h-2.5 min-w-px rounded-sm bg-brand/75"
                  style={{ width: `${Math.max(2, (Math.abs(row.value) / maximum) * 100)}%` }}
                  aria-label={`${row.label} 相对规模 ${Math.round((Math.abs(row.value) / maximum) * 100)}%`}
                />
              </td>
              <td className="border-b border-border/70 px-3 py-3 text-right font-mono text-sm tabular-nums text-foreground">
                {new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 2 }).format(row.value)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

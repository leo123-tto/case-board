import { useEffect, useRef, useState } from "react";
import type { EChartsCoreOption } from "echarts/core";

import type { CaseGraphDataset, CaseGraphView } from "./types";
import { ECHARTS_CASEBOARD_THEME } from "./visualizationTheme";
import { configBool, configChoice } from "./viewConfig";

function configString(view: CaseGraphView, key: string): string | undefined {
  const value = view.config[key];
  return typeof value === "string" ? value : undefined;
}

export function buildChartOption(
  dataset: CaseGraphDataset,
  view: CaseGraphView,
): EChartsCoreOption {
  if (!(["bar", "line", "heatmap"] as const).includes(view.kind as "bar" | "line" | "heatmap")) {
    throw new Error(`不支持的量化视图：${view.kind}`);
  }
  const textColumns = dataset.columns.filter((column) => column.type !== "number");
  const numberColumns = dataset.columns.filter((column) => column.type === "number");
  const categoryKey = configString(view, "category_key") ?? textColumns[0]?.key;
  const valueKey = configString(view, "value_key") ?? numberColumns[0]?.key;
  if (!categoryKey || !valueKey) throw new Error("量化视图缺少分类列或数值列");

  const base: EChartsCoreOption = {
    aria: {
      enabled: true,
      decal: { show: true },
      description: `${view.title}。数据来源：${dataset.title}。`,
    },
    animationDuration: 180,
    color: ECHARTS_CASEBOARD_THEME.color,
    dataset: {
      dimensions: dataset.columns.map((column) => column.key),
      source: dataset.rows,
    },
    tooltip: { trigger: "axis", confine: true },
    grid: { left: 56, right: 28, top: 36, bottom: 54, containLabel: true },
  };

  if (view.kind === "heatmap") {
    const xKey = configString(view, "x_key") ?? categoryKey;
    const yKey = configString(view, "y_key") ?? textColumns[1]?.key ?? categoryKey;
    return {
      ...base,
      tooltip: { position: "top", confine: true },
      xAxis: { type: "category", name: xKey, splitArea: { show: true } },
      yAxis: { type: "category", name: yKey, splitArea: { show: true } },
      visualMap: {
        min: 0,
        calculable: true,
        orient: "horizontal",
        left: "center",
        bottom: 0,
        inRange: { color: ["#eef3f7", "#8aa4bc", "#244f7a"] },
      },
      series: [{ type: "heatmap", encode: { x: xKey, y: yKey, value: valueKey } }],
    };
  }

  const horizontal = view.kind === "bar"
    && configChoice(view, "orientation", ["vertical", "horizontal"] as const, "vertical") === "horizontal";
  const showLabels = configBool(view, "show_labels", false);
  const smooth = view.kind === "line" ? configBool(view, "smooth", true) : false;

  return {
    ...base,
    xAxis: horizontal
      ? { type: "value", name: valueKey, splitLine: { lineStyle: { type: "dashed" } } }
      : { type: "category", name: categoryKey, axisLabel: { interval: 0, rotate: 18 } },
    yAxis: horizontal
      ? { type: "category", name: categoryKey, axisLabel: { interval: 0 } }
      : { type: "value", name: valueKey, splitLine: { lineStyle: { type: "dashed" } } },
    series: [
      {
        type: view.kind,
        encode: horizontal
          ? { x: valueKey, y: categoryKey, itemName: categoryKey, tooltip: [categoryKey, valueKey] }
          : { x: categoryKey, y: valueKey, itemName: categoryKey, tooltip: [categoryKey, valueKey] },
        barMaxWidth: view.kind === "bar" ? 42 : undefined,
        smooth: view.kind === "line" ? (smooth ? 0.18 : false) : undefined,
        label: showLabels ? { show: true, position: horizontal ? "right" : "top" } : undefined,
      },
    ],
  };
}

interface Props {
  dataset: CaseGraphDataset;
  view: CaseGraphView;
}

export default function QuantitativeChartView({ dataset, view }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    let resizeObserver: ResizeObserver | null = null;
    let chart: { resize: () => void; dispose: () => void } | null = null;
    const host = hostRef.current;
    if (!host) return;
    setError(null);

    void Promise.all([
      import("echarts/core"),
      import("echarts/charts"),
      import("echarts/components"),
      import("echarts/renderers"),
    ])
      .then(([core, charts, components, renderers]) => {
        if (disposed) return;
        core.use([
          charts.BarChart,
          charts.LineChart,
          charts.HeatmapChart,
          components.DatasetComponent,
          components.GridComponent,
          components.TooltipComponent,
          components.VisualMapComponent,
          components.AriaComponent,
          renderers.CanvasRenderer,
        ]);
        core.registerTheme("caseboard-visual", ECHARTS_CASEBOARD_THEME);
        const instance = core.init(host, "caseboard-visual", { renderer: "canvas" });
        instance.setOption(buildChartOption(dataset, view));
        chart = instance;
        if (typeof ResizeObserver !== "undefined") {
          resizeObserver = new ResizeObserver(() => instance.resize());
          resizeObserver.observe(host);
        }
      })
      .catch((reason: unknown) => {
        if (!disposed) setError(reason instanceof Error ? reason.message : "图表渲染失败");
      });

    return () => {
      disposed = true;
      resizeObserver?.disconnect();
      chart?.dispose();
    };
  }, [dataset, view]);

  if (error) {
    return (
      <div className="flex h-full items-center justify-center px-6 text-center text-sm text-destructive">
        图表无法显示：{error}
      </div>
    );
  }
  return <div ref={hostRef} data-visual-export-root className="h-full min-h-[360px] w-full" aria-label={view.title} />;
}

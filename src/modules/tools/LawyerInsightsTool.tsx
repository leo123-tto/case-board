import { useEffect, useMemo, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { BarChart3, Download, RefreshCw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { toast } from "@/components/ui/toast";
import {
  exportLawyerInsightsMarkdown,
  getLawyerInsights,
  revealInFinder,
  type InsightBucket,
  type LawyerInsightsReport,
} from "@/lib/api";
import { formatYuan } from "@/lib/format";
import { cn } from "@/lib/utils";

export function LawyerInsightsTool() {
  const [report, setReport] = useState<LawyerInsightsReport | null>(null);
  const [loading, setLoading] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      setReport(await getLawyerInsights());
    } catch (e) {
      setError(`办案画像生成失败:${e}`);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const mainInsight = useMemo(
    () => report?.strengths[0] ?? "正在整理办案画像。",
    [report],
  );

  const handleExport = async () => {
    if (!report) return;
    let savePath: string | null;
    try {
      savePath = await save({
        defaultPath: "办案画像报告.md",
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
    } catch (e) {
      toast(`打开保存对话框失败:${e}`, "error");
      return;
    }
    if (!savePath) return;

    setExporting(true);
    try {
      const written = await exportLawyerInsightsMarkdown(savePath);
      toast(`办案画像已导出:${written}`, "success", 8000);
      await revealInFinder(written).catch(() => {});
    } catch (e) {
      toast(`导出办案画像失败:${e}`, "error");
    } finally {
      setExporting(false);
    }
  };

  if (!report && loading) {
    return (
      <div className="rounded-xl border border-border bg-card p-5">
        <div className="h-4 w-28 rounded bg-muted" />
        <div className="mt-4 grid grid-cols-2 gap-3 md:grid-cols-4">
          {Array.from({ length: 4 }).map((_, idx) => (
            <div
              key={idx}
              className="h-16 rounded-lg border border-border bg-muted/30"
            />
          ))}
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="rounded-xl border border-destructive/30 bg-card p-5">
        <div className="flex items-center justify-between gap-3">
          <div>
            <h2 className="text-base font-semibold tracking-tight">办案画像</h2>
            <p className="mt-1 text-sm text-destructive">{error}</p>
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => void load()}
          >
            <RefreshCw className="size-3.5" />
            重试
          </Button>
        </div>
      </div>
    );
  }

  if (!report) return null;

  return (
    <div className="space-y-5">
      <section className="rounded-xl border border-border bg-card p-5 shadow-sm">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <BarChart3 className="size-4 text-foreground" />
              <h2 className="text-base font-semibold tracking-tight">办案画像</h2>
              <span className="rounded-full border border-border px-2 py-0.5 text-[11px] text-muted-foreground">
                本机统计
              </span>
            </div>
            <p className="mt-2 max-w-3xl text-sm leading-6 text-muted-foreground">
              {mainInsight}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => void load()}
              disabled={loading}
            >
              <RefreshCw className={cn("size-3.5", loading && "animate-spin")} />
              刷新
            </Button>
            <Button
              type="button"
              size="sm"
              onClick={() => void handleExport()}
              disabled={exporting || report.total_cases === 0}
            >
              <Download className="size-3.5" />
              导出 Markdown
            </Button>
          </div>
        </div>

        <div className="mt-5 grid grid-cols-2 gap-3 md:grid-cols-4">
          <Metric label="案件总数" value={`${report.total_cases}`} />
          <Metric
            label="在办 / 已结"
            value={`${report.active_cases} / ${report.closed_cases}`}
          />
          <Metric label="已生成报告" value={`${report.analyzed_cases}`} />
          <Metric
            label="平均标的"
            value={
              report.average_claim_amount
                ? formatYuan(report.average_claim_amount)
                : "暂无"
            }
          />
        </div>

        <div className="mt-5 grid grid-cols-1 gap-5 lg:grid-cols-3">
          <BucketList title="高频案由" buckets={report.top_causes} />
          <BucketList title="主要法院/地域" buckets={report.top_courts} />
          <BucketList title="代理立场" buckets={report.our_side_mix} />
        </div>

        <div className="mt-5 grid grid-cols-1 gap-5 border-t border-border pt-5 lg:grid-cols-2">
          <InsightList title="画像判断" items={report.strengths} />
          <InsightList
            title="可交给 AI 继续深挖"
            items={report.next_questions.slice(0, 3)}
          />
        </div>

        {report.data_gaps.length > 0 && (
          <div className="mt-5 border-t border-border pt-4">
            <p className="text-xs font-medium text-muted-foreground">数据边界</p>
            <p className="mt-1 text-xs leading-5 text-muted-foreground">
              {report.data_gaps[0]}
            </p>
          </div>
        )}
      </section>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-border bg-background/60 px-3 py-3">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="mt-1 truncate font-mono text-lg font-semibold tracking-tight text-foreground">
        {value}
      </p>
    </div>
  );
}

function BucketList({
  title,
  buckets,
}: {
  title: string;
  buckets: InsightBucket[];
}) {
  return (
    <div className="min-w-0">
      <h3 className="text-sm font-medium text-foreground">{title}</h3>
      <div className="mt-3 space-y-2">
        {buckets.slice(0, 4).map((bucket) => (
          <div key={bucket.label} className="min-w-0">
            <div className="flex items-center justify-between gap-3 text-sm">
              <span className="min-w-0 truncate text-foreground">
                {bucket.label}
              </span>
              <span className="shrink-0 font-mono text-xs text-muted-foreground">
                {bucket.count}件 · {Math.round(bucket.ratio * 100)}%
              </span>
            </div>
            <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-muted">
              <div
                className="h-full rounded-full bg-foreground"
                style={{
                  width: `${Math.max(4, Math.round(bucket.ratio * 100))}%`,
                }}
              />
            </div>
          </div>
        ))}
        {buckets.length === 0 && (
          <p className="text-sm text-muted-foreground">暂无可统计数据</p>
        )}
      </div>
    </div>
  );
}

function InsightList({ title, items }: { title: string; items: string[] }) {
  return (
    <div className="min-w-0">
      <h3 className="text-sm font-medium text-foreground">{title}</h3>
      <ul className="mt-3 space-y-2 text-sm leading-6 text-muted-foreground">
        {items.map((item) => (
          <li key={item} className="flex gap-2">
            <span className="mt-2 size-1.5 shrink-0 rounded-full bg-foreground/70" />
            <span>{item}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

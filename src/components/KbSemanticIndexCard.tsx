// 设置页 · 「法律向量检索」维护卡(raw 完整正文的语义补召回索引)
//
// 公开功能(进开源版):显示索引规模 + 手动重建(带进度)+ 自动维护开关。
// 自动维护:出报告 / 启动 / chat 完成后后台增量索引(见 Rust spawn_kb_auto_index);
// 这里只管「显示状态 + 手动重建 + 开关」。
import { useCallback, useEffect, useState } from "react";
import { Database, RefreshCw } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import {
  getLocalKbIndexStats,
  buildLocalKbSemanticIndex,
  type KbIndexStats,
} from "../lib/api";
import { formatIndexProgress, indexProgressPercent } from "./kbSemanticProgress";

interface IndexProgress {
  done: number;
  total: number;
  remaining?: number;
  phase: string;
}

export function KbSemanticIndexCard({
  embeddingConfigured,
  autoIndex,
  onAutoChange,
}: {
  embeddingConfigured: boolean;
  autoIndex: boolean;
  onAutoChange: (v: boolean) => void;
}) {
  const [stats, setStats] = useState<KbIndexStats | null>(null);
  const [building, setBuilding] = useState(false);
  const [progress, setProgress] = useState<IndexProgress | null>(null);
  const [msg, setMsg] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStats(await getLocalKbIndexStats());
    } catch {
      /* 没启用 KB 时静默 */
    }
  }, []);

  useEffect(() => {
    void refresh();
    const un = listen<IndexProgress>("kb_index_progress", (e) => {
      const p = e.payload;
      if (p.phase === "needs_manual") {
        setMsg(
          `检测到 ${p.total} 个待索引文件(法律/案例较多),自动索引已跳过——请点「重建索引」手动建一次。`,
        );
        return;
      }
      setProgress(p);
      if (p.phase === "done") {
        setTimeout(() => setProgress(null), 1500);
        void refresh();
      }
    });
    return () => {
      void un.then((f) => f());
    };
  }, [refresh]);

  const onBuild = useCallback(async () => {
    setBuilding(true);
    setMsg(null);
    setProgress({
      done: stats?.chunks ?? 0,
      total: stats?.total_chunks ?? stats?.chunks ?? 0,
      phase: "start",
    });
    try {
      const s = await buildLocalKbSemanticIndex();
      setStats(s);
      setMsg(`索引更新完成:${s.files} 个文件 / ${s.chunks} 个切片。`);
    } catch (e) {
      setMsg(`建索引失败:${String(e)}`);
    } finally {
      setBuilding(false);
    }
  }, []);

  return (
    <section>
      <div className="mb-3">
        <h3 className="text-sm font-semibold text-foreground">法律向量检索</h3>
        <p className="mt-0.5 text-xs text-muted-foreground">
          只把 raw 中的法规 / 案例 / 完整材料建成向量索引；Wiki 卡片、企业档案和归档不切向量。
        </p>
      </div>
      <div className="space-y-3 rounded-lg border border-border bg-background/50 p-4">
        {!embeddingConfigured && (
          <p className="rounded-md bg-amber-50 px-3 py-2 text-xs text-amber-800">
            需先在上方配置并验证 embedding 服务，才能建立向量索引；请求会使用该服务额度。
          </p>
        )}

        <div className="flex items-start justify-between gap-4 text-sm">
          <span className="shrink-0 text-muted-foreground">索引进度</span>
          {stats ? (
            <div className="text-right">
              <p className="font-medium text-foreground">
                {formatIndexProgress({ done: stats.chunks, total: stats.total_chunks })}
              </p>
              <p className="mt-0.5 text-xs text-muted-foreground">
                文件 {stats.files.toLocaleString("en-US")} / {stats.total_files.toLocaleString("en-US")}
              </p>
            </div>
          ) : (
            <span className="font-medium text-foreground">未建 / 读取中</span>
          )}
        </div>

        {progress && (
          <div className="space-y-1">
            <div className="flex flex-wrap justify-between gap-2 text-xs text-muted-foreground">
              <span>{progress.phase === "done" ? "完成" : "正在 embed…"}</span>
              <span>{formatIndexProgress(progress)}</span>
            </div>
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
              <div
                className="h-full bg-sky-500 transition-all"
                style={{
                  width: `${indexProgressPercent(progress)}%`,
                }}
              />
            </div>
          </div>
        )}

        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={onBuild}
            disabled={building || !embeddingConfigured}
            className="inline-flex items-center gap-1.5 rounded-md border border-sky-200 bg-sky-50 px-3 py-1.5 text-sm font-medium text-sky-700 transition-colors hover:bg-sky-100 disabled:opacity-50"
          >
            {building ? (
              <RefreshCw className="size-4 animate-spin" />
            ) : (
              <Database className="size-4" />
            )}
            {building ? "更新索引中…" : "继续 / 更新索引"}
          </button>
          <label className="ml-auto inline-flex cursor-pointer items-center gap-2 text-xs text-muted-foreground">
            <input
              type="checkbox"
              checked={autoIndex}
              onChange={(e) => onAutoChange(e.target.checked)}
              className="size-3.5 accent-sky-600"
            />
            自动维护(出报告 / 启动后台增量)
          </label>
        </div>

        {msg && (
          <div className="rounded-md bg-sky-50 px-3 py-2 text-xs text-sky-800">
            {msg}
          </div>
        )}
        <p className="text-xs text-muted-foreground">
          法规逐条切，普通正文均衡分段；切片与向量保存在本机并长期复用。查询文本仍会发送给你配置的 embedding 服务生成查询向量。
        </p>
      </div>
    </section>
  );
}

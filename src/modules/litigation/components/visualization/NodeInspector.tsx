import { FileText, LockKeyhole, UnlockKeyhole } from "lucide-react";

import { Button } from "@/components/ui/button";

import type { CaseGraphNode, FactStatus } from "./types";
import { NODE_KIND_LABELS, statusVisual } from "./visualizationTheme";

const STATUS_OPTIONS: FactStatus[] = [
  "confirmed",
  "our_claim",
  "opponent_claim",
  "disputed",
  "inferred",
  "unknown",
];

interface Props {
  node: CaseGraphNode | null;
  onChange: (node: CaseGraphNode) => void;
  onOpenSource?: (documentId: string) => void;
}

export default function NodeInspector({ node, onChange, onOpenSource }: Props) {
  if (!node) {
    return (
      <aside className="flex h-full items-center justify-center border-l border-border bg-surface-muted px-5 text-center text-xs text-muted-foreground">
        选择节点后可核对事实状态、材料依据并编辑
      </aside>
    );
  }

  function update<K extends keyof CaseGraphNode>(field: K, value: CaseGraphNode[K]) {
    if (!node) return;
    onChange({
      ...node,
      [field]: value,
      provenance: "user",
      locked_fields: Array.from(new Set([...(node.locked_fields ?? []), String(field)])),
    });
  }

  function toggleLock(field: keyof CaseGraphNode) {
    if (!node) return;
    const locks = new Set(node.locked_fields ?? []);
    if (locks.has(String(field))) locks.delete(String(field));
    else locks.add(String(field));
    onChange({ ...node, provenance: "user", locked_fields: [...locks] });
  }

  const visual = statusVisual(node.status);
  const labelLocked = node.locked_fields?.includes("label") ?? false;
  const detailLocked = node.locked_fields?.includes("detail") ?? false;

  return (
    <aside className="h-full overflow-y-auto border-l border-border bg-surface-muted">
      <div className="border-b border-border px-4 py-3">
        <div className="flex items-center justify-between gap-2">
          <div>
            <p className="text-[11px] text-muted-foreground">{NODE_KIND_LABELS[node.kind]}</p>
            <h2 className="mt-0.5 text-sm font-semibold text-foreground">节点详情</h2>
          </div>
          <span
            className="rounded border px-1.5 py-0.5 text-[10px] font-medium"
            style={{ color: visual.color, borderColor: visual.color, borderStyle: visual.borderStyle }}
          >
            {visual.marker} {visual.label}
          </span>
        </div>
      </div>

      <div className="space-y-5 px-4 py-4">
        <section>
          <div className="mb-1.5 flex items-center justify-between">
            <label htmlFor={`visual-label-${node.id}`} className="text-xs font-medium text-foreground">
              节点标题
            </label>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              className="h-6 px-1.5 text-[10px] text-muted-foreground"
              onClick={() => toggleLock("label")}
            >
              {labelLocked ? <LockKeyhole className="size-3" /> : <UnlockKeyhole className="size-3" />}
              {labelLocked ? "已锁定" : "未锁定"}
            </Button>
          </div>
          <input
            id={`visual-label-${node.id}`}
            aria-label="节点标题"
            value={node.label}
            onChange={(event) => update("label", event.target.value)}
            className="w-full rounded-md border border-border bg-background px-2.5 py-2 text-xs text-foreground outline-none focus:border-brand focus:ring-2 focus:ring-brand/15"
          />
        </section>

        <section>
          <div className="mb-1.5 flex items-center justify-between">
            <label htmlFor={`visual-detail-${node.id}`} className="text-xs font-medium text-foreground">
              律师备注 / 说明
            </label>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              className="h-6 px-1.5 text-[10px] text-muted-foreground"
              onClick={() => toggleLock("detail")}
            >
              {detailLocked ? <LockKeyhole className="size-3" /> : <UnlockKeyhole className="size-3" />}
              {detailLocked ? "已锁定" : "未锁定"}
            </Button>
          </div>
          <textarea
            id={`visual-detail-${node.id}`}
            aria-label="律师备注 / 说明"
            value={node.detail ?? ""}
            onChange={(event) => update("detail", event.target.value || undefined)}
            rows={5}
            className="w-full resize-y rounded-md border border-border bg-background px-2.5 py-2 text-xs leading-5 text-foreground outline-none focus:border-brand focus:ring-2 focus:ring-brand/15"
          />
        </section>

        <section className="grid grid-cols-2 gap-2">
          <label className="text-xs font-medium text-foreground">
            事实状态
            <select
              aria-label="事实状态"
              value={node.status}
              onChange={(event) => update("status", event.target.value as FactStatus)}
              className="mt-1.5 w-full rounded-md border border-border bg-background px-2 py-2 text-xs outline-none focus:border-brand"
            >
              {STATUS_OPTIONS.map((status) => (
                <option key={status} value={status}>
                  {statusVisual(status).label}
                </option>
              ))}
            </select>
          </label>
          <label className="text-xs font-medium text-foreground">
            确切日期
            <input
              aria-label="确切日期"
              type="date"
              value={node.date ?? ""}
              onChange={(event) => update("date", event.target.value || undefined)}
              className="mt-1.5 w-full rounded-md border border-border bg-background px-2 py-2 text-xs outline-none focus:border-brand"
            />
          </label>
        </section>

        <section>
          <label className="text-xs font-medium text-foreground">
            不完整日期描述
            <input
              aria-label="不完整日期描述"
              value={node.date_label ?? ""}
              onChange={(event) => update("date_label", event.target.value || undefined)}
              placeholder="例如：2026 年 2 月上旬"
              className="mt-1.5 w-full rounded-md border border-border bg-background px-2.5 py-2 text-xs outline-none focus:border-brand"
            />
          </label>
          <p className="mt-1 text-[10px] leading-4 text-muted-foreground">
            日期无法确认到日时保留文字描述，不补造具体日期。
          </p>
        </section>

        <section>
          <div className="mb-2 flex items-center justify-between">
            <h3 className="text-xs font-semibold text-foreground">材料依据</h3>
            <span className="text-[10px] text-muted-foreground">{node.source_refs.length} 项</span>
          </div>
          {node.source_refs.length === 0 ? (
            <div className="rounded-md border border-dashed border-border px-3 py-3 text-xs text-muted-foreground">
              尚未绑定材料，不能作为已确认关键事实直接使用。
            </div>
          ) : (
            <div className="space-y-1.5">
              {node.source_refs.map((source, index) => (
                <button
                  key={`${source.document_id}-${index}`}
                  type="button"
                  onClick={() => onOpenSource?.(source.document_id)}
                  className="flex w-full items-start gap-2 rounded-md border border-border bg-background px-2.5 py-2 text-left hover:border-brand/35 hover:bg-brand-soft/35"
                  aria-label={`打开材料 ${source.filename}`}
                >
                  <FileText className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
                  <span className="min-w-0">
                    <span className="block truncate text-xs font-medium text-foreground">{source.filename}</span>
                    {source.locator && (
                      <span className="mt-0.5 block text-[10px] text-muted-foreground">{source.locator}</span>
                    )}
                    {source.quote && (
                      <span className="mt-1 block line-clamp-3 text-[10px] leading-4 text-muted-foreground">
                        “{source.quote}”
                      </span>
                    )}
                  </span>
                </button>
              ))}
            </div>
          )}
        </section>
      </div>
    </aside>
  );
}

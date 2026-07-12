import { useMemo, useState } from "react";
import { LoaderCircle, Sparkles } from "lucide-react";

import { Button } from "@/components/ui/button";

import type { CaseGraphView } from "./types";

interface Props {
  view: CaseGraphView;
  busy: boolean;
  onSubmit: (request: string) => void | Promise<void>;
}

function presetsFor(view: CaseGraphView): string[] {
  if (view.kind === "timeline") return ["改成横向总览", "改成纵向阅读", "突出争议节点", "精简次要事件"];
  if (view.kind === "relationship" || view.kind === "mindmap") {
    return ["改成从左到右", "改成从上到下", "突出关键主体", "精简次要关系"];
  }
  if (view.kind === "evidence_matrix") return ["按争议焦点重组", "突出证据缺口", "补充不利证据", "精简重复材料"];
  if (["bar", "line", "heatmap", "bar_table"].includes(view.kind)) {
    return ["换一种更清楚的图形", "调整排序", "显示数值标签", "突出最大差异"];
  }
  return ["精简当前视图", "突出关键内容"];
}

export function buildAiVisualRequest(view: CaseGraphView, instruction: string): string {
  return [
    `请修改当前案情可视化工作区中的《${view.title}》（view_id: ${view.id}，类型: ${view.kind}）。`,
    `我的要求：${instruction.trim()}`,
    "请先调用 get_case_visualization 读取当前 revision 和稳定 id，再调用 apply_case_visual_update 直接应用最小修改。不要重建工作区，不要覆盖人工锁定字段，不要移动或改写案件原文件。",
    "若只是展示方式调整，请修改该 view 的受控 config；若涉及事实内容，必须保留事实状态和真实材料来源。完成后说明修改已经直接应用，可以查看或撤销。",
  ].join("\n");
}

export default function AiVisualAdjustPanel({ view, busy, onSubmit }: Props) {
  const [instruction, setInstruction] = useState("");
  const presets = useMemo(() => presetsFor(view), [view]);

  function submit() {
    const value = instruction.trim();
    if (!value || busy) return;
    void onSubmit(buildAiVisualRequest(view, value));
  }

  return (
    <aside className="h-full w-full overflow-y-auto border-l border-border bg-surface-muted" aria-label="AI 调整当前视图">
      <div className="border-b border-border px-4 py-3">
        <div className="flex items-center gap-2">
          <Sparkles className="size-4 text-brand" />
          <div>
            <p className="text-xs font-semibold text-foreground">让 AI 调整当前视图</p>
            <p className="mt-0.5 text-[10px] text-muted-foreground">{view.title}</p>
          </div>
        </div>
      </div>
      <div className="space-y-4 px-4 py-4">
        <section>
          <p className="text-[11px] font-medium text-muted-foreground">常用调整</p>
          <div className="mt-2 flex flex-wrap gap-1.5">
            {presets.map((preset) => (
              <button
                key={preset}
                type="button"
                onClick={() => setInstruction(preset)}
                disabled={busy}
                className="rounded-md border border-border bg-background px-2 py-1 text-[11px] text-foreground hover:border-brand/30 hover:bg-brand-soft/40 disabled:opacity-50"
              >
                {preset}
              </button>
            ))}
          </div>
        </section>
        <label className="block text-xs font-medium text-foreground">
          AI 修改要求
          <textarea
            aria-label="AI 修改要求"
            value={instruction}
            onChange={(event) => setInstruction(event.target.value)}
            disabled={busy}
            rows={6}
            placeholder="例如：改成横向时间线，只保留合同履行和争议发生阶段，并突出日期不明确的节点。"
            className="mt-1.5 w-full resize-y rounded-md border border-border bg-background px-2.5 py-2 text-xs leading-5 outline-none focus:border-brand focus:ring-2 focus:ring-brand/15 disabled:opacity-60"
          />
        </label>
        <Button type="button" size="sm" className="w-full" onClick={submit} disabled={busy || !instruction.trim()}>
          {busy ? <LoaderCircle className="size-3.5 animate-spin" /> : <Sparkles className="size-3.5" />}
          {busy ? "AI 正在应用修改" : "应用 AI 修改"}
        </Button>
        <p className="text-[10px] leading-4 text-muted-foreground">
          本次要求会直接应用到当前视图，并保留修订历史；如效果不合适，可使用撤销。
        </p>
      </div>
    </aside>
  );
}

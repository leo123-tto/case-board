import { SlidersHorizontal } from "lucide-react";

import type { CaseGraphView, ViewConfigValue } from "./types";
import { configBool, configChoice, withViewConfig } from "./viewConfig";

interface Props {
  view: CaseGraphView;
  onChange: (view: CaseGraphView) => void;
}

function SelectSetting({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: Array<{ value: string; label: string }>;
  onChange: (value: string) => void;
}) {
  return (
    <label className="block text-xs font-medium text-foreground">
      {label}
      <select
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        className="mt-1.5 w-full rounded-md border border-border bg-background px-2.5 py-2 text-xs outline-none focus:border-brand"
      >
        {options.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
      </select>
    </label>
  );
}

function ToggleSetting({ label, checked, onChange }: { label: string; checked: boolean; onChange: (value: boolean) => void }) {
  return (
    <label className="flex items-center justify-between gap-3 text-xs font-medium text-foreground">
      {label}
      <input
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
        className="size-4 accent-primary"
        aria-label={label}
      />
    </label>
  );
}

export default function ViewSettingsPanel({ view, onChange }: Props) {
  const update = (key: string, value: ViewConfigValue) => onChange(withViewConfig(view, key, value));

  return (
    <aside className="h-full w-full overflow-y-auto border-l border-border bg-surface-muted" aria-label="展示设置">
      <div className="border-b border-border px-4 py-3">
        <div className="flex items-center gap-2">
          <SlidersHorizontal className="size-4 text-brand" />
          <div>
            <p className="text-xs font-semibold text-foreground">展示设置</p>
            <p className="mt-0.5 text-[10px] text-muted-foreground">{view.title}</p>
          </div>
        </div>
      </div>
      <div className="space-y-5 px-4 py-4">
        {view.kind === "timeline" && (
          <>
            <SelectSetting
              label="时间线方向"
              value={configChoice(view, "orientation", ["vertical", "horizontal"] as const, "vertical")}
              options={[{ value: "vertical", label: "纵向阅读" }, { value: "horizontal", label: "横向总览" }]}
              onChange={(value) => update("orientation", value)}
            />
            <SelectSetting
              label="信息密度"
              value={configChoice(view, "density", ["comfortable", "compact"] as const, "comfortable")}
              options={[{ value: "comfortable", label: "舒展" }, { value: "compact", label: "紧凑" }]}
              onChange={(value) => update("density", value)}
            />
          </>
        )}
        {(view.kind === "relationship" || view.kind === "mindmap") && (
          <SelectSetting
            label="关系图方向"
            value={configChoice(view, "direction", ["LR", "TB"] as const, view.kind === "mindmap" ? "LR" : "TB")}
            options={[{ value: "LR", label: "从左到右" }, { value: "TB", label: "从上到下" }]}
            onChange={(value) => update("direction", value)}
          />
        )}
        {view.kind === "bar" && (
          <SelectSetting
            label="柱状图方向"
            value={configChoice(view, "orientation", ["vertical", "horizontal"] as const, "vertical")}
            options={[{ value: "vertical", label: "纵向柱" }, { value: "horizontal", label: "横向条" }]}
            onChange={(value) => update("orientation", value)}
          />
        )}
        {(view.kind === "bar" || view.kind === "line") && (
          <ToggleSetting label="显示数值标签" checked={configBool(view, "show_labels", false)} onChange={(value) => update("show_labels", value)} />
        )}
        {view.kind === "line" && (
          <ToggleSetting label="平滑曲线" checked={configBool(view, "smooth", true)} onChange={(value) => update("smooth", value)} />
        )}
        {!["timeline", "relationship", "mindmap", "bar", "line"].includes(view.kind) && (
          <p className="rounded-md border border-dashed border-border px-3 py-3 text-xs leading-5 text-muted-foreground">
            当前视图暂无额外展示选项，可使用节点编辑或让 AI 调整内容。
          </p>
        )}
        <p className="text-[10px] leading-4 text-muted-foreground">展示设置只影响当前视图，不改变案件原始材料。</p>
      </div>
    </aside>
  );
}

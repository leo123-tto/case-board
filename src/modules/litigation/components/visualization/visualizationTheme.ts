import type { FactStatus, NodeKind } from "./types";

export interface StatusVisual {
  label: string;
  color: string;
  background: string;
  borderStyle: "solid" | "dashed" | "dotted";
  lineStyle: "solid" | "dashed" | "dotted";
  marker: string;
}

const STATUS_VISUALS: Record<FactStatus, StatusVisual> = {
  confirmed: {
    label: "材料确认",
    color: "#31594a",
    background: "#edf5f1",
    borderStyle: "solid",
    lineStyle: "solid",
    marker: "✓",
  },
  our_claim: {
    label: "我方主张",
    color: "#244f7a",
    background: "#edf3f9",
    borderStyle: "solid",
    lineStyle: "solid",
    marker: "我",
  },
  opponent_claim: {
    label: "对方主张",
    color: "#8a4b38",
    background: "#f9f0ed",
    borderStyle: "solid",
    lineStyle: "solid",
    marker: "对",
  },
  disputed: {
    label: "存在争议",
    color: "#9a6117",
    background: "#fbf5e8",
    borderStyle: "dashed",
    lineStyle: "dashed",
    marker: "!",
  },
  inferred: {
    label: "AI 推断",
    color: "#66527e",
    background: "#f3eff7",
    borderStyle: "dashed",
    lineStyle: "dashed",
    marker: "推",
  },
  unknown: {
    label: "未知",
    color: "#68707a",
    background: "#f1f3f5",
    borderStyle: "dotted",
    lineStyle: "dotted",
    marker: "?",
  },
};

export function statusVisual(status: FactStatus): StatusVisual {
  return STATUS_VISUALS[status];
}

export const NODE_KIND_LABELS: Record<NodeKind, string> = {
  actor: "主体",
  event: "事件",
  claim: "诉请",
  issue: "争议焦点",
  legal_basis: "法律依据",
  element: "构成要件",
  defense: "抗辩",
  evidence: "证据",
  amount: "金额",
  document: "材料",
  action: "下一步",
};

export const ECHARTS_CASEBOARD_THEME = {
  color: ["#244f7a", "#31594a", "#9a6117", "#8a4b38", "#66527e"],
  textStyle: {
    color: "#262b31",
    fontFamily: 'system-ui, -apple-system, "SF Pro Text", sans-serif',
  },
  categoryAxis: {
    axisLine: { lineStyle: { color: "#c8cdd3" } },
    axisTick: { show: false },
    axisLabel: { color: "#5d6670" },
    splitLine: { show: false },
  },
  valueAxis: {
    axisLine: { show: false },
    axisTick: { show: false },
    axisLabel: { color: "#5d6670" },
    splitLine: { lineStyle: { color: "#e5e8eb", type: "dashed" } },
  },
};

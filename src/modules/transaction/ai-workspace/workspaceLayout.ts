export type WorkspacePane = "sidebar" | "document" | "chat";

export interface WorkspaceLayout {
  left: number;
  right: number;
  collapsed: { sidebar: boolean; chat: boolean };
  fullscreen: WorkspacePane | null;
}

export const WORKSPACE_LAYOUT_STORAGE_KEY = "caseboard.ai-workspace.layout.v1";
export const DEFAULT_WORKSPACE_LAYOUT: WorkspaceLayout = {
  left: 264,
  right: 360,
  collapsed: { sidebar: false, chat: false },
  fullscreen: null,
};

const MIN_LEFT = 220;
const MAX_LEFT = 420;
const MIN_RIGHT = 300;
const MAX_RIGHT = 520;
const MIN_CENTER = 420;

const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

export function normalizeWorkspaceLayout(
  layout: WorkspaceLayout,
  totalWidth: number,
): WorkspaceLayout {
  let left = clamp(layout.left, MIN_LEFT, MAX_LEFT);
  let right = clamp(layout.right, MIN_RIGHT, MAX_RIGHT);
  const availableForSides = Math.max(MIN_LEFT + MIN_RIGHT, totalWidth - MIN_CENTER);
  const overflow = left + right - availableForSides;
  if (overflow > 0) {
    const rightReduction = Math.min(overflow, right - MIN_RIGHT);
    right -= rightReduction;
    left = Math.max(MIN_LEFT, left - (overflow - rightReduction));
  }
  return { ...layout, left, right };
}

export function resizeWorkspacePane(
  layout: WorkspaceLayout,
  pane: "sidebar" | "chat",
  delta: number,
  totalWidth: number,
): WorkspaceLayout {
  return normalizeWorkspaceLayout(
    pane === "sidebar"
      ? { ...layout, left: layout.left + delta }
      : { ...layout, right: layout.right - delta },
    totalWidth,
  );
}

export function toggleWorkspacePane(
  layout: WorkspaceLayout,
  pane: "sidebar" | "chat",
): WorkspaceLayout {
  return {
    ...layout,
    collapsed: { ...layout.collapsed, [pane]: !layout.collapsed[pane] },
  };
}

export function toggleWorkspaceFullscreen(
  layout: WorkspaceLayout,
  pane: WorkspacePane,
): WorkspaceLayout {
  return { ...layout, fullscreen: layout.fullscreen === pane ? null : pane };
}

export function isCompactWorkspace(width: number): boolean {
  return width < 980;
}

export function loadWorkspaceLayout(): WorkspaceLayout {
  try {
    const raw = localStorage.getItem(WORKSPACE_LAYOUT_STORAGE_KEY);
    if (!raw) return DEFAULT_WORKSPACE_LAYOUT;
    const parsed = JSON.parse(raw) as Partial<WorkspaceLayout>;
    return {
      left: typeof parsed.left === "number" ? parsed.left : DEFAULT_WORKSPACE_LAYOUT.left,
      right: typeof parsed.right === "number" ? parsed.right : DEFAULT_WORKSPACE_LAYOUT.right,
      collapsed: {
        sidebar: parsed.collapsed?.sidebar === true,
        chat: parsed.collapsed?.chat === true,
      },
      fullscreen:
        parsed.fullscreen === "sidebar" ||
        parsed.fullscreen === "document" ||
        parsed.fullscreen === "chat"
          ? parsed.fullscreen
          : null,
    };
  } catch {
    return DEFAULT_WORKSPACE_LAYOUT;
  }
}

export function saveWorkspaceLayout(layout: WorkspaceLayout): void {
  localStorage.setItem(WORKSPACE_LAYOUT_STORAGE_KEY, JSON.stringify(layout));
}

import { useCallback, useState, type ReactNode } from "react";
import { Columns2, Maximize2 } from "lucide-react";

import { Button } from "@/components/ui/button";

export type HomeProductivityCardWidth = "half" | "full";
export type HomeProductivityCardId = "calendar" | "ticktick";

const HOME_CARD_WIDTH_KEYS: Record<HomeProductivityCardId, string> = {
  calendar: "caseboard:home-card-width:calendar:v1",
  ticktick: "caseboard:home-card-width:ticktick:v1",
};

export function loadHomeProductivityCardWidth(
  card: HomeProductivityCardId,
): HomeProductivityCardWidth {
  try {
    return window.localStorage.getItem(HOME_CARD_WIDTH_KEYS[card]) === "full" ? "full" : "half";
  } catch {
    return "half";
  }
}

export function saveHomeProductivityCardWidth(
  card: HomeProductivityCardId,
  width: HomeProductivityCardWidth,
): void {
  try {
    window.localStorage.setItem(HOME_CARD_WIDTH_KEYS[card], width);
  } catch {
    // localStorage 不可用时只影响跨会话记忆，不阻断本次宽度切换。
  }
}

export function useHomeProductivityCardWidth(card: HomeProductivityCardId) {
  const [width, setWidthState] = useState<HomeProductivityCardWidth>(() =>
    loadHomeProductivityCardWidth(card),
  );
  const setWidth = useCallback(
    (next: HomeProductivityCardWidth) => {
      setWidthState(next);
      saveHomeProductivityCardWidth(card, next);
    },
    [card],
  );
  return [width, setWidth] as const;
}

export function HomeProductivityCardWidthToggle({
  cardLabel,
  width,
  onWidthChange,
}: {
  cardLabel: string;
  width: HomeProductivityCardWidth;
  onWidthChange: (width: HomeProductivityCardWidth) => void;
}) {
  const nextWidth: HomeProductivityCardWidth = width === "half" ? "full" : "half";
  const nextLabel = nextWidth === "full" ? "整行" : "半行";
  const Icon = nextWidth === "full" ? Maximize2 : Columns2;

  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      onClick={() => onWidthChange(nextWidth)}
      aria-label={`${cardLabel}切换为${nextLabel}`}
      title={`${cardLabel}切换为${nextLabel}`}
      className="h-7 gap-1.5 px-2 text-xs text-muted-foreground"
    >
      <Icon className="size-3.5" />
      {nextLabel}
    </Button>
  );
}

export function homeProductivityCardSpan(width: HomeProductivityCardWidth): string {
  return width === "full" ? "md:col-span-2" : "md:col-span-1";
}

/** 首页日程/待办卡片统一栅格：窄窗口单列，桌面端每张卡等宽占半行。 */
export function HomeProductivityGrid({ children }: { children: ReactNode }) {
  return (
    <div
      role="region"
      aria-label="首页日程与待办"
      className="mb-6 grid grid-cols-1 gap-4 empty:hidden md:grid-cols-2"
    >
      {children}
    </div>
  );
}

/**
 * 法律计算工具卡片 — 可点击,跳到工具视图。
 *
 * 可选 badge(如"重写中")— 在标题旁加 amber 小标签。
 */

import { ChevronRight, type LucideIcon } from "lucide-react";

interface Props {
  icon: LucideIcon;
  title: string;
  desc: string;
  onClick: () => void;
  /** 可选状态标签(如 "重写中" / "Beta" / "新" / "即将上线") */
  badge?: string;
  /** 置灰、不可点(用于「即将上线」占位卡) */
  disabled?: boolean;
}

export function LegalToolCard({
  icon: Icon,
  title,
  desc,
  onClick,
  badge,
  disabled,
}: Props) {
  return (
    <button
      type="button"
      onClick={disabled ? undefined : onClick}
      disabled={disabled}
      className={`interactive-surface group flex min-h-[76px] items-start gap-3 rounded-xl border border-border bg-card/90 px-4 py-3.5 text-left ${
        disabled
          ? "cursor-not-allowed opacity-50"
          : "hover:border-brand/25 hover:bg-card"
      }`}
    >
      <span
        className={`flex size-9 shrink-0 items-center justify-center rounded-lg border transition-[transform,background-color,border-color,color] duration-200 ${
          disabled
            ? "border-border bg-muted text-foreground/40"
            : "border-brand/10 bg-brand-soft/70 text-brand group-hover:scale-[1.04] group-hover:border-brand/20 group-hover:bg-brand-soft"
        }`}
      >
        <Icon className="size-[18px]" />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <h3 className="text-sm font-medium text-foreground">{title}</h3>
          {badge && (
            <span className="rounded bg-amber-100 px-1.5 py-0.5 text-[9px] font-medium uppercase tracking-wider text-amber-800">
              {badge}
            </span>
          )}
        </div>
        <p className="mt-0.5 line-clamp-2 text-label leading-relaxed text-muted-foreground">
          {desc}
        </p>
      </div>
      {!disabled && (
        <ChevronRight className="mt-2 size-4 shrink-0 text-muted-foreground/45 transition-transform duration-200 group-hover:translate-x-0.5 group-hover:text-brand" />
      )}
    </button>
  );
}

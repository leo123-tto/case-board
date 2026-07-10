/**
 * 即时悬停说明气泡。
 *
 * 取代原生 `title` —— 原生 tooltip 在 Tauri WebView 里延迟约 2-3 秒、还是不显眼的
 * 系统小黄条(作者反馈"看不到")。本组件用纯 CSS group-hover 即时弹出,
 * 统一使用与主界面一致的浅色磨砂浮层,默认往上弹、水平居中。
 *
 * 用法:用它包住任意可悬停元素(按钮 / 图标 / 文字):
 *   <HoverHint hint="仅导出元典缓存,不含笔记/案件">
 *     <Button>导出资料包</Button>
 *   </HoverHint>
 *
 * 注意:气泡是绝对定位往上弹,若所在容器有 `overflow-hidden` 可能被裁;
 * 用在滚动区中部一般没问题(上方有内容垫着)。
 */

import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

export function HoverHint({
  hint,
  children,
  className,
}: {
  hint: string;
  children: ReactNode;
  /** 覆盖外层 wrapper 的样式(默认 inline-flex) */
  className?: string;
}) {
  return (
    <span className={cn("group relative inline-flex", className)}>
      {children}
      <span
        role="tooltip"
        className="glass-tooltip pointer-events-none invisible absolute bottom-full left-1/2 z-50 mb-2 w-max max-w-[260px] -translate-x-1/2 translate-y-1 scale-[0.98] rounded-lg px-3 py-2 text-xs opacity-0 transition-[opacity,transform,visibility] duration-150 ease-out group-focus-within:visible group-focus-within:translate-y-0 group-focus-within:scale-100 group-focus-within:opacity-100 group-hover:visible group-hover:translate-y-0 group-hover:scale-100 group-hover:opacity-100"
      >
        {hint}
      </span>
    </span>
  );
}

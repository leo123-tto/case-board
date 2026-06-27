/**
 * 顶部模块导航条:左边「首页」按钮 + 三 tab(诉讼 · 非诉 · 工具)。
 *
 * 设计(2026-05-24 b 作者拍板):顶部横排,当前 tab 下划线高亮,苹果风克制。
 * 2026-05-24 e:加首页按钮,任何模块/任何子页都可以一键回诉讼首页(HomeView)。
 *
 * App.tsx 用 useState 跟 activeModule,这里只是个 dumb component。
 */

import { useLayoutEffect, useRef, useState, type ComponentType } from "react";
import {
  Briefcase,
  Brain,
  FileQuestion,
  Gavel,
  Home,
  Scale,
  Settings as SettingsIcon,
  Users,
  Wrench,
} from "lucide-react";

import { cn } from "@/lib/utils";
import { BetaBadge } from "@/components/BetaBadge";
// 私人专属功能接缝(双轨发布模型):开源仓 getPrivateTopTabs() 返回 [] → 无「独立」标签。
import { getPrivateTopTabs } from "@/private";

type TabIcon = ComponentType<{ className?: string }>;

export type ModuleId =
  | "litigation"
  | "criminal"
  | "execution"
  | "transaction"
  | "tools"
  | "memory"
  | "team"
  | "settings";

// 2026-05-24 j · 加「执行」tab(诉讼之后),自动筛 workflow_status='执行中' 的案件
// 2026-05-25 V0.1.8 · 加「设置」tab(工具之后),作者反馈:别人找不到右上角齿轮
const MODULES: { id: string; label: string; icon: TabIcon; beta?: boolean }[] = [
  { id: "litigation", label: "诉讼", icon: Briefcase },
  // 2026-06-17 · 加「刑事」tab(诉讼之后):复刻诉讼看板框架,只显示刑事案件,
  // AI 助手只保留「刑事深度分析」单 chip(三阶层鉴定式,借鉴游初 gutachten-criminal-case)。
  // 2026-06-18 · 标 Beta(尚在打磨,结果需自行核对)。
  { id: "criminal", label: "刑事", icon: Scale, beta: true },
  { id: "execution", label: "执行", icon: Gavel },
  { id: "transaction", label: "非诉", icon: FileQuestion },
  { id: "tools", label: "工具", icon: Wrench },
  { id: "memory", label: "记忆", icon: Brain },
  // 2026-06-10 团队版 Phase 1:LAN 接力同步团队看板(未入团显示引导页)
  { id: "team", label: "团队", icon: Users },
  { id: "settings", label: "设置", icon: SettingsIcon },
];

export function ModuleTabs({
  active,
  onSwitch,
  onGoHome,
  rightSlot,
}: {
  active: string;
  onSwitch: (id: string) => void;
  /** 「首页」按钮点击:切到诉讼模块 + 重置到 HomeView(由 App.tsx 处理) */
  onGoHome: () => void;
  /** 2026-05-24 e:右侧自定义插槽(给 DeepSeekBalanceChip 等用) */
  rightSlot?: React.ReactNode;
}) {
  // 单条「滑动下划线」:跟踪当前激活 tab 的位置/宽度,切换时 transition-all 平滑滑过去
  // (取代原来每个 tab 各自条件渲染下划线 → 切换时硬切)。
  const rowRef = useRef<HTMLDivElement>(null);
  const [underline, setUnderline] = useState<{ left: number; width: number }>({
    left: 0,
    width: 0,
  });
  useLayoutEffect(() => {
    const el = rowRef.current?.querySelector<HTMLElement>(
      `[data-tab="${active}"]`,
    );
    if (el) {
      // 内缩 8px(对齐原 inset-x-2 视觉),让下划线比 tab 略窄更精致
      setUnderline({ left: el.offsetLeft + 8, width: el.offsetWidth - 16 });
    }
  }, [active]);

  // 核心 6 tab + 私人专属顶层 tab(开源仓为空)。「独立」排最后(设置之后)。
  const allTabs: { id: string; label: string; icon: TabIcon; beta?: boolean }[] =
    [
      ...MODULES,
      ...getPrivateTopTabs().map((t) => ({
        id: t.id,
        label: t.label,
        icon: t.icon,
      })),
    ];

  return (
    <nav className="flex shrink-0 border-b border-border bg-card/50 px-8">
      <div
        ref={rowRef}
        className="relative mx-auto flex w-full max-w-6xl items-center gap-1"
      >
        {/* 左侧首页按钮 — 单独样式,跟 tab 视觉区分(图标 + 边框 + 不带下划线) */}
        <button
          type="button"
          onClick={onGoHome}
          className="mr-2 inline-flex items-center gap-1.5 rounded-md border border-border bg-card px-2.5 py-1.5 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          title="回到首页(诉讼案件看板)"
          aria-label="首页"
        >
          <Home className="size-3.5" />
          <span className="font-medium">首页</span>
        </button>

        {/* 核心 tab + 私人「独立」tab */}
        {allTabs.map((m) => {
          const isActive = m.id === active;
          const Icon = m.icon;
          return (
            <button
              key={m.id}
              type="button"
              data-tab={m.id}
              onClick={() => onSwitch(m.id)}
              className={cn(
                "relative flex items-center gap-1.5 px-4 py-3 text-sm transition-colors",
                isActive
                  ? "text-foreground"
                  : "text-muted-foreground hover:text-foreground",
              )}
              aria-current={isActive ? "page" : undefined}
            >
              <Icon className="size-4" />
              <span className="font-medium">{m.label}</span>
              {m.beta && <BetaBadge className="ml-0.5" />}
            </button>
          );
        })}

        {/* 单条滑动下划线(平滑移动到激活 tab) */}
        <span
          className="pointer-events-none absolute bottom-0 h-0.5 rounded-full bg-foreground transition-all duration-300 ease-out"
          style={{ left: underline.left, width: underline.width }}
        />

        {/* 右侧插槽(DeepSeek 余额 chip 等) */}
        {rightSlot && <div className="ml-auto flex items-center gap-2">{rightSlot}</div>}
      </div>
    </nav>
  );
}

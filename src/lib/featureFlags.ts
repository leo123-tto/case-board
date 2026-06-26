import { useState, useEffect, useCallback } from "react";

/**
 * 首页 / 界面功能开关(feature flags)统一机制。
 *
 * 作者偏好「清清爽爽的界面」:首页新功能默认关闭,用户想要再去设置 / 对应功能里打开。
 * 约定:以后首页新增模块 → 在 FEATURE_FLAGS 注册表里加一条即可:
 *   - 设置页「界面 / 首页」分区会自动渲染 location==="settings" 的开关;
 *   - 首页组件用 useFeatureFlag(name) 条件渲染;
 *   - 无需改后端 settings(纯前端 UI 偏好)。
 *
 * 存储:localStorage,key = `caseboard:feature:<name>`。
 *   per-device、不跨设备同步 —— 符合「这台机器界面清不清爽」的语义,
 *   与首页其它 `caseboard:*` 偏好(view_mode / sort / filters …)一致。
 */

const PREFIX = "caseboard:feature:";
const CHANGE_EVENT = "caseboard:feature-change";

export type FeatureFlagName =
  | "home_filter_bar"
  | "home_ticktick"
  | "case_court_filing"
  | "case_todos"
  | "case_work_logs"
  | "case_work_reports"
  | "reference_materials";

export interface FeatureFlagMeta {
  name: FeatureFlagName;
  /** 开关旁显示的标题 */
  title: string;
  /** 一句话说明 */
  description: string;
  /** 默认值(作者偏好:一律默认关,保持清爽) */
  defaultValue: boolean;
  /** 开关 UI 主要放在哪:settings=设置页分区渲染;feature=对应功能页自己放 */
  location: "settings" | "feature";
}

/** 注册表:所有首页功能开关。**新增首页功能 → 在这里加一条。** */
export const FEATURE_FLAGS: FeatureFlagMeta[] = [
  {
    name: "home_filter_bar",
    title: "首页筛选工具栏",
    description:
      "显示首页的排序 / 筛选 / 列表视图 / 多选工具栏。关闭则回到清爽的纯案件卡片网格。",
    defaultValue: false,
    location: "settings",
  },
  {
    name: "home_ticktick",
    title: "首页滴答清单待办",
    description: "在首页显示滴答清单同步的待办汇总。关闭则首页不显示待办块。",
    defaultValue: false,
    location: "feature",
  },
  {
    name: "case_court_filing",
    title: "案件详情显示「辅助在线立案」",
    description:
      "在案件详情页底部显示「辅助在线立案」区(实验性,依赖本机 Python 运行时)。默认关闭,保持详情页清爽;需要时在此打开。",
    defaultValue: false,
    location: "feature",
  },
  {
    name: "case_todos",
    title: "案件待办清单",
    description:
      "开启后在案件详情显示待办清单，可设置日期并汇总到首页。",
    defaultValue: false,
    location: "settings",
  },
  {
    name: "case_work_logs",
    title: "案件工作记录",
    description:
      "开启后在案件详情显示工作记录，可直接保存，或选择由 AI 整理后保存。",
    defaultValue: false,
    location: "settings",
  },
  {
    name: "case_work_reports",
    title: "案件工作汇报",
    description:
      "开启后在案件详情显示工作汇报，可按需汇总案件概况、办案时间轴和工作记录，并导出 Word。",
    defaultValue: false,
    location: "settings",
  },
  {
    name: "reference_materials",
    title: "参考材料自动识别",
    description:
      "开启后识别参考材料、参考案例等文件夹，默认不纳入本案事实分析。",
    defaultValue: false,
    location: "settings",
  },
];

const META_BY_NAME: Record<string, FeatureFlagMeta> = Object.fromEntries(
  FEATURE_FLAGS.map((f) => [f.name, f]),
);

function defaultOf(name: FeatureFlagName): boolean {
  return META_BY_NAME[name]?.defaultValue ?? false;
}

export function getFeatureFlag(name: FeatureFlagName): boolean {
  try {
    const raw = localStorage.getItem(PREFIX + name);
    if (raw === null) return defaultOf(name);
    return raw === "1" || raw === "true";
  } catch {
    return defaultOf(name);
  }
}

export function setFeatureFlag(name: FeatureFlagName, value: boolean): void {
  try {
    localStorage.setItem(PREFIX + name, value ? "1" : "0");
  } catch {
    /* localStorage 不可用时静默 —— UI 偏好不致命 */
  }
  // storage 事件只跨窗口、不触发本窗口,这里用自定义事件让同窗口其它组件即时同步
  try {
    window.dispatchEvent(
      new CustomEvent(CHANGE_EVENT, { detail: { name, value } }),
    );
  } catch {
    /* ignore */
  }
}

/** React hook:读 + 写一个开关,跨组件即时同步。返回 [enabled, setEnabled]。 */
export function useFeatureFlag(
  name: FeatureFlagName,
): [boolean, (value: boolean) => void] {
  const [enabled, setEnabled] = useState<boolean>(() => getFeatureFlag(name));

  useEffect(() => {
    const onChange = (e: Event) => {
      const detail = (e as CustomEvent).detail as
        | { name: FeatureFlagName; value: boolean }
        | undefined;
      if (detail && detail.name === name) setEnabled(detail.value);
    };
    const onStorage = (e: StorageEvent) => {
      if (e.key === PREFIX + name) setEnabled(getFeatureFlag(name));
    };
    window.addEventListener(CHANGE_EVENT, onChange);
    window.addEventListener("storage", onStorage);
    return () => {
      window.removeEventListener(CHANGE_EVENT, onChange);
      window.removeEventListener("storage", onStorage);
    };
  }, [name]);

  const set = useCallback(
    (value: boolean) => {
      setFeatureFlag(name, value);
      setEnabled(value);
    },
    [name],
  );

  return [enabled, set];
}

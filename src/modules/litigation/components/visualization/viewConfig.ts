import type { CaseGraphView, ViewConfigValue } from "./types";

export function configChoice<T extends string>(
  view: CaseGraphView,
  key: string,
  allowed: readonly T[],
  fallback: T,
): T {
  const value = view.config[key];
  return typeof value === "string" && allowed.includes(value as T) ? value as T : fallback;
}

export function configBool(view: CaseGraphView, key: string, fallback: boolean): boolean {
  const value = view.config[key];
  return typeof value === "boolean" ? value : fallback;
}

export function withViewConfig(
  view: CaseGraphView,
  key: string,
  value: ViewConfigValue,
): CaseGraphView {
  return { ...view, config: { ...view.config, [key]: value } };
}

export const THEME_STORAGE_KEY = "caseboard-theme";

export const THEMES = [
  {
    id: "default",
    label: "默认",
    description: "现有的冷灰蓝与墨蓝配色",
  },
  {
    id: "emerald_ivory",
    label: "墨绿象牙",
    description: "外部贡献者设计的墨绿强调色与暖象牙底色",
  },
] as const;

export type ThemeId = (typeof THEMES)[number]["id"];

export function isThemeId(value: string | null): value is ThemeId {
  return THEMES.some((theme) => theme.id === value);
}

export function getThemePreference(): ThemeId {
  if (typeof window === "undefined") return "default";
  const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
  return isThemeId(stored) ? stored : "default";
}

export function applyThemePreference(theme = getThemePreference()): ThemeId {
  if (typeof document === "undefined") return theme;
  if (theme === "default") {
    document.documentElement.removeAttribute("data-theme");
  } else {
    document.documentElement.dataset.theme = theme;
  }
  return theme;
}

export function setThemePreference(theme: ThemeId): void {
  window.localStorage.setItem(THEME_STORAGE_KEY, theme);
  applyThemePreference(theme);
  window.dispatchEvent(new CustomEvent("caseboard-theme-change", { detail: theme }));
}

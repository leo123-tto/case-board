export type HomeViewMode = "grid" | "list";
export type HomeSortKey = "status" | "amount" | "filed_at" | "hearing";
export type HomeSortDir = "asc" | "desc";

export interface HomeListPreferences {
  viewMode: HomeViewMode;
  sortKey: HomeSortKey;
  sortDir: HomeSortDir;
}

export const HOME_LIST_PREFERENCES_KEY = "caseboard:home-list-preferences:v1";

const DEFAULT_PREFERENCES: HomeListPreferences = {
  viewMode: "grid",
  sortKey: "status",
  sortDir: "asc",
};

const VIEW_MODES = new Set<HomeViewMode>(["grid", "list"]);
const SORT_KEYS = new Set<HomeSortKey>(["status", "amount", "filed_at", "hearing"]);
const SORT_DIRS = new Set<HomeSortDir>(["asc", "desc"]);

export function parseHomeListPreferences(raw: string | null): HomeListPreferences {
  if (!raw) return { ...DEFAULT_PREFERENCES };
  try {
    const value = JSON.parse(raw) as Partial<HomeListPreferences>;
    return {
      viewMode: VIEW_MODES.has(value.viewMode as HomeViewMode)
        ? (value.viewMode as HomeViewMode)
        : DEFAULT_PREFERENCES.viewMode,
      sortKey: SORT_KEYS.has(value.sortKey as HomeSortKey)
        ? (value.sortKey as HomeSortKey)
        : DEFAULT_PREFERENCES.sortKey,
      sortDir: SORT_DIRS.has(value.sortDir as HomeSortDir)
        ? (value.sortDir as HomeSortDir)
        : DEFAULT_PREFERENCES.sortDir,
    };
  } catch {
    return { ...DEFAULT_PREFERENCES };
  }
}

export function loadHomeListPreferences(): HomeListPreferences {
  try {
    return parseHomeListPreferences(window.localStorage.getItem(HOME_LIST_PREFERENCES_KEY));
  } catch {
    return { ...DEFAULT_PREFERENCES };
  }
}

export function saveHomeListPreferences(preferences: HomeListPreferences): void {
  try {
    window.localStorage.setItem(HOME_LIST_PREFERENCES_KEY, JSON.stringify(preferences));
  } catch {
    // localStorage 被系统策略禁用时仍允许使用本次会话，不阻断首页。
  }
}

import {
  STATUS_LIST,
  type StatusId,
} from "@/modules/litigation/lib/inferStatus";

export type HomeViewMode = "grid" | "list";
export type HomeSortKey = "status" | "amount" | "filed_at" | "hearing";
export type HomeSortDir = "asc" | "desc";

export interface HomeListPreferences {
  viewMode: HomeViewMode;
  sortKey: HomeSortKey;
  sortDir: HomeSortDir;
  statusFilters: StatusId[];
  courtFilter: string;
}

export const HOME_LIST_PREFERENCES_KEY = "caseboard:home-list-preferences:v1";

const DEFAULT_PREFERENCES: HomeListPreferences = {
  viewMode: "grid",
  sortKey: "status",
  sortDir: "asc",
  statusFilters: [],
  courtFilter: "",
};

const VIEW_MODES = new Set<HomeViewMode>(["grid", "list"]);
const SORT_KEYS = new Set<HomeSortKey>(["status", "amount", "filed_at", "hearing"]);
const SORT_DIRS = new Set<HomeSortDir>(["asc", "desc"]);
const STATUS_IDS = new Set<StatusId>(STATUS_LIST.map(({ id }) => id));

export function parseHomeListPreferences(raw: string | null): HomeListPreferences {
  if (!raw) return { ...DEFAULT_PREFERENCES };
  try {
    const value = JSON.parse(raw) as Partial<HomeListPreferences>;
    const statusFilters = Array.isArray(value.statusFilters)
      ? [
          ...new Set(
            value.statusFilters.filter(
              (id): id is StatusId =>
                typeof id === "string" && STATUS_IDS.has(id as StatusId),
            ),
          ),
        ]
      : [];
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
      statusFilters,
      courtFilter: typeof value.courtFilter === "string" ? value.courtFilter : "",
    };
  } catch {
    return { ...DEFAULT_PREFERENCES };
  }
}

export function clearHomeListFilters(
  preferences: HomeListPreferences,
): HomeListPreferences {
  return { ...preferences, statusFilters: [], courtFilter: "" };
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

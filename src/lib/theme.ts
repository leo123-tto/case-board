import { getFeatureFlag } from "@/lib/featureFlags";

const THEME_ATTR = "data-theme";

export function applyThemePreference(enabled = getFeatureFlag("theme_emerald")) {
  const root = document.documentElement;
  if (enabled) {
    root.setAttribute(THEME_ATTR, "emerald");
  } else {
    root.removeAttribute(THEME_ATTR);
  }
}

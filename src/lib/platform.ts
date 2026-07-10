export type UiPlatform = "macos" | "windows" | "linux" | "unknown";

export function detectUiPlatform(userAgent = navigator.userAgent): UiPlatform {
  const ua = userAgent.toLowerCase();
  if (ua.includes("windows")) return "windows";
  if (ua.includes("macintosh") || ua.includes("mac os")) return "macos";
  if (ua.includes("linux")) return "linux";
  return "unknown";
}

/** 在 React 首屏前标记平台,供字体、滚动条和高 DPI 布局做轻量差异化。 */
export function applyPlatformIdentity(): void {
  try {
    document.documentElement.dataset.platform = detectUiPlatform();
  } catch {
    /* 非浏览器 / 测试环境不处理 */
  }
}

/**
 * 格式化工具函数。
 */

/**
 * 把人民币金额格式化成「¥ 千位分隔」展示。统一**案件金额**(诉讼请求 / 律师费 / 还款 / 执行余额)
 * 口径,消除各视图小数位 drift(原先 HomeView 0 位、CaseSnapshot 2 位、执行模块默认各写一套)。
 *
 * 策略:千位逗号 + 最多 2 位小数 + 去尾随 0(`¥ 500,000` / `¥ 1,234.5` / `¥ 99.99`)。
 * 仅用于"给人看"的展示;编辑态仍显示纯数字字符串(让用户改)。
 * 注:DeepSeek 余额等 API 成本(恒 2 位 `toFixed(2)`)是另一域,不走这个。
 *
 * @example
 *   formatYuan(500000)   // "¥ 500,000"
 *   formatYuan(1234.5)   // "¥ 1,234.5"
 *   formatYuan(null)     // "—"
 */
export function formatYuan(amount: number | null | undefined): string {
  if (amount == null || Number.isNaN(amount)) return "—";
  return `¥ ${amount.toLocaleString("zh-CN", { maximumFractionDigits: 2 })}`;
}

/**
 * 把字节数格式化成易读的 KB / MB / GB。
 *
 * @example
 *   formatBytes(0)       // "0 B"
 *   formatBytes(1023)    // "1023 B"
 *   formatBytes(1024)    // "1.0 KB"
 *   formatBytes(1500000) // "1.4 MB"
 */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) {
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

/**
 * 截断长路径,显示开头几段 + ... + 结尾几段,适合在卡片里展示。
 *
 * @example
 *   shortenPath("/a/b/c/d/e/f/g", 2)
 *   // → "/a/b/.../f/g"
 */
export function shortenPath(path: string, keepEnds = 2): string {
  const separator = path.includes("\\") ? "\\" : "/";
  const parts = path.split(/[\\/]/).filter(Boolean);
  if (parts.length <= keepEnds * 2 + 1) return path;
  const head = parts.slice(0, keepEnds);
  const tail = parts.slice(-keepEnds);
  const prefix = path.startsWith("\\\\") ? "\\\\" : path.startsWith("/") ? "/" : "";
  return prefix + [...head, "...", ...tail].join(separator);
}

/**
 * 把 ISO 8601 时间格式化成中文相对时间。
 *
 * SQLite 用 `datetime('now')` 存的是 `YYYY-MM-DD HH:MM:SS`(UTC),要先转 ISO。
 *
 * @example
 *   formatRelativeTime("2026-05-22 09:30:00")  // → "几秒前" / "5 分钟前" / "2 小时前" / "昨天" / "3 天前" / "2026-04-12"
 */
export function formatRelativeTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  // SQLite datetime() 输出是 'YYYY-MM-DD HH:MM:SS' UTC,转 ISO
  const normalized = iso.includes("T") ? iso : iso.replace(" ", "T") + "Z";
  const past = new Date(normalized);
  if (Number.isNaN(past.getTime())) return iso;

  const diffMs = Date.now() - past.getTime();
  const diffSec = Math.floor(diffMs / 1000);
  if (diffSec < 60) return "刚刚";

  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin} 分钟前`;

  const diffHour = Math.floor(diffMin / 60);
  if (diffHour < 24) return `${diffHour} 小时前`;

  const diffDay = Math.floor(diffHour / 24);
  if (diffDay === 1) return "昨天";
  if (diffDay < 7) return `${diffDay} 天前`;

  // 超过一周直接显示日期 YYYY-MM-DD
  const y = past.getFullYear();
  const m = String(past.getMonth() + 1).padStart(2, "0");
  const d = String(past.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}`;
}

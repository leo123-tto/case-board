/**
 * LPR 历史数据 + 查询函数。
 *
 * 数据从 2019-08-20(中国人民银行公布 LPR 新机制起点)到 2026-06-22,
 * 每月一档,80 个数据点。日期对应公布日,利率适用于其后至下次公布前。
 *
 * 维护:每月 20 日左右人民银行公布最新 LPR,需要在数组末尾追加一条。
 */

export interface LprPoint {
  date: string; // YYYY-MM-DD
  lpr1y: number; // 1 年期 LPR(%)
  lpr5y: number; // 5 年期以上 LPR(%)
}

export const LPR_DATA: LprPoint[] = [
  { date: "2019-08-20", lpr1y: 4.25, lpr5y: 4.85 },
  { date: "2019-09-20", lpr1y: 4.2, lpr5y: 4.85 },
  { date: "2019-10-21", lpr1y: 4.2, lpr5y: 4.85 },
  { date: "2019-11-20", lpr1y: 4.15, lpr5y: 4.8 },
  { date: "2019-12-20", lpr1y: 4.15, lpr5y: 4.8 },
  { date: "2020-01-20", lpr1y: 4.15, lpr5y: 4.8 },
  { date: "2020-02-20", lpr1y: 4.05, lpr5y: 4.75 },
  { date: "2020-03-20", lpr1y: 4.05, lpr5y: 4.75 },
  { date: "2020-04-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2020-05-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2020-06-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2020-07-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2020-08-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2020-09-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2020-10-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2020-11-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2020-12-21", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2021-01-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2021-02-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2021-03-22", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2021-04-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2021-05-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2021-06-21", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2021-07-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2021-08-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2021-09-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2021-10-20", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2021-11-22", lpr1y: 3.85, lpr5y: 4.65 },
  { date: "2021-12-20", lpr1y: 3.8, lpr5y: 4.65 },
  { date: "2022-01-20", lpr1y: 3.7, lpr5y: 4.6 },
  { date: "2022-02-21", lpr1y: 3.7, lpr5y: 4.6 },
  { date: "2022-03-21", lpr1y: 3.7, lpr5y: 4.6 },
  { date: "2022-04-20", lpr1y: 3.7, lpr5y: 4.6 },
  { date: "2022-05-20", lpr1y: 3.7, lpr5y: 4.45 },
  { date: "2022-06-20", lpr1y: 3.7, lpr5y: 4.45 },
  { date: "2022-07-20", lpr1y: 3.7, lpr5y: 4.45 },
  { date: "2022-08-22", lpr1y: 3.65, lpr5y: 4.3 },
  { date: "2022-09-20", lpr1y: 3.65, lpr5y: 4.3 },
  { date: "2022-10-20", lpr1y: 3.65, lpr5y: 4.3 },
  { date: "2022-11-21", lpr1y: 3.65, lpr5y: 4.3 },
  { date: "2022-12-20", lpr1y: 3.65, lpr5y: 4.3 },
  { date: "2023-01-20", lpr1y: 3.65, lpr5y: 4.3 },
  { date: "2023-02-20", lpr1y: 3.65, lpr5y: 4.3 },
  { date: "2023-03-20", lpr1y: 3.65, lpr5y: 4.3 },
  { date: "2023-04-20", lpr1y: 3.65, lpr5y: 4.3 },
  { date: "2023-05-22", lpr1y: 3.65, lpr5y: 4.3 },
  { date: "2023-06-20", lpr1y: 3.55, lpr5y: 4.2 },
  { date: "2023-07-20", lpr1y: 3.55, lpr5y: 4.2 },
  { date: "2023-08-21", lpr1y: 3.45, lpr5y: 4.2 },
  { date: "2023-09-20", lpr1y: 3.45, lpr5y: 4.2 },
  { date: "2023-10-20", lpr1y: 3.45, lpr5y: 4.2 },
  { date: "2023-11-20", lpr1y: 3.45, lpr5y: 4.2 },
  { date: "2023-12-20", lpr1y: 3.45, lpr5y: 4.2 },
  { date: "2024-01-22", lpr1y: 3.45, lpr5y: 4.2 },
  { date: "2024-02-20", lpr1y: 3.45, lpr5y: 3.95 },
  { date: "2024-03-20", lpr1y: 3.45, lpr5y: 3.95 },
  { date: "2024-04-20", lpr1y: 3.45, lpr5y: 3.95 },
  { date: "2024-05-20", lpr1y: 3.45, lpr5y: 3.95 },
  { date: "2024-06-20", lpr1y: 3.45, lpr5y: 3.95 },
  { date: "2024-07-22", lpr1y: 3.35, lpr5y: 3.85 },
  { date: "2024-08-20", lpr1y: 3.35, lpr5y: 3.85 },
  { date: "2024-09-20", lpr1y: 3.35, lpr5y: 3.85 },
  { date: "2024-10-20", lpr1y: 3.1, lpr5y: 3.6 },
  { date: "2024-11-20", lpr1y: 3.1, lpr5y: 3.6 },
  { date: "2024-12-20", lpr1y: 3.1, lpr5y: 3.6 },
  { date: "2025-01-20", lpr1y: 3.1, lpr5y: 3.6 },
  { date: "2025-02-20", lpr1y: 3.1, lpr5y: 3.6 },
  { date: "2025-03-20", lpr1y: 3.1, lpr5y: 3.6 },
  { date: "2025-04-21", lpr1y: 3.1, lpr5y: 3.6 },
  { date: "2025-05-20", lpr1y: 3.1, lpr5y: 3.6 },
  { date: "2025-06-20", lpr1y: 3.0, lpr5y: 3.5 },
  { date: "2025-07-21", lpr1y: 3.0, lpr5y: 3.5 },
  { date: "2025-08-20", lpr1y: 3.0, lpr5y: 3.5 },
  { date: "2025-09-22", lpr1y: 3.0, lpr5y: 3.5 },
  { date: "2025-10-20", lpr1y: 3.0, lpr5y: 3.5 },
  { date: "2025-11-20", lpr1y: 3.0, lpr5y: 3.5 },
  { date: "2025-12-22", lpr1y: 3.0, lpr5y: 3.5 },
  { date: "2026-01-20", lpr1y: 3.0, lpr5y: 3.5 },
  { date: "2026-02-24", lpr1y: 3.0, lpr5y: 3.5 },
  { date: "2026-03-20", lpr1y: 3.0, lpr5y: 3.5 },
  { date: "2026-04-20", lpr1y: 3.0, lpr5y: 3.5 },
  { date: "2026-05-20", lpr1y: 3.0, lpr5y: 3.5 },
  { date: "2026-06-22", lpr1y: 3.0, lpr5y: 3.5 },
];

export type LprTerm = "1y" | "5y+";

/**
 * 找指定日期适用的 LPR 利率(%)— 最后一个 ≤ targetDate 的 LPR 公布点。
 * @param dateStr "YYYY-MM-DD"
 * @param term 期限选择
 * @returns 利率,如 3.65;无对应数据时返 null
 */
export function getLprForDate(dateStr: string, term: LprTerm): number | null {
  if (!dateStr) return null;
  const target = new Date(dateStr);
  let lpr: number | null = null;
  for (const item of LPR_DATA) {
    const d = new Date(item.date);
    if (d <= target) {
      lpr = term === "5y+" ? item.lpr5y : item.lpr1y;
    } else {
      break;
    }
  }
  return lpr;
}

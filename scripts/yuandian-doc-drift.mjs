#!/usr/bin/env node
//
// 元典官方文档 → 本地快照的反向漂移检查。
//
// 跟 `yuandian-catalog-contract.mjs` 的分工:
//   - catalog-contract:线上目录 vs CaseBoard **代码契约**(方法/积分/参数名),候选版门禁跑。
//   - 本脚本:线上文档 vs **本地文档快照**(docs/元典API-llms.txt + 积分计费明细),
//     逐行比对参数的「类型 / 必填 / 默认值 / 枚举取值范围」——这些光比参数名看不出来,
//     但改了会直接让线上调用失败或静默改变结果。
//
// 用法:node scripts/yuandian-doc-drift.mjs   (联网,无需 API key;有漂移退出码 1)
// 发现漂移后:确认影响 → 改代码 → 用 llms-full.txt 刷新本地快照 → 更新积分计费明细。
//
// ⚠️ 本机专用,别挂进候选版门禁:依赖的 docs/元典API-llms.txt 是 **gitignored** 的本地快照,
// 换机器/新 clone 上要么没有这个文件,要么是没补录过的官方原版(官方静态文档滞后于 JSON 目录),
// 跑出来会是一堆假漂移。下面的 assertSnapshotUsable 会先把这种情况报清楚,而不是伪装成漂移。

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SNAPSHOT = path.join(ROOT, "docs/元典API-llms.txt");
const PRICING = path.join(ROOT, "docs/元典接口-积分计费明细.md");
const CATALOG_URL =
  "https://open.chineselaw.com/api/apis?pageNum=1&pageSize=200&sortBy=latest";

// 表头/返回字段说明里的通用词,不是请求参数
const NOISE = new Set(["msg", "code", "extra", "key"]);

/** 参数表 → Map<字段名, 规范化后的整行>,保留类型/必填/说明以便逐行比对。 */
function rowsOf(markdown = "") {
  const rows = new Map();
  const lines = markdown.split("\n").map((line) => line.replace(/\r$/, "").trim());
  const headerRows = new Set();
  lines.forEach((line, index) => {
    if (/^\|[\s:|-]+\|$/.test(line) && index > 0) headerRows.add(index - 1);
  });
  lines.forEach((line, index) => {
    if (headerRows.has(index) || !line.startsWith("|")) return;
    const cols = line
      .split("|")
      .map((col) => col.replace(/[`*]/g, "").replace(/\s+/g, " ").trim());
    const name = cols[1];
    if (!name || !/^[A-Za-z_][A-Za-z0-9_.]*$/.test(name) || NOISE.has(name)) return;
    if (!rows.has(name)) rows.set(name, cols.slice(2).filter(Boolean).join(" | "));
  });
  return rows;
}

/** 只取「N. 请求参数」到「N. 返回/响应」之间,避开返回字段说明表。标题可能带 ** 加粗。 */
function requestSection(block) {
  const match = block.match(
    /##\s*\*{0,2}\s*\d+\.\s*请求参数[\s\S]*?(?=\n##\s*\*{0,2}\s*\d+\.\s*(?:返回|响应)|$)/,
  );
  return match ? match[0] : "";
}

/** 标题措辞在快照和目录之间不完全一致("接口"后缀、`、` vs ` / `),归一化后再匹配。 */
function normalizeTitle(title) {
  return title
    .replace(/[\s、,，/｜|]/g, "")
    .replace(/接口(文档)?$/, "")
    .toLowerCase();
}

/**
 * 按官方目录里的接口名当锚点切块。
 * 不能简单按 `^# ` 切:快照里各接口的标题层级并不统一(多数是 `# 名称`,
 * 语义检索等几个只有 `## 名称`),按固定层级切会把它们并进上一块。
 */
/** 快照缺失/未补录时直接说清楚,别让使用者把环境问题读成"元典改了接口"。 */
function assertSnapshotUsable() {
  if (!fs.existsSync(SNAPSHOT)) {
    console.error(
      `找不到本地快照 ${SNAPSHOT}。\n` +
        "它是 gitignored 的本机文件,新 clone 上没有。先下载 https://open.chineselaw.com/llms-full.txt " +
        "存成该路径,再按 JSON 目录补录官方静态文档缺的接口,然后重跑。",
    );
    process.exit(2);
  }
  if (!fs.readFileSync(SNAPSHOT, "utf8").includes("CaseBoard 补录说明")) {
    console.error(
      "本地快照是官方 llms-full.txt 原版,没做过补录。\n" +
        "官方静态文档滞后于 JSON 目录(2026-07-26 时少 rh_ssgsgg_search 等),直接比会得到一堆假漂移。\n" +
        "先按 https://open.chineselaw.com/api/apis 把缺的接口补进快照并保留「CaseBoard 补录说明」标记,再重跑。",
    );
    process.exit(2);
  }
}

function loadSnapshot(names) {
  const lines = fs.readFileSync(SNAPSHOT, "utf8").split("\n");
  const wanted = new Map(names.map((name) => [normalizeTitle(name), name]));

  const anchors = [];
  lines.forEach((line, index) => {
    const match = line.match(/^#{1,3}\s+(.+?)\s*$/);
    if (!match) return;
    const key = normalizeTitle(match[1]);
    if (wanted.has(key)) anchors.push({ key, index });
  });

  const byTitle = new Map();
  anchors.forEach((anchor, position) => {
    const end = anchors[position + 1]?.index ?? lines.length;
    const section = requestSection(lines.slice(anchor.index, end).join("\n"));
    // 同一个接口名会出现两次(目录索引 + 正文),只有正文那块带「请求参数」章节。
    if (section) byTitle.set(anchor.key, rowsOf(section));
  });
  return byTitle;
}

function loadPricing() {
  const text = fs.readFileSync(PRICING, "utf8");
  const prices = new Map();
  for (const match of text.matchAll(/^\|\s*`([a-zA-Z_]+)`\s*\|[^|]*\|\s*(\d+)\s*\|/gm)) {
    prices.set(match[1], Number(match[2]));
  }
  return prices;
}

async function fetchJson(url) {
  const response = await fetch(url, { headers: { Accept: "application/json" } });
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  const payload = await response.json();
  if (payload.code !== 200 || !payload.data) {
    throw new Error(`${url}: invalid response code ${payload.code}`);
  }
  return payload.data;
}

async function main() {
  assertSnapshotUsable();
  const pricing = loadPricing();
  const live = (await fetchJson(CATALOG_URL)).list ?? [];
  const snapshot = loadSnapshot(live.map((item) => item.name));

  const paramDrift = [];
  const priceDrift = [];
  const unmatched = [];
  let clean = 0;

  for (const item of live.sort((a, b) => a.id - b.id)) {
    const detail = await fetchJson(`https://open.chineselaw.com/api/apis/${item.id}`);

    const documented = pricing.get(item.routeKey);
    if (documented === undefined) {
      priceDrift.push(`${item.routeKey}: 积分计费明细未收录(线上 ${item.price} 分)`);
    } else if (documented !== Number(item.price)) {
      priceDrift.push(
        `${item.routeKey}: 线上 ${item.price} 分 ≠ 计费明细 ${documented} 分`,
      );
    }

    const local = snapshot.get(normalizeTitle(item.name));
    if (!local) {
      unmatched.push(`${item.routeKey} (id=${item.id}, ${item.name})`);
      continue;
    }

    const liveRows = rowsOf(requestSection(detail.requestParams || "") || detail.requestParams || "");
    const diffs = [];
    for (const [name, row] of liveRows) {
      if (!local.has(name)) diffs.push(`  + 新增参数 ${name}: ${row}`);
      else if (local.get(name) !== row) {
        diffs.push(`  ~ ${name}\n      快照: ${local.get(name)}\n      线上: ${row}`);
      }
    }
    for (const name of local.keys()) {
      if (!liveRows.has(name)) diffs.push(`  - 线上已移除参数 ${name}`);
    }

    if (diffs.length) paramDrift.push(`${item.routeKey} (${item.name})\n${diffs.join("\n")}`);
    else clean += 1;
  }

  console.log(`元典线上接口 ${live.length} 个,与本地快照逐行一致 ${clean} 个。`);
  if (paramDrift.length) {
    console.error("\n参数漂移(含类型/必填/默认值/枚举):");
    for (const entry of paramDrift) console.error(entry);
  }
  if (priceDrift.length) {
    console.error("\n积分漂移:");
    for (const entry of priceDrift) console.error(`- ${entry}`);
  }
  if (unmatched.length) {
    console.error("\n本地快照无对应章节(需人工确认是新接口还是标题改写):");
    for (const entry of unmatched) console.error(`- ${entry}`);
  }

  if (paramDrift.length || priceDrift.length || unmatched.length) {
    console.error(
      "\n处理办法:确认影响 → 改代码 → 用 https://open.chineselaw.com/llms-full.txt 刷新 docs/元典API-llms.txt → 同步积分计费明细。",
    );
    process.exitCode = 1;
  }
}

await main();

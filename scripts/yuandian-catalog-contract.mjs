#!/usr/bin/env node

import { pathToFileURL } from "node:url";

const CATALOG_URL =
  "https://open.chineselaw.com/api/apis?pageNum=1&pageSize=200&sortBy=latest";
const DETAIL_URL = (id) => `https://open.chineselaw.com/api/apis/${id}`;

// 元典开放平台**全量**目录基线(2026-07-26 核验,37 个接口)。
// 覆盖范围要求:线上有几个就登记几个 —— 只登记 CaseBoard 当前调用的那些会留下盲区,
// 元典新上接口时 checkLiveCatalog 会静默通过(2026-07-26 就是这样漏掉 rh_ssgsgg_search 的)。
// params 记录官方文档「请求参数」表里出现的字段(含嵌套 filter 子表字段),用于双向比对。
export const EXPECTED_YUANDIAN_CATALOG = [
  {
    id: 7,
    routeKey: "rh_ptal_search",
    method: "POST",
    price: 10,
    params: ["ah", "title", "ssqy", "ay", "jbdw", "xzqh_p", "wszl", "ajlb", "ja_start", "ja_end", "qw", "fxgc", "search_mode", "yyft", "ft_search_mode", "top_k"],
  },
  {
    id: 8,
    routeKey: "rh_qwal_search",
    method: "POST",
    price: 10,
    params: ["ah", "title", "ay", "jbdw", "source", "xzqh_p", "wszl", "ajlb", "ja_start", "ja_end", "qw", "search_mode", "top_k"],
  },
  {
    id: 9,
    routeKey: "rh_case_details",
    method: "GET",
    price: 5,
    params: ["id", "ah", "type"],
  },
  {
    id: 10,
    routeKey: "rh_fg_search",
    method: "POST",
    price: 10,
    params: ["keyword", "search_mode", "fgmc", "sxx", "dy", "xljb_1", "fbbm", "fbrq_start", "fbrq_end", "ssrq_start", "ssrq_end", "top_k"],
  },
  {
    id: 11,
    routeKey: "rh_ft_search",
    method: "POST",
    price: 10,
    params: ["keyword", "search_mode", "fgmc", "xljb_1", "sxx", "dy", "fbbm", "fbrq_start", "fbrq_end", "ssrq_start", "ssrq_end", "top_k"],
  },
  {
    id: 12,
    routeKey: "rh_fg_detail",
    method: "POST",
    price: 5,
    params: ["id", "fgmc", "refer_date"],
  },
  {
    id: 13,
    routeKey: "rh_ft_detail",
    method: "POST",
    price: 1,
    params: ["id", "fgmc", "ftnum", "refer_date"],
  },
  {
    id: 14,
    routeKey: "rh_company_info",
    method: "GET",
    price: 10,
    params: ["name", "num"],
  },
  {
    id: 15,
    routeKey: "rh_company_detail",
    method: "GET",
    price: 10,
    params: ["id", "tyshxydm"],
  },
  {
    id: 16,
    routeKey: "case_vector_search",
    method: "POST",
    price: 10,
    params: ["query", "rewrite_flag", "wenshu_filter", "return_num", "wenshu_type", "ay", "wszl", "ja_start", "ja_end", "dianxing", "fayuan", "source", "cj", "xzqh_p", "xzqh_c"],
  },
  {
    id: 17,
    routeKey: "law_vector_search",
    method: "POST",
    price: 10,
    params: ["query", "rewrite_flag", "fatiao_filter", "return_num", "sxx", "effect1", "law_start", "law_end"],
  },
  {
    id: 19,
    routeKey: "rh_enterpriseSearch",
    method: "GET",
    price: 1,
    params: ["name", "top_k"],
  },
  {
    id: 20,
    routeKey: "rh_enterpriseSeriousIllegal",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 21,
    routeKey: "rh_enterpriseCorporateTax",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 22,
    routeKey: "rh_enterpriseAbnormalOperation",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 23,
    routeKey: "rh_enterpriseGuaranty",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 24,
    routeKey: "rh_enterprisePledge",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 25,
    routeKey: "rh_enterprisePunishment",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 26,
    routeKey: "rh_enterpriseFrozenEquity",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 27,
    routeKey: "rh_enterpriseExecutedPerson",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 28,
    routeKey: "rh_enterpriseExecutions",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 29,
    routeKey: "rh_enterpriseCourtNotice",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 30,
    routeKey: "rh_enterpriseCourtSessionNotice",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 31,
    routeKey: "rh_enterpriseWritList",
    method: "GET",
    price: 10,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 32,
    routeKey: "rh_enterpriseWritAgg",
    method: "GET",
    price: 10,
    params: ["id", "tyshxydm"],
  },
  {
    id: 33,
    routeKey: "rh_enterpriseChangeInfo",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 34,
    routeKey: "rh_enterpriseIcp",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 35,
    routeKey: "rh_enterpriseWorksRight",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 36,
    routeKey: "rh_enterpriseSoftRight",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 37,
    routeKey: "rh_enterprisePatent",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 38,
    routeKey: "rh_enterpriseBrand",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 39,
    routeKey: "rh_enterpriseOutInvest",
    method: "GET",
    price: 5,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 40,
    routeKey: "rh_enterpriseBaseInfo",
    method: "GET",
    price: 10,
    params: ["id", "tyshxydm"],
  },
  {
    id: 41,
    routeKey: "rh_enterpriseAggregationSummary",
    method: "GET",
    price: 10,
    params: ["id", "tyshxydm"],
  },
  {
    id: 42,
    routeKey: "hall_detect",
    method: "POST",
    price: 50,
    params: ["text"],
  },
  {
    id: 50,
    routeKey: "rh_enterpriseAnnualReport",
    method: "GET",
    price: 10,
    params: ["id", "tyshxydm", "year"],
  },
  {
    id: 54,
    routeKey: "rh_ssgsgg_search",
    method: "POST",
    price: 10,
    params: ["search_mode", "title", "name", "jc", "content", "fbrq_start", "fbrq_end", "market", "area", "zsx_type", "top_k"],
  },
];

// 官方文档的参数表格式不统一:多数接口字段名带反引号(`id`),rh_ssgsgg_search 等则是裸名。
// 两种都要认,否则裸名接口会被判成"一个参数都没有"。
export function extractDocumentedParams(markdown = "") {
  const params = new Set();
  const lines = markdown.split("\n").map((line) => line.replace(/\r$/, "").trim());
  // 表头行(紧邻 |---|---| 分隔行的上一行)不是参数 —— 如编码对照表的 `| key | 含义 |`。
  const headerRows = new Set();
  lines.forEach((line, index) => {
    if (/^\|[\s:|-]+\|$/.test(line) && index > 0) headerRows.add(index - 1);
  });
  lines.forEach((line, index) => {
    if (headerRows.has(index)) return;
    const match = line.match(/^\|\s*`?([A-Za-z_][A-Za-z0-9_.]*)`?\s*\|/);
    if (match) params.add(match[1]);
  });
  return params;
}

export function validateCatalogPayloads(list, details) {
  const drift = [];
  const byRoute = new Map(list.map((item) => [item.routeKey, item]));
  const expectedRoutes = new Set(
    EXPECTED_YUANDIAN_CATALOG.map((entry) => entry.routeKey),
  );

  // 反向检查:线上新上的接口必须显式纳入基线,否则整表静默通过等于没设防。
  for (const item of list) {
    if (!expectedRoutes.has(item.routeKey)) {
      drift.push(
        `${item.routeKey}: 线上新增接口未纳入契约(id=${item.id}, ${item.httpMethod}, ${item.price} 分)`,
      );
    }
  }

  for (const expected of EXPECTED_YUANDIAN_CATALOG) {
    const item = byRoute.get(expected.routeKey);
    if (!item) {
      drift.push(`${expected.routeKey}: missing from catalog`);
      continue;
    }
    if (item.id !== expected.id) {
      drift.push(`${expected.routeKey}: id ${item.id} != ${expected.id}`);
    }
    if (String(item.httpMethod).toUpperCase() !== expected.method) {
      drift.push(
        `${expected.routeKey}: method ${item.httpMethod} != ${expected.method}`,
      );
    }
    if (Number(item.price) !== expected.price) {
      drift.push(`${expected.routeKey}: price ${item.price} != ${expected.price}`);
    }

    const detail = details.get(expected.id);
    if (!detail) {
      drift.push(`${expected.routeKey}: missing detail document`);
      continue;
    }
    const documented = extractDocumentedParams(detail.requestParams);
    const missing = expected.params.filter((param) => !documented.has(param));
    if (missing.length > 0) {
      drift.push(`${expected.routeKey}: missing params ${missing.join(", ")}`);
    }
    // 反向检查:官方文档新加的参数也算漂移 —— 可能是新过滤维度(值得接),
    // 也可能是原字段被拆分(不接就少查数据)。两种都要人看一眼。
    const extra = [...documented].filter(
      (param) => !expected.params.includes(param),
    );
    if (extra.length > 0) {
      drift.push(`${expected.routeKey}: undeclared params ${extra.join(", ")}`);
    }
  }
  return drift;
}

async function fetchJson(url, fetchImpl) {
  const response = await fetchImpl(url, {
    headers: { Accept: "application/json" },
  });
  if (!response.ok) {
    throw new Error(`${url}: HTTP ${response.status}`);
  }
  const payload = await response.json();
  if (payload.code !== 200 || !payload.data) {
    throw new Error(`${url}: invalid response code ${payload.code}`);
  }
  return payload.data;
}

export async function checkLiveCatalog(fetchImpl = fetch) {
  const catalog = await fetchJson(CATALOG_URL, fetchImpl);
  const list = catalog.list ?? [];
  const detailEntries = await Promise.all(
    EXPECTED_YUANDIAN_CATALOG.map(async ({ id }) => [
      id,
      await fetchJson(DETAIL_URL(id), fetchImpl),
    ]),
  );
  return validateCatalogPayloads(list, new Map(detailEntries));
}

async function main() {
  const drift = await checkLiveCatalog();
  if (drift.length > 0) {
    console.error("元典开放平台目录与 CaseBoard 合同发生漂移：");
    for (const item of drift) console.error(`- ${item}`);
    process.exitCode = 1;
    return;
  }
  console.log(
    `元典目录合同通过：${EXPECTED_YUANDIAN_CATALOG.length} 个接口的方法、积分和参数均匹配。`,
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}

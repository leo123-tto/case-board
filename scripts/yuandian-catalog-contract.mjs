#!/usr/bin/env node

import { pathToFileURL } from "node:url";

const CATALOG_URL =
  "https://open.chineselaw.com/api/apis?pageNum=1&pageSize=200&sortBy=latest";
const DETAIL_URL = (id) => `https://open.chineselaw.com/api/apis/${id}`;

export const EXPECTED_YUANDIAN_CATALOG = [
  {
    id: 11,
    routeKey: "rh_ft_search",
    method: "POST",
    price: 10,
    params: [
      "keyword",
      "search_mode",
      "fgmc",
      "xljb_1",
      "sxx",
      "dy",
      "fbbm",
      "fbrq_start",
      "fbrq_end",
      "ssrq_start",
      "ssrq_end",
      "top_k",
    ],
  },
  {
    id: 13,
    routeKey: "rh_ft_detail",
    method: "POST",
    price: 1,
    params: ["id", "fgmc", "ftnum", "refer_date"],
  },
  {
    id: 10,
    routeKey: "rh_fg_search",
    method: "POST",
    price: 10,
    params: [
      "keyword",
      "search_mode",
      "fgmc",
      "xljb_1",
      "sxx",
      "dy",
      "fbbm",
      "fbrq_start",
      "fbrq_end",
      "ssrq_start",
      "ssrq_end",
      "top_k",
    ],
  },
  {
    id: 12,
    routeKey: "rh_fg_detail",
    method: "POST",
    price: 5,
    params: ["id", "fgmc", "refer_date"],
  },
  {
    id: 17,
    routeKey: "law_vector_search",
    method: "POST",
    price: 10,
    params: ["query", "rewrite_flag", "fatiao_filter", "return_num"],
  },
  {
    id: 7,
    routeKey: "rh_ptal_search",
    method: "POST",
    price: 10,
    params: [
      "ah",
      "title",
      "ay",
      "jbdw",
      "ssqy",
      "fxgc",
      "yyft",
      "ft_search_mode",
      "xzqh_p",
      "wszl",
      "ajlb",
      "ja_start",
      "ja_end",
      "qw",
      "search_mode",
      "top_k",
    ],
  },
  {
    id: 8,
    routeKey: "rh_qwal_search",
    method: "POST",
    price: 10,
    params: [
      "ah",
      "title",
      "ay",
      "jbdw",
      "source",
      "xzqh_p",
      "wszl",
      "ajlb",
      "ja_start",
      "ja_end",
      "qw",
      "search_mode",
      "top_k",
    ],
  },
  {
    id: 9,
    routeKey: "rh_case_details",
    method: "GET",
    price: 5,
    params: ["id", "ah", "type"],
  },
  {
    id: 16,
    routeKey: "case_vector_search",
    method: "POST",
    price: 10,
    params: ["query", "rewrite_flag", "wenshu_filter", "return_num"],
  },
  {
    id: 19,
    routeKey: "rh_enterpriseSearch",
    method: "GET",
    price: 1,
    params: ["name", "top_k"],
  },
  {
    id: 41,
    routeKey: "rh_enterpriseAggregationSummary",
    method: "GET",
    price: 10,
    params: ["id", "tyshxydm"],
  },
  {
    id: 40,
    routeKey: "rh_enterpriseBaseInfo",
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
    id: 31,
    routeKey: "rh_enterpriseWritList",
    method: "GET",
    price: 10,
    params: ["id", "tyshxydm", "pageNo"],
  },
  {
    id: 50,
    routeKey: "rh_enterpriseAnnualReport",
    method: "GET",
    price: 10,
    params: ["id", "tyshxydm", "year"],
  },
  {
    id: 42,
    routeKey: "hall_detect",
    method: "POST",
    price: 50,
    params: ["text"],
  },
];

export function extractDocumentedParams(markdown = "") {
  const params = new Set();
  const pattern = /^\|\s*`([^`]+)`\s*\|/gm;
  for (const match of markdown.matchAll(pattern)) {
    params.add(match[1]);
  }
  return params;
}

export function validateCatalogPayloads(list, details) {
  const drift = [];
  const byRoute = new Map(list.map((item) => [item.routeKey, item]));

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

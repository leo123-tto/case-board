/**
 * 2026-05-26 V0.1.13 · 案件画像用户手改 overlay 工具。
 *
 * 设计:让律师在案件详情页编辑模式手改的字段、删的词条、隐藏的卡片、拖的排序
 * 永远不被下一次 LLM 全局抽覆盖。LLM 输出留在 cases.agg_* 原位,用户值通过
 * `cases.user_overrides_json` 这一列叠加在渲染层。
 *
 * 字段路径(field path):dotted path 标记 agg_* 字段以及子表行内字段。
 *   - 平字段:`agg_cause` / `agg_filed_at` / `case_summary`
 *   - 子表行内字段(**row-key based,不用 index**):
 *       `agg_party_contacts.{李四|被告}.phone`
 *       `agg_court_contacts.{张法官|审判长}.phone`
 *       `agg_key_dates.{开庭|2024-09-15}.note`
 *       `agg_fees.{律师代理费|5000}.note`
 *     原因:LLM 重抽后子表顺序可能变,index 会让用户改的电话错位到别人头上。
 *     row key 用 `rowKeyOf(field, row)` 生成,跟 `deleted_rows` 共用一套规则。
 *   - 数字字段:`agg_claim_amount`(写入字符串值,由 applyFieldOverrides 解析)
 *
 * **不存案件级整体 JSON 副本**,只存"用户改了什么",其他全用 LLM 抽的值。
 */

export interface UserOverrides {
  /**
   * 字段级覆盖。Key 是 dotted path,Value 是用户填的字符串(数值字段也用字符串,
   * 由调用方在 `applyOverrides` 后做类型转换;为 null 表示"用户清空了这个字段")。
   *
   * @example
   * { "agg_cause": "机动车交通事故责任纠纷",
   *   "agg_party_contacts[0].phone": "13800138000" }
   */
  fields?: Record<string, string | null>;

  /**
   * 用户隐藏的卡片(section)的标题集合。渲染 CardSection 时按这个判断是否显示。
   * 不删 DB,只删显示。退出编辑模式后下次进来还在,可以从编辑模式重新展开。
   *
   * @example ["收费记录", "财产保全"]
   */
  hidden_sections?: string[];

  /**
   * 用户在表格里删的行,key 是字段名(agg_party_contacts),value 是被删行的 row key 数组。
   * 渲染表格时把 row key 在 deleted set 里的过滤掉。
   *
   * row key 用 `rowKeyOf(field, row)` 生成。规则同 fields 子表 path 的 row key 段。
   *
   * @example { "agg_party_contacts": ["李四|被告"] }
   */
  deleted_rows?: Record<string, string[]>;

  /**
   * 用户手动拖动的卡片顺序(标题数组)。
   * 渲染时按这个顺序排卡片,不在数组里的卡片按默认顺序追加在末尾。
   *
   * @example ["当事人联系人", "办案时间轴", "案件基本信息"]
   */
  section_order?: string[];

  /** 首页日历中无法映射到 agg_key_dates 的派生事件人工覆盖。 */
  calendar_events?: Record<string, CalendarEventOverride>;
}

export interface CalendarEventOverride {
  date?: string;
  title?: string;
  note?: string | null;
  hidden?: boolean;
}

/** 空的 overlay 对象 — 老案件 user_overrides_json = null 时用这个 */
const EMPTY_OVERRIDES: UserOverrides = {};

/**
 * 支持用户编辑 + 删除 + dotted-path 覆盖的子表字段名。
 * 新加子表时(比如未来的 agg_preservations),要在这里注册并更新 rowKeyOf 规则。
 */
export type SubtableField =
  | "agg_party_contacts"
  | "agg_court_contacts"
  | "agg_key_dates"
  | "agg_fees";

/**
 * 为子表行生成 stable row key。一处规则,渲染层 / deleted_rows / 子表 path 共用。
 *
 * **不要在子表里挑可变字段做 key**(比如 phone — 用户改电话时 key 会变,
 * 整个 override 就找不到了)。规则统一选"不易变的身份字段组合":
 *   - party_contacts:`name|role`(姓名 + 角色)
 *   - court_contacts:`name|role`
 *   - key_dates:`event|date`(事件 + 日期)
 *   - fees:`item|amount`(收费项 + 金额)
 *
 * row 字段值缺失时用空串占位(`"|被告"` / `"开庭|"`),仍是 stable 的 key。
 */
export function rowKeyOf(
  field: SubtableField,
  row: Readonly<Record<string, unknown>> | object,
): string {
  const r = row as Record<string, unknown>;
  const s = (key: string): string => {
    const v = r[key];
    if (v == null) return "";
    return String(v);
  };
  switch (field) {
    case "agg_party_contacts":
    case "agg_court_contacts":
      return `${s("name")}|${s("role")}`;
    case "agg_key_dates":
      // snapshot 里 KeyDate 用 event_type,LLM 原 schema 用 event。两个都试。
      return `${s("event_type") || s("event")}|${s("date")}`;
    case "agg_fees":
      return `${s("item")}|${s("amount")}`;
  }
}

/**
 * 构造子表 dotted path:`<field>.{<rowKey>}.<inner>`
 *
 * `{}` 包住 row key 是为了让 dotted parser 一眼看清楚边界(row key 自己可能含 `.`,
 * 比如日期"2024-09-15"虽然没 dot 但 LLM 偶尔返回带点的格式)。
 */
export function subtableFieldPath(
  field: SubtableField,
  rowKey: string,
  inner: string,
): string {
  return `${field}.{${rowKey}}.${inner}`;
}

/**
 * 从 cases.user_overrides_json 字符串 parse 出 UserOverrides。
 * 解析失败/null/空字符串都兜底返回 EMPTY_OVERRIDES,不抛错(避免一个坏 JSON 让整个详情页崩)。
 */
export function parseOverrides(json: string | null | undefined): UserOverrides {
  if (!json) return EMPTY_OVERRIDES;
  try {
    const parsed = JSON.parse(json);
    if (parsed && typeof parsed === "object") {
      return parsed as UserOverrides;
    }
    return EMPTY_OVERRIDES;
  } catch {
    return EMPTY_OVERRIDES;
  }
}

/** 序列化回 JSON 字符串,准备写回 DB。null/空对象都返回 null 让 DB 清空。 */
export function serializeOverrides(o: UserOverrides | null | undefined): string | null {
  if (!o) return null;
  const hasContent =
    (o.fields && Object.keys(o.fields).length > 0) ||
    (o.hidden_sections && o.hidden_sections.length > 0) ||
    (o.deleted_rows && Object.keys(o.deleted_rows).length > 0) ||
    (o.section_order && o.section_order.length > 0) ||
    (o.calendar_events && Object.keys(o.calendar_events).length > 0);
  if (!hasContent) return null;
  return JSON.stringify(o);
}

/**
 * 取一个字段路径的覆盖值。返回 undefined = 没改过(用 LLM 值),
 * 返回 null = 用户主动清空,返回字符串 = 用户填的值。
 *
 * @example getFieldOverride(o, "agg_cause") // "机动车..." 或 undefined
 */
export function getFieldOverride(
  o: UserOverrides,
  path: string,
): string | null | undefined {
  return o.fields?.[path];
}

/**
 * 在不可变方式下设置一个字段路径的覆盖值。返回新的 overlay(不修改入参)。
 * value = null 表示"用户清空",value = string 是用户填的新值。
 * 若 fields 为 undefined 则自动初始化为空对象。
 */
export function setFieldOverride(
  o: UserOverrides,
  path: string,
  value: string | null,
): UserOverrides {
  return {
    ...o,
    fields: { ...(o.fields ?? {}), [path]: value },
  };
}

/** 移除一个字段路径的覆盖(回到 LLM 值)。 */
export function clearFieldOverride(o: UserOverrides, path: string): UserOverrides {
  if (!o.fields || !(path in o.fields)) return o;
  const next = { ...o.fields };
  delete next[path];
  return { ...o, fields: next };
}

/** 切换卡片隐藏状态。 */
export function toggleHiddenSection(o: UserOverrides, title: string): UserOverrides {
  const cur = o.hidden_sections ?? [];
  const next = cur.includes(title)
    ? cur.filter((t) => t !== title)
    : [...cur, title];
  return { ...o, hidden_sections: next };
}

/** 标记某行被删(只删显示,不删 DB)。row key 必须用 `rowKeyOf` 生成。 */
export function markRowDeleted(
  o: UserOverrides,
  field: SubtableField,
  rowKey: string,
): UserOverrides {
  const cur = o.deleted_rows ?? {};
  const fieldKeys = cur[field] ?? [];
  if (fieldKeys.includes(rowKey)) return o;
  return {
    ...o,
    deleted_rows: { ...cur, [field]: [...fieldKeys, rowKey] },
  };
}

/** 撤销某行删除标记(用户改主意了)。 */
export function unmarkRowDeleted(
  o: UserOverrides,
  field: SubtableField,
  rowKey: string,
): UserOverrides {
  const cur = o.deleted_rows;
  if (!cur || !cur[field]) return o;
  const filtered = cur[field].filter((k) => k !== rowKey);
  if (filtered.length === cur[field].length) return o;
  const nextField = { ...cur };
  if (filtered.length === 0) {
    delete nextField[field];
  } else {
    nextField[field] = filtered;
  }
  return { ...o, deleted_rows: nextField };
}

/** 查询一行是否被用户标记为删除。row key 用 `rowKeyOf` 生成。 */
export function isRowDeleted(
  o: UserOverrides,
  field: SubtableField,
  rowKey: string,
): boolean {
  return o.deleted_rows?.[field]?.includes(rowKey) ?? false;
}

/** 设置卡片排序(用户拖完后整个数组一起写)。 */
export function setSectionOrder(o: UserOverrides, order: string[]): UserOverrides {
  return { ...o, section_order: order };
}

/**
 * 按用户排序 + 默认顺序合并出最终卡片顺序。
 * 用户排过的卡片按用户顺序在前,没排过的按默认顺序追加在后。
 * 没在 default 里的卡片会被忽略(防止 schema 变了之后渲染坏卡片)。
 *
 * @param defaultOrder 案件详情页默认渲染顺序(见 CaseSnapshotView 的 CARDS_DEFAULT_ORDER)
 */
export function resolveSectionOrder(
  o: UserOverrides,
  defaultOrder: string[],
): string[] {
  const user = o.section_order ?? [];
  const defaultSet = new Set(defaultOrder);
  const result: string[] = [];
  const seen = new Set<string>();
  for (const title of user) {
    if (defaultSet.has(title) && !seen.has(title)) {
      result.push(title);
      seen.add(title);
    }
  }
  for (const title of defaultOrder) {
    if (!seen.has(title)) {
      result.push(title);
      seen.add(title);
    }
  }
  return result;
}

/* -----------------------------------------------------------------------
 * applyFieldOverrides — 把 fields overrides 叠加到 snapshot
 *
 * P2 范围:只处理顶级 string / number 字段(Hero + 案件基本信息卡片用到的)。
 *   - 顶级 path:agg_cause / agg_case_no / agg_court / agg_filed_at /
 *     agg_status_text / agg_claim_amount 等
 *   - 数组型字段(plaintiffs/defendants/judges):用 "agg_plaintiffs.0" 改单项
 *
 * P3 接 UI 时补:子表 dotted path("agg_party_contacts[0].phone" 等)。
 * 现阶段 path 命中不到映射就静默忽略,不抛错(用户改的字段没被叠加上,P3 补)。
 * ----------------------------------------------------------------------- */

/**
 * Snapshot 字段集合 — UI 字段名 → overrides 用的 path(agg_* 命名)。
 * 用户改 Hero 的"案由"会写到 fields["agg_cause"],apply 时映射回 snap.cause。
 */
const PATH_TO_SNAPSHOT_KEY: Record<
  string,
  | "case_no"
  | "court"
  | "cause"
  | "case_stage"
  | "case_status"
  | "filed_at"
  | "expected_close_at"
  | "case_note"
  | "case_type"
  | "summary"
  | "resolution"
  | "status_text"
  | "our_side"
> = {
  agg_case_no: "case_no",
  agg_court: "court",
  agg_cause: "cause",
  agg_status_text: "status_text",
  agg_filed_at: "filed_at",
  agg_resolution: "resolution",
  agg_our_side: "our_side",
  case_summary: "summary",
  case_stage: "case_stage",
  case_status: "case_status",
  case_type: "case_type",
  expected_close_at: "expected_close_at",
  case_note: "case_note",
};

/** snapshot 里 number 类型的字段,值要 parseFloat。 */
const NUMBER_PATHS: Record<string, "claim_amount"> = {
  agg_claim_amount: "claim_amount",
};

/**
 * 子表里是 number 类型的 inner 字段(用户改时要 parseFloat,不能字符串直写)。
 * 不在这里的 inner 默认按字符串写。
 *
 * 例:agg_fees.amount 是 number,没列就会被字符串覆盖,后续 toLocaleString
 * 会出错(string.toLocaleString 不补千位符)。
 */
const SUBTABLE_NUMERIC_INNER: Partial<Record<SubtableField, Set<string>>> = {
  agg_fees: new Set(["amount"]),
};

/**
 * 清洗 + parseFloat 一个数值字符串。失败返回 undefined(调用方决定是否保留旧值)。
 * 兼容用户输入的 ¥ ￥ $ 元 , ， 空格 全角空格 等噪声字符。
 */
function parseCleanNumber(raw: string): number | undefined {
  const cleaned = raw.replace(/[¥￥$元，,\s　]/g, "");
  const parsed = parseFloat(cleaned);
  return Number.isFinite(parsed) ? parsed : undefined;
}

/**
 * 子表 LLM 字段名 → snapshot 数组字段名 的映射。
 * 子表 path `agg_party_contacts.{name|role}.phone` 解析后,在 snap.party_contacts
 * 数组里按 rowKeyOf 找匹配行,改 inner 字段。
 */
const SUBTABLE_SNAPSHOT_KEY: Record<
  SubtableField,
  "party_contacts" | "court_contacts" | "key_dates" | "fees"
> = {
  agg_party_contacts: "party_contacts",
  agg_court_contacts: "court_contacts",
  agg_key_dates: "key_dates",
  agg_fees: "fees",
};

/**
 * 解析子表 dotted path:`agg_xxx.{row-key}.inner` → 结构化对象。
 * 失败返回 null(交给主循环忽略)。
 *
 * row key 内部可能含 `|`(`name|role` 格式),不能含 `}` 字符 —
 * subtableFieldPath 用 `{...}` 包 key 就是为了画出明确边界。
 */
function parseSubtablePath(
  path: string,
): { field: SubtableField; rowKey: string; inner: string } | null {
  const match = path.match(/^(agg_[a-z_]+)\.\{([^}]*)\}\.([a-z_]+)$/);
  if (!match) return null;
  const field = match[1] as SubtableField;
  if (!(field in SUBTABLE_SNAPSHOT_KEY)) return null;
  return { field, rowKey: match[2], inner: match[3] };
}

/**
 * 把 fields overrides 应用到 snapshot,返回新的 snapshot(不修改入参)。
 *
 * - 顶级 string 字段:命中映射就替换
 * - 数值字段(agg_claim_amount):parseFloat,失败就保留原值
 * - 子表 row-key path:在 raw 数据上预算 key→index map,避免 mutation 顺序 bug
 *   (用户同时改 name 和 phone,如果用 mutated row 找 key,改 name 后 phone 找不到行)
 * - undefined 值(用户没改):不动 snapshot
 * - null 值(用户清空):snapshot 字段设 null
 *
 * 用 `unknown` 中转避开 CaseSnapshot 没有 index signature 的限制(故意如此 —
 * 让 CaseSnapshot 保持精确的字段类型,override 这种"按 key 写"的操作走这一层)。
 */
export function applyFieldOverrides<S extends object>(
  snap: S,
  overrides: UserOverrides,
): S {
  if (!overrides.fields) return snap;
  const next = { ...snap } as Record<string, unknown>;

  // 关键:rowKey → idx 用 **原始 snap** 预算,不用 mutation 中的 next。
  // 否则用户同行改 name 后改 phone,phone 用 mutated row 算 rowKeyOf 会得新 name,
  // 跟 path 里的旧 rowKey 不匹配 → 静默孤儿。
  const rowKeyIndex = new Map<SubtableField, Map<string, number>>();
  for (const field of Object.keys(SUBTABLE_SNAPSHOT_KEY) as SubtableField[]) {
    const snapKey = SUBTABLE_SNAPSHOT_KEY[field];
    const origRows = (snap as unknown as Record<string, unknown>)[snapKey] as
      | Array<Record<string, unknown>>
      | undefined;
    if (!origRows) continue;
    const m = new Map<string, number>();
    origRows.forEach((r, i) => {
      const k = rowKeyOf(field, r);
      if (!m.has(k)) m.set(k, i);
    });
    rowKeyIndex.set(field, m);
  }

  for (const [path, raw] of Object.entries(overrides.fields)) {
    // 顶级 string
    const stringKey = PATH_TO_SNAPSHOT_KEY[path];
    if (stringKey) {
      if (raw === null) {
        next[stringKey] = null;
      } else if (typeof raw === "string") {
        // 空串等同于"清空"
        next[stringKey] = raw.trim() === "" ? null : raw;
      }
      continue;
    }
    // 顶级数值字段
    const numberKey = NUMBER_PATHS[path];
    if (numberKey) {
      if (raw === null || (typeof raw === "string" && raw.trim() === "")) {
        next[numberKey] = null;
      } else if (typeof raw === "string") {
        const parsed = parseCleanNumber(raw);
        if (parsed !== undefined) next[numberKey] = parsed;
      }
      continue;
    }
    // 子表 row-key path:agg_party_contacts.{name|role}.phone 等
    const subtable = parseSubtablePath(path);
    if (subtable) {
      const idx = rowKeyIndex.get(subtable.field)?.get(subtable.rowKey);
      // orphan 保护:LLM 重抽换序后找不到就静默跳过,不删 override
      if (idx === undefined) continue;
      const snapKey = SUBTABLE_SNAPSHOT_KEY[subtable.field];
      const rows = (next[snapKey] as
        | Array<Record<string, unknown>>
        | undefined) ?? [];
      const newRows = rows.slice();
      const newRow: Record<string, unknown> = { ...rows[idx] };
      const isNumericInner =
        SUBTABLE_NUMERIC_INNER[subtable.field]?.has(subtable.inner) ?? false;
      // null / 空串清空字段
      if (raw === null || (typeof raw === "string" && raw.trim() === "")) {
        newRow[subtable.inner] = null;
      } else if (typeof raw === "string") {
        if (isNumericInner) {
          // 数值 inner:parseFloat 清噪声,失败保留旧值(不写)
          const parsed = parseCleanNumber(raw);
          if (parsed !== undefined) {
            newRow[subtable.inner] = parsed;
          } else {
            continue; // 解析不出来不动,免得把 number 字段写成字符串破坏 toLocaleString
          }
        } else {
          newRow[subtable.inner] = raw;
        }
      }
      newRows[idx] = newRow;
      next[snapKey] = newRows;
      continue;
    }
    // 其他 path:暂未实现(忽略,不抛错)
  }
  return next as S;
}

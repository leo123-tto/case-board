import type { CaseRepresentation } from "./types";

export const REPRESENTATION_SIDES = ["原告方", "被告方", "第三人"] as const;

export type RepresentationSide = (typeof REPRESENTATION_SIDES)[number];

export type CaseRepresentationParseResult =
  | { status: "missing" }
  | { status: "valid"; representation: CaseRepresentation }
  | { status: "invalid"; reason: string };

type PartySnapshot = {
  plaintiffs: string[];
  defendants: string[];
  third_parties: string[];
};

export function isRepresentationSide(value: unknown): value is RepresentationSide {
  return REPRESENTATION_SIDES.includes(value as RepresentationSide);
}

/**
 * 读取精确委托人，与 Rust `effective_representation` 对齐：只有 representation 键缺失
 * 才允许回退旧粗立场；任何已存在但损坏的精确数据都必须显式标记为异常。
 */
export function parseCaseRepresentation(
  userOverridesJson: string | null | undefined,
): CaseRepresentationParseResult {
  if (userOverridesJson == null) return { status: "missing" };

  let root: Record<string, unknown>;
  try {
    const parsed: unknown = JSON.parse(userOverridesJson);
    if (!isRecord(parsed)) return invalid("user_overrides_json 不是对象");
    root = parsed;
  } catch {
    return invalid("user_overrides_json 已损坏");
  }

  if (!hasOwn(root, "representation")) return { status: "missing" };
  if (!isRecord(root.representation)) return invalid("representation 格式无效");

  const candidate = root.representation;
  if (candidate.version !== 1) return invalid("representation.version 必须为 1");
  if (!isRepresentationSide(candidate.side)) return invalid("representation.side 无效");
  if (!Array.isArray(candidate.parties) || candidate.parties.length === 0) {
    return invalid("representation.parties 不能为空");
  }

  const role = roleForSide(candidate.side);
  const names = new Set<string>();
  const parties: CaseRepresentation["parties"] = [];
  for (const party of candidate.parties) {
    if (!isRecord(party) || typeof party.name !== "string" || typeof party.role !== "string") {
      return invalid("representation.party 格式无效");
    }
    if (!party.name || party.name !== party.name.trim()) {
      return invalid("representation.party.name 必须为非空规范姓名");
    }
    if (names.has(party.name)) return invalid("representation.party.name 重复");
    names.add(party.name);
    if (party.role !== role) return invalid("representation.party.role 与阵营不符");
    parties.push({ name: party.name, role: party.role });
  }

  const fields = root.fields;
  if (fields !== undefined) {
    if (!isRecord(fields)) return invalid("user_overrides_json.fields 不是对象");
    if (hasOwn(fields, "agg_our_side")) {
      const ourSide = fields.agg_our_side;
      if (ourSide !== null && typeof ourSide !== "string") {
        return invalid("fields.agg_our_side 格式无效");
      }
      if (typeof ourSide === "string" && ourSide.trim() && ourSide.trim() !== candidate.side) {
        return invalid("fields.agg_our_side 与 representation.side 冲突");
      }
    }
  }

  return {
    status: "valid",
    representation: { version: 1, side: candidate.side, parties },
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function hasOwn(value: object, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(value, key);
}

function invalid(reason: string): CaseRepresentationParseResult {
  return { status: "invalid", reason };
}

function roleForSide(side: RepresentationSide): string {
  return side === "原告方" ? "原告" : side === "被告方" ? "被告" : "第三人";
}

/** 返回一个诉讼阵营中可供精确选择的去重当事人名单。 */
export function representationCandidates(
  snapshot: PartySnapshot,
  side: RepresentationSide,
): string[] {
  const source =
    side === "原告方"
      ? snapshot.plaintiffs
      : side === "被告方"
        ? snapshot.defendants
        : snapshot.third_parties;
  const seen = new Set<string>();
  return source.reduce<string[]>((names, value) => {
    const name = value.trim();
    if (name && !seen.has(name)) {
      seen.add(name);
      names.push(name);
    }
    return names;
  }, []);
}

/** 已保存的精确人名若不在当前聚合名单中，保留出来供律师核对。 */
export function unresolvedRepresentationParties(
  representation: CaseRepresentation,
  candidates: string[],
): string[] {
  const knownNames = new Set(candidates);
  const seen = new Set<string>();
  return representation.parties.reduce<string[]>((names, party) => {
    if (!knownNames.has(party.name) && !seen.has(party.name)) {
      seen.add(party.name);
      names.push(party.name);
    }
    return names;
  }, []);
}

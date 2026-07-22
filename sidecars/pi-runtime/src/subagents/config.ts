import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import type { Credential } from "@earendil-works/pi-ai";

export interface CaseBoardSubagentDefinition {
  name: string;
  description: string;
  tools: string[];
  prompt: string;
}

export interface SubagentRuntimeEnvironment {
  rootDir: string;
  agentDir: string;
  agentsDir: string;
  agents: CaseBoardSubagentDefinition[];
  env: Record<string, string>;
  activate(): () => void;
  cleanup(): void;
}

const AGENTS: CaseBoardSubagentDefinition[] = [
  {
    name: "legal-researcher",
    description: "检索公开中文法律资料、微信公众号文章和权威入口",
    tools: ["exa_search", "exa_find_similar", "firecrawl_search"],
    prompt: "围绕父任务开展聚焦检索。优先找权威、近期且能直接回答争点的来源，保留标题、机构、日期和 URL；不得把案件隐私写入检索词。",
  },
  {
    name: "source-reader",
    description: "深读指定网页并提炼可引用的裁判观点、事实边界和法源",
    tools: ["exa_contents", "firecrawl_scrape"],
    prompt: "只深读父任务指定或检索到的公开来源。区分原文观点、你的归纳和待核验结论；微信公众号抓取失败时可评估 Firecrawl enhanced。",
  },
  {
    name: "legal-analyst",
    description: "归纳多份研究结果，识别冲突、适用边界与证据层级",
    tools: ["exa_contents"],
    prompt: "基于父任务和已给出的研究材料做法律分析。不要为了显得忙碌重新搜索；明确规则、例外、适用边界、冲突和仍需权威来源核验的事项。",
  },
  {
    name: "source-verifier",
    description: "核验公开来源的机构、日期、引用完整性和过时风险",
    tools: ["exa_search", "exa_contents", "firecrawl_scrape"],
    prompt: "核验来源身份、发布日期、正文是否支持所述观点及是否存在过时风险。公众号属于二手来源，标出需要现行法、官方案例、元典或本地知识库交叉核验的结论。",
  },
];

const SUBAGENT_INTEGRITY_CONTRACT = `
真实性硬约束：
- 不得编造事实、法律法规、案例、案号、网址或检索过程。
- 用户或父任务要求检索、读取或核验时必须真实调用获准工具；只有工具真实返回的内容才能写成已检索、已读取或已核验。工具失败、未执行或无结果时必须如实说明。
- 默认任务中，失效、废止或尚未生效的法源不得作为当前依据，只能用于继续核验现行替代法源。只有父任务明确要求历史时点或旧版本时，才可使用工具返回且带 historical_research_only 标记的正文，并必须写明版本、适用时点及非现行状态。
- 证据不足时允许明确回答不知道、未检索到或无法核验，不得自行补全。
`;

function agentMarkdown(agent: CaseBoardSubagentDefinition): string {
  return `---\nname: ${agent.name}\ndescription: ${agent.description}\ntools: ${agent.tools.join(", ")}\nthinking: medium\nsystemPromptMode: replace\ninheritProjectContext: false\ninheritSkills: false\ndefaultProgress: true\n---\n\n你是 CaseBoard 主 Pi 派出的受限子代理。你只能完成父任务明确分派的工作，不得继续派生子代理，不得调用终端、Git、软件安装或 Pi 原生文件写入。\n${SUBAGENT_INTEGRITY_CONTRACT}\n${agent.prompt}\n`;
}

export function createSubagentRuntimeEnvironment(
  executablePath = process.execPath,
  availableTools?: ReadonlySet<string>,
): SubagentRuntimeEnvironment {
  const rootDir = mkdtempSync(join(tmpdir(), "caseboard-pi-subagents-"));
  const agentDir = join(rootDir, "agent");
  const agentsDir = join(rootDir, "agents");
  const extensionConfigDir = join(agentDir, "extensions", "subagent");
  mkdirSync(agentsDir, { recursive: true, mode: 0o700 });
  mkdirSync(extensionConfigDir, { recursive: true, mode: 0o700 });
  writeFileSync(
    join(agentDir, "settings.json"),
    `${JSON.stringify({ subagents: { disableBuiltins: true, disableThinking: false } }, null, 2)}\n`,
    { mode: 0o600 },
  );
  writeFileSync(
    join(extensionConfigDir, "config.json"),
    `${JSON.stringify({
      asyncByDefault: false,
      asyncWidget: false,
      maxSubagentDepth: 1,
      maxSubagentSpawnsPerSession: 8,
      globalConcurrencyLimit: 3,
      proactiveSkillSubagents: false,
      scheduledRuns: { enabled: false },
    }, null, 2)}\n`,
    { mode: 0o600 },
  );
  const agents = AGENTS
    .map((agent) => ({
      ...agent,
      tools: availableTools ? agent.tools.filter((tool) => availableTools.has(tool)) : [...agent.tools],
    }))
    .filter((agent) => agent.tools.length > 0);
  for (const agent of agents) {
    writeFileSync(join(agentsDir, `${agent.name}.md`), agentMarkdown(agent), { mode: 0o600 });
  }
  const env = {
    PI_CODING_AGENT_DIR: agentDir,
    PI_SUBAGENT_EXTRA_AGENT_DIRS: agentsDir,
    PI_SUBAGENT_PI_BINARY: executablePath,
    PI_SUBAGENT_MAX_DEPTH: "1",
    PI_SUBAGENT_MAX_SPAWNS_PER_SESSION: "8",
    PI_OFFLINE: "1",
  };
  return {
    rootDir,
    agentDir,
    agentsDir,
    agents,
    env,
    activate() {
      const previous = new Map<string, string | undefined>();
      for (const [name, value] of Object.entries(env)) {
        previous.set(name, process.env[name]);
        process.env[name] = value;
      }
      return () => {
        for (const [name, value] of previous) {
          if (value === undefined) delete process.env[name];
          else process.env[name] = value;
        }
      };
    },
    cleanup() {
      rmSync(rootDir, { recursive: true, force: true });
    },
  };
}

function childEnvName(name: string): string {
  const safe = name.toUpperCase().replace(/[^A-Z0-9_]/g, "_").replace(/^([^A-Z_])/, "_$1");
  return `CASEBOARD_PI_CHILD_PROVIDER_${safe || "VALUE"}`;
}

export function configureSubagentCredential(
  runtime: SubagentRuntimeEnvironment,
  providerId: string,
  credential: Credential | undefined,
): void {
  let stored: Credential | undefined = credential;
  if (credential?.type === "api_key") {
    const env: Record<string, string> = {};
    if (credential.key !== undefined) {
      runtime.env.CASEBOARD_PI_CHILD_API_KEY = credential.key;
    }
    for (const [name, value] of Object.entries(credential.env ?? {})) {
      if (typeof value !== "string") continue;
      const envName = childEnvName(name);
      runtime.env[envName] = value;
      env[name] = `$${envName}`;
    }
    stored = {
      type: "api_key",
      ...(credential.key !== undefined ? { key: "$CASEBOARD_PI_CHILD_API_KEY" } : {}),
      ...(Object.keys(env).length > 0 ? { env } : {}),
    };
  }
  writeFileSync(
    join(runtime.agentDir, "auth.json"),
    `${JSON.stringify(stored ? { [providerId]: stored } : {}, null, 2)}\n`,
    { mode: 0o600 },
  );
}

export function readSubagentConfig(runtime: SubagentRuntimeEnvironment): unknown {
  return JSON.parse(readFileSync(join(runtime.agentDir, "settings.json"), "utf8"));
}

export function mergeExtraAgentDirs(existing: string | undefined, agentsDir: string): string {
  return existing ? `${agentsDir}${delimiter}${existing}` : agentsDir;
}

import {
  createEventBus,
  createSyntheticSourceInfo,
  createExtensionRuntime,
  type Extension,
  type ExtensionRuntime,
  type LoadExtensionsResult,
  type ResourceLoader,
  type Skill,
} from "@earendil-works/pi-coding-agent";
import registerSubagentExtension from "./vendor/pi-subagents.js";
import { loadExtensionFromFactory } from "../node_modules/@earendil-works/pi-coding-agent/dist/core/extensions/loader.js";

import type { HostSkillDefinition } from "./protocol";

function escapeXml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&apos;");
}

export function createIsolatedResourceLoader(
  systemPrompt: string,
  definitions: HostSkillDefinition[] = [],
  loadedExtensions?: { extensions: Extension[]; errors: LoadExtensionsResult["errors"]; runtime: ExtensionRuntime },
): ResourceLoader {
  const skills: Skill[] = definitions.map((definition) => ({
    name: definition.name,
    description: definition.description,
    filePath: definition.file_path,
    baseDir: definition.base_dir,
    sourceInfo: createSyntheticSourceInfo(definition.file_path, { source: definition.source }),
    disableModelInvocation: false,
  }));
  const skillsPrompt = definitions.length > 0
    ? `\n\n<available_skills>\n${definitions
        .map((definition) => `  <skill name="${escapeXml(definition.name)}">${escapeXml(definition.description)}</skill>`)
        .join("\n")}\n</available_skills>`
    : "";
  const extensionResources = loadedExtensions ?? {
    extensions: [],
    errors: [],
    runtime: createExtensionRuntime(),
  };
  return {
    getExtensions: () => extensionResources,
    getSkills: () => ({ skills, diagnostics: [] }),
    getPrompts: () => ({ prompts: [], diagnostics: [] }),
    getThemes: () => ({ themes: [], diagnostics: [] }),
    getAgentsFiles: () => ({ agentsFiles: [] }),
    // Pi only auto-appends its formatted Skill manifest when the unrestricted builtin `read`
    // tool is enabled. CaseBoard intentionally disables it, so provide a path-free manifest and
    // route full content through the bounded `read_legal_skill` host tool instead.
    getSystemPrompt: () => `${systemPrompt}${skillsPrompt}`,
    getAppendSystemPrompt: () => skills.length > 0 ? [
      "CaseBoard 提供的 Skills 均为只读。需要按描述调用某个 Skill 时，请使用 read_legal_skill(name) 读取完整指令；不要尝试创建、修改或安装 Skill。",
    ] : [],
    extendResources: () => {},
    reload: async () => {},
  };
}

export async function loadCaseBoardSubagentResources(
  cwd: string,
): Promise<{ extensions: Extension[]; errors: LoadExtensionsResult["errors"]; runtime: ExtensionRuntime }> {
  const runtime = createExtensionRuntime();
  const eventBus = createEventBus();
  const extension = await loadExtensionFromFactory(
    registerSubagentExtension,
    cwd,
    eventBus,
    runtime,
    "pi-subagents@0.35.1",
  );
  return { extensions: [extension], errors: [], runtime };
}

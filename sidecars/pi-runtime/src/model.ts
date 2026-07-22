import type { Api, Model } from "@earendil-works/pi-ai";
import { streamSimple } from "@earendil-works/pi-ai/compat";
import { ModelRuntime } from "@earendil-works/pi-coding-agent";

import type { HostModelConfig } from "./protocol";
import { HostCredentialStore } from "./credential-store";

type OpenAiModel = Model<"openai-completions">;

export async function createRequestModel(config: HostModelConfig): Promise<{
  modelRuntime: ModelRuntime;
  model: Model<Api>;
  credentialStore: HostCredentialStore;
}> {
  const credentials = new HostCredentialStore(config.provider_id, config.credential);
  const modelRuntime = await ModelRuntime.create({
    credentials,
    modelsPath: null,
    allowModelNetwork: false,
  });

  if (config.provider_id === "caseboard-custom") {
    const custom = config.caseboard_custom;
    if (!custom) throw new Error("自定义 OpenAI 兼容模型缺少配置");
    modelRuntime.registerProvider("caseboard-custom", {
      name: "CaseBoard 自定义模型",
      baseUrl: custom.base_url,
      api: "openai-completions",
      authHeader: custom.auth_header,
      headers: custom.headers,
      streamSimple: (requestModel, context, options) =>
        streamSimple(requestModel, context, { ...options, temperature: custom.temperature }),
      models: [
        {
          id: config.model_id,
          name: config.model_id,
          api: "openai-completions",
          baseUrl: custom.base_url,
          reasoning: custom.reasoning,
          input: ["text"],
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
          contextWindow: custom.context_window,
          maxTokens: custom.max_tokens,
          headers: custom.headers,
          compat: custom.compat as OpenAiModel["compat"],
        },
      ],
    });
  } else if (config.caseboard_custom) {
    throw new Error("只有 caseboard-custom provider 可以携带自定义模型配置");
  }

  const model = modelRuntime.getModel(config.provider_id, config.model_id);
  if (!model) throw new Error("模型不在当前 Pi Runtime 目录");
  return { modelRuntime, model, credentialStore: credentials };
}

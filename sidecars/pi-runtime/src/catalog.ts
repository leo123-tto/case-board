import { getSupportedThinkingLevels, InMemoryCredentialStore } from "@earendil-works/pi-ai";
import { ModelRuntime } from "@earendil-works/pi-coding-agent";

import type { PiProviderCatalog } from "./protocol";

export async function createProviderCatalog(): Promise<PiProviderCatalog> {
  const runtime = await ModelRuntime.create({
    credentials: new InMemoryCredentialStore(),
    modelsPath: null,
    allowModelNetwork: false,
  });

  const providers = runtime
    .getProviders()
    .filter((provider) => {
      const supportsApiKey = provider.auth.apiKey?.login !== undefined;
      const supportsOAuth = provider.auth.oauth !== undefined;
      return (supportsApiKey || supportsOAuth) && runtime.getModels(provider.id).length > 0;
    })
    .map((provider) => ({
      id: provider.id,
      name: provider.name,
      auth_types: [
        ...(provider.auth.apiKey?.login ? (["api_key"] as const) : []),
        ...(provider.auth.oauth ? (["oauth"] as const) : []),
      ],
      models: runtime.getModels(provider.id).map((model) => ({
        id: model.id,
        name: model.name,
        api: model.api,
        reasoning: model.reasoning,
        thinking_levels: getSupportedThinkingLevels(model),
        context_window: model.contextWindow,
        max_tokens: model.maxTokens,
        input: [...model.input],
      })),
    }))
    .sort((left, right) => left.name.localeCompare(right.name));

  return { providers };
}

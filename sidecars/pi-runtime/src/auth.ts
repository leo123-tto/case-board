import {
  InMemoryCredentialStore,
  type AuthEvent,
  type AuthPrompt,
} from "@earendil-works/pi-ai";
import { ModelRuntime } from "@earendil-works/pi-coding-agent";

import {
  PROTOCOL_VERSION,
  type AuthStartRequest,
  type SidecarMessage,
} from "./protocol";

interface PendingPrompt {
  id: string;
  resolve(value: string): void;
  reject(error: Error): void;
  abort?: () => void;
}

export class AuthBridge {
  private readonly controller = new AbortController();
  private pending: PendingPrompt | undefined;
  private promptSequence = 0;
  private readonly secretValues = new Set<string>();
  private wasCancelled = false;

  constructor(
    private readonly requestId: string,
    private readonly emit: (message: SidecarMessage) => void,
  ) {}

  get signal(): AbortSignal {
    return this.controller.signal;
  }

  get cancelled(): boolean {
    return this.wasCancelled;
  }

  async waitForPrompt(prompt: AuthPrompt): Promise<string> {
    if (this.pending) throw new Error("已有认证输入等待处理");
    if (this.signal.aborted || prompt.signal?.aborted) throw new Error("认证已取消");
    const promptId = `prompt-${++this.promptSequence}`;
    this.emit({
      type: "auth_prompt",
      protocol_version: PROTOCOL_VERSION,
      request_id: this.requestId,
      prompt_id: promptId,
      prompt_type: prompt.type,
      message: prompt.message,
      ...(!("placeholder" in prompt) || prompt.placeholder === undefined
        ? {}
        : { placeholder: prompt.placeholder }),
      ...(prompt.type === "select" ? { options: [...prompt.options] } : {}),
    });
    return new Promise<string>((resolve, reject) => {
      const abort = () => {
        if (this.pending?.id === promptId) this.pending = undefined;
        reject(new Error("认证已取消"));
      };
      this.pending = { id: promptId, resolve, reject, abort };
      this.signal.addEventListener("abort", abort, { once: true });
      prompt.signal?.addEventListener("abort", abort, { once: true });
    });
  }

  resolvePrompt(promptId: string, value: string): boolean {
    const pending = this.pending;
    if (!pending || pending.id !== promptId) return false;
    this.pending = undefined;
    if (value) this.secretValues.add(value);
    pending.resolve(value);
    return true;
  }

  notify(event: AuthEvent): void {
    if (event.type === "info") {
      this.emit({
        type: "auth_info",
        protocol_version: PROTOCOL_VERSION,
        request_id: this.requestId,
        message: event.message,
        ...(event.links ? { links: [...event.links] } : {}),
      });
    } else if (event.type === "auth_url") {
      this.emit({
        type: "auth_url",
        protocol_version: PROTOCOL_VERSION,
        request_id: this.requestId,
        url: event.url,
        ...(event.instructions === undefined ? {} : { instructions: event.instructions }),
      });
    } else if (event.type === "device_code") {
      this.emit({
        type: "auth_device_code",
        protocol_version: PROTOCOL_VERSION,
        request_id: this.requestId,
        user_code: event.userCode,
        verification_uri: event.verificationUri,
        ...(event.intervalSeconds === undefined ? {} : { interval_seconds: event.intervalSeconds }),
        ...(event.expiresInSeconds === undefined ? {} : { expires_in_seconds: event.expiresInSeconds }),
      });
    } else {
      this.emit({
        type: "auth_progress",
        protocol_version: PROTOCOL_VERSION,
        request_id: this.requestId,
        message: event.message,
      });
    }
  }

  succeed(providerId: string, credential: Extract<SidecarMessage, { type: "auth_success" }>["credential"]): void {
    this.emit({
      type: "auth_success",
      protocol_version: PROTOCOL_VERSION,
      request_id: this.requestId,
      provider_id: providerId,
      credential,
    });
  }

  cancel(): void {
    this.wasCancelled = true;
    this.controller.abort();
    this.pending?.reject(new Error("认证已取消"));
    this.pending = undefined;
  }

  secrets(): string[] {
    return [...this.secretValues];
  }
}

export function safeAuthError(error: unknown, secrets: string[] = []): string {
  let message = error instanceof Error
    ? error.message
    : typeof error === "string"
      ? error
      : typeof error === "object"
        && error !== null
        && "message" in error
        && typeof error.message === "string"
        ? error.message
        : "Pi Runtime 认证失败";
  for (const secret of secrets) message = message.split(secret).join("[REDACTED]");
  return message.replace(/[\r\n\t]+/g, " ").slice(0, 1_000) || "Pi Runtime 认证失败";
}

export async function runProviderAuth(
  request: AuthStartRequest,
  bridge: AuthBridge,
): Promise<void> {
  const runtime = await ModelRuntime.create({
    credentials: new InMemoryCredentialStore(),
    modelsPath: null,
    allowModelNetwork: false,
  });
  const provider = runtime.getProvider(request.provider_id);
  if (!provider) throw new Error("provider 不在当前 Pi Runtime 目录");
  if (request.auth_type === "api_key" && !provider.auth.apiKey?.login) {
    throw new Error("该 provider 不支持交互式 API Key 配置");
  }
  if (request.auth_type === "oauth" && !provider.auth.oauth) {
    throw new Error("该 provider 不支持 OAuth");
  }
  const credential = await runtime.login(request.provider_id, request.auth_type, {
    signal: bridge.signal,
    prompt: (prompt) => bridge.waitForPrompt(prompt),
    notify: (event) => bridge.notify(event),
  });
  bridge.notify({ type: "progress", message: "认证完成" });
  // auth_success 是唯一允许携带新凭据的认证结果消息。
  bridge.succeed(request.provider_id, credential);
}

import { createInterface } from "node:readline";

import { registerBunOAuthFlows } from "@earendil-works/pi-ai/bun-oauth";

import { AuthBridge, runProviderAuth, safeAuthError } from "./auth";
import { createProviderCatalog } from "./catalog";
import {
  PROTOCOL_VERSION,
  parseHostMessage,
  type SidecarMessage,
  type AuthStartRequest,
  type StartRequest,
} from "./protocol";
import { runPiSession, type ActiveSessionControl } from "./session";
import { ToolBridge } from "./tool-bridge";
import { runSubagentChild } from "./subagents/child";
import { PI_SDK_VERSION, SIDECAR_VERSION } from "./version";

// Bun 单文件编译无法保留 OAuth 模块的变量动态 import。必须在读取 provider 目录前
// 注册 SDK 随包携带的认证流，否则 OpenAI Codex 会在生成登录 URL 前直接失败。
registerBunOAuthFlows();

export function createHealthMessage(): Extract<SidecarMessage, { type: "health" }> {
  return {
    type: "health",
    protocol_version: PROTOCOL_VERSION,
    sidecar_version: SIDECAR_VERSION,
    pi_sdk_version: PI_SDK_VERSION,
    platform: process.platform,
    arch: process.arch,
    capabilities: {
      subagents: {
        package: "pi-subagents",
        version: "0.35.1",
        child_mode: true,
      },
    },
  };
}

function writeMessage(message: SidecarMessage): void {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

function requestSecrets(request: StartRequest | undefined): string[] {
  const credential = request?.model.credential;
  if (!credential) return [];
  return credential.type === "api_key"
    ? credential.key
      ? [credential.key]
      : []
    : [credential.access, credential.refresh].filter(Boolean);
}

function safeErrorMessage(error: unknown, secrets: string[] = []): string {
  let message = error instanceof Error ? error.message : "Pi Runtime 未知错误";
  for (const secret of secrets) message = message.split(secret).join("[REDACTED]");
  return message.replace(/[\r\n\t]+/g, " ").slice(0, 1_000) || "Pi Runtime 未知错误";
}

export async function runMain(): Promise<void> {
  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
  let activeRequest: StartRequest | undefined;
  let activeAuthRequest: AuthStartRequest | undefined;
  let activeBridge: ToolBridge | undefined;
  let activeAuthBridge: AuthBridge | undefined;
  let activeControl: ActiveSessionControl | undefined;
  let activeTask: Promise<void> | undefined;
  let sessionFinished = false;
  let cancelRequested = false;
  const pendingSteering: string[] = [];

  for await (const line of lines) {
    let message;
    try {
      message = parseHostMessage(line);
    } catch (error) {
      writeMessage({
        type: "error",
        protocol_version: PROTOCOL_VERSION,
        request_id: activeRequest?.request_id,
        code: "protocol_error",
        message: safeErrorMessage(error, [
          ...requestSecrets(activeRequest),
          ...(activeAuthBridge?.secrets() ?? []),
        ]),
      });
      continue;
    }

    if (message.type === "health_check") {
      if (activeRequest || activeAuthRequest) {
        writeMessage({
          type: "error",
          protocol_version: PROTOCOL_VERSION,
          request_id: activeRequest?.request_id ?? activeAuthRequest?.request_id,
          code: "busy",
          message: "Pi Runtime 正在处理请求",
        });
        continue;
      }
      writeMessage(createHealthMessage());
      lines.close();
      break;
    }

    if (message.type === "catalog_request") {
      if (activeRequest || activeAuthRequest) {
        writeMessage({
          type: "error",
          protocol_version: PROTOCOL_VERSION,
          request_id: activeRequest?.request_id ?? activeAuthRequest?.request_id,
          code: "busy",
          message: "Pi Runtime 正在处理请求",
        });
        continue;
      }
      const catalog = await createProviderCatalog();
      writeMessage({
        type: "catalog",
        protocol_version: PROTOCOL_VERSION,
        providers: catalog.providers,
      });
      lines.close();
      break;
    }

    if (message.type === "auth_start") {
      if (activeRequest || activeAuthRequest) {
        writeMessage({
          type: "auth_error",
          protocol_version: PROTOCOL_VERSION,
          request_id: message.request_id,
          message: "每个 Pi Sidecar 进程只允许一个操作",
        });
        continue;
      }
      activeAuthRequest = message;
      activeAuthBridge = new AuthBridge(message.request_id, writeMessage);
      activeTask = runProviderAuth(message, activeAuthBridge)
        .catch((error) => {
          if (activeAuthBridge?.cancelled) {
            writeMessage({
              type: "auth_cancelled",
              protocol_version: PROTOCOL_VERSION,
              request_id: message.request_id,
            });
          } else {
            writeMessage({
              type: "auth_error",
              protocol_version: PROTOCOL_VERSION,
              request_id: message.request_id,
              message: safeAuthError(error, activeAuthBridge?.secrets()),
            });
          }
        })
        .finally(() => {
          sessionFinished = true;
          lines.close();
        });
      continue;
    }

    if (message.type === "start") {
      if (activeRequest || activeAuthRequest) {
        writeMessage({
          type: "error",
          protocol_version: PROTOCOL_VERSION,
          request_id: message.request_id,
          code: "one_operation_per_process",
          message: "每个 Pi Sidecar 进程只允许一个操作",
        });
        continue;
      }
      activeRequest = message;
      activeBridge = new ToolBridge(message.request_id, writeMessage);
      activeTask = runPiSession(message, activeBridge, writeMessage, (control) => {
        activeControl = control;
        if (control) {
          for (const content of pendingSteering.splice(0)) control.steer(content);
          if (cancelRequested) void control.abort();
        }
      })
        .catch((error) => {
          writeMessage({
            type: "error",
            protocol_version: PROTOCOL_VERSION,
            request_id: message.request_id,
            code: "session_error",
            message: safeErrorMessage(error, requestSecrets(message)),
          });
        })
        .finally(() => {
          sessionFinished = true;
          lines.close();
        });
      continue;
    }

    if (activeAuthRequest) {
      if (message.request_id !== activeAuthRequest.request_id) {
        writeMessage({
          type: "auth_error",
          protocol_version: PROTOCOL_VERSION,
          request_id: message.request_id,
          message: "消息 request_id 与当前认证不匹配",
        });
      } else if (message.type === "auth_prompt_response") {
        if (!activeAuthBridge?.resolvePrompt(message.prompt_id, message.value)) {
          writeMessage({
            type: "auth_error",
            protocol_version: PROTOCOL_VERSION,
            request_id: message.request_id,
            message: "认证输入已失效或 prompt_id 不匹配",
          });
        }
      } else if (message.type === "auth_cancel") {
        activeAuthBridge?.cancel();
      } else {
        writeMessage({
          type: "auth_error",
          protocol_version: PROTOCOL_VERSION,
          request_id: message.request_id,
          message: "认证期间不接受该消息类型",
        });
      }
      continue;
    }

    if (!activeRequest || message.request_id !== activeRequest.request_id) {
      writeMessage({
        type: "error",
        protocol_version: PROTOCOL_VERSION,
        request_id: message.request_id,
        code: "request_mismatch",
        message: "消息 request_id 与当前请求不匹配",
      });
      continue;
    }

    if (message.type === "tool_result") {
      activeBridge?.resolve(message);
    } else if (message.type === "steer") {
      if (activeControl) activeControl.steer(message.content);
      else pendingSteering.push(message.content);
    } else if (message.type === "cancel") {
      activeBridge?.cancel("用户取消");
      await activeControl?.abort();
    }
  }

  if (activeTask) {
    if (!sessionFinished) {
      cancelRequested = true;
      activeBridge?.cancel("Sidecar 输入已关闭");
      activeAuthBridge?.cancel();
      await activeControl?.abort();
    }
    await activeTask;
  }
}

if (import.meta.main) {
  if (process.env.PI_SUBAGENT_CHILD === "1") {
    await runSubagentChild();
  } else {
    await runMain();
  }
}

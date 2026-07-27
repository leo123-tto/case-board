import type {
  CredentialStatusView,
  DeviceSyncStatus,
  Settings,
  TeamStatus,
} from "./types";

export const SECRET_SETTING_KEYS = [
  "mineru_api_key",
  "paddle_vl_api_key",
  "cloud_llm_api_key",
  "minimax_api_key",
  "compat_llm_api_key",
  "glm_llm_api_key",
  "mimo_llm_api_key",
  "kimi_llm_api_key",
  "custom_llm_api_key",
  "yuandian_api_key",
  "kuaidi100_key",
  "embedding_api_key",
  "feishu_app_token",
  "feishu_webhook_url",
  "court_filing_password",
] as const satisfies readonly (keyof Settings)[];

type UnknownRecord = Record<string, unknown>;

function sanitizeMcpTransport(transport: unknown): unknown {
  if (!transport || typeof transport !== "object") return transport;
  const safe = { ...(transport as UnknownRecord) };
  delete safe.env;
  delete safe.headers;
  return safe;
}

function sanitizeNestedIdentity(identity: unknown): unknown {
  if (!identity || typeof identity !== "object") return identity;
  const safe = { ...(identity as UnknownRecord) };
  delete safe.team_secret;
  delete safe.group_secret;
  delete safe.pairing_code;
  return safe;
}

/**
 * Transitional frontend boundary for the 0.4 bridge.
 *
 * Task 3B will make `get_settings` metadata-only at the Rust boundary. Until
 * that lands, every frontend caller must pass its response through this
 * function before storing it in React state.
 */
export function sanitizeSettingsForFrontend(settings: Settings): Settings {
  const safe = { ...settings } as Settings & UnknownRecord;
  for (const key of SECRET_SETTING_KEYS) delete safe[key];
  safe.team = sanitizeNestedIdentity(settings.team) as Settings["team"];
  safe.device_sync = sanitizeNestedIdentity(settings.device_sync) as Settings["device_sync"];
  safe.mcp_servers = (settings.mcp_servers ?? []).map((server) => ({
    ...server,
    transport: sanitizeMcpTransport(server.transport),
  })) as Settings["mcp_servers"];
  return safe;
}

export function sanitizeTeamStatusForFrontend(status: TeamStatus): TeamStatus {
  if (!status.identity) return status;
  return {
    ...status,
    identity: sanitizeNestedIdentity(status.identity) as TeamStatus["identity"],
  };
}

export function sanitizeDeviceSyncStatusForFrontend(
  status: DeviceSyncStatus,
): DeviceSyncStatus {
  if (!status.identity) return status;
  return {
    ...status,
    identity: sanitizeNestedIdentity(status.identity) as DeviceSyncStatus["identity"],
  };
}

export function credentialStatusLabel(status: CredentialStatusView | null | undefined): string {
  if (!status) return "未配置";
  if (!status.secret_present) return "需处理";
  if (status.state === "valid") return "已验证";
  if (status.state === "unverified" || status.state === "pending_migration") return "已配置";
  return "需处理";
}

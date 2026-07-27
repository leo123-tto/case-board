import { useState } from "react";

import type { LegacyCredentialMigrationStatus } from "@/lib/types";

const DECLINED_KEY = "caseboard.legacy-credential-migration.declined";

interface LegacyCredentialMigrationCardProps {
  status: LegacyCredentialMigrationStatus;
  onMigrate: (confirmed: true) => Promise<unknown>;
}

const RESULT_LABELS: Record<
  LegacyCredentialMigrationStatus["items"][number]["state"],
  string
> = {
  pending: "待检查",
  imported: "已导入",
  already_imported: "已是最新",
  missing: "未找到",
  unreadable: "无法读取",
  failed: "导入失败",
};

export function LegacyCredentialMigrationCard({
  status,
  onMigrate,
}: LegacyCredentialMigrationCardProps) {
  const [declined, setDeclined] = useState(
    () => localStorage.getItem(DECLINED_KEY) === "true",
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if ((!status.pending && !status.attempted) || status.declined || declined) return null;

  const migrate = async () => {
    setBusy(true);
    setError(null);
    try {
      await onMigrate(true);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };

  const decline = () => {
    localStorage.setItem(DECLINED_KEY, "true");
    setDeclined(true);
  };

  return (
    <section className="space-y-3 rounded-lg border border-amber-300 bg-amber-50 p-4 text-sm text-amber-950">
      <h3 className="font-semibold">
        {status.attempted ? "已有登录导入结果" : "安全迁移已有登录"}
      </h3>
      <p>已有凭据始终留在这台电脑；CaseBoard 不需要也不会收到你的 Mac 登录密码。</p>
      <p>仅在这一次主动迁移过程中，macOS 可能显示一次系统授权提示。</p>
      {status.attempted && (
        <div className="max-h-64 space-y-1 overflow-auto rounded-md border border-amber-200 bg-background/70 p-2">
          {status.items.map((item) => (
            <div
              key={item.stable_inventory_id}
              className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-0.5 py-1 text-xs"
            >
              <span className="truncate font-mono">{item.provider_or_connector_id}</span>
              <span>{RESULT_LABELS[item.state]}</span>
              {(item.reconnect_required || item.error) && (
                <span className="col-span-2 text-muted-foreground">
                  {item.reconnect_required ? "快照已过期，0.5 接管时需要重新连接。" : item.error}
                </span>
              )}
            </div>
          ))}
        </div>
      )}
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={() => void migrate()}
          disabled={busy}
          className="rounded-md bg-foreground px-3 py-2 font-medium text-background disabled:opacity-50"
        >
          {busy
            ? "迁移中…"
            : status.attempted
              ? "重新检查并导入"
              : "安全迁移已有登录"}
        </button>
        <button
          type="button"
          onClick={decline}
          disabled={busy}
          className="rounded-md border border-border bg-background px-3 py-2"
        >
          暂不迁移
        </button>
      </div>
      <p className="text-xs">稍后仍可在设置中重试，或改用浏览器登录 / 重新填写 API Key。</p>
      {error && <p role="alert" className="text-xs text-destructive">{error}</p>}
    </section>
  );
}

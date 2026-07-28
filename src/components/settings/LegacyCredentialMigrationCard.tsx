import { useState } from "react";

import type { LegacyCredentialMigrationStatus } from "@/lib/types";

const DECLINED_KEY = "caseboard.legacy-credential-migration.declined";

interface LegacyCredentialMigrationCardProps {
  status: LegacyCredentialMigrationStatus;
  onMigrate: (confirmed: true) => Promise<unknown>;
}

type ItemState = LegacyCredentialMigrationStatus["items"][number]["state"];

const RESULT_LABELS: Record<ItemState, string> = {
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

  const dismiss = () => {
    localStorage.setItem(DECLINED_KEY, "true");
    setDeclined(true);
  };

  // 导入后分三类:成功(已导入/已是最新)逐项展示;出错(无法读取/导入失败)逐项展示并保留重试;
  // 「未找到」只汇总一行 —— 没用过的服务本来就没有登录,不该平铺一长串吓用户(2026-07-27 真机反馈)。
  const successItems = status.items.filter(
    (item) => item.state === "imported" || item.state === "already_imported",
  );
  const problemItems = status.items.filter(
    (item) => item.state === "unreadable" || item.state === "failed",
  );
  const missingCount = status.items.filter((item) => item.state === "missing").length;

  return (
    <section className="space-y-3 rounded-lg border border-amber-300 bg-amber-50 p-4 text-sm text-amber-950">
      <h3 className="font-semibold">
        {status.attempted ? "已有登录导入结果" : "安全迁移已有登录"}
      </h3>
      {!status.attempted && (
        <>
          <p>已有凭据始终留在这台电脑；CaseBoard 不需要也不会收到你的 Mac 登录密码。</p>
          <p>仅在这一次主动迁移过程中，macOS 可能显示一次系统授权提示。</p>
        </>
      )}
      {status.attempted && (
        <>
          <p>
            {successItems.length > 0
              ? `找到并安全保存了 ${successItems.length} 项已有登录。`
              : "没有找到可导入的已有登录。"}
            {missingCount > 0 &&
              `另有 ${missingCount} 项未找到——没用过的服务本来就没有登录,无需处理。`}
          </p>
          {(successItems.length > 0 || problemItems.length > 0) && (
            <div className="max-h-64 space-y-1 overflow-auto rounded-md border border-amber-200 bg-background/70 p-2">
              {[...successItems, ...problemItems].map((item) => (
                <div
                  key={item.stable_inventory_id}
                  className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-0.5 py-1 text-xs"
                >
                  <span className="truncate font-mono">{item.provider_or_connector_id}</span>
                  <span>{RESULT_LABELS[item.state]}</span>
                  {(item.reconnect_required || item.error) && (
                    <span className="col-span-2 text-muted-foreground">
                      {item.reconnect_required
                        ? "快照已过期，0.5 接管时需要重新连接。"
                        : item.error}
                    </span>
                  )}
                </div>
              ))}
            </div>
          )}
        </>
      )}
      <div className="flex flex-wrap gap-2">
        {status.attempted ? (
          <>
            <button
              type="button"
              onClick={dismiss}
              disabled={busy}
              className="rounded-md bg-foreground px-3 py-2 font-medium text-background disabled:opacity-50"
            >
              完成
            </button>
            <button
              type="button"
              onClick={() => void migrate()}
              disabled={busy}
              className="rounded-md border border-border bg-background px-3 py-2"
            >
              {busy ? "迁移中…" : "重新检查并导入"}
            </button>
          </>
        ) : (
          <>
            <button
              type="button"
              onClick={() => void migrate()}
              disabled={busy}
              className="rounded-md bg-foreground px-3 py-2 font-medium text-background disabled:opacity-50"
            >
              {busy ? "迁移中…" : "安全迁移已有登录"}
            </button>
            <button
              type="button"
              onClick={dismiss}
              disabled={busy}
              className="rounded-md border border-border bg-background px-3 py-2"
            >
              暂不迁移
            </button>
          </>
        )}
      </div>
      {!status.attempted && (
        <p className="text-xs">稍后仍可在设置中重试，或改用浏览器登录 / 重新填写 API Key。</p>
      )}
      {status.attempted && (
        <p className="text-xs">已导入的登录不会因关闭本卡片而丢失；以后仍可在设置中重新检查。</p>
      )}
      {error && <p role="alert" className="text-xs text-destructive">{error}</p>}
    </section>
  );
}

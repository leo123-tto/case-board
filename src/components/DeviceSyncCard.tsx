import { useEffect, useState } from "react";
import { Check, Copy, Laptop, Loader2, RefreshCw, Search, Unplug } from "lucide-react";

import { Button } from "@/components/ui/button";
import { confirmDialog } from "@/lib/dialog";
import {
  deviceSyncCreate,
  deviceSyncDefaultName,
  deviceSyncDiscover,
  deviceSyncForget,
  deviceSyncJoin,
  deviceSyncNow,
  deviceSyncRefreshCode,
  deviceSyncSetEnabled,
  deviceSyncStatus,
} from "@/lib/api";
import type { DeviceSyncStatus, DiscoveredDeviceGroup } from "@/lib/types";

const inputCls =
  "h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none transition-colors focus:border-ring focus:ring-1 focus:ring-ring";

function when(value: string | null) {
  if (!value) return "尚未同步";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString("zh-CN", { hour12: false });
}

export function DeviceSyncCard() {
  const [status, setStatus] = useState<DeviceSyncStatus | null>(null);
  const [deviceName, setDeviceName] = useState("");
  const [groupName, setGroupName] = useState("我的案件看板");
  const [groups, setGroups] = useState<DiscoveredDeviceGroup[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [code, setCode] = useState("");
  const [oneTimePairingCode, setOneTimePairingCode] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  async function reload() {
    const next = await deviceSyncStatus();
    setStatus(next);
    return next;
  }

  useEffect(() => {
    Promise.all([reload(), deviceSyncDefaultName()])
      .then(([, name]) => setDeviceName(name))
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    if (!status?.enabled) return;
    const timer = window.setInterval(() => {
      void reload().catch(() => {});
    }, 10_000);
    return () => window.clearInterval(timer);
  }, [status?.enabled]);

  async function run(label: string, action: () => Promise<unknown>) {
    setBusy(label);
    setError(null);
    setMessage(null);
    try {
      await action();
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }

  if (!status) {
    return (
      <section className="rounded-lg border border-border bg-card p-5">
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin" /> 加载个人设备同步…
        </div>
      </section>
    );
  }

  const identity = status.identity;
  return (
    <section className="rounded-lg border border-border bg-card p-5">
      <div className="mb-4 flex items-start justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <Laptop className="size-4 text-brand" />
            <h3 className="text-sm font-semibold">我的设备同步</h3>
          </div>
          <p className="mt-1 text-xs leading-5 text-muted-foreground">
            业务状态、派生 Markdown 和云端 API 配置多向同步；副设备新增的源文件只加密归集到主力设备。
          </p>
        </div>
        <label className="flex shrink-0 cursor-pointer items-center gap-2 text-xs">
          <input
            type="checkbox"
            checked={status.enabled}
            disabled={busy !== null}
            onChange={(e) =>
              void run("toggle", async () => {
                const next = await deviceSyncSetEnabled(e.target.checked);
                setStatus(next);
              })
            }
            className="size-4 accent-brand"
          />
          {status.enabled ? "已开启" : "已关闭"}
        </label>
      </div>

      {status.enabled && !identity && (
        <div className="space-y-4">
          <label className="block space-y-1.5 text-xs">
            <span className="font-medium">本机名称</span>
            <input
              value={deviceName}
              onChange={(e) => setDeviceName(e.target.value)}
              className={inputCls}
              placeholder="例：办公室 MacBook / 家里 Windows 台式机"
            />
          </label>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className="rounded-md border border-border p-3">
              <p className="text-xs font-medium">这是第一台设备</p>
              <p className="mt-1 text-[11px] leading-4 text-muted-foreground">
                创建后本机将成为唯一主力设备，集中保存所有设备新增的源文件。
              </p>
              <input
                value={groupName}
                onChange={(e) => setGroupName(e.target.value)}
                className={`${inputCls} mt-2`}
                placeholder="设备组名称"
              />
              <Button
                size="sm"
                className="mt-2 w-full"
                disabled={busy !== null || !deviceName.trim() || !groupName.trim()}
                onClick={() =>
                  void run("create", async () => {
                    await deviceSyncCreate(groupName, deviceName);
                    setOneTimePairingCode(await deviceSyncRefreshCode());
                  })
                }
              >
                {busy === "create" && <Loader2 className="mr-1 size-3.5 animate-spin" />}
                创建设备组
              </Button>
            </div>
            <div className="rounded-md border border-border p-3">
              <div className="flex items-center justify-between">
                <p className="text-xs font-medium">加入另一台设备</p>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busy !== null}
                  onClick={() =>
                    void run("discover", async () => {
                      const found = await deviceSyncDiscover();
                      setGroups(found);
                      setSelected(found[0]?.group_id ?? null);
                      setMessage(found.length ? null : "没有发现开放配对的设备");
                    })
                  }
                >
                  {busy === "discover" ? (
                    <Loader2 className="mr-1 size-3.5 animate-spin" />
                  ) : (
                    <Search className="mr-1 size-3.5" />
                  )}
                  搜索
                </Button>
              </div>
              {groups.length > 0 && (
                <select
                  value={selected ?? ""}
                  onChange={(e) => setSelected(e.target.value)}
                  className={`${inputCls} mt-2`}
                >
                  {groups.map((group) => (
                    <option key={group.device_id} value={group.group_id}>
                      {group.group_name} · {group.device_name}
                    </option>
                  ))}
                </select>
              )}
              <input
                value={code}
                onChange={(e) => setCode(e.target.value.toUpperCase())}
                className={`${inputCls} mt-2 font-mono`}
                placeholder="输入另一台电脑显示的配对口令"
              />
              <p className="mt-1 text-[11px] text-muted-foreground">
                加入后本机是副设备；本机新增源文件会自动回传主力设备。
              </p>
              <Button
                variant="outline"
                size="sm"
                className="mt-2 w-full"
                disabled={busy !== null || !selected || !deviceName.trim() || !code.trim()}
                onClick={() =>
                  void run("join", async () => {
                    await deviceSyncJoin(selected!, code.trim(), deviceName.trim());
                    setCode("");
                  })
                }
              >
                {busy === "join" && <Loader2 className="mr-1 size-3.5 animate-spin" />}
                加入设备组
              </Button>
            </div>
          </div>
        </div>
      )}

      {status.enabled && identity && (
        <div className="space-y-3">
          <div className="rounded-md bg-muted/60 p-3 text-xs leading-5">
            <div className="flex items-center justify-between gap-3">
              <span className="font-medium">{identity.group_name}</span>
              <span className="text-muted-foreground">
                本机：{identity.device_name} · {identity.is_primary ? "主力设备" : "副设备"}
              </span>
            </div>
            <div className="mt-1 text-muted-foreground">
              上次同步：{when(status.last_sync_at)} · 已知设备 {status.devices} 台 · 业务记录 {status.records} 条 · Markdown {status.artifacts} 份
              {status.pending_cases > 0 && ` · 待关联 ${status.pending_cases} 份`}
              {status.conflicts > 0 && ` · 冲突副本 ${status.conflicts} 份`}
              {status.source_pending > 0 &&
                ` · ${identity.is_primary ? "接收中" : "待归集源文件"} ${status.source_pending} 份`}
            </div>
          </div>

          {oneTimePairingCode && status.enabled && (
            <div className="rounded-md border border-brand/20 bg-brand-soft/40 p-3">
              <div className="flex items-center justify-between gap-2">
                <div>
                  <p className="text-xs font-medium">另一台电脑的配对口令</p>
                  <code className="mt-1 block select-all break-all text-xs font-semibold tracking-wide">
                    {oneTimePairingCode}
                  </code>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    void navigator.clipboard.writeText(oneTimePairingCode);
                    setCopied(true);
                    window.setTimeout(() => setCopied(false), 1500);
                  }}
                >
                  {copied ? <Check className="size-4" /> : <Copy className="size-4" />}
                </Button>
              </div>
            </div>
          )}

          {status.platform === "windows" && status.enabled && (
            <p className="rounded-md bg-amber-50 px-3 py-2 text-xs text-amber-900 dark:bg-amber-950/30 dark:text-amber-200">
              Windows 首次启用时，请在系统防火墙提示中允许“专用网络”；不要开放“公用网络”。
            </p>
          )}

          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              disabled={!status.enabled || busy !== null}
              onClick={() =>
                void run("sync", async () => {
                  const result = await deviceSyncNow();
                  setMessage(
                    result.peers_found === 0
                      ? "没有发现同组在线设备"
                      : `同步完成：发送 ${result.sent}，接收 ${result.received} 项变更`,
                  );
                  if (result.errors.length) setError(result.errors.join("；"));
                })
              }
            >
              {busy === "sync" ? (
                <Loader2 className="mr-1 size-3.5 animate-spin" />
              ) : (
                <RefreshCw className="mr-1 size-3.5" />
              )}
              立即同步
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={!status.enabled || busy !== null}
              onClick={() =>
                 void run("code", async () => {
                   setOneTimePairingCode(await deviceSyncRefreshCode());
                 })
              }
            >
              刷新配对口令
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="text-muted-foreground"
              disabled={busy !== null}
              onClick={() =>
                void (async () => {
                  const ok = await confirmDialog(
                    "只清除本机配对关系，不删除任何工作区文档。",
                    {
                      title: "断开个人设备组？",
                      okLabel: "断开",
                      danger: true,
                    },
                  );
                  if (ok) {
                    await run("forget", async () => {
                      await deviceSyncForget();
                      setOneTimePairingCode(null);
                    });
                  }
                })()
              }
            >
              <Unplug className="mr-1 size-3.5" /> 断开
            </Button>
          </div>
        </div>
      )}

      {(message || error || status.last_error) && (
        <p className={`mt-3 text-xs ${error || status.last_error ? "text-destructive" : "text-muted-foreground"}`}>
          {error ?? status.last_error ?? message}
        </p>
      )}
    </section>
  );
}

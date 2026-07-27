/**
 * 快递查询 / 跟踪面板(V0.3 · 快递100 实时查询)。
 *
 * 律师寄送达 / 材料追踪用:输单号 + 选公司 → 查询并跟踪(落本地 express_tracks.json)。
 * 每个单号一张卡片(单号/状态/最新动态),点开看完整轨迹。打开面板时自动刷新在途单号
 * (同单号 40 天内重查免费),已签收停更;超 30 天归档(折叠,可查历史不再更新)。
 *
 * ⚠️ 订阅推送接口要公网回调地址,桌面 app 用不了 → 改用「每次打开自动刷新」等效。
 * 需先在「设置 → 快递100」填 customer + key(申请见 api.kuaidi100.com)。
 */
import { useEffect, useState } from "react";
import {
  Loader2,
  Truck,
  PackageCheck,
  MapPin,
  RefreshCw,
  Trash2,
  ChevronDown,
  ChevronRight,
  Settings as SettingsIcon,
  ExternalLink,
  Save,
} from "lucide-react";

import {
  listExpressTracks,
  refreshExpressTracks,
  queryExpress,
  deleteExpressTrack,
  getSettings,
  listCredentialMetadata,
  revokeCredential,
  saveCredential,
  saveSettings,
  openUrl,
  type ExpressTrack,
} from "@/lib/api";
import type { CredentialMetadata, Settings } from "@/lib/types";
import { toast } from "@/components/ui/toast";
import { confirmDialog } from "@/lib/dialog";
import { CredentialField } from "@/components/settings/CredentialField";

const CARRIERS: { code: string; name: string }[] = [
  { code: "shunfeng", name: "顺丰" },
  { code: "ems", name: "EMS / 邮政" },
  { code: "zhongtong", name: "中通" },
  { code: "yuantong", name: "圆通" },
  { code: "yunda", name: "韵达" },
  { code: "shentong", name: "申通" },
  { code: "jd", name: "京东" },
];

// 快递100 规定:顺丰 / 中通查询必须带收寄件人手机号(否则报 408),其它快递选填。
const PHONE_REQUIRED = new Set(["shunfeng", "zhongtong"]);

/** 按单号字母前缀猜快递公司(高置信前缀才猜;纯数字各家重叠不强猜,返回 null)。 */
function guessCarrier(num: string): string | null {
  const s = num.trim().toUpperCase();
  if (!s) return null;
  if (/^SF\d/.test(s)) return "shunfeng";
  if (/^JD[A-Z]?\d/.test(s)) return "jd";
  if (/^YT\d/.test(s)) return "yuantong";
  if (/^ZT[O]?\d/.test(s)) return "zhongtong";
  if (/^STO\d/.test(s)) return "shentong";
  if (/^[A-Z]{2}\d{9}CN$/.test(s)) return "ems"; // EMS 标准格式 如 EA123456789CN
  return null;
}

const daysSince = (iso: string) => {
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return 0;
  return Math.floor((Date.now() - t) / 86400000);
};

function StateBadge({ t }: { t: ExpressTrack }) {
  const cls = t.delivered
    ? "bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300"
    : t.state === "2" || t.state.startsWith("4") || t.state.startsWith("6")
      ? "bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-300"
      : "bg-sky-100 text-sky-700 dark:bg-sky-900/40 dark:text-sky-300";
  return (
    <span className={`rounded px-1.5 py-0.5 text-caption font-medium ${cls}`}>
      {t.state_text}
    </span>
  );
}

function TrackCard({
  t,
  onDelete,
}: {
  t: ExpressTrack;
  onDelete: (num: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const latest = t.nodes[0];
  return (
    <div className="rounded-lg border border-border bg-card">
      <div className="flex items-start gap-2 p-3">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="mt-0.5 shrink-0 text-muted-foreground hover:text-foreground"
        >
          {open ? <ChevronDown className="size-4" /> : <ChevronRight className="size-4" />}
        </button>
        <div className="min-w-0 flex-1" onClick={() => setOpen((v) => !v)} role="button">
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-xs font-medium text-foreground">{t.com_name}</span>
            <span className="font-mono text-caption text-muted-foreground">{t.num}</span>
            <StateBadge t={t} />
          </div>
          {latest && (
            <p className="mt-1 truncate text-xs text-muted-foreground">{latest.context}</p>
          )}
          {latest && (
            <p className="mt-0.5 font-mono text-caption text-muted-foreground/60">
              {latest.time}
            </p>
          )}
        </div>
        <button
          type="button"
          onClick={() => onDelete(t.num)}
          title="删除跟踪"
          className="shrink-0 rounded p-1 text-muted-foreground/50 transition-colors hover:bg-red-50 hover:text-red-500 dark:hover:bg-red-950/30"
        >
          <Trash2 className="size-3.5" />
        </button>
      </div>
      {open && t.nodes.length > 0 && (
        <ol className="space-y-0 border-t border-border px-3 py-3 pl-5">
          {t.nodes.map((n, i) => (
            <li key={i} className="flex gap-3">
              <div className="flex flex-col items-center">
                <span
                  className={`mt-1 size-2 shrink-0 rounded-full ${
                    i === 0 ? "bg-sky-500" : "bg-muted-foreground/30"
                  }`}
                />
                {i < t.nodes.length - 1 && <span className="w-px flex-1 bg-border" />}
              </div>
              <div className={`pb-3 ${i === 0 ? "text-foreground" : "text-muted-foreground"}`}>
                <p className="text-xs leading-snug">{n.context}</p>
                <p className="mt-0.5 flex items-center gap-1 font-mono text-caption text-muted-foreground/70">
                  <MapPin className="size-2.5" />
                  {n.time}
                </p>
              </div>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}

export function CourierTool() {
  const [tracks, setTracks] = useState<ExpressTrack[]>([]);
  const [com, setCom] = useState("shunfeng");
  const [num, setNum] = useState("");
  const [phone, setPhone] = useState("");
  const [querying, setQuerying] = useState(false);
  const [refreshing, setRefreshing] = useState(false);

  // 快递100 接口配置(2026-06-16:从设置页迁到这里,就近配置 customer + key)
  const [settings, setSettings] = useState<Settings | null>(null);
  const [custCfg, setCustCfg] = useState("");
  const [credential, setCredential] = useState<CredentialMetadata | null>(null);
  const [savingCfg, setSavingCfg] = useState(false);
  const [cfgDirty, setCfgDirty] = useState(false);
  const [cfgOpen, setCfgOpen] = useState(false);
  const configured = !!(
    settings?.kuaidi100_customer?.trim() && credential?.secret_present
  );

  // 打开面板:读 settings(快递100 配置)+ 本地单号 + 自动刷新在途单号(40 天内免费)
  useEffect(() => {
    Promise.all([getSettings(), listCredentialMetadata().catch(() => [])])
      .then(([s, metadata]) => {
        setSettings(s);
        setCustCfg(s.kuaidi100_customer ?? "");
        const current =
          metadata.find(
            (item) =>
              item.provider_or_connector_id === "connector.kuaidi100"
              && item.kind === "api_key",
          ) ?? null;
        setCredential(current);
        // 未配置则展开配置区提示填写,已配置则收起
        setCfgOpen(!(s.kuaidi100_customer?.trim() && current?.secret_present));
      })
      .catch(() => {});
    listExpressTracks()
      .then(setTracks)
      .catch(() => {});
    refreshExpressTracks()
      .then(setTracks)
      .catch(() => {});
  }, []);

  const saveKuaidiCfg = async () => {
    if (!settings) return;
    setSavingCfg(true);
    try {
      const next: Settings = {
        ...settings,
        kuaidi100_customer: custCfg.trim() || null,
      };
      await saveSettings(next);
      setSettings(next);
      setCfgDirty(false);
      toast("快递100 配置已保存", "info");
    } catch (e) {
      toast(`保存失败:${e}`, "error");
    } finally {
      setSavingCfg(false);
    }
  };

  const handleQuery = async () => {
    if (!num.trim()) return;
    const carrier = CARRIERS.find((c) => c.code === com);
    if (PHONE_REQUIRED.has(com) && !phone.trim()) {
      toast(`${carrier?.name ?? "该快递"}查询需填收寄件人手机号(后 4 位即可)`, "error");
      return;
    }
    setQuerying(true);
    try {
      const list = await queryExpress(com, carrier?.name ?? com, num.trim(), phone.trim());
      setTracks(list);
      setNum("");
      setPhone("");
    } catch (e) {
      toast(`${e}`, "error");
    } finally {
      setQuerying(false);
    }
  };

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      setTracks(await refreshExpressTracks());
    } catch (e) {
      toast(`刷新失败:${e}`, "error");
    } finally {
      setRefreshing(false);
    }
  };

  const handleDelete = async (n: string) => {
    const ok = await confirmDialog(`不再跟踪单号 ${n}?(历史记录一并删除)`);
    if (!ok) return;
    try {
      setTracks(await deleteExpressTrack(n));
    } catch (e) {
      toast(`删除失败:${e}`, "error");
    }
  };

  const active = tracks.filter((t) => daysSince(t.created_at) <= 30);
  const archived = tracks.filter((t) => daysSince(t.created_at) > 30);
  const needsPhone = PHONE_REQUIRED.has(com);

  return (
    <div className="space-y-4">
      <div className="rounded-lg border border-sky-200 bg-sky-50 px-4 py-3 text-xs text-sky-800 dark:border-sky-800/50 dark:bg-sky-950/30 dark:text-sky-200">
        输单号查物流并<strong>持续跟踪</strong>:打开本页自动刷新在途单号(同单号 40 天内重查免费),
        已签收停更,超 30 天归档。需先在下方<strong>快递100 接口配置</strong>填 customer + key;
        顺丰 / 中通查询需填<strong>收寄件人手机号</strong>(后 4 位即可)。
      </div>

      {/* 快递100 接口配置(2026-06-16:从设置页迁来,就近配置)*/}
      <details
        open={cfgOpen}
        onToggle={(e) => setCfgOpen(e.currentTarget.open)}
        className="rounded-lg border border-border bg-card"
      >
        <summary className="flex cursor-pointer list-none items-center gap-2 px-4 py-2.5 text-xs font-medium text-foreground">
          <SettingsIcon className="size-3.5 text-muted-foreground" />
          快递100 接口配置
          <span
            className={`rounded px-1.5 py-0.5 text-caption font-medium ${
              configured
                ? "bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300"
                : "bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-300"
            }`}
          >
            {configured ? "已配置" : "未配置 · 先填这里才能查询"}
          </span>
          <button
            type="button"
            onClick={(e) => {
              e.preventDefault();
              openUrl("https://api.kuaidi100.com/").catch(() => {});
            }}
            className="ml-auto inline-flex items-center gap-1 rounded-md border border-sky-200 bg-sky-50 px-2 py-0.5 text-caption font-medium text-sky-700 hover:bg-sky-100 dark:border-sky-800/50 dark:bg-sky-950/30 dark:text-sky-300"
          >
            <ExternalLink className="size-3" />
            申请 customer / key
          </button>
        </summary>
        <div className="space-y-3 border-t border-border px-4 py-3">
          <div className="space-y-1">
            <label className="block text-caption text-muted-foreground">
              customer(授权码)
            </label>
            <input
              type="text"
              value={custCfg}
              onChange={(e) => {
                setCustCfg(e.target.value);
                setCfgDirty(true);
              }}
              placeholder="快递100 后台的 customer"
              autoComplete="off"
              className="h-9 w-full rounded-md border border-border bg-background px-3 text-sm placeholder:text-muted-foreground/50 focus:border-foreground focus:outline-none focus:ring-1 focus:ring-foreground/20"
            />
          </div>
          <CredentialField
            label="key（授权 key）"
            status={credential}
            onSave={async (secretInput) => {
              const next = await saveCredential({
                handle: credential?.handle ?? null,
                providerOrConnectorId: "connector.kuaidi100",
                kind: "api_key",
                secretInput,
              });
              setCredential(next);
              return next;
            }}
            onRevoke={async (handle, revision) => {
              const next = await revokeCredential(handle, revision);
              setCredential(next);
              return next;
            }}
            placeholder="输入新的快递100 key"
          />
          <div className="flex items-center justify-end gap-2">
            <span className="text-caption text-muted-foreground">
              只存本机,不上传任何地方
            </span>
            <button
              type="button"
              onClick={saveKuaidiCfg}
              disabled={savingCfg || !cfgDirty || !settings}
              className="inline-flex items-center gap-1.5 rounded-md bg-sky-600 px-3 py-1.5 text-xs font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-40"
            >
              {savingCfg ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <Save className="size-3.5" />
              )}
              保存
            </button>
          </div>
        </div>
      </details>

      {/* 添加 */}
      <div className="flex flex-wrap items-end gap-2">
        <div className="space-y-1">
          <label className="block text-caption text-muted-foreground">快递公司</label>
          <select
            value={com}
            onChange={(e) => setCom(e.target.value)}
            className="h-9 rounded-md border border-border bg-background px-2.5 text-sm focus:border-foreground focus:outline-none"
          >
            {CARRIERS.map((c) => (
              <option key={c.code} value={c.code}>
                {c.name}
              </option>
            ))}
          </select>
        </div>
        <div className="flex-1 space-y-1" style={{ minWidth: 200 }}>
          <label className="block text-caption text-muted-foreground">运单号</label>
          <input
            type="text"
            value={num}
            onChange={(e) => {
              const v = e.target.value;
              setNum(v);
              const g = guessCarrier(v);
              if (g && g !== com) setCom(g); // 按前缀自动识别快递公司(猜不到不动,可手动改)
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") handleQuery();
            }}
            placeholder="粘贴快递单号,回车或点查询"
            className="h-9 w-full rounded-md border border-border bg-background px-3 text-sm placeholder:text-muted-foreground/50 focus:border-foreground focus:outline-none focus:ring-1 focus:ring-foreground/20"
          />
        </div>
        <div className="space-y-1" style={{ minWidth: 150 }}>
          <label className="block text-caption text-muted-foreground">
            手机号
            {needsPhone ? (
              <span className="text-red-500"> · 顺丰/中通必填</span>
            ) : (
              <span className="text-muted-foreground/60"> · 选填</span>
            )}
          </label>
          <input
            type="text"
            inputMode="numeric"
            value={phone}
            onChange={(e) => setPhone(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") handleQuery();
            }}
            placeholder="收寄件人手机后4位"
            className={`h-9 w-full rounded-md border bg-background px-3 text-sm placeholder:text-muted-foreground/50 focus:outline-none focus:ring-1 focus:ring-foreground/20 ${
              needsPhone && !phone.trim()
                ? "border-red-300 focus:border-red-400"
                : "border-border focus:border-foreground"
            }`}
          />
        </div>
        <button
          type="button"
          onClick={handleQuery}
          disabled={querying || !num.trim()}
          className="inline-flex items-center gap-1.5 rounded-md bg-sky-600 px-3.5 py-2 text-xs font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-40"
        >
          {querying ? <Loader2 className="size-3.5 animate-spin" /> : <Truck className="size-3.5" />}
          查询并跟踪
        </button>
      </div>

      {/* 在途 / 跟踪中 */}
      <div className="space-y-2">
        <div className="flex items-center justify-between px-1">
          <h3 className="text-xs font-semibold text-foreground">
            跟踪中 {active.length > 0 && `(${active.length})`}
          </h3>
          {active.length > 0 && (
            <button
              type="button"
              onClick={handleRefresh}
              disabled={refreshing}
              className="inline-flex items-center gap-1 text-caption text-muted-foreground transition-colors hover:text-foreground disabled:opacity-40"
            >
              <RefreshCw className={`size-3 ${refreshing ? "animate-spin" : ""}`} />
              刷新
            </button>
          )}
        </div>
        {active.length === 0 ? (
          <div className="flex flex-col items-center justify-center rounded-lg border border-dashed border-border py-8 text-center">
            <PackageCheck className="size-6 text-muted-foreground/40" />
            <p className="mt-2 text-xs text-muted-foreground">还没有跟踪的快递</p>
            <p className="mt-1 text-caption text-muted-foreground/70">
              上面输个单号开始跟踪
            </p>
          </div>
        ) : (
          <div className="space-y-2">
            {active.map((t) => (
              <TrackCard key={`${t.com}-${t.num}`} t={t} onDelete={handleDelete} />
            ))}
          </div>
        )}
      </div>

      {/* 已归档(超 30 天,不再自动更新,可查历史) */}
      {archived.length > 0 && (
        <details className="space-y-2">
          <summary className="cursor-pointer px-1 text-xs font-semibold text-muted-foreground">
            已归档 ({archived.length}) · 超 30 天,不再自动更新
          </summary>
          <div className="mt-2 space-y-2 opacity-70">
            {archived.map((t) => (
              <TrackCard key={`${t.com}-${t.num}`} t={t} onDelete={handleDelete} />
            ))}
          </div>
        </details>
      )}
    </div>
  );
}

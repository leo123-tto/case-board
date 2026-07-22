import { useEffect, useState } from "react";
import { CheckCircle2, Loader2, Search, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { confirmDialog } from "@/lib/dialog";
import {
  getNetworkResearchStatuses,
  removeNetworkResearchKey,
  saveNetworkResearchKey,
  verifyNetworkResearchProvider,
  type FirecrawlCreditUsage,
  type ResearchCredentialStatus,
  type ResearchProvider,
} from "@/lib/api";

const inputCls =
  "h-9 min-w-0 flex-1 rounded-md border border-input bg-background px-3 text-sm outline-none transition-colors focus:border-ring focus:ring-1 focus:ring-ring";

type BusyAction = `${ResearchProvider}:${"save" | "verify" | "remove"}` | "load";

function label(provider: ResearchProvider) {
  return provider === "exa" ? "Exa" : "Firecrawl";
}

export function NetworkResearchSettingsCard() {
  const [statuses, setStatuses] = useState<ResearchCredentialStatus[]>([]);
  const [keys, setKeys] = useState<Record<ResearchProvider, string>>({ exa: "", firecrawl: "" });
  const [busy, setBusy] = useState<BusyAction | null>("load");
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [credits, setCredits] = useState<FirecrawlCreditUsage | null>(null);

  async function reload() {
    setStatuses(await getNetworkResearchStatuses());
  }

  useEffect(() => {
    reload()
      .catch((reason) => setError(String(reason)))
      .finally(() => setBusy(null));
  }, []);

  function status(provider: ResearchProvider) {
    return statuses.find((item) => item.provider === provider);
  }

  async function run(action: BusyAction, operation: () => Promise<void>) {
    setBusy(action);
    setError(null);
    setMessage(null);
    try {
      await operation();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  }

  async function save(provider: ResearchProvider) {
    const key = keys[provider].trim();
    if (!key) {
      setError(`请先填入 ${label(provider)} API Key`);
      return;
    }
    await run(`${provider}:save`, async () => {
      await saveNetworkResearchKey(provider, key);
      setKeys((previous) => ({ ...previous, [provider]: "" }));
      await reload();
      setMessage(`${label(provider)} Key 已保存到系统凭据库，请继续验证`);
    });
  }

  async function verify(provider: ResearchProvider) {
    await run(`${provider}:verify`, async () => {
      const result = await verifyNetworkResearchProvider(provider);
      if (provider === "firecrawl") setCredits(result.credit_usage);
      await reload();
      setMessage(result.message);
    });
  }

  async function remove(provider: ResearchProvider) {
    const confirmed = await confirmDialog(
      `将从系统凭据库移除 ${label(provider)} API Key，是否继续？`,
      { title: `移除 ${label(provider)} Key`, okLabel: "移除" },
    );
    if (!confirmed) return;
    await run(`${provider}:remove`, async () => {
      await removeNetworkResearchKey(provider);
      if (provider === "firecrawl") setCredits(null);
      await reload();
      setMessage(`${label(provider)} Key 已移除`);
    });
  }

  return (
    <details className="group rounded-lg border border-border bg-background px-3 py-3">
      <summary className="flex cursor-pointer list-none items-start justify-between gap-3">
        <div className="flex items-start gap-2">
          <Search className="mt-0.5 size-4 text-brand" />
          <div>
            <h4 className="text-xs font-semibold text-foreground">网络研究</h4>
            <p className="mt-1 text-xs leading-5 text-muted-foreground">
              Exa 负责发现优质链接，Firecrawl 负责读取难抓页面；按需配置。
            </p>
          </div>
        </div>
        <span className="shrink-0 text-[11px] text-muted-foreground">
          <span className="group-open:hidden">展开配置</span>
          <span className="hidden group-open:inline">收起</span>
        </span>
      </summary>

      <div className="mt-3 grid gap-3 lg:grid-cols-2">
        {(["exa", "firecrawl"] as const).map((provider) => {
          const current = status(provider);
          const name = label(provider);
          return (
            <div key={provider} className="rounded-md border border-border bg-muted/20 p-3">
              <div className="flex items-center justify-between gap-2">
                <span className="text-xs font-medium">{name}</span>
                <span className={current?.verified_at ? "text-xs text-green-700" : "text-xs text-muted-foreground"}>
                  {current?.verified_at
                    ? "✓ 已验证"
                    : current?.configured
                      ? "已保存，待验证"
                      : "未配置"}
                </span>
              </div>
              <div className="mt-2 flex flex-wrap gap-2">
                <input
                  type="password"
                  aria-label={`${name} API Key`}
                  value={keys[provider]}
                  onChange={(event) =>
                    setKeys((previous) => ({ ...previous, [provider]: event.target.value }))
                  }
                  placeholder={current?.configured ? "输入新 Key 可替换" : "输入 API Key"}
                  autoComplete="off"
                  className={inputCls}
                />
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => void save(provider)}
                  disabled={busy !== null || !keys[provider].trim()}
                >
                  {busy === `${provider}:save` && <Loader2 className="size-3.5 animate-spin" />}
                  保存 {name} Key
                </Button>
              </div>
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => void verify(provider)}
                  disabled={busy !== null || !current?.configured}
                >
                  {busy === `${provider}:verify` ? (
                    <Loader2 className="size-3.5 animate-spin" />
                  ) : (
                    <CheckCircle2 className="size-3.5" />
                  )}
                  验证 {name}
                </Button>
                {current?.configured && (
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    onClick={() => void remove(provider)}
                    disabled={busy !== null}
                  >
                    <Trash2 className="size-3.5" />
                    移除
                  </Button>
                )}
              </div>
              {provider === "exa" && (
                <p className="mt-2 text-[11px] leading-4 text-muted-foreground">
                  验证会执行一次仅返回 1 条结果的最小搜索，可能产生极少量用量。
                </p>
              )}
              {provider === "firecrawl" && credits && (
                <p className="mt-2 text-[11px] text-green-700">
                  额度：总计 {credits.total} · 已用 {credits.used} · 剩余 {credits.remaining}
                </p>
              )}
            </div>
          );
        })}
      </div>
      {message && <p className="mt-2 text-xs text-green-700">✓ {message}</p>}
      {error && <p className="mt-2 text-xs text-red-600">✗ {error}</p>}
    </details>
  );
}

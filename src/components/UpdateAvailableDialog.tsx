/**
 * 发现新版本提示对话框。
 *
 * 2026-05-25 V0.1.8 作者拍板:**不强制更新**,只提示。
 * 2026-06-12 v0.3.14 加应用内自动更新(Tauri updater,带签名验证 + 进度条):
 *   - 「立即更新」→ 应用内下载(进度条)→ 验签 → 自动安装 → 重启
 *     (更新包由作者私钥签名、app 内置公钥强制验签,详见 lib/updater.ts)
 *   - 应用内更新不可用 / 失败(老 OS、endpoint 不可达、签名异常)→ 自动回退「去下载」手动链接
 *   - 「取消」→ 关闭,本次会话不再提示(下次启动还会再检测)
 *
 * 注:0.3.13 及更早版本没有 updater 代码,本次仍走手动下载;从 0.3.14 起后续版本才能一键更新。
 */

import { useState } from "react";
import { Download, X, RefreshCw, AlertTriangle } from "lucide-react";

import { Button } from "@/components/ui/button";
import { openUrl } from "@/lib/api";
import { checkAppUpdate, downloadInstallRelaunch } from "@/lib/updater";
import type { UpdateInfo } from "@/lib/types";

interface Props {
  info: UpdateInfo;
  onClose: () => void;
}

type Phase = "idle" | "working" | "error";

export function UpdateAvailableDialog({ info, onClose }: Props) {
  const [phase, setPhase] = useState<Phase>("idle");
  const [downloaded, setDownloaded] = useState(0);
  const [total, setTotal] = useState(0);
  const [errMsg, setErrMsg] = useState<string | null>(null);

  const pct = total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : 0;
  const mb = (n: number) => (n / 1024 / 1024).toFixed(1);

  const handleManualDownload = async () => {
    const url = info.download_url ?? "https://lawtools.top/";
    try {
      await openUrl(url);
    } catch (e) {
      alert(`打开浏览器失败:${e}\n\n请手动访问 ${url}`);
    }
    onClose();
  };

  const handleInAppUpdate = async () => {
    setPhase("working");
    setErrMsg(null);
    setDownloaded(0);
    setTotal(0);
    try {
      const update = await checkAppUpdate(info);
      if (!update) {
        // endpoint 还没上线 / 不可达 / 当前版本其实不低于远端 —— 回退手动下载
        setErrMsg(
          "暂时无法在应用内更新(可能是更新源未就绪或网络问题),已为你保留「去下载」手动方式。",
        );
        setPhase("error");
        return;
      }
      // 下载(进度)→ 验签 → 安装 → 重启(成功则进程在 relaunch 后退出,不会走到下面)
      await downloadInstallRelaunch(update, (p) => {
        setDownloaded(p.downloaded);
        setTotal(p.total);
      });
    } catch (e) {
      setErrMsg(`应用内更新失败:${e}\n\n可改用「去下载」手动安装新版。`);
      setPhase("error");
    }
  };

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/40 p-4 animate-in fade-in-0 duration-200">
      <div className="w-full max-w-md overflow-hidden rounded-lg border border-border bg-card shadow-2xl animate-in zoom-in-95 fade-in-0 duration-300">
        {/* 顶部 */}
        <header className="flex items-start justify-between gap-3 border-b border-border bg-card/95 px-5 py-4">
          <div>
            <h2 className="text-base font-semibold">🎉 发现新版本</h2>
            <p className="mt-1 text-xs text-muted-foreground">
              当前 <span className="font-mono">v{info.current}</span> →
              最新 <span className="font-mono font-semibold text-foreground">v{info.latest}</span>
              {info.released_at && (
                <span className="ml-2 text-caption">({info.released_at})</span>
              )}
            </p>
          </div>
          {phase !== "working" && (
            <button
              type="button"
              onClick={onClose}
              className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
              aria-label="关闭"
            >
              <X className="size-4" />
            </button>
          )}
        </header>

        {/* 更新内容 */}
        {info.notes && (
          <div className="border-b border-border bg-muted/30 px-5 py-3">
            <p className="text-label font-medium uppercase tracking-wide text-muted-foreground">
              更新内容
            </p>
            <p className="mt-1.5 whitespace-pre-wrap text-xs leading-relaxed text-foreground">
              {info.notes}
            </p>
          </div>
        )}

        {/* 主体:按阶段切换 */}
        <div className="px-5 py-4">
          {phase === "idle" && (
            <p className="text-xs text-muted-foreground">
              点「立即更新」会在应用内下载并自动安装,完成后自动重启 —— 不用手动重新下载。
              更新包经数字签名校验,确保来自作者本人。
            </p>
          )}

          {phase === "working" && (
            <div className="space-y-2">
              <div className="flex items-center justify-between text-xs">
                <span className="flex items-center gap-1.5 text-foreground">
                  <RefreshCw className="size-3.5 animate-spin" />
                  {total > 0 ? "正在下载更新…" : "正在准备更新…"}
                </span>
                <span className="font-mono text-muted-foreground">
                  {total > 0 ? `${mb(downloaded)} / ${mb(total)} MB` : ""}
                </span>
              </div>
              <div className="h-2 w-full overflow-hidden rounded-full bg-muted">
                <div
                  className="h-full rounded-full bg-sky-500 transition-all duration-200"
                  style={{ width: total > 0 ? `${pct}%` : "30%" }}
                />
              </div>
              <p className="text-caption text-muted-foreground">
                下载完成后会自动验签、安装并重启,请勿关闭窗口。
              </p>
            </div>
          )}

          {phase === "error" && errMsg && (
            <div className="flex gap-2 rounded-md bg-amber-50 px-3 py-2.5 text-xs text-amber-900">
              <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-500" />
              <span className="whitespace-pre-wrap leading-relaxed">{errMsg}</span>
            </div>
          )}
        </div>

        {/* 按钮 */}
        <footer className="flex items-center justify-end gap-2 border-t border-border bg-card/95 px-5 py-3">
          {phase === "working" ? (
            <span className="text-xs text-muted-foreground">更新中,请稍候…</span>
          ) : (
            <>
              <Button variant="outline" size="sm" onClick={handleManualDownload}>
                <Download className="mr-1.5 size-3.5" />
                去下载(手动)
              </Button>
              {phase !== "error" && (
                <Button variant="outline" size="sm" onClick={onClose}>
                  取消
                </Button>
              )}
              <Button size="sm" onClick={handleInAppUpdate}>
                <RefreshCw className="mr-1.5 size-3.5" />
                {phase === "error" ? "重试" : "立即更新"}
              </Button>
            </>
          )}
        </footer>
      </div>
    </div>
  );
}

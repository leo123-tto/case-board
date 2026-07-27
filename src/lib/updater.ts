/**
 * 应用内自动更新封装(Tauri 2 updater plugin)。
 *
 * 安全:更新包由作者私钥(minisign)签名,app 内置公钥(tauri.conf plugins.updater.pubkey)
 * 在下载后强制验签 —— 签名不对直接拒装。私钥永不进 git / 不开源,只有作者本机 + CI secret 持有。
 * 因此即便源码全公开、CDN 被黑、中间人劫持,攻击者拿不到私钥就签不出能装的包。
 *
 * 流程:check() → 弹窗给用户看更新内容 → downloadAndInstall(进度回调) → relaunch()。
 * 失败兜底:返回 null / 抛错时,前端回退到「去下载」手动链接,不影响老链路。
 *
 * 升级成功提示:install 前把 {version, notes} 存进 localStorage 的 PENDING_KEY;
 * relaunch 后启动时 consumeJustUpdated() 比对当前版本,命中则弹「✅ 已升级」提示。
 */

import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";

import type { UpdateInfo } from "@/lib/types";

const PENDING_KEY = "caseboard.pending_update";
const CHECK_TIMEOUT_MS = 8_000;

export interface PendingUpdate {
  version: string;
  notes: string | null;
}

export type DownloadProgress = {
  /** 已下载字节 */
  downloaded: number;
  /** 总字节(可能为 0,服务器未给 content-length) */
  total: number;
};

/**
 * 检查是否有可用的应用内更新。`updater_target` 由 Rust 按平台/OS 版本安全选择,
 * tauri.conf 的 endpoint 模板据此读取与提示完全相同的 Stable/Legacy manifest。
 * 有更新返回 Update 句柄;无更新或不可达返回 null(静默,由上层决定是否回退手动下载)。
 */
export async function checkAppUpdate(
  info: Pick<UpdateInfo, "latest" | "updater_target">,
): Promise<Update | null> {
  if (!info.updater_target || !info.latest) return null;

  try {
    const update = await check({
      target: info.updater_target,
      timeout: CHECK_TIMEOUT_MS,
    });
    if (!update) return null;

    // manifest 在提示后发生变化时不静默安装另一个版本。关闭旧 resource,
    // 让用户重新检查后再确认新的版本与说明。
    if (update.version !== info.latest) {
      try {
        await update.close();
      } catch {
        /* resource 关闭失败也不能放行版本不一致的安装 */
      }
      return null;
    }
    return update;
  } catch {
    // endpoint 不可达 / 签名校验失败 / 非 Tauri 环境 —— 一律静默,交给上层兜底
    return null;
  }
}

/**
 * 下载并安装更新,期间回调进度。安装成功后写 PENDING 记录并重启 app。
 * 任一步抛错都向上抛,前端展示错误并回退到手动下载。
 */
export async function downloadInstallRelaunch(
  update: Update,
  onProgress: (p: DownloadProgress) => void,
): Promise<void> {
  let downloaded = 0;
  let total = 0;

  await update.downloadAndInstall((event) => {
    switch (event.event) {
      case "Started":
        total = event.data.contentLength ?? 0;
        onProgress({ downloaded: 0, total });
        break;
      case "Progress":
        downloaded += event.data.chunkLength;
        onProgress({ downloaded, total });
        break;
      case "Finished":
        onProgress({ downloaded: total || downloaded, total: total || downloaded });
        break;
    }
  });

  // 安装成功 —— 记下本次升级的目标版本与更新内容,供重启后弹「升级成功」提示
  try {
    const pending: PendingUpdate = { version: update.version, notes: update.body ?? null };
    localStorage.setItem(PENDING_KEY, JSON.stringify(pending));
  } catch {
    /* localStorage 不可用就跳过成功提示,不影响更新本身 */
  }

  await relaunch();
}

/**
 * 启动时调用:若上次刚装过更新且当前版本已等于目标版本,返回该升级信息(供弹成功提示),
 * 并清掉 PENDING 记录(只弹一次)。否则返回 null。
 */
export async function consumeJustUpdated(): Promise<PendingUpdate | null> {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(PENDING_KEY);
  } catch {
    return null;
  }
  if (!raw) return null;

  let pending: PendingUpdate;
  try {
    pending = JSON.parse(raw) as PendingUpdate;
  } catch {
    try {
      localStorage.removeItem(PENDING_KEY);
    } catch {
      /* ignore */
    }
    return null;
  }

  // 只有当前实际运行版本 == 目标版本,才认定升级真的生效了
  let current = "";
  try {
    current = await getVersion();
  } catch {
    return null;
  }

  // 不管命中与否都清掉,避免卡住反复弹
  try {
    localStorage.removeItem(PENDING_KEY);
  } catch {
    /* ignore */
  }

  if (current === pending.version) {
    return pending;
  }
  return null;
}

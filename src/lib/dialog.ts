import { confirm as tauriConfirm } from "@tauri-apps/plugin-dialog";

/**
 * 二次确认对话框。**一律用这个,不要再用 `window.confirm`**。
 *
 * 2026-05-31 真机暴露:`window.confirm` 在本 app 的 WKWebView 里**不弹窗、直接返回 true**
 * (假确认)—— 删除「一点就没了」,误操作直接丢数据(老板原话)。本函数走 Tauri dialog
 * 插件的**原生 OS 模态对话框**,可靠阻塞,返回用户是否点了确认。
 *
 * 对照已知坑 #10(`window.alert` 是真模态,但 `window.confirm` 不是)。
 */
export function confirmDialog(
  message: string,
  opts?: { title?: string; okLabel?: string; cancelLabel?: string; danger?: boolean },
): Promise<boolean> {
  return tauriConfirm(message, {
    title: opts?.title ?? "请确认",
    kind: opts?.danger ? "warning" : "info",
    okLabel: opts?.okLabel ?? "确定",
    cancelLabel: opts?.cancelLabel ?? "取消",
  });
}

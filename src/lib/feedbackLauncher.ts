export const OPEN_FEEDBACK_EVENT = "caseboard:open-feedback";

export interface OpenFeedbackDetail {
  description?: string;
}

/**
 * 反馈窗口的统一入口。首页看板助手、顶部反馈按钮和后续诊断入口都通过
 * 同一事件契约打开窗口，避免各自维护一套弹窗状态和预填逻辑。
 */
export function openFeedback(description?: string): void {
  window.dispatchEvent(
    new CustomEvent<OpenFeedbackDetail>(OPEN_FEEDBACK_EVENT, {
      detail: { description: description?.trim() || undefined },
    }),
  );
}

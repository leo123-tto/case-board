/**
 * 案件 AI 助手 · 独立 OS 窗口。
 *
 * 把 CaseChatPanel 单独拆到一个 Tauri 窗口里满屏渲染,与主窗口共享同一份本地聊天记录
 * (SQLite,按 case_id)。由 App.tsx 在 URL 含 ?window=chat 时渲染;
 * caseId / caseName / domain 从 URL query 解析传入。
 *
 * 用户视角:一个可最大化、可拖到外接屏的聊天窗口。
 * 系统视角:与主窗口内嵌的是同一个 CaseChatPanel,仅 standalone=true —— 附件 / 引用卡 /
 *   工具调用轨迹 / save_artifact 落文书等能力全部保留。
 */
import { CaseChatPanel } from "./CaseChatPanel";

export function ChatStandaloneWindow({
  caseId,
  caseName,
  domain = "civil",
}: {
  caseId: string | null;
  caseName?: string | null;
  domain?: "civil" | "criminal";
}) {
  return (
    <div className="flex h-screen w-screen flex-col">
      <CaseChatPanel
        standalone
        caseId={caseId}
        caseName={caseName}
        domain={domain}
      />
    </div>
  );
}

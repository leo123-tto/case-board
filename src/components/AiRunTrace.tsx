import { useMemo, useState } from "react";
import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  ExternalLink,
  Loader2,
  Search,
} from "lucide-react";

import { openUrl, type ChatActivity } from "@/lib/api";
import type { AgentRuntime } from "@/lib/piRuntime";
import type { ToolCallRecord } from "@/lib/types";
import { cn } from "@/lib/utils";

const TOOL_LABELS: Record<string, string> = {
  search_local_kb: "检索本地知识库",
  semantic_search_local_kb: "语义检索本地知识库",
  read_kb_file: "读取知识库材料",
  search_laws: "检索法律法规",
  get_law_article: "读取法律条文",
  search_regulations: "检索规范性文件",
  get_regulation_detail: "读取规范性文件",
  law_vector_search: "语义检索法律法规",
  search_cases_normal: "检索裁判案例",
  search_cases_authority: "检索权威案例",
  get_case_detail: "读取案例详情",
  case_vector_search: "语义检索裁判案例",
  enterprise_search: "元典：定位企业主体",
  enterprise_aggregation_summary: "元典：查询企业总览",
  enterprise_base_info: "元典：查询企业工商档案",
  enterprise_change_info: "元典：查询企业变更记录",
  enterprise_writ_list: "元典：查询企业涉诉信息",
  enterprise_annual_report: "元典：查询企业年报",
  web_search: "联网搜索",
  web_fetch: "读取网页资料",
  exa_search: "Exa 联网搜索",
  exa_contents: "Exa 读取网页正文",
  exa_find_similar: "Exa 查找相似来源",
  firecrawl_search: "Firecrawl 联网搜索",
  firecrawl_scrape: "Firecrawl 抓取网页正文",
  verify_legal_citations: "核验法律引用",
  save_artifact: "保存工作区文稿",
  edit_artifact: "编辑工作区文稿",
  read_legal_skill: "读取法律 Skill",
  list_workspace_files: "查看工作区文件",
  read_workspace_file: "读取工作区文件",
  create_workspace_file: "新建工作区文稿",
  write_workspace_file: "更新工作区文稿",
  rename_workspace_file: "重命名工作区文稿",
  copy_workspace_file: "复制为工作区文稿",
  archive_workspace_file: "归档工作区文件",
};

function formatElapsed(milliseconds: number): string {
  if (milliseconds > 0 && milliseconds < 1_000) return "<1s";
  const seconds = Math.max(0, Math.floor(milliseconds / 1_000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  return remainder > 0 ? `${minutes}m ${remainder}s` : `${minutes}m`;
}

function statusLabel(status: string): string {
  if (status === "streaming" || status === "queued" || status === "running") return "处理中";
  if (status === "completed" || status === "done") return "已处理";
  if (status === "incomplete") return "处理未完整结束";
  if (status === "cancelled") return "已停止";
  return "处理失败";
}

function toolLabel(tool: string): string {
  return TOOL_LABELS[tool] ?? "调用辅助工具";
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

function stringValue(value: unknown): string | null {
  if (typeof value === "string" && value.trim()) return value.trim();
  if (typeof value === "number") return String(value);
  return null;
}

function previewRows(record: ToolCallRecord): Array<{ label: string; value: string }> {
  const args = asRecord(record.args);
  const rows: Array<{ label: string; value: string }> = [];
  const add = (label: string, key: string) => {
    const value = stringValue(args[key]);
    if (value) rows.push({ label, value });
  };
  add("检索词", "query");
  add("检索词", "keyword");
  add("限定站点", "site");
  add("限定地区", "region");
  add("效力层级", "effect_level");
  add("知识库文件", "relative_path");
  add("工作区文档", "document_id");
  add("文稿标题", "title");
  return rows;
}

function lawReference(record: ToolCallRecord): string | null {
  const args = asRecord(record.args);
  const law = stringValue(args.fgmc);
  const article = stringValue(args.ftnum);
  if (!law) return null;
  return `《${law.replace(/^《|》$/g, "")}》${article ? `第${article.replace(/^第|条$/g, "")}条` : ""}`;
}

function previewLinks(record: ToolCallRecord): Array<{ title: string; url: string }> {
  const preview = asRecord(record.result_preview);
  const results = Array.isArray(preview.results) ? preview.results : [];
  const links = results.flatMap((item) => {
    const row = asRecord(item);
    const url = stringValue(row.url);
    if (!url || !/^https?:\/\//i.test(url)) return [];
    return [{ title: stringValue(row.title) ?? url, url }];
  });
  const argUrl = stringValue(asRecord(record.args).url);
  if (argUrl && /^https?:\/\//i.test(argUrl) && !links.some((item) => item.url === argUrl)) {
    links.unshift({ title: argUrl, url: argUrl });
  }
  return links.slice(0, 10);
}

function previewLaws(record: ToolCallRecord): Array<{ reference: string; excerpt: string | null }> {
  const preview = asRecord(record.result_preview);
  const laws = Array.isArray(preview.laws) ? preview.laws : [];
  return laws.flatMap((item) => {
    const row = asRecord(item);
    const law = stringValue(row.law);
    const article = stringValue(row.article);
    return law ? [{
      reference: `《${law.replace(/^《|》$/g, "")}》${article ? `第${article.replace(/^第|条$/g, "")}条` : ""}`,
      excerpt: stringValue(row.excerpt),
    }] : [];
  });
}

function ToolAuditRow({ record, index }: { record: ToolCallRecord; index: number }) {
  const [open, setOpen] = useState(false);
  const rows = previewRows(record);
  const law = lawReference(record);
  const links = previewLinks(record);
  const laws = previewLaws(record);
  const standaloneLaw = law && !laws.some((item) => item.reference === law) ? law : null;
  const elapsed = formatElapsed(Math.max(0, record.finished_at_ms - record.started_at_ms));
  const label = record.tool === "__agent_round__"
    ? `第 ${String(asRecord(record.args).iteration ?? index + 1)} 轮 AI 分析`
    : toolLabel(record.tool);
  const hasDetails = record.tool !== "__agent_round__"
    && (rows.length > 0 || Boolean(law) || links.length > 0 || laws.length > 0
      || record.kb_hit || record.credits_used > 0 || Boolean(record.error_short));

  if (!hasDetails) {
    return (
      <div className="flex items-center gap-2">
        {record.success ? <CheckCircle2 className="size-3.5 text-emerald-600" /> : <CircleAlert className="size-3.5 text-destructive" />}
        <span className={cn(!record.success && "text-destructive")}>{label}</span>
        <span className="ml-auto text-[10px]">{elapsed}</span>
      </div>
    );
  }

  return (
    <div className="rounded-md border border-border/70 bg-background/60">
      <button
        type="button"
        aria-label={`${label}，${open ? "收起详情" : "展开详情"}`}
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-center gap-2 px-2 py-1.5 text-left hover:bg-muted/50"
      >
        {record.success ? <CheckCircle2 className="size-3.5 text-emerald-600" /> : <CircleAlert className="size-3.5 text-destructive" />}
        <span className={cn(!record.success && "text-destructive")}>{label}</span>
        <span className="ml-auto text-[10px]">{elapsed}</span>
        {open ? <ChevronDown className="size-3" /> : <ChevronRight className="size-3" />}
      </button>
      {open ? (
        <div className="max-h-80 space-y-1 overflow-auto border-t border-border/60 px-2 py-2 text-[10px] leading-relaxed">
          {standaloneLaw ? <p className="font-medium text-foreground">{standaloneLaw}</p> : null}
          {rows.map((row, rowIndex) => (
            <p key={`${row.label}-${row.value}-${rowIndex}`} className="break-words">
              <span className="text-muted-foreground/80">{row.label}：</span>
              <span className="select-text text-foreground">{row.value}</span>
            </p>
          ))}
          {record.kb_hit ? <p className="text-emerald-700">本地知识库命中 · 元典 0 积分</p> : null}
          {record.credits_used > 0 ? <p className="text-amber-700">元典实际消耗 {record.credits_used} 积分</p> : null}
          {laws.length > 0 ? <p className="text-muted-foreground/80">真实命中 {laws.length} 条法规记录</p> : null}
          {laws.map((item) => (
            <div key={`${item.reference}-${item.excerpt ?? ""}`} className="select-text text-foreground">
              <p>命中：{item.reference}</p>
              {item.excerpt ? <blockquote className="mt-0.5 border-l-2 border-border pl-2 text-muted-foreground">{item.excerpt}</blockquote> : null}
            </div>
          ))}
          {links.length > 0 ? <p className="text-muted-foreground/80">真实返回或读取 {links.length} 个网址</p> : null}
          {links.map((item) => (
            <button
              key={item.url}
              type="button"
              aria-label={`打开网页：${item.title}`}
              title={item.url}
              onClick={() => void openUrl(item.url).catch((error) => console.warn("open_url failed", error))}
              className="flex w-full items-start gap-1.5 rounded px-1 py-1 text-left text-brand hover:bg-brand-soft"
            >
              <ExternalLink className="mt-0.5 size-3 shrink-0" />
              <span className="min-w-0 break-all">{item.title}<span className="block text-[9px] text-muted-foreground">{item.url}</span></span>
            </button>
          ))}
          {record.error_short ? <p className="break-words text-destructive">失败原因：{record.error_short}</p> : null}
        </div>
      ) : null}
    </div>
  );
}

export function AiRunTrace({
  status,
  elapsedMs,
  reasoningObserved,
  toolCalls,
  activities = [],
  runtimeHint,
}: {
  status: string;
  elapsedMs: number;
  reasoningObserved: boolean;
  toolCalls: ToolCallRecord[];
  activities?: ChatActivity[];
  runtimeHint?: AgentRuntime | null;
}) {
  const [expanded, setExpanded] = useState(false);
  const running = ["queued", "streaming", "running"].includes(status);
  const runtime = activities.find((activity) => activity.runtime)?.runtime ?? runtimeHint;
  const runtimeLabel = runtime === "pi" ? "Pi" : runtime === "native" ? "原生" : "AI";
  const timeline = useMemo(() => {
    const turns = activities.filter(
      (activity) => activity.phase === "turn" && activity.status === "completed",
    );
    const latestByTool = new Map<string, ChatActivity>();
    for (const activity of activities) {
      if (activity.phase === "tool" && activity.tool) latestByTool.set(activity.tool, activity);
    }
    const tools = activities.filter(
      (activity) => activity.phase === "tool" && activity.status !== "started",
    );
    const activeTools = [...latestByTool.values()].filter(
      (activity) => activity.status === "started",
    );
    return { turns, tools, activeTools };
  }, [activities]);

  const summaryDetails = [
    timeline.turns.length > 0 ? `${timeline.turns.length} 轮` : null,
    Math.max(timeline.tools.length, toolCalls.filter((call) => call.tool !== "__agent_round__").length) > 0
      ? `${Math.max(timeline.tools.length, toolCalls.filter((call) => call.tool !== "__agent_round__").length)} 个工具`
      : null,
  ].filter(Boolean).join(" · ");
  const yuandianCredits = toolCalls.reduce(
    (total, call) => total + Math.max(0, call.credits_used || 0),
    0,
  );

  return (
    <div className="mb-2 border-b border-border/70 pb-2 text-[11px] text-muted-foreground">
      <button
        type="button"
        aria-label={expanded ? "收起执行过程" : "展开执行过程"}
        aria-expanded={expanded}
        onClick={() => setExpanded((value) => !value)}
        className="flex w-full items-center gap-1.5 rounded px-1 py-0.5 text-left hover:bg-muted/60"
      >
        {running ? (
          <Loader2 className="size-3.5 animate-spin text-brand" />
        ) : ["completed", "done"].includes(status) ? (
          <CheckCircle2 className="size-3.5 text-emerald-600" />
        ) : (
          <CircleAlert className="size-3.5 text-amber-600" />
        )}
        <span>
          {runtimeLabel} · {statusLabel(status)} {formatElapsed(elapsedMs)}
          {!running ? <> · 元典 {yuandianCredits} 积分</> : null}
        </span>
        {summaryDetails ? <span className="text-[10px] text-muted-foreground/70">· {summaryDetails}</span> : null}
        {expanded ? <ChevronDown className="ml-auto size-3.5" /> : <ChevronRight className="ml-auto size-3.5" />}
      </button>
      {running ? <span className="sr-only">AI 正在持续处理，可随时点击停止</span> : null}

      {expanded ? (
        <div className="mt-2 space-y-1.5 pl-1">
          {timeline.turns.map((activity) => (
            <div key={`turn-${activity.sequence}`} className="flex items-center gap-2">
              <CheckCircle2 className="size-3.5 text-emerald-600" />
              <span>第 {activity.turn ?? timeline.turns.indexOf(activity) + 1} 轮分析</span>
              <span className="ml-auto text-[10px]">{formatElapsed(activity.elapsed_ms ?? 0)}</span>
            </div>
          ))}
          {toolCalls.filter((record) => record.tool !== "__agent_round__").length === 0 ? timeline.tools.map((activity) => (
            <div key={`tool-${activity.sequence}`} className="flex items-center gap-2">
              {activity.status === "completed" ? <CheckCircle2 className="size-3.5 text-emerald-600" /> : <CircleAlert className="size-3.5 text-destructive" />}
              <span className={cn(activity.status !== "completed" && "text-destructive")}>{toolLabel(activity.tool ?? "")}</span>
              <span className="ml-auto text-[10px]">{formatElapsed(activity.elapsed_ms ?? 0)}</span>
            </div>
          )) : null}
          {activities.length === 0 && reasoningObserved ? (
            <div className="flex items-center gap-2"><Search className="size-3.5 text-brand" /><span>分析任务</span></div>
          ) : null}
          {toolCalls.map((record, index) => (
            <ToolAuditRow key={`${record.tool}-${record.started_at_ms}-${index}`} record={record} index={index} />
          ))}
          {timeline.activeTools.map((activity) => (
            <div key={`active-${activity.sequence}`} className="flex items-center gap-2">
              <Loader2 className="size-3.5 animate-spin text-brand" />
              <span>{toolLabel(activity.tool ?? "")}</span>
              <span className="ml-auto text-[10px]">进行中</span>
            </div>
          ))}
          {running && timeline.activeTools.length === 0 ? (
            <div className="flex items-center gap-2"><Loader2 className="size-3.5 animate-spin text-brand" /><span>{timeline.tools.length > 0 || toolCalls.some((call) => call.tool !== "__agent_round__") ? "正在依据检索结果继续分析" : "正在继续分析"}</span><span className="ml-auto text-[10px]">进行中</span></div>
          ) : null}
          {!running && ["completed", "done"].includes(status) ? (
            <div className="flex items-center gap-2"><CheckCircle2 className="size-3.5 text-emerald-600" /><span>整理并生成结果</span></div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

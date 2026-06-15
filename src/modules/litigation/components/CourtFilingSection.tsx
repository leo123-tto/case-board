/**
 * 2026-06-15 · 法院一张网在线立案区块(挂在案件详情页)。
 *
 * 案件信息预填 → 立案类型 → 律师选择 → spawn CLI → 进度事件 → 验证码弹窗。
 */
import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { AlertCircle, ChevronDown, ChevronUp, FolderOpen, Play } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  startCourtFiling,
  listCourtFilingJobs,
  submitCaptchaAnswer,
  listLawyerProfiles,
  revealInFinder,
} from "@/lib/api";
import type {
  CourtFilingJob,
  CourtFilingProgress,
  CourtFilingCaptcha,
  LawyerProfile,
  Case,
} from "@/lib/types";

// 阶段名 → 中文映射
const STAGE_LABELS: Record<string, string> = {
  "cli.started": "启动",
  "cli.info": "信息",
  "cli.params": "参数",
  "filing.start": "开始立案",
  "materials.preflight": "材料预检",
  "filing.success": "✅ 到达预览页（未提交）",
  "filing.failed": "❌ 立案失败",
  "login.start": "登录一张网…",
  "login.success": "登录成功",
  "login.failed": "❌ 登录失败",
  "playwright.start": "进入浏览器立案流程",
  "playwright.step.open_case_type": "打开案件类型页",
  "playwright.step.select_court": "选择受理法院",
  "playwright.step.read_notice": "阅读立案须知",
  "playwright.step.select_cause": "选择案由",
  "playwright.step.upload_materials": "上传诉讼材料",
  "playwright.step.fill_case_info": "完善当事人信息",
  "playwright.step.next": "进入预览页",
  "playwright.step.select_execution_basis": "填写执行依据",
  "playwright.step.fill_execution_target": "填写执行标的",
  "playwright.success": "✅ 到达预览页（未提交）",
  "playwright.failed": "❌ 浏览器立案失败",
  "captcha.required": "⏳ 等待输入验证码",
  "captcha.answered": "验证码已输入",
  "captcha.timeout": "⏰ 验证码等待超时",
  "captcha.degrading": "自动识别失败，降级人工",
  "http.start": "HTTP主链路开始",
  "http.success": "HTTP主链路成功",
  "http.failed": "HTTP主链路失败",
  "cli.spawn_failed": "❌ CLI 启动失败",
  "cli.done": "办理结果",
};

function stageLabel(stage: string): string {
  return STAGE_LABELS[stage] || stage;
}

function looksTechnical(text?: string | null): boolean {
  if (!text) return false;
  return /Locator|Timeout|Call log|playwright|\.uni-|CLI|Traceback|selector|stderr/i.test(text);
}

function parseProgress(progressJson: string | null) {
  if (!progressJson) return null;
  try {
    return JSON.parse(progressJson);
  } catch {
    return null;
  }
}

function jobStatusLabel(j: CourtFilingJob) {
  if (j.status === "completed") return "已到预览页";
  if (j.status === "failed") return "失败";
  if (j.status === "running") return "进行中";
  if (j.status === "waiting_captcha") return "等验证码";
  if (j.status === "pending") return "准备中";
  return j.status;
}

// 验证码弹窗
function CaptchaModal({
  captcha,
  onSubmit,
  onClose,
}: {
  captcha: CourtFilingCaptcha;
  onSubmit: (answer: string) => void;
  onClose: () => void;
}) {
  const [answer, setAnswer] = useState("");
  const [countdown, setCountdown] = useState(captcha.timeout_sec);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    setCountdown(captcha.timeout_sec);
    timerRef.current = setInterval(() => {
      setCountdown((c) => {
        if (c <= 1) {
          if (timerRef.current) clearInterval(timerRef.current);
          onClose();
          return 0;
        }
        return c - 1;
      });
    }, 1000);
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [captcha]);

  function handleSubmit() {
    if (answer.trim()) {
      onSubmit(answer.trim());
      if (timerRef.current) clearInterval(timerRef.current);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="w-80 rounded-lg bg-background p-4 shadow-lg space-y-3">
        <p className="text-sm font-medium">请输入验证码（第 {captcha.round} 轮）</p>
        <img
          src={captcha.image_base64}
          alt="验证码"
          className="mx-auto rounded border"
          style={{ maxHeight: 80 }}
        />
        <input
          className="w-full rounded border border-input bg-background px-2 py-1 text-sm"
          placeholder="输入验证码"
          value={answer}
          onChange={(e) => setAnswer(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleSubmit()}
          autoFocus
        />
        <div className="flex items-center justify-between">
          <span className="text-xs text-muted-foreground">
            {countdown > 0 ? `${countdown}s` : "已超时"}
          </span>
          <div className="flex gap-2">
            <Button type="button" size="sm" variant="outline" onClick={onClose}>
              取消
            </Button>
            <Button type="button" size="sm" onClick={handleSubmit} disabled={!answer.trim()}>
              提交
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}

const REQUIRED_MATERIALS = {
  civil: ["起诉状", "主体资格材料", "证据目录及证据材料", "授权委托手续（如有代理）", "送达地址确认书"],
  execution: ["执行申请书", "执行依据文书", "申请人主体资格材料", "授权委托手续（如有代理）", "送达地址确认书"],
} as const;

function FilingPrepModal({
  caseData,
  filingType,
  selectedFolder,
  onPickFolder,
  onCancel,
  onConfirm,
}: {
  caseData: Case;
  filingType: "civil" | "execution";
  selectedFolder: string;
  onPickFolder: () => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const courtName = caseData.agg_court || caseData.court || "未填写";
  const cause = caseData.agg_cause || caseData.cause || "未填写";
  const ready = Boolean(selectedFolder);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/45 p-4">
      <div className="w-full max-w-xl rounded-lg bg-background p-4 shadow-lg">
        <div className="space-y-3">
          <div>
            <p className="text-sm font-semibold">开始立案前，请先确认</p>
            <p className="mt-1 text-xs text-muted-foreground">
              只会上传你选择的材料文件夹里的 PDF。法院或材料不对，就不要开始。
            </p>
          </div>

          <div className="rounded border border-border p-3 text-xs">
            <p>受理法院：<span className="font-medium">{courtName}</span></p>
            <p className="mt-1">案由：{cause}</p>
            <p className="mt-1 text-amber-700">如果法院不对，请先回案件档案里改法院，再开始立案。</p>
          </div>

          <div className="rounded border border-border p-3">
            <p className="text-xs font-medium">请把这些 PDF 放进同一个文件夹</p>
            <div className="mt-2 grid grid-cols-1 gap-1 text-xs text-muted-foreground sm:grid-cols-2">
              {REQUIRED_MATERIALS[filingType].map((item) => (
                <span key={item}>· {item}</span>
              ))}
            </div>
          </div>

          <div className="rounded border border-border p-3">
            <div className="flex items-center justify-between gap-2">
              <div className="min-w-0 text-xs">
                <p className="font-medium">材料文件夹</p>
                <p className={`mt-1 truncate ${selectedFolder ? "text-muted-foreground" : "text-red-600"}`}>
                  {selectedFolder || "还没有选择，不能开始"}
                </p>
              </div>
              <Button type="button" size="sm" variant="outline" onClick={onPickFolder}>
                <FolderOpen className="size-4" />
                选择
              </Button>
            </div>
          </div>

          <div className="flex justify-end gap-2">
            <Button type="button" size="sm" variant="outline" onClick={onCancel}>
              取消
            </Button>
            <Button type="button" size="sm" onClick={onConfirm} disabled={!ready}>
              确认开始
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}

export function CourtFilingSection({ caseData }: { caseData: Case }) {
  const caseId = caseData.id;
  const [filingType, setFilingType] = useState<"civil" | "execution">("civil");
  const [originalCaseNo, setOriginalCaseNo] = useState("");
  const [selectedAgents, setSelectedAgents] = useState<string[]>([]);
  const [materialFolder, setMaterialFolder] = useState("");
  const [prepOpen, setPrepOpen] = useState(false);
  const [lawyers, setLawyers] = useState<LawyerProfile[]>([]);
  const [jobs, setJobs] = useState<CourtFilingJob[]>([]);
  const [running, setRunning] = useState(false);
  const [progressMsg, setProgressMsg] = useState("");
  const [progressStage, setProgressStage] = useState("");
  const [progressDetail, setProgressDetail] = useState("");
  const [captchaModal, setCaptchaModal] = useState<CourtFilingCaptcha | null>(null);
  const [expandedJobId, setExpandedJobId] = useState<string | null>(null);

  async function refresh() {
    try {
      setJobs(await listCourtFilingJobs(caseId));
      const profiles = await listLawyerProfiles();
      setLawyers(profiles);
      // 默认选中 is_default 的律师
      if (selectedAgents.length === 0) {
        const defaults = profiles.filter((p) => p.is_default).map((p) => p.id);
        if (defaults.length > 0) setSelectedAgents(defaults);
      }
    } catch {
      /* 首次可能还没建表 */
    }
  }

  useEffect(() => {
    refresh();
    // 监听进度事件
    const unlistenProgress = listen<CourtFilingProgress>("court-filing-progress", (e) => {
      const p = e.payload;
      if (p.case_id !== caseId) return;
      const failed = p.stage.includes("failed") || p.level === "error";
      setProgressMsg(failed && looksTechnical(p.message) ? "正在整理失败原因…" : p.message);
      setProgressStage(p.stage);
      setProgressDetail(looksTechnical(p.detail) ? "" : p.detail || "");
      if (p.stage === "filing.success" || p.stage === "playwright.success") {
        setRunning(false);
        refresh();
      }
      if (p.stage === "filing.failed" || p.stage === "playwright.failed" || p.stage === "login.failed") {
        setRunning(false);
        refresh();
      }
    });
    // 监听验证码事件
    const unlistenCaptcha = listen<CourtFilingCaptcha>("court-filing-captcha", (e) => {
      const c = e.payload;
      if (c.case_id !== caseId) return;
      setCaptchaModal(c);
    });
    return () => {
      unlistenProgress.then((fn) => fn());
      unlistenCaptcha.then((fn) => fn());
    };
  }, [caseId]);

  async function pickMaterialFolder() {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "选择本次立案材料文件夹",
      defaultPath: caseData.source_folder || undefined,
    });
    if (typeof selected === "string") {
      setMaterialFolder(selected);
    }
  }

  async function handleStart() {
    if (selectedAgents.length === 0) {
      setProgressMsg("请先选择代理律师（设置→律师档案）");
      return;
    }
    setPrepOpen(true);
  }

  async function runStartAfterPrep() {
    if (!materialFolder) {
      setProgressMsg("请先选择本次立案材料文件夹");
      setProgressStage("materials.preflight");
      return;
    }
    setPrepOpen(false);
    setRunning(true);
    setProgressMsg("已提交，正在启动…");
    setProgressStage("cli.started");
    setProgressDetail("");
    try {
      await startCourtFiling(
        caseId,
        filingType,
        selectedAgents,
        filingType === "execution" ? originalCaseNo || undefined : undefined,
        materialFolder,
      );
    } catch (e) {
      setRunning(false);
      setProgressMsg("启动失败: " + String(e));
      setProgressDetail("请按提示补齐信息后再开始。");
    }
  }

  async function handleCaptchaSubmit(answer: string) {
    if (!captchaModal) return;
    try {
      await submitCaptchaAnswer(
        captchaModal.job_id,
        captchaModal.task_id,
        captchaModal.round,
        answer,
      );
      setCaptchaModal(null);
      setProgressMsg("验证码已提交，继续立案…");
      setProgressDetail("");
    } catch (e) {
      setProgressMsg("提交验证码失败: " + String(e));
      setProgressDetail("");
    }
  }

  function toggleAgent(id: string) {
    setSelectedAgents((prev) =>
      prev.includes(id) ? prev.filter((a) => a !== id) : [...prev, id],
    );
  }

  return (
    <div className="space-y-2">
      {/* 立案类型选择 */}
      <div className="flex items-center gap-4">
        <span className="text-xs text-muted-foreground">立案类型</span>
        {(["civil", "execution"] as const).map((t) => (
          <label key={t} className="flex items-center gap-1 text-xs">
            <input type="radio" checked={filingType === t} onChange={() => setFilingType(t)} />
            {t === "civil" ? "民事一审" : "申请执行"}
          </label>
        ))}
      </div>
      {filingType === "execution" && (
        <input
          className="w-full rounded border border-input bg-background px-2 py-1 text-sm"
          placeholder="执行依据案号（必填，如 (2024)粤01民初123号）"
          value={originalCaseNo}
          onChange={(e) => setOriginalCaseNo(e.target.value)}
        />
      )}

      {/* 律师选择 */}
      {lawyers.length > 0 && (
        <div className="space-y-1">
          <span className="text-xs text-muted-foreground">代理律师</span>
          <div className="flex flex-wrap gap-2">
            {lawyers.map((l) => (
              <label
                key={l.id}
                className={`flex items-center gap-1 rounded border px-2 py-0.5 text-xs cursor-pointer ${
                  selectedAgents.includes(l.id) ? "border-primary bg-primary/5" : "border-border"
                }`}
              >
                <input
                  type="checkbox"
                  checked={selectedAgents.includes(l.id)}
                  onChange={() => toggleAgent(l.id)}
                />
                {l.name}
                {l.law_firm ? ` · ${l.law_firm}` : ""}
                {l.is_default ? " ⭐" : ""}
              </label>
            ))}
          </div>
        </div>
      )}
      {lawyers.length === 0 && (
        <p className="text-xs text-amber-600">
          未配置律师档案，请先在「设置→律师档案」中添加。
        </p>
      )}

      {/* 开始按钮 */}
      <Button type="button" size="sm" onClick={handleStart} disabled={running || selectedAgents.length === 0}>
        <Play className="size-4" />
        {running ? "立案中…" : "开始立案"}
      </Button>
      {materialFolder ? (
        <p className="truncate text-xs text-muted-foreground">本次材料文件夹：{materialFolder}</p>
      ) : null}

      {/* 进度展示 */}
      {progressMsg && (
        <div
          className={`rounded border px-3 py-2 text-xs ${
            progressStage.includes("failed") || progressStage.includes("error")
              ? "border-red-200 bg-red-50 text-red-700"
              : progressStage.includes("success")
              ? "border-green-200 bg-green-50 text-green-700"
              : progressStage.includes("captcha") || progressStage.includes("preflight")
              ? "border-amber-200 bg-amber-50 text-amber-700"
              : "border-blue-200 bg-blue-50 text-blue-700"
          }`}
        >
          <div className="flex items-center gap-2">
            {progressStage.includes("failed") ? <AlertCircle className="size-4" /> : null}
            <span className="font-medium">{stageLabel(progressStage)}</span>
            <span>{progressMsg}</span>
          </div>
          {progressDetail ? (
            <p className="mt-1 text-[11px] opacity-80">{progressDetail}</p>
          ) : null}
        </div>
      )}

      {/* 历史任务列表 */}
      {jobs.length > 0 && (
        <div className="space-y-1">
          <p className="text-xs font-medium text-muted-foreground">立案记录</p>
          {jobs.map((j) => {
            const parsed = parseProgress(j.progress_json);
            const isExpanded = expandedJobId === j.id;
            const fallbackText = looksTechnical(parsed?.detail) || looksTechnical(parsed?.message)
              ? ""
              : parsed?.detail || parsed?.message || "";
            const errorText = j.error || fallbackText;
            return (
            <div
              key={j.id}
              className="rounded border border-border px-2 py-1.5 text-xs"
            >
              <div className="flex items-center justify-between gap-2">
                <span className="min-w-0 truncate">
                  {j.filing_type === "civil" ? "民事" : "执行"} · {j.court_name || "—"} ·{" "}
                  <span
                    className={
                      j.status === "failed"
                        ? "text-red-600"
                        : j.status === "completed"
                        ? "text-green-600"
                        : "text-muted-foreground"
                    }
                  >
                    {jobStatusLabel(j)}
                  </span>
                  {j.timing_json ? (() => {
                    try {
                      const t = JSON.parse(j.timing_json);
                      const secs = t.overall_ms ? Math.round(t.overall_ms / 1000) : 0;
                      return secs > 0 ? ` · ${secs}秒` : null;
                    } catch { return null; }
                  })() : null}
                </span>
                <div className="flex shrink-0 items-center gap-1">
                  {j.output_dir ? (
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      className="h-7 px-2 text-xs"
                      title="打开诊断目录"
                      onClick={() => revealInFinder(j.output_dir!)}
                    >
                      <FolderOpen className="size-3.5" />
                      诊断
                    </Button>
                  ) : null}
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    className="h-7 px-2 text-xs"
                    title={isExpanded ? "收起详情" : "查看详情"}
                    onClick={() => setExpandedJobId(isExpanded ? null : j.id)}
                  >
                    {isExpanded ? <ChevronUp className="size-3.5" /> : <ChevronDown className="size-3.5" />}
                    详情
                  </Button>
                </div>
              </div>
              {j.status === "failed" && errorText ? (
                <p className="mt-1 truncate text-red-600" title={errorText}>
                  {errorText}
                </p>
              ) : null}
              {isExpanded ? (
                <div className="mt-2 space-y-1 rounded bg-muted/40 p-2 text-[11px] text-muted-foreground">
                  <p>最近阶段：{parsed?.stage ? stageLabel(parsed.stage) : "—"}</p>
                  <p>最近提示：{looksTechnical(parsed?.message) ? "已记录到诊断文件" : parsed?.message || "—"}</p>
                  {j.error ? <p className="text-red-600">失败归因：{j.error}</p> : null}
                  {j.output_dir ? (
                    <p className="break-all">
                      诊断目录：{j.output_dir}（含 material_preflight.json、progress_events.jsonl、stderr.log、final_diagnosis.json）
                    </p>
                  ) : null}
                </div>
              ) : null}
            </div>
          );})}
        </div>
      )}

      {/* 验证码弹窗 */}
      {captchaModal && (
        <CaptchaModal
          captcha={captchaModal}
          onSubmit={handleCaptchaSubmit}
          onClose={() => setCaptchaModal(null)}
        />
      )}
      {prepOpen && (
        <FilingPrepModal
          caseData={caseData}
          filingType={filingType}
          selectedFolder={materialFolder}
          onPickFolder={() => void pickMaterialFolder()}
          onCancel={() => setPrepOpen(false)}
          onConfirm={() => void runStartAfterPrep()}
        />
      )}
    </div>
  );
}

/**
 * 执行案件详情视图(2026-05-24 j-6)。
 *
 * 跟诉讼模块的 CaseView 完全分开 — **只显示执行相关字段**,审判阶段的当事人列表 /
 * 起诉状内容 / 审判流程 等都不显示,让律师专注在执行管理上。
 *
 * 数据全部来自 LLM 全局抽出的 cases.agg_*(JSON 字段)+ case_report_path。
 * V0.2 会接元典 API 查财产线索 + 跟「利息执行款」工具联动。
 */

import {
  ArrowLeft,
  BookOpen,
  Calculator,
  FileText,
  Gavel,
  Phone,
  Plus,
  Search,
  AlertTriangle,
  Microscope,
  Trash2,
} from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { MarkdownModal } from "@/components/MarkdownModal";
import { SourceDocumentViewerDrawer } from "@/components/SourceDocumentViewerDrawer";
import { formatYuan } from "@/lib/format";
import { normalizeCaseStatusText } from "@/lib/caseSnapshot";
import { todayIsoLocal } from "@/lib/date";
import { confirmDialog } from "@/lib/dialog";
import {
  type Case,
  type CourtContact,
  type Document,
  type PartyContact,
  parseJsonArray,
} from "@/lib/types";
import {
  type DigHint,
  type Payment,
  type YuandianP1Response,
  addPayment,
  deletePayment,
  getCaseWithDocs,
  getSettings,
  globalExtractCase,
  listPayments,
  openUrl,
  yuandianBasicQuery,
  yuandianDeepDive,
  yuandianFullReport,
} from "@/lib/api";
import { getFieldOverride, parseOverrides } from "@/lib/userOverrides";
import { useFeatureFlag } from "@/lib/featureFlags";
import { TodosCard } from "@/components/TodosCard";
import { useRunningTask } from "@/contexts/RunningTaskContext";
import type { InterestPrefill } from "@/modules/tools/calculators/InterestCalculator";
import { CaseWorkLogSection } from "@/modules/litigation/components/CaseWorkLogSection";

export function ExecutionDetailView({
  caseData,
  documents,
  onBack,
  onCalculateInterest,
}: {
  caseData: Case;
  documents?: Document[];
  onBack: () => void;
  onCalculateInterest?: (prefill: InterestPrefill) => void;
}) {
  const [current, setCurrent] = useState<Case>(caseData);
  const [sourceDocs, setSourceDocs] = useState<Document[]>(documents ?? []);
  const [reportOpen, setReportOpen] = useState(false);
  // 元典 P1 状态
  const [yuandianResult, setYuandianResult] = useState<YuandianP1Response | null>(null);
  const [riskOpen, setRiskOpen] = useState(false);
  // P2 深挖状态
  const [deepDiveOpen, setDeepDiveOpen] = useState(false);
  // 完整报告(V0.1.7 合并风险 + 深挖)
  const [fullReportOpen, setFullReportOpen] = useState(false);
  // 还款记录
  const [payments, setPayments] = useState<Payment[]>([]);
  const [paymentSourceOpen, setPaymentSourceOpen] = useState<Document | null>(null);
  const [showTodos] = useFeatureFlag("case_todos");
  const [showWorkLogs] = useFeatureFlag("case_work_logs");

  // 2026-05-25 V0.1.7 · 全局任务锁,所有长任务走这条
  const { task, runWithLock } = useRunningTask();
  const isLocked = task !== null;

  useEffect(() => {
    setCurrent(caseData);
  }, [caseData]);

  useEffect(() => {
    setSourceDocs(documents ?? []);
  }, [documents]);

  const refreshPayments = async () => {
    const p = await listPayments(caseData.id);
    setPayments(p);
    return p;
  };

  useEffect(() => {
    let cancelled = false;
    listPayments(caseData.id)
      .then((p) => {
        if (!cancelled) setPayments(p);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [caseData.id]);

  const totalPaid = payments.reduce((s, p) => s + p.amount, 0);
  const remaining =
    current.agg_claim_amount != null ? Math.max(0, current.agg_claim_amount - totalPaid) : null;
  const statusText = normalizeCaseStatusText(current.agg_status_text);

  const handleAddPayment = async (input: {
    amount: number;
    paid_at: string;
    note: string | null;
  }) => {
    try {
      const p = await addPayment({
        case_id: current.id,
        amount: input.amount,
        paid_at: input.paid_at,
        note: input.note,
      });
      setPayments((prev) => [p, ...prev].sort((a, b) => (a.paid_at < b.paid_at ? 1 : -1)));
    } catch (e) {
      alert(`录入失败:${e}`);
    }
  };

  const handleDeletePayment = async (id: string) => {
    if (
      !(await confirmDialog("确认删除这条还款记录?", {
        danger: true,
        okLabel: "删除",
      }))
    )
      return;
    try {
      await deletePayment(id);
      setPayments((prev) => prev.filter((p) => p.id !== id));
    } catch (e) {
      alert(`删除失败:${e}`);
    }
  };

  useEffect(() => {
    setCurrent(caseData);
  }, [caseData]);

  // 解析 LLM JSON 字段
  const defendants = parseJsonArray(current.agg_defendants);
  const plaintiffs = parseJsonArray(current.agg_plaintiffs);
  const partyContacts = safeJson<PartyContact[]>(current.agg_party_contacts, []);
  const courtContacts = safeJson<CourtContact[]>(current.agg_court_contacts, []);
  const keyDates = safeJson<{ date: string; event: string; note?: string }[]>(
    current.agg_key_dates,
    [],
  );

  // 执行节点:只挑跟执行 / 还款 / 保全相关的日期(已抽到的)
  const executionDates = keyDates
    .filter((d) => d.date && d.event)
    .filter((d) => /执行|还款|付款|保全|续封|查封|查询|强制/.test(d.event))
    .sort((a, b) => (a.date < b.date ? -1 : 1));

  // Phase 2:执行立场。我方=被告方 → 债务人模式(查我方客户做暴露面自查,标签转防御)。
  // 用户在详情页确认的立场(user_overrides_json)优先,否则用 LLM 抽的 agg_our_side。
  const ovOurSide = getFieldOverride(
    parseOverrides(current.user_overrides_json),
    "agg_our_side",
  );
  const effectiveOurSide =
    ovOurSide !== undefined ? ovOurSide : current.agg_our_side;
  const isDebtor = effectiveOurSide === "被告方";

  // 查询对象联系人:债权人模式取对方(被执行人);债务人模式取我方客户本人(排代理人)。
  const targetContacts = partyContacts.filter((p) =>
    isDebtor
      ? p.is_our_side === true && !/代理/.test(p.role ?? "")
      : p.is_our_side === false || /被告|被执行|被申请/.test(p.role ?? ""),
  );
  // 标签随立场切换
  const queryActionLabel = isDebtor ? "执行风险自查" : "查被执行人";
  const targetCardTitle = isDebtor ? "我方当事人(被执行人)信息" : "被执行人信息";

  // 2026-05-25 V0.1.7 · 防呆:已查过 X 天的报告再查时,弹确认
  async function confirmRerunIfRecent(
    lastAt: string | null,
    label: string,
  ): Promise<boolean> {
    if (!lastAt) return true;
    const daysAgo = Math.floor(
      (Date.now() - new Date(lastAt).getTime()) / 86400000,
    );
    if (daysAgo >= 30) return true;
    return confirmDialog(
      `${label}已在 ${daysAgo} 天前查询过。元典数据每月更新一次,建议间隔 30 天再查以节省 API 调用次数。仍要重新查询?`,
      { okLabel: "仍要查询" },
    );
  }

  // 2026-05-25 V0.1.8 · 防呆:没填元典 API key 前不能用执行报告类功能
  // 触发条件:点「查被执行人 / 深挖 / 完整报告」前先调一次
  // 返回 true = 可以继续;false = 已弹对话框引导用户去注册,中止操作
  async function ensureYuandianKey(): Promise<boolean> {
    try {
      const settings = await getSettings();
      if (settings.yuandian_api_key?.trim()) return true;
    } catch (e) {
      alert(`读取设置失败:${e}`);
      return false;
    }
    const wantOpen = await confirmDialog(
      "⚠ 未配置元典 API key。「查被执行人」/「深挖」/「完整报告」需要元典法律开放平台的 API key," +
        "工具不内置任何人的 key,你需要自己申请(免费)。点「打开申请页」后,在元典「个人中心」申请 API key,再到「设置 → 元典法律开放平台」填入并点「验证」。",
      { okLabel: "打开申请页" },
    );
    if (wantOpen) {
      try {
        await openUrl("https://open.chineselaw.com/profile");
      } catch (e) {
        alert(`打开浏览器失败:${e}\n\n请手动访问 https://open.chineselaw.com/profile`);
      }
    }
    return false;
  }

  // 立场未确认时的确认闸(Phase 2 / advisor 命门:避免默认债权人模式去查自己客户)。
  // effectiveOurSide 为空(LLM 没抽出、用户也没确认)→ 后端默认按债权人处理;
  // 若我方其实代理被执行人,会变成"查自己客户财产" → 先弹确认让律师去确认立场。
  async function confirmStanceIfUnknown(): Promise<boolean> {
    if (effectiveOurSide) return true; // 已知立场(原告方/被告方/...)直接放行
    return confirmDialog(
      "尚未确认「我方代理立场」。执行查询将默认按**债权人(申请执行人)**处理 —— 查对方(被执行人)财产。\n\n" +
        "⚠️ 如果你代理的是**被执行人(债务人)**,请先点「返回 → 案件详情 → 编辑」确认立场为「被告方」,否则会查到自己客户头上。\n\n是否仍按债权人继续?",
      { okLabel: "按债权人继续" },
    );
  }

  // 「🔍 查被执行人」按钮:跑元典 P1(可能 30-90 秒,聚合优先约 5-8 端点 + LLM 写报告)
  const handleYuandianQuery = async () => {
    if (!(await ensureYuandianKey())) return;
    if (!(await confirmStanceIfUnknown())) return;
    if (!(await confirmRerunIfRecent(current.risk_assessment_at, "风险报告")))
      return;
    await runWithLock(
      {
        kind: "yuandian_basic",
        caseId: current.id,
        caseName: current.name,
        label: "正在查被执行人 · 元典聚合查询 + LLM 风险报告,预计 30-90 秒",
      },
      async () => {
        try {
          const r = await yuandianBasicQuery(current.id);
          setYuandianResult(r);
          const fresh = await getCaseWithDocs(current.id);
          setCurrent(fresh.case);
          setSourceDocs(fresh.documents);
          if (fresh.case.risk_assessment_path) {
            setRiskOpen(true);
          } else if (r.assessment.error) {
            alert(`报告生成失败:${r.assessment.error}`);
          }
        } catch (e) {
          alert(`查询失败:${e}`);
        }
      },
    );
  };

  // P2 深挖按钮
  const handleDeepDive = async () => {
    if (!(await ensureYuandianKey())) return;
    if (!(await confirmRerunIfRecent(current.deep_dive_at, "深挖报告"))) return;
    await runWithLock(
      {
        kind: "yuandian_deep_dive",
        caseId: current.id,
        caseName: current.name,
        label:
          "正在 P2 深挖 · 拉关联公司/案号/第三方 + LLM 深查报告,预计 60-180 秒",
      },
      async () => {
        try {
          const r = await yuandianDeepDive(current.id);
          if (r.error) {
            alert(`深挖失败:${r.error}`);
            return;
          }
          const fresh = await getCaseWithDocs(current.id);
          setCurrent(fresh.case);
          setSourceDocs(fresh.documents);
          if (fresh.case.deep_dive_report_path) {
            setDeepDiveOpen(true);
          }
        } catch (e) {
          alert(`深挖失败:${e}`);
        }
      },
    );
  };

  // 2026-05-25 V0.1.7 · 完整报告(合并风险报告 + 深挖报告 → 第三份)
  const handleFullReport = async () => {
    // 已有 → 直接弹查看(已生成的报告不需要 key 即可查看)
    if (current.full_report_path) {
      setFullReportOpen(true);
      return;
    }
    // 没生成过 → 需要调 DeepSeek 合并,虽然不直接调元典,但前置两份报告
    // 已经依赖元典数据,这里再防呆一次更稳
    if (!(await ensureYuandianKey())) return;
    // 前置:必须先有两份报告
    if (!current.risk_assessment_path) {
      alert("请先点「查被执行人」生成风险报告");
      return;
    }
    if (!current.deep_dive_report_path) {
      alert("请先点「深挖」生成深挖报告");
      return;
    }
    await runWithLock(
      {
        kind: "yuandian_full_report",
        caseId: current.id,
        caseName: current.name,
        label: "正在合并完整报告 · DeepSeek 综合两份报告,预计 30-60 秒",
      },
      async () => {
        try {
          const r = await yuandianFullReport(current.id);
          if (r.error) {
            alert(`完整报告生成失败:${r.error}`);
            return;
          }
          const fresh = await getCaseWithDocs(current.id);
          setCurrent(fresh.case);
          setSourceDocs(fresh.documents);
          if (fresh.case.full_report_path) {
            setFullReportOpen(true);
          }
        } catch (e) {
          alert(`完整报告生成失败:${e}`);
        }
      },
    );
  };

  const handleOpenReport = async () => {
    if (current.case_report_path) {
      setReportOpen(true);
      return;
    }
    await runWithLock(
      {
        kind: "global_extract",
        caseId: current.id,
        caseName: current.name,
        label: "正在生成案件分析报告 · LLM 通读所有文档,预计 10-30 秒",
      },
      async () => {
        try {
          const r = await globalExtractCase(current.id);
          if (r.error) {
            alert(`报告生成失败:${r.error}`);
            return;
          }
          const fresh = await getCaseWithDocs(current.id);
          setCurrent(fresh.case);
          setSourceDocs(fresh.documents);
          await refreshPayments().catch(() => {});
          if (fresh.case.case_report_path) setReportOpen(true);
        } catch (e) {
          alert(`报告生成失败:${e}`);
        }
      },
    );
  };

  return (
    <main className="flex h-full w-full flex-col bg-background">
      {/* 头部 */}
      <header className="border-b border-border bg-card/50 px-8 py-5">
        <div className="mx-auto flex max-w-6xl items-start gap-4">
          <button
            type="button"
            onClick={onBack}
            className="mt-1 inline-flex items-center gap-1.5 rounded-md border border-border bg-card px-2.5 py-1.5 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" />
            返回执行列表
          </button>
          <div className="flex-1 min-w-0">
            <div className="flex flex-wrap items-baseline gap-3">
              <Gavel className="size-4 text-muted-foreground" />
              <h1 className="text-lg font-semibold text-foreground">
                {current.name}
              </h1>
              {current.agg_case_no && (
                <span className="rounded bg-muted px-2 py-0.5 font-mono text-xs text-muted-foreground">
                  {current.agg_case_no}
                </span>
              )}
              <span className="rounded bg-foreground px-2 py-0.5 text-xs text-background">
                执行中
              </span>
            </div>
            {current.case_summary && (
              <p className="mt-2 text-sm text-muted-foreground">
                {current.case_summary}
              </p>
            )}
            {statusText && (
              <p className="mt-1 text-xs text-muted-foreground/80">
                {statusText}
              </p>
            )}
          </div>
          <div className="flex shrink-0 flex-col items-end gap-2">
            {/* 「查被执行人」+「查看执行报告」一行 */}
            <div className="flex items-center gap-2">
              {current.risk_assessment_path && (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => setRiskOpen(true)}
                  title="查看上次生成的风险提示报告(MD)"
                >
                  <BookOpen className="size-3.5" />
                  查看执行报告
                </Button>
              )}
              <Button
                size="sm"
                onClick={handleYuandianQuery}
                disabled={isLocked}
                className="bg-amber-700 text-white hover:bg-amber-700/90"
                title={
                  isDebtor
                    ? "我方代理被执行人:查我方客户的执行敞口(查封冻结状态 / 可主张豁免 / 执行异议线索 / 和解策略),元典 + LLM 写防御向报告"
                    : "调元典 API 查被执行人:失信 / 限消 / 关联公司 / 名下财产 / 工商风险 + LLM 写风险提示报告"
                }
              >
                <Search className="size-3.5" />
                {current.risk_assessment_path
                  ? `重新${queryActionLabel}`
                  : queryActionLabel}
              </Button>
            </div>
            <Button
              size="sm"
              onClick={handleOpenReport}
              disabled={isLocked}
              className="bg-foreground text-background hover:bg-foreground/90"
            >
              <BookOpen className="size-3.5" />
              案件报告
            </Button>
          </div>
        </div>
      </header>

      {/* 主区 */}
      <div className="flex-1 overflow-auto px-8 py-8">
        <div className="mx-auto max-w-6xl space-y-6">
          {/* 第一行:执行标的 + 当事人快览 */}
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            <Card title="执行标的 / 申请执行金额">
              {current.agg_claim_amount != null ? (
                <div className="font-mono text-2xl font-semibold text-foreground">
                  {formatYuan(current.agg_claim_amount)}
                </div>
              ) : (
                <div className="text-sm text-muted-foreground/60">—</div>
              )}
              {(remaining != null || current.execution_remaining != null) && (
                <div className="mt-2 space-y-1 text-sm">
                  {totalPaid > 0 && (
                    <div className="text-muted-foreground">
                      已收:
                      <span className="ml-1 font-mono font-medium text-foreground">
                        {formatYuan(totalPaid)}
                      </span>
                      <span className="ml-1 text-xs text-muted-foreground/70">
                        ({payments.length} 笔)
                      </span>
                    </div>
                  )}
                  <div className="text-muted-foreground">
                    剩余:
                    <span className="ml-1 font-mono font-medium text-foreground">
                      {formatYuan(remaining ?? current.execution_remaining ?? 0)}
                    </span>
                  </div>
                </div>
              )}
              {onCalculateInterest &&
                (current.execution_total ?? current.agg_claim_amount) != null && (
                  <Button
                    variant="outline"
                    size="sm"
                    className="mt-3"
                    onClick={() => {
                      // 起算日:优先用判决书 / 调解书签发日(从 key_dates 找),次选立案日
                      const enforceableDate =
                        keyDates.find((d) =>
                          /调解|判决|裁定/.test(d.event ?? ""),
                        )?.date ??
                        current.agg_filed_at ??
                        "";
                      // 2026-06-11:直达执行款 tab,本金/起算日/还款记录全部预填
                      onCalculateInterest({
                        mode: "execution",
                        principal: String(
                          current.execution_total ?? current.agg_claim_amount,
                        ),
                        startDate: enforceableDate,
                        note: `${current.name} · ${current.agg_case_no ?? ""}`,
                        repayments: payments.map((p) => ({
                          date: p.paid_at,
                          amount: p.amount,
                        })),
                      });
                    }}
                    title="跳到「执行款计算」,自动预填执行标的、起算日和已录入的还款记录"
                  >
                    <Calculator className="size-3.5" />
                    算执行款
                  </Button>
                )}
            </Card>

            <Card title="当事人">
              <div className="space-y-1.5 text-sm">
                {plaintiffs.length > 0 && (
                  <Row label="申请执行人" value={plaintiffs.join("、")} />
                )}
                {defendants.length > 0 && (
                  <Row
                    label="被执行人"
                    value={defendants.join("、")}
                    emphasize
                  />
                )}
                {current.agg_cause && (
                  <Row label="案由" value={current.agg_cause} />
                )}
                {current.agg_court && (
                  <Row label="执行法院" value={current.agg_court} />
                )}
              </div>
            </Card>
          </div>

          {/* 调解 / 判决结果 */}
          {current.agg_resolution && (
            <Card title="生效法律文书 · 履行约定">
              <p className="text-sm leading-7 text-foreground/80 whitespace-pre-wrap">
                {current.agg_resolution}
              </p>
            </Card>
          )}

          {/* 查询对象详情(债权人=被执行人;债务人=我方客户) */}
          {targetContacts.length > 0 && (
            <Card title={targetCardTitle}>
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-border text-xs text-muted-foreground">
                      <th className="px-3 py-2 text-left">姓名</th>
                      <th className="px-3 py-2 text-left">身份证号</th>
                      <th className="px-3 py-2 text-left">地址</th>
                      <th className="px-3 py-2 text-left">电话</th>
                    </tr>
                  </thead>
                  <tbody>
                    {targetContacts.map((p, i) => (
                      <tr key={i} className="border-b border-border/50">
                        <td className="px-3 py-2 font-medium">{p.name}</td>
                        <td className="px-3 py-2 font-mono text-xs text-muted-foreground">
                          {(p as PartyContact & { id_no?: string }).id_no ?? "—"}
                        </td>
                        <td className="px-3 py-2 text-xs text-muted-foreground">
                          {(p as PartyContact & { address?: string }).address ?? "—"}
                        </td>
                        <td className="px-3 py-2 font-mono text-xs">
                          {p.phone ?? "—"}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </Card>
          )}

          {/* 执行节点时间轴 */}
          {executionDates.length > 0 && (
            <Card title="执行节点">
              <ol className="relative space-y-3 border-l border-border pl-5">
                {executionDates.map((d, i) => (
                  <li key={i} className="relative">
                    <span className="absolute -left-[26px] mt-1 size-2.5 rounded-full bg-foreground ring-2 ring-card" />
                    <div className="flex flex-wrap items-baseline gap-2">
                      <span className="text-sm font-medium text-foreground">
                        {d.event}
                      </span>
                      <span className="font-mono text-xs text-muted-foreground">
                        {d.date}
                      </span>
                    </div>
                    {d.note && (
                      <p className="mt-0.5 text-xs text-muted-foreground/80">
                        {d.note}
                      </p>
                    )}
                  </li>
                ))}
              </ol>
            </Card>
          )}

          {/* 法院联系 */}
          {courtContacts.length > 0 && (
            <Card title="执行法院联系">
              <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
                {courtContacts.map((c, i) => (
                  <div
                    key={i}
                    className="flex items-center justify-between rounded-md border border-border bg-card/50 px-3 py-2 text-sm"
                  >
                    <div>
                      <span className="font-medium">{c.name}</span>
                      {c.role && (
                        <span className="ml-2 text-xs text-muted-foreground">
                          {c.role}
                        </span>
                      )}
                    </div>
                    {c.phone && (
                      <div className="flex items-center gap-1.5 font-mono text-xs text-muted-foreground">
                        <Phone className="size-3" />
                        {c.phone}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </Card>
          )}

          {/* 风险报告卡 — 跑过元典 P1 之后才显示 */}
          {(current.risk_assessment_path || yuandianResult) && (
            <RiskAssessmentCard
              caseData={current}
              result={yuandianResult}
              onDeepDive={handleDeepDive}
              isLocked={isLocked}
              onOpenDeepDive={() => setDeepDiveOpen(true)}
              onFullReport={handleFullReport}
            />
          )}

          {/* 还款记录 */}
          <PaymentsCard
            payments={payments}
            sourceDocs={sourceDocs}
            onAdd={handleAddPayment}
            onDelete={handleDeletePayment}
            onOpenSource={setPaymentSourceOpen}
          />

          {(showTodos || showWorkLogs) && (
            <div className="space-y-4">
              {showTodos && (
                <Card title="待办清单">
                  <TodosCard caseId={current.id} />
                </Card>
              )}
              {showWorkLogs && <CaseWorkLogSection caseId={current.id} />}
            </div>
          )}
        </div>
      </div>

      {reportOpen && current.case_report_path && (
        <MarkdownModal
          path={current.case_report_path}
          filename={`${current.name} · 案件分析报告.md`}
          badge="LLM 全局抽"
          onClose={() => setReportOpen(false)}
          exportCase={{ id: current.id, name: current.name }}
        />
      )}

      {riskOpen && current.risk_assessment_path && (
        <MarkdownModal
          path={current.risk_assessment_path}
          filename={`${current.name} · 被执行人风险提示报告.md`}
          badge="元典 + LLM"
          onClose={() => setRiskOpen(false)}
          exportMd={{
            mdPath: current.risk_assessment_path,
            title: `${current.name}_风险报告`,
          }}
        />
      )}

      {deepDiveOpen && current.deep_dive_report_path && (
        <MarkdownModal
          path={current.deep_dive_report_path}
          filename={`${current.name} · 深查报告.md`}
          badge="P2 深挖"
          onClose={() => setDeepDiveOpen(false)}
          exportMd={{
            mdPath: current.deep_dive_report_path,
            title: `${current.name}_深挖报告`,
          }}
        />
      )}

      {fullReportOpen && current.full_report_path && (
        <MarkdownModal
          path={current.full_report_path}
          filename={`${current.name} · 完整执行追踪报告.md`}
          badge="风险 + 深挖 合并"
          onClose={() => setFullReportOpen(false)}
          exportMd={{
            mdPath: current.full_report_path,
            title: `${current.name}_完整报告`,
          }}
        />
      )}

      {paymentSourceOpen && (
        <SourceDocumentViewerDrawer
          doc={paymentSourceOpen}
          caseFolder={current.source_folder}
          onClose={() => setPaymentSourceOpen(null)}
        />
      )}
    </main>
  );
}

/* ============ 风险报告卡(元典查询完后显示) ============ */
function RiskAssessmentCard({
  caseData,
  result,
  onDeepDive,
  isLocked,
  onOpenDeepDive,
  onFullReport,
}: {
  caseData: Case;
  result: YuandianP1Response | null;
  onDeepDive: () => void;
  isLocked: boolean;
  onOpenDeepDive: () => void;
  onFullReport: () => void;
}) {
  const digHints = result?.assessment.dig_hints ?? [];
  const subjectCount = result?.orchestrator.subjects.length ?? 0;
  const rawCount = result?.orchestrator.raw_files.length ?? 0;
  const failureCount = result?.orchestrator.failures.length ?? 0;
  // 是否值得显示「🔬 深挖」按钮:必须有 dig_hints(刚跑完 P1)或者已经跑过深挖
  const canDeepDive = digHints.length > 0 || caseData.deep_dive_report_path;
  // 已经有深挖报告就可以触发完整报告(必须同时有 risk + deep_dive 才会显示)
  const canFullReport =
    caseData.risk_assessment_path && caseData.deep_dive_report_path;
  return (
    <section className="rounded-lg border border-amber-700/30 bg-amber-50/40 p-5">
      <div className="mb-3 flex items-start justify-between gap-3">
        <div className="flex items-center gap-2">
          <AlertTriangle className="size-4 text-amber-700" />
          <h3 className="text-sm font-semibold text-foreground">
            被执行人风险提示
          </h3>
          {caseData.risk_assessment_at && (
            <span className="text-xs text-muted-foreground">
              {new Date(caseData.risk_assessment_at).toLocaleString("zh-CN", {
                month: "2-digit",
                day: "2-digit",
                hour: "2-digit",
                minute: "2-digit",
              })}
            </span>
          )}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {/* 2026-05-25 V0.1.8 hotfix:删除「查看风险报告」按钮 — 跟顶部「查看执行报告」是同一份 MD,作者反馈避免重复 */}
          {canDeepDive && (
            <>
              {caseData.deep_dive_report_path && (
                <Button size="sm" variant="outline" onClick={onOpenDeepDive}>
                  <BookOpen className="size-3.5" />
                  查看深挖报告
                </Button>
              )}
              <Button
                size="sm"
                onClick={onDeepDive}
                disabled={isLocked}
                className="bg-amber-700 text-white hover:bg-amber-700/90"
                title="按 LLM 给的深挖建议拉关联公司/案号/第三方主体 → 出深查报告(60-180 秒)"
              >
                <Microscope className="size-3.5" />
                {caseData.deep_dive_report_path ? "重新深挖" : "🔬 深挖"}
              </Button>
            </>
          )}
          {canFullReport && (
            <Button
              size="sm"
              onClick={onFullReport}
              disabled={isLocked}
              className="bg-foreground text-background hover:bg-foreground/90"
              title={
                caseData.full_report_path
                  ? "查看综合完整报告(已生成)"
                  : "把风险报告 + 深挖报告喂给 DeepSeek,出第三份完整报告(30-60 秒)"
              }
            >
              <BookOpen className="size-3.5" />
              {caseData.full_report_path ? "查看完整报告" : "生成完整报告"}
            </Button>
          )}
        </div>
      </div>

      {result && (
        <p className="mb-3 text-xs text-muted-foreground">
          查询 {subjectCount} 个被执行人 · 拉了 {rawCount} 个元典端点数据
          {failureCount > 0 && (
            <span className="text-amber-700"> · {failureCount} 个端点失败</span>
          )}
          {result.assessment.corpus_chars > 0 && (
            <span> · 喂 LLM {result.assessment.corpus_chars} chars,{(result.assessment.elapsed_ms / 1000).toFixed(1)} 秒出报告</span>
          )}
        </p>
      )}

      {digHints.length > 0 && (
        <div className="mt-3 rounded-md border border-border bg-card p-3">
          <div className="mb-2 text-label font-medium uppercase tracking-wider text-muted-foreground">
            LLM 深挖建议({digHints.length} 条 · 「🔬 深挖」按钮会全部跑一遍)
          </div>
          <ul className="space-y-1.5 text-xs">
            {digHints.map((h: DigHint, i: number) => (
              <li key={i} className="flex items-baseline gap-2">
                <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-caption font-medium text-muted-foreground">
                  {kindLabel(h.kind)}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="font-medium text-foreground">{h.target}</div>
                  <div className="text-muted-foreground/80">{h.reason}</div>
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}

/* ============ 还款记录卡(2026-05-25)============ */
function PaymentsCard({
  payments,
  sourceDocs,
  onAdd,
  onDelete,
  onOpenSource,
}: {
  payments: Payment[];
  sourceDocs: Document[];
  onAdd: (input: { amount: number; paid_at: string; note: string | null }) => void;
  onDelete: (id: string) => void;
  onOpenSource: (doc: Document) => void;
}) {
  const [showInput, setShowInput] = useState(false);
  const [amount, setAmount] = useState("");
  const [paidAt, setPaidAt] = useState(() => todayIsoLocal());
  const [note, setNote] = useState("");

  const submit = () => {
    const n = parseFloat(amount);
    if (!Number.isFinite(n) || n <= 0) {
      alert("金额需要是大于 0 的数字");
      return;
    }
    if (!paidAt) {
      alert("付款日期必填");
      return;
    }
    onAdd({ amount: n, paid_at: paidAt, note: note.trim() || null });
    setAmount("");
    setNote("");
    setShowInput(false);
  };

  return (
    <section className="rounded-lg border border-border bg-card p-5">
      <div className="mb-3 flex items-center justify-between">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          还款记录 ({payments.length})
        </h3>
        <Button
          variant="outline"
          size="sm"
          onClick={() => setShowInput((v) => !v)}
        >
          <Plus className="size-3.5" />
          {showInput ? "收起" : "录入还款"}
        </Button>
      </div>

      {showInput && (
        <div className="mb-4 rounded-md border border-border bg-muted/30 p-3">
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-12">
            <div className="sm:col-span-3">
              <label className="text-caption text-muted-foreground">金额(元)</label>
              <input
                type="number"
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                placeholder="50000"
                className="mt-1 w-full rounded-md border border-border bg-card px-2 py-1.5 text-sm outline-none focus:border-foreground/50"
              />
            </div>
            <div className="sm:col-span-3">
              <label className="text-caption text-muted-foreground">付款日期</label>
              <input
                type="date"
                value={paidAt}
                onChange={(e) => setPaidAt(e.target.value)}
                className="mt-1 w-full rounded-md border border-border bg-card px-2 py-1.5 text-sm outline-none focus:border-foreground/50"
              />
            </div>
            <div className="sm:col-span-4">
              <label className="text-caption text-muted-foreground">备注</label>
              <input
                type="text"
                value={note}
                onChange={(e) => setNote(e.target.value)}
                placeholder="转账银行 / 担保人代付 / 强制执行"
                className="mt-1 w-full rounded-md border border-border bg-card px-2 py-1.5 text-sm outline-none focus:border-foreground/50"
              />
            </div>
            <div className="flex items-end sm:col-span-2">
              <Button size="sm" onClick={submit} className="w-full">
                确认
              </Button>
            </div>
          </div>
        </div>
      )}

      {payments.length === 0 ? (
        <div className="rounded-md border border-dashed border-border bg-muted/20 px-3 py-4 text-center text-xs text-muted-foreground">
          还没有还款记录 — 点「录入还款」加第一笔;案件文件夹里放转账截图/汇款凭证,重新抽取后会自动识别入账
        </div>
      ) : (
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border text-xs text-muted-foreground">
              <th className="px-2 py-2 text-left">日期</th>
              <th className="px-2 py-2 text-right">金额</th>
              <th className="px-2 py-2 text-left">备注</th>
              <th className="px-2 py-2 w-12"></th>
            </tr>
          </thead>
          <tbody>
            {payments.map((p) => (
              <tr key={p.id} className="border-b border-border/50">
                <td className="px-2 py-2 font-mono text-xs">{p.paid_at}</td>
                <td className="px-2 py-2 text-right font-mono font-medium">
                  {formatYuan(p.amount)}
                </td>
                <td className="px-2 py-2 text-xs text-muted-foreground">
                  {/* 2026-06-11:AI 从转账截图/凭证自动识别入账的,标 chip 提示可核对、识别错可删 */}
                  {p.note?.startsWith("[AI识别]") ? (
                    <span className="inline-flex flex-wrap items-center gap-1">
                      <span className="rounded bg-sky-50 px-1.5 py-0.5 text-caption font-medium text-sky-700 dark:bg-sky-950/40 dark:text-sky-300">
                        AI 识别
                      </span>
                      {p.note.slice("[AI识别]".length).trim() || "—"}
                      {paymentSourceDoc(p, sourceDocs) && (
                        <button
                          type="button"
                          onClick={() => {
                            const doc = paymentSourceDoc(p, sourceDocs);
                            if (doc) onOpenSource(doc);
                          }}
                          className="ml-1 inline-flex items-center gap-1 rounded border border-border bg-card px-1.5 py-0.5 text-caption font-medium text-foreground transition-colors hover:bg-muted"
                          title="在案件看板内打开这笔还款的源文件"
                        >
                          <FileText className="size-3" />
                          查看源文件
                        </button>
                      )}
                    </span>
                  ) : (
                    (p.note ?? "—")
                  )}
                </td>
                <td className="px-2 py-2 text-right">
                  <button
                    type="button"
                    onClick={() => onDelete(p.id)}
                    className="rounded p-1 text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
                    title="删除"
                    aria-label="删除"
                  >
                    <Trash2 className="size-3.5" />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

function paymentSourceDoc(payment: Payment, docs: Document[]): Document | null {
  const sourceDoc =
    (payment.source_document_id
      ? docs.find((doc) => doc.id === payment.source_document_id)
      : null) ??
    (payment.source_path
      ? docs.find((doc) => doc.source_path === payment.source_path)
      : null);
  if (sourceDoc) return sourceDoc;
  if (!payment.source_path || !payment.source_filename) return null;
  return {
    id: payment.source_document_id ?? `payment-source:${payment.id}`,
    case_id: payment.case_id,
    source_path: payment.source_path,
    filename: payment.source_filename,
    stage: null,
    category: "还款凭证",
    is_ai_artifact: false,
    source: "scan",
    mime_type: null,
    size_bytes: 0,
    modified_at: null,
    extracted_fields: null,
    extraction_status: "pending",
    missing: false,
    created_at: payment.created_at,
    deleted_at: null,
    extracted_text_path: null,
    extracted_text_hash: null,
    cache_key: null,
    pinned_at: null,
    ocr_backend_override: null,
    display_name: null,
    display_name_source: null,
  };
}

function kindLabel(kind: string): string {
  switch (kind) {
    case "enterprise":
      return "公司";
    case "case":
      return "案号";
    case "person":
      return "人";
    default:
      return kind;
  }
}

/* ============ utility ============ */

function safeJson<T>(json: string | null, fallback: T): T {
  if (!json) return fallback;
  try {
    return JSON.parse(json) as T;
  } catch {
    return fallback;
  }
}

function Card({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-lg border border-border bg-card p-5">
      <h3 className="mb-3 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
        {title}
      </h3>
      {children}
    </section>
  );
}

function Row({
  label,
  value,
  emphasize = false,
}: {
  label: string;
  value: string;
  emphasize?: boolean;
}) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span
        className={
          emphasize
            ? "font-semibold text-foreground"
            : "font-medium text-foreground"
        }
      >
        {value}
      </span>
    </div>
  );
}

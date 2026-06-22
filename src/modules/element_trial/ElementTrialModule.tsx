/**
 * 要素式审判智能辅助 — 主模块
 *
 * 三大 Tab:
 * 1. 诉状生成 — 庭审前要素式起诉状/答辩状
 * 2. 要件分析 — 庭审中证据→要件→争点归依
 * 3. 攻防策略 — 三级递进攻防模型
 */

import { useCallback, useEffect, useState } from "react";
import {
  BookOpen,
  FileText,
  Gavel,
  Loader2,
  RefreshCw,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { toast } from "@/components/ui/toast";
import type { Case } from "@/lib/types";
import {
  getElementTemplates,
  getElementFacts,
  getDisputedFacts,
  getTrialStrategies,
  upsertElementFacts,
  saveTrialStrategy,
  getElementComplaints,
  listTemplateCauses,
  type ElementTemplate,
  type ElementFact,
  type TrialStrategy,
  type ElementComplaint,
} from "./lib/api";

/* ------------------------------------------------------------------ */
/* Tab 定义                                                            */
/* ------------------------------------------------------------------ */

type ElementTrialTab = "complaint" | "facts" | "strategy";

const TABS: { key: ElementTrialTab; label: string; icon: typeof FileText }[] = [
  { key: "complaint", label: "诉状生成", icon: FileText },
  { key: "facts", label: "要件分析", icon: BookOpen },
  { key: "strategy", label: "攻防策略", icon: Gavel },
];

/* ------------------------------------------------------------------ */
/* 主组件                                                              */
/* ------------------------------------------------------------------ */

export function ElementTrialModule({ selectedCase }: { selectedCase: Case | null }) {
  const [activeTab, setActiveTab] = useState<ElementTrialTab>("complaint");

  if (!selectedCase) {
    return (
      <div className="flex flex-col items-center justify-center h-64 text-muted-foreground gap-2">
        <BookOpen className="w-10 h-10 opacity-30" />
        <p className="text-sm">请先选择一个案件</p>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      {/* Tab 导航 */}
      <div className="flex gap-1 px-4 pt-3 pb-2 border-b">
        {TABS.map((tab) => {
          const Icon = tab.icon;
          return (
            <button
              key={tab.key}
              onClick={() => setActiveTab(tab.key)}
              className={`flex items-center gap-1.5 px-3 py-1.5 rounded text-sm transition-colors ${
                activeTab === tab.key
                  ? "bg-primary text-primary-foreground"
                  : "hover:bg-muted text-muted-foreground"
              }`}
            >
              <Icon className="w-4 h-4" />
              {tab.label}
            </button>
          );
        })}
      </div>

      {/* Tab 内容 */}
      <div className="flex-1 overflow-auto p-4">
        {activeTab === "complaint" && (
          <ComplaintGeneratorPanel caseId={selectedCase.id} cause={selectedCase.cause ?? ""} />
        )}
        {activeTab === "facts" && (
          <FactAnalysisPanel caseId={selectedCase.id} cause={selectedCase.cause ?? ""} />
        )}
        {activeTab === "strategy" && (
          <StrategyPanel caseId={selectedCase.id} cause={selectedCase.cause ?? ""} />
        )}
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Tab 1: 诉状生成面板                                                  */
/* ------------------------------------------------------------------ */

function ComplaintGeneratorPanel({ caseId, cause }: { caseId: string; cause: string }) {
  const [templates, setTemplates] = useState<ElementTemplate[]>([]);
  const [complaints, setComplaints] = useState<ElementComplaint[]>([]);
  const [loading, setLoading] = useState(false);
  const [causeList, setCauseList] = useState<string[]>([]);
  const [selectedCause, setSelectedCause] = useState(cause);

  const loadData = useCallback(async () => {
    setLoading(true);
    try {
      const [causes, temps, comps] = await Promise.all([
        listTemplateCauses(),
        selectedCause ? getElementTemplates(selectedCause) : Promise.resolve([]),
        getElementComplaints(caseId),
      ]);
      setCauseList(causes);
      setTemplates(temps);
      setComplaints(comps);
    } catch (e) {
      toast({ title: "加载失败", description: String(e) });
    } finally {
      setLoading(false);
    }
  }, [caseId, selectedCause]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="font-semibold">要素式诉辩状智能生成</h3>
          <p className="text-xs text-muted-foreground mt-0.5">
            庭审前自动识别案件事实,按案由要素模板填写起诉状/答辩状
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={loadData} disabled={loading}>
          <RefreshCw className={`w-3.5 h-3.5 mr-1 ${loading ? "animate-spin" : ""}`} />
          刷新
        </Button>
      </div>

      {/* 案由选择 */}
      {causeList.length > 0 && (
        <div className="flex items-center gap-2">
          <span className="text-xs text-muted-foreground">案由:</span>
          <select
            value={selectedCause}
            onChange={(e) => setSelectedCause(e.target.value)}
            className="text-xs border rounded px-2 py-1 bg-background"
          >
            <option value="">-- 选择案由 --</option>
            {causeList.map((c) => (
              <option key={c} value={c}>{c}</option>
            ))}
          </select>
        </div>
      )}

      {loading && (
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="w-4 h-4 animate-spin" />
          加载中...
        </div>
      )}

      {/* 起诉状要素模板 */}
      {templates.filter((t) => t.direction === "起诉").length > 0 && (
        <div className="border rounded-lg p-4">
          <h4 className="text-sm font-medium mb-3">起诉状要素框架</h4>
          <div className="space-y-2">
            {templates
              .filter((t) => t.direction === "起诉")
              .map((tpl) => (
                <div key={tpl.id} className="flex items-start gap-3 p-2 bg-muted/30 rounded">
                  <span className={`text-xs px-1.5 py-0.5 rounded ${tpl.is_required ? "bg-red-100 text-red-700" : "bg-gray-100 text-gray-500"}`}>
                    {tpl.is_required ? "必备" : "可选"}
                  </span>
                  <div className="min-w-0">
                    <div className="text-sm font-medium">{tpl.element_name}</div>
                    <div className="text-xs text-muted-foreground mt-0.5">{tpl.element_desc}</div>
                    {tpl.evidence_hint && (
                      <div className="text-xs text-blue-600 mt-0.5">证据建议: {tpl.evidence_hint}</div>
                    )}
                  </div>
                </div>
              ))}
          </div>
          <Button className="mt-3" size="sm">
            <FileText className="w-3.5 h-3.5 mr-1" />
            AI 生成起诉状
          </Button>
        </div>
      )}

      {/* 答辩状要素模板 */}
      {templates.filter((t) => t.direction === "答辩").length > 0 && (
        <div className="border rounded-lg p-4">
          <h4 className="text-sm font-medium mb-3">答辩状要素框架</h4>
          <div className="space-y-2">
            {templates
              .filter((t) => t.direction === "答辩")
              .map((tpl) => (
                <div key={tpl.id} className="flex items-start gap-3 p-2 bg-muted/30 rounded">
                  <span className={`text-xs px-1.5 py-0.5 rounded ${tpl.is_required ? "bg-red-100 text-red-700" : "bg-gray-100 text-gray-500"}`}>
                    {tpl.is_required ? "必备" : "可选"}
                  </span>
                  <div className="min-w-0">
                    <div className="text-sm font-medium">{tpl.element_name}</div>
                    <div className="text-xs text-muted-foreground mt-0.5">{tpl.element_desc}</div>
                    {tpl.evidence_hint && (
                      <div className="text-xs text-blue-600 mt-0.5">证据建议: {tpl.evidence_hint}</div>
                    )}
                  </div>
                </div>
              ))}
          </div>
          <Button className="mt-3" size="sm" variant="secondary">
            <FileText className="w-3.5 h-3.5 mr-1" />
            AI 生成答辩状
          </Button>
        </div>
      )}

      {/* 历史生成文书 */}
      {complaints.length > 0 && (
        <div className="border rounded-lg p-4">
          <h4 className="text-sm font-medium mb-2">已生成文书</h4>
          <div className="space-y-1">
            {complaints.map((c) => (
              <div key={c.id} className="flex items-center gap-2 text-sm py-1">
                <FileText className="w-3.5 h-3.5 text-muted-foreground" />
                <span>{c.doc_type}</span>
                <span className="text-xs text-muted-foreground">v{c.version}</span>
                {c.is_final && <span className="text-xs px-1 bg-green-100 text-green-700 rounded">终稿</span>}
              </div>
            ))}
          </div>
        </div>
      )}

      {!loading && templates.length === 0 && (
        <div className="text-center py-8 text-sm text-muted-foreground">
          {selectedCause ? "该案由暂无要素模板,请先在数据库中预置模板数据" : "请选择案由查看要素模板"}
        </div>
      )}
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Tab 2: 要件事实分析面板                                               */
/* ------------------------------------------------------------------ */

function FactAnalysisPanel({ caseId, cause }: { caseId: string; cause: string }) {
  const [facts, setFacts] = useState<ElementFact[]>([]);
  const [disputedFacts, setDisputedFacts] = useState<ElementFact[]>([]);
  const [loading, setLoading] = useState(false);

  const loadFacts = useCallback(async () => {
    setLoading(true);
    try {
      const [allFacts, dispFacts] = await Promise.all([
        getElementFacts(caseId),
        getDisputedFacts(caseId),
      ]);
      setFacts(allFacts);
      setDisputedFacts(dispFacts);
    } catch (e) {
      toast({ title: "加载要件事实失败", description: String(e) });
    } finally {
      setLoading(false);
    }
  }, [caseId]);

  useEffect(() => {
    loadFacts();
  }, [loadFacts]);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="font-semibold">要件事实分析研判</h3>
          <p className="text-xs text-muted-foreground mt-0.5">
            证据→要件→争点三重归依模型,逐条分析举证状态与法院认定预测
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={loadFacts} disabled={loading}>
            <RefreshCw className={`w-3.5 h-3.5 mr-1 ${loading ? "animate-spin" : ""}`} />
            刷新
          </Button>
          <Button size="sm">AI 分析要件</Button>
        </div>
      </div>

      {loading && (
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="w-4 h-4 animate-spin" /> 加载中...
        </div>
      )}

      {/* 争点汇总 */}
      {disputedFacts.length > 0 && (
        <div className="border border-orange-200 bg-orange-50 rounded-lg p-4">
          <h4 className="text-sm font-medium text-orange-800 mb-2">
            当前争点 ({disputedFacts.length})
          </h4>
          <div className="space-y-2">
            {disputedFacts.map((f) => (
              <div key={f.id} className="text-sm bg-white rounded p-2 border border-orange-100">
                <span className="font-medium">{f.fact_name}</span>
                {f.opponent_rebuttal && (
                  <p className="text-xs text-muted-foreground mt-1">
                    对方抗辩: {f.opponent_rebuttal}
                  </p>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* 全部要件事实表 */}
      {facts.length > 0 ? (
        <div className="border rounded-lg overflow-hidden">
          <table className="w-full text-sm">
            <thead>
              <tr className="bg-muted/50">
                <th className="text-left px-3 py-2 font-medium">要件名称</th>
                <th className="text-left px-3 py-2 font-medium">主张方</th>
                <th className="text-left px-3 py-2 font-medium">举证状态</th>
                <th className="text-left px-3 py-2 font-medium">法院认定</th>
                <th className="text-left px-3 py-2 font-medium">成立</th>
              </tr>
            </thead>
            <tbody>
              {facts.map((f) => (
                <tr key={f.id} className={`border-t ${f.is_disputed ? "bg-orange-50/50" : ""}`}>
                  <td className="px-3 py-2">
                    <div className="font-medium">{f.fact_name}</div>
                    {f.fact_desc && (
                      <div className="text-xs text-muted-foreground mt-0.5">{f.fact_desc}</div>
                    )}
                  </td>
                  <td className="px-3 py-2">{f.claim_party ?? "-"}</td>
                  <td className="px-3 py-2">
                    <span className={`text-xs px-1.5 py-0.5 rounded ${
                      f.proof_status === "已举证" ? "bg-green-100 text-green-700" :
                      f.proof_status === "待补证" ? "bg-yellow-100 text-yellow-700" :
                      f.proof_status === "举证不能" ? "bg-red-100 text-red-700" :
                      "bg-gray-100 text-gray-500"
                    }`}>
                      {f.proof_status}
                    </span>
                  </td>
                  <td className="px-3 py-2 text-xs">{f.court_finding ?? "-"}</td>
                  <td className="px-3 py-2">
                    {f.is_established === null ? (
                      <span className="text-xs text-muted-foreground">待定</span>
                    ) : f.is_established ? (
                      <span className="text-xs text-green-600">成立</span>
                    ) : (
                      <span className="text-xs text-red-600">不成立</span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : !loading ? (
        <div className="text-center py-8 text-sm text-muted-foreground">
          暂无要件事实数据,请先导入案件资料后使用 AI 分析
        </div>
      ) : null}
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Tab 3: 攻防策略面板                                                   */
/* ------------------------------------------------------------------ */

function StrategyPanel({ caseId, cause }: { caseId: string; cause: string }) {
  const [strategies, setStrategies] = useState<TrialStrategy[]>([]);
  const [loading, setLoading] = useState(false);

  const loadStrategies = useCallback(async () => {
    setLoading(true);
    try {
      const result = await getTrialStrategies(caseId);
      setStrategies(result);
    } catch (e) {
      toast({ title: "加载策略失败", description: String(e) });
    } finally {
      setLoading(false);
    }
  }, [caseId]);

  useEffect(() => {
    loadStrategies();
  }, [loadStrategies]);

  const layerLabel: Record<string, string> = {
    "主张责任": "第一层",
    "证明责任": "第二层",
    "举证行为": "第三层",
  };

  const layerColor: Record<string, string> = {
    "主张责任": "border-l-blue-400",
    "证明责任": "border-l-yellow-400",
    "举证行为": "border-l-green-400",
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="font-semibold">攻防策略搭建</h3>
          <p className="text-xs text-muted-foreground mt-0.5">
            三级递进模型:主张责任 → 证明责任 → 举证行为
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={loadStrategies} disabled={loading}>
            <RefreshCw className={`w-3.5 h-3.5 mr-1 ${loading ? "animate-spin" : ""}`} />
            刷新
          </Button>
          <Button size="sm">
            <Gavel className="w-3.5 h-3.5 mr-1" />
            AI 生成策略
          </Button>
        </div>
      </div>

      {loading && (
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="w-4 h-4 animate-spin" /> 加载中...
        </div>
      )}

      {/* 三级递进策略卡片 */}
      {strategies.length > 0 ? (
        <div className="space-y-3">
          {strategies.map((s) => (
            <div
              key={s.id}
              className={`border rounded-lg p-4 border-l-4 ${layerColor[s.strategy_layer] ?? "border-l-gray-300"}`}
            >
              <div className="flex items-center gap-2 mb-2">
                <span className="text-xs px-1.5 py-0.5 bg-muted rounded">
                  {layerLabel[s.strategy_layer] ?? s.strategy_layer}
                </span>
                <span className="text-sm font-medium">{s.strategy_layer}分析</span>
                {s.risk_level && (
                  <span className={`text-xs px-1.5 py-0.5 rounded ${
                    s.risk_level === "高" ? "bg-red-100 text-red-700" :
                    s.risk_level === "中" ? "bg-yellow-100 text-yellow-700" :
                    "bg-green-100 text-green-700"
                  }`}>
                    风险: {s.risk_level}
                  </span>
                )}
              </div>
              <div className="text-sm whitespace-pre-wrap text-muted-foreground">
                {s.strategy_content.length > 500
                  ? s.strategy_content.slice(0, 500) + "…"
                  : s.strategy_content}
              </div>
              {s.recommended_actions && (
                <div className="mt-2 text-xs text-blue-600 bg-blue-50 rounded p-2">
                  建议动作: {s.recommended_actions}
                </div>
              )}
            </div>
          ))}
        </div>
      ) : !loading ? (
        <div className="text-center py-8 text-sm text-muted-foreground">
          暂无策略记录,请先完成要件事实分析后生成攻防策略
        </div>
      ) : null}

      {/* 三级递进说明 */}
      <div className="text-xs text-muted-foreground bg-muted/30 rounded-lg p-3">
        <p className="font-medium mb-1">三级递进模型说明:</p>
        <ol className="list-decimal list-inside space-y-0.5">
          <li><strong>主张责任</strong> — 明确我方主张内容及法律依据,预判对方可能主张</li>
          <li><strong>证明责任</strong> — 分析各要件证明责任归属,评估举证难度与现有证据覆盖度</li>
          <li><strong>举证行为</strong> — 制定具体证据链构建路径,按优先级排序补证动作</li>
        </ol>
      </div>
    </div>
  );
}

export default ElementTrialModule;

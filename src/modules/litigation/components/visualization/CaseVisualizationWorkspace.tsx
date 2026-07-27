import {
  Component,
  Suspense,
  lazy,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ErrorInfo,
  type ReactNode,
} from "react";
import {
  ArrowLeft,
  Check,
  ChevronDown,
  Download,
  FileSearch,
  GitBranch,
  LoaderCircle,
  Redo2,
  RotateCcw,
  Save,
  SlidersHorizontal,
  Sparkles,
  Trash2,
  Undo2,
  X,
} from "lucide-react";
import { save } from "@tauri-apps/plugin-dialog";

import { Button } from "@/components/ui/button";
import {
  exportCaseVisual,
  getCaseVisualWorkspace,
  listCaseVisualProposals,
  resolveCaseVisualProposal,
  revealInFinder,
  saveCaseVisualUserRevision,
  writeCaseVisualExport,
} from "@/lib/api";
import { confirmDialog } from "@/lib/dialog";
import { cn } from "@/lib/utils";

import type { CanvasPositions } from "./CaseGraphCanvas";
import type {
  CaseGraph,
  CaseGraphNode,
  CaseGraphView,
  JsonValue,
  CaseGraphPatch,
  VisualProposal,
  VisualWorkspace,
} from "./types";
import { pushHistory, redoHistory, removeGraphView, undoHistory } from "./graphReducer";
import { statusVisual } from "./visualizationTheme";
import {
  buildVisualPdfBase64,
  createVisualCoverDataUrl,
  dataUrlBase64,
  textToBase64,
} from "./visualExport";

const CaseGraphCanvas = lazy(() => import("./CaseGraphCanvas"));
const LegalTimelineView = lazy(() => import("./LegalTimelineView"));
const EvidenceMatrixView = lazy(() => import("./EvidenceMatrixView"));
const QuantitativeChartView = lazy(() => import("./QuantitativeChartView"));
const BarTableView = lazy(() => import("./BarTableView"));
const NodeInspector = lazy(() => import("./NodeInspector"));
const ProposalReviewPanel = lazy(() => import("./ProposalReviewPanel"));
const ViewSettingsPanel = lazy(() => import("./ViewSettingsPanel"));
const AiVisualAdjustPanel = lazy(() => import("./AiVisualAdjustPanel"));

interface ErrorBoundaryProps {
  children: ReactNode;
  resetKey: string;
}

interface ErrorBoundaryState {
  error: Error | null;
}

export class VisualizationErrorBoundary extends Component<
  ErrorBoundaryProps,
  ErrorBoundaryState
> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(_error: Error, _info: ErrorInfo) {
    // 错误只留在本地视图，不能把案件内容或节点标题写进遥测。
  }

  componentDidUpdate(previous: ErrorBoundaryProps) {
    if (previous.resetKey !== this.props.resetKey && this.state.error) {
      this.setState({ error: null });
    }
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="flex h-full items-center justify-center px-6 text-center">
        <div>
          <p className="text-sm font-medium text-foreground">当前视图无法显示</p>
          <p className="mt-1 text-xs text-muted-foreground">
            语义数据仍然安全保留。可以切换到其他视图，或重试当前视图。
          </p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="mt-3"
            onClick={() => this.setState({ error: null })}
          >
            <RotateCcw className="size-3.5" />
            重试当前视图
          </Button>
        </div>
      </div>
    );
  }
}

function viewKindLabel(view: CaseGraphView): string {
  const labels: Record<CaseGraphView["kind"], string> = {
    timeline: "时间线",
    relationship: "关系图",
    mindmap: "思维导图",
    evidence_matrix: "证据矩阵",
    bar: "柱状图",
    line: "折线图",
    heatmap: "热力图",
    bar_table: "数据条表格",
  };
  return labels[view.kind];
}

function safeExportStem(title: string): string {
  const value = title.replace(/[\\/:*?"<>|\r\n]/g, "_").trim().replace(/^\.+|\.+$/g, "");
  return value || "案情可视化";
}

function waitForVisualRender(): Promise<void> {
  return new Promise((resolve) => {
    window.requestAnimationFrame(() => {
      window.requestAnimationFrame(() => window.setTimeout(resolve, 320));
    });
  });
}

function readPositions(layout: Record<string, JsonValue>): Record<string, CanvasPositions> {
  const value = layout.positions_by_view;
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const output: Record<string, CanvasPositions> = {};
  for (const [viewId, positionsValue] of Object.entries(value)) {
    if (!positionsValue || typeof positionsValue !== "object" || Array.isArray(positionsValue)) continue;
    const positions: CanvasPositions = {};
    for (const [nodeId, pointValue] of Object.entries(positionsValue)) {
      if (!pointValue || typeof pointValue !== "object" || Array.isArray(pointValue)) continue;
      const x = pointValue.x;
      const y = pointValue.y;
      if (typeof x === "number" && Number.isFinite(x) && typeof y === "number" && Number.isFinite(y)) {
        positions[nodeId] = { x, y };
      }
    }
    output[viewId] = positions;
  }
  return output;
}

interface Props {
  caseId: string;
  initialWorkspace: VisualWorkspace;
  onClose: () => void;
  onOpenSource?: (documentId: string) => void;
  onSaved?: (workspace: VisualWorkspace) => void;
  onRequestAiChange?: (request: string) => Promise<void>;
}

export default function CaseVisualizationWorkspace({
  caseId,
  initialWorkspace,
  onClose,
  onOpenSource,
  onSaved,
  onRequestAiChange,
}: Props) {
  const [workspace, setWorkspace] = useState(initialWorkspace);
  const [graph, setGraph] = useState(initialWorkspace.graph);
  const [currentViewId, setCurrentViewId] = useState(
    initialWorkspace.graph.views[0]?.id ?? "",
  );
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [past, setPast] = useState<CaseGraph[]>([]);
  const [future, setFuture] = useState<CaseGraph[]>([]);
  const [positionsByView, setPositionsByView] = useState<Record<string, CanvasPositions>>(() =>
    readPositions(initialWorkspace.layout),
  );
  const [semanticDirty, setSemanticDirty] = useState(false);
  const [layoutDirty, setLayoutDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveState, setSaveState] = useState<"idle" | "saved">("idle");
  const [error, setError] = useState<string | null>(null);
  const [proposals, setProposals] = useState<VisualProposal[]>([]);
  const [activePanel, setActivePanel] = useState<"proposal" | "node" | "settings" | "ai">("node");
  const [panelOpen, setPanelOpen] = useState(false);
  const [proposalBusy, setProposalBusy] = useState(false);
  const [aiBusy, setAiBusy] = useState(false);
  const [exportMenuOpen, setExportMenuOpen] = useState(false);
  const [exportMenuPosition, setExportMenuPosition] = useState({ top: 56, left: 8 });
  const [exporting, setExporting] = useState<"png" | "pdf" | "markdown" | "json" | null>(null);
  const saveInFlight = useRef(false);
  const savedStateTimer = useRef<number | null>(null);
  const canvasRef = useRef<HTMLElement>(null);
  const exportButtonRef = useRef<HTMLButtonElement>(null);
  const reconciledProposalIds = useRef<Set<string>>(new Set());

  const currentView = useMemo(
    () => graph.views.find((view) => view.id === currentViewId) ?? graph.views[0] ?? null,
    [currentViewId, graph.views],
  );
  const selectedNode = useMemo(
    () => graph.nodes.find((node) => node.id === selectedNodeId) ?? null,
    [graph.nodes, selectedNodeId],
  );
  const currentDataset = useMemo(
    () => graph.datasets.find((dataset) => dataset.id === currentView?.dataset_id) ?? null,
    [currentView?.dataset_id, graph.datasets],
  );
  const activeProposal = useMemo(
    () => proposals.find((proposal) => proposal.status === "pending") ?? null,
    [proposals],
  );

  const loadProposals = useCallback(async (workspaceId: string) => {
    try {
      const next = await listCaseVisualProposals(caseId, workspaceId);
      setProposals(next);
      return next;
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "读取 AI 变更建议失败");
      return [];
    }
  }, [caseId]);

  const adoptWorkspace = useCallback((next: VisualWorkspace) => {
    setWorkspace(next);
    setGraph(next.graph);
    setPositionsByView(readPositions(next.layout));
    setPast([]);
    setFuture([]);
    setSemanticDirty(false);
    setLayoutDirty(false);
    setSelectedNodeId(null);
    setCurrentViewId((current) => next.graph.views.some((view) => view.id === current)
      ? current
      : next.graph.views[0]?.id ?? "");
    onSaved?.(next);
  }, [onSaved]);

  useEffect(() => {
    let cancelled = false;
    async function reconcileLegacyProposal() {
      const next = await loadProposals(workspace.id);
      if (cancelled) return;
      const pending = next.find((proposal) =>
        proposal.status === "pending"
        && proposal.base_revision === workspace.revision
        && !reconciledProposalIds.current.has(proposal.id));
      if (!pending) return;
      reconciledProposalIds.current.add(pending.id);
      try {
        const merged = await resolveCaseVisualProposal({
          caseId,
          workspaceId: workspace.id,
          proposalId: pending.id,
          action: "accept",
          acceptedPatch: pending.patch,
        });
        if (cancelled) return;
        adoptWorkspace(merged);
        setProposals([]);
      } catch (reason) {
        if (!cancelled) {
          setError(reason instanceof Error ? reason.message : "旧版可视化建议自动应用失败，请刷新后重试");
        }
      }
    }
    void reconcileLegacyProposal();
    return () => {
      cancelled = true;
    };
  }, [adoptWorkspace, caseId, loadProposals, workspace.id, workspace.revision]);

  useEffect(() => {
    if (!exportMenuOpen) return;
    const close = () => setExportMenuOpen(false);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
    };
  }, [exportMenuOpen]);

  const persist = useCallback(
    async (summary: string): Promise<VisualWorkspace | null> => {
      if (saveInFlight.current) return null;
      saveInFlight.current = true;
      setSaving(true);
      setError(null);
      try {
        const currentViewIds = new Set(graph.views.map((view) => view.id));
        const savedPositions = Object.fromEntries(
          Object.entries(positionsByView).filter(([viewId]) => currentViewIds.has(viewId)),
        );
        const layout: Record<string, JsonValue> = {
          ...workspace.layout,
          positions_by_view: savedPositions,
          current_view_id: currentView?.id ?? "",
        };
        const saved = await saveCaseVisualUserRevision({
          caseId,
          workspaceId: workspace.id,
          expectedRevision: workspace.revision,
          graph,
          layout,
          summary,
        });
        setWorkspace(saved);
        setGraph(saved.graph);
        setSemanticDirty(false);
        setLayoutDirty(false);
        setSaveState("saved");
        onSaved?.(saved);
        if (savedStateTimer.current !== null) window.clearTimeout(savedStateTimer.current);
        savedStateTimer.current = window.setTimeout(() => setSaveState("idle"), 1500);
        return saved;
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : "保存可视化工作区失败");
        return null;
      } finally {
        saveInFlight.current = false;
        setSaving(false);
      }
    }, [caseId, currentView?.id, graph, onSaved, positionsByView, workspace]);

  useEffect(() => {
    if (!layoutDirty || semanticDirty || saving) return;
    const timer = window.setTimeout(() => void persist("调整视图布局"), 600);
    return () => window.clearTimeout(timer);
  }, [layoutDirty, persist, saving, semanticDirty]);

  useEffect(() => {
    function beforeUnload(event: BeforeUnloadEvent) {
      if (!semanticDirty && !layoutDirty) return;
      event.preventDefault();
    }
    window.addEventListener("beforeunload", beforeUnload);
    return () => window.removeEventListener("beforeunload", beforeUnload);
  }, [layoutDirty, semanticDirty]);

  useEffect(
    () => () => {
      if (savedStateTimer.current !== null) window.clearTimeout(savedStateTimer.current);
    },
    [],
  );

  function changeNode(node: CaseGraphNode) {
    setPast((items) => pushHistory(items, graph));
    setFuture([]);
    setGraph((current) => ({
      ...current,
      nodes: current.nodes.map((item) => (item.id === node.id ? node : item)),
    }));
    setSemanticDirty(true);
    setSaveState("idle");
  }

  function changeView(view: CaseGraphView) {
    setPast((items) => pushHistory(items, graph));
    setFuture([]);
    setGraph((current) => ({
      ...current,
      views: current.views.map((item) => item.id === view.id ? view : item),
    }));
    setSemanticDirty(true);
    setSaveState("idle");
  }

  async function deleteCurrentView() {
    if (!currentView) return;
    if (!(await confirmDialog(`确定删除《${currentView.title}》吗？删除后可在保存前撤销。`, { danger: true, okLabel: "删除" }))) return;
    const currentIndex = graph.views.findIndex((view) => view.id === currentView.id);
    const nextGraph = removeGraphView(graph, currentView.id);
    const nextView = nextGraph.views[Math.min(currentIndex, nextGraph.views.length - 1)] ?? null;
    setPast((items) => pushHistory(items, graph));
    setFuture([]);
    setGraph(nextGraph);
    setCurrentViewId(nextView?.id ?? "");
    setSelectedNodeId(null);
    setSemanticDirty(true);
    setLayoutDirty(true);
    setSaveState("idle");
    setActivePanel("node");
    setPanelOpen(false);
  }

  function undo() {
    const result = undoHistory(past, graph, future);
    if (!result.changed) return;
    setPast(result.past);
    setGraph(result.current);
    setFuture(result.future);
    setSemanticDirty(true);
  }

  function redo() {
    const result = redoHistory(past, graph, future);
    if (!result.changed) return;
    setPast(result.past);
    setGraph(result.current);
    setFuture(result.future);
    setSemanticDirty(true);
  }

  async function requestClose() {
    if ((semanticDirty || layoutDirty) && !(await confirmDialog("还有未保存的可视化修改，确定关闭吗？", { okLabel: "关闭" }))) {
      return;
    }
    onClose();
  }

  function toggleExportMenu() {
    if (exportMenuOpen) {
      setExportMenuOpen(false);
      return;
    }
    const rect = exportButtonRef.current?.getBoundingClientRect();
    if (rect) {
      setExportMenuPosition({
        top: rect.bottom + 4,
        left: Math.max(8, Math.min(window.innerWidth - 184, rect.right - 176)),
      });
    }
    setExportMenuOpen(true);
  }

  function openPanel(panel: "proposal" | "node" | "settings" | "ai") {
    setActivePanel(panel);
    setPanelOpen(true);
  }

  async function captureCurrentView(): Promise<string> {
    if (!canvasRef.current) throw new Error("可视化画布尚未准备完成");
    const { toPng } = await import("html-to-image");
    const target = canvasRef.current.querySelector<HTMLElement>("[data-visual-export-root]")
      ?? canvasRef.current;
    const width = Math.max(target.clientWidth, target.scrollWidth);
    const height = Math.max(target.clientHeight, target.scrollHeight);
    return toPng(target, {
      backgroundColor: "#ffffff",
      cacheBust: true,
      pixelRatio: 1.5,
      width,
      height,
      style: {
        width: `${width}px`,
        height: `${height}px`,
        overflow: "visible",
      },
    });
  }

  async function runExport(kind: "png" | "pdf" | "markdown" | "json") {
    if (saving || exporting) return;
    setExportMenuOpen(false);
    setExporting(kind);
    setError(null);
    const originalViewId = currentViewId;
    try {
      let exportWorkspace = workspace;
      if (semanticDirty || layoutDirty) {
        const saved = await persist("导出前保存可视化工作区");
        if (!saved) return;
        exportWorkspace = saved;
      }

      const structured = kind === "markdown" || kind === "json"
        ? await exportCaseVisual({ caseId, workspaceId: exportWorkspace.id, format: kind })
        : null;
      const extension = kind === "markdown" ? "md" : kind;
      const defaultPath = structured?.filename
        ?? `${safeExportStem(`${graph.title}-${kind === "png" ? currentView?.title ?? "当前视图" : "案情可视化"}`)}.${extension}`;
      const savePath = await save({
        defaultPath,
        filters: [{
          name: kind === "png" ? "PNG 图片" : kind === "pdf" ? "PDF 文档" : kind === "markdown" ? "Markdown" : "JSON",
          extensions: [extension],
        }],
      });
      if (!savePath) return;

      let mimeType: "image/png" | "application/pdf" | "text/markdown" | "application/json";
      let dataBase64: string;
      if (kind === "png") {
        mimeType = "image/png";
        dataBase64 = dataUrlBase64(await captureCurrentView(), mimeType);
      } else if (kind === "pdf") {
        mimeType = "application/pdf";
        const captures: string[] = [];
        for (const view of graph.views) {
          setCurrentViewId(view.id);
          setSelectedNodeId(null);
          await waitForVisualRender();
          captures.push(await captureCurrentView());
        }
        dataBase64 = await buildVisualPdfBase64(
          createVisualCoverDataUrl(graph, exportWorkspace.revision),
          captures,
        );
      } else {
        mimeType = kind === "markdown" ? "text/markdown" : "application/json";
        dataBase64 = textToBase64(structured?.content ?? "");
      }

      const written = await writeCaseVisualExport({
        savePath,
        format: kind,
        mimeType,
        dataBase64,
      });
      await revealInFinder(written).catch(() => {});
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      if (kind === "pdf") {
        setCurrentViewId(originalViewId);
        await waitForVisualRender();
      }
      setExporting(null);
    }
  }

  async function applyProposal(proposal: VisualProposal, patch: CaseGraphPatch) {
    if (semanticDirty || layoutDirty) {
      setError("请先保存当前人工修改，再应用 AI 变更建议");
      return;
    }
    setProposalBusy(true);
    setError(null);
    try {
      const merged = await resolveCaseVisualProposal({
        caseId,
        workspaceId: workspace.id,
        proposalId: proposal.id,
        action: "accept",
        acceptedPatch: patch,
      });
      adoptWorkspace(merged);
      setProposals((current) => current.filter((item) => item.id !== proposal.id));
      await loadProposals(merged.id);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "应用 AI 变更建议失败");
      await loadProposals(workspace.id);
    } finally {
      setProposalBusy(false);
    }
  }

  async function rejectProposal(proposal: VisualProposal) {
    setProposalBusy(true);
    setError(null);
    try {
      await resolveCaseVisualProposal({
        caseId,
        workspaceId: workspace.id,
        proposalId: proposal.id,
        action: "reject",
      });
      setProposals((current) => current.filter((item) => item.id !== proposal.id));
      await loadProposals(workspace.id);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "拒绝 AI 变更建议失败");
    } finally {
      setProposalBusy(false);
    }
  }

  async function refreshWorkspace() {
    if (semanticDirty || layoutDirty) {
      setError("请先保存当前人工修改，再刷新工作区");
      return;
    }
    setProposalBusy(true);
    setError(null);
    try {
      const latest = await getCaseVisualWorkspace(caseId);
      if (!latest) throw new Error("可视化工作区不存在");
      adoptWorkspace(latest);
      await loadProposals(latest.id);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "刷新可视化工作区失败");
    } finally {
      setProposalBusy(false);
    }
  }

  async function requestAiChange(request: string) {
    if (semanticDirty || layoutDirty) {
      setError("请先保存当前人工修改，再让 AI 调整视图");
      return;
    }
    if (!onRequestAiChange) {
      setError("当前无法连接案件 AI 助手");
      return;
    }
    setAiBusy(true);
    setError(null);
    try {
      await onRequestAiChange(request);
      const latest = await getCaseVisualWorkspace(caseId);
      if (!latest) throw new Error("AI 修改完成后未找到可视化工作区");
      adoptWorkspace(latest);
      setProposals([]);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "AI 调整视图失败");
    } finally {
      setAiBusy(false);
    }
  }

  function renderSidePanel() {
    if (activeProposal && activePanel === "proposal") {
      return (
        <ProposalReviewPanel
          proposal={activeProposal}
          currentGraph={graph}
          applying={proposalBusy}
          onApply={(patch) => applyProposal(activeProposal, patch)}
          onReject={() => rejectProposal(activeProposal)}
          onRefresh={refreshWorkspace}
        />
      );
    }
    if (currentView && activePanel === "settings") {
      return <ViewSettingsPanel view={currentView} onChange={changeView} />;
    }
    if (currentView && activePanel === "ai") {
      return <AiVisualAdjustPanel view={currentView} busy={aiBusy} onSubmit={requestAiChange} />;
    }
    return <NodeInspector node={selectedNode} onChange={changeNode} onOpenSource={onOpenSource} />;
  }

  function renderCurrentView() {
    if (!currentView) {
      return <div className="flex h-full items-center justify-center text-sm text-muted-foreground">工作区还没有视图</div>;
    }
    if (currentView.kind === "timeline") {
      return (
        <LegalTimelineView
          graph={graph}
          view={currentView}
          selectedNodeId={selectedNodeId}
          onSelectNode={setSelectedNodeId}
        />
      );
    }
    if (currentView.kind === "relationship" || currentView.kind === "mindmap") {
      return (
        <CaseGraphCanvas
          graph={graph}
          view={currentView}
          positions={positionsByView[currentView.id]}
          onSelectNode={setSelectedNodeId}
          onPositionsChange={(positions) => {
            setPositionsByView((current) => ({ ...current, [currentView.id]: positions }));
            setLayoutDirty(true);
          }}
        />
      );
    }
    if (currentView.kind === "evidence_matrix") {
      return <EvidenceMatrixView graph={graph} view={currentView} onSelectNode={setSelectedNodeId} />;
    }
    if (currentView.kind === "bar_table") {
      return currentDataset ? (
        <BarTableView dataset={currentDataset} view={currentView} />
      ) : (
        <div className="flex h-full items-center justify-center text-sm text-muted-foreground">当前视图引用的数据集不存在</div>
      );
    }
    return currentDataset ? (
      <QuantitativeChartView dataset={currentDataset} view={currentView} />
    ) : (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">当前视图引用的数据集不存在</div>
    );
  }

  return (
    <section className="fixed inset-0 z-[140] flex min-h-[100dvh] flex-col bg-background text-foreground">
      <header className="flex min-h-14 shrink-0 flex-wrap items-center justify-between gap-2 border-b border-border bg-background px-3 py-2 sm:h-14 sm:flex-nowrap sm:gap-3 sm:px-4 sm:py-0">
        <div className="flex w-full min-w-0 items-center gap-3 sm:w-auto">
          <Button type="button" variant="ghost" size="sm" onClick={requestClose} aria-label="返回案件页">
            <ArrowLeft className="size-4" />
            返回案件
          </Button>
          <div className="h-5 w-px bg-border" aria-hidden />
          <div className="min-w-0">
            <h1 className="truncate text-sm font-semibold">{graph.title}</h1>
            <p className="truncate text-[11px] text-muted-foreground">
              案情可视化工作台，修订 {workspace.revision}
            </p>
          </div>
        </div>
        <div className="flex w-full items-center gap-1.5 overflow-x-auto sm:w-auto">
          <Button
            type="button"
            variant={activePanel === "node" ? "secondary" : "outline"}
            size="sm"
            onClick={() => openPanel("node")}
            aria-pressed={activePanel === "node"}
          >
            <FileSearch className="size-3.5" />
            节点编辑
          </Button>
          <Button
            type="button"
            variant={activePanel === "settings" ? "secondary" : "outline"}
            size="sm"
            onClick={() => openPanel("settings")}
            aria-pressed={activePanel === "settings"}
            disabled={!currentView}
          >
            <SlidersHorizontal className="size-3.5" />
            展示设置
          </Button>
          <Button
            type="button"
            variant={activePanel === "ai" ? "secondary" : "outline"}
            size="sm"
            onClick={() => openPanel("ai")}
            aria-pressed={activePanel === "ai"}
            disabled={!currentView || !onRequestAiChange || aiBusy}
          >
            {aiBusy ? <LoaderCircle className="size-3.5 animate-spin" /> : <Sparkles className="size-3.5" />}
            让 AI 调整
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="text-destructive hover:border-destructive/30 hover:bg-destructive/8 hover:text-destructive"
            onClick={deleteCurrentView}
            disabled={!currentView || saving}
          >
            <Trash2 className="size-3.5" />
            删除当前图表
          </Button>
          <Button type="button" variant="ghost" size="sm" disabled={past.length === 0} onClick={undo} aria-label="撤销">
            <Undo2 className="size-3.5" />
            撤销
          </Button>
          <Button type="button" variant="ghost" size="sm" disabled={future.length === 0} onClick={redo} aria-label="重做">
            <Redo2 className="size-3.5" />
            重做
          </Button>
          {currentView && ["relationship", "mindmap"].includes(currentView.kind) && (
            <Button
              ref={exportButtonRef}
              type="button"
              variant="outline"
              size="sm"
              onClick={() => {
                setPositionsByView((current) => ({ ...current, [currentView.id]: {} }));
                setLayoutDirty(true);
              }}
            >
              <GitBranch className="size-3.5" />
              重新布局
            </Button>
          )}
          <div className="relative">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={saving || exporting !== null}
              onClick={toggleExportMenu}
              aria-expanded={exportMenuOpen}
              aria-haspopup="menu"
            >
              {exporting ? <LoaderCircle className="size-3.5 animate-spin" /> : <Download className="size-3.5" />}
              {exporting ? "导出中" : "导出"}
              {!exporting && <ChevronDown className="size-3" />}
            </Button>
            {exportMenuOpen && (
              <div
                className="fixed z-[180] w-44 rounded-md border border-border bg-card p-1 shadow-lg"
                style={{ top: exportMenuPosition.top, left: exportMenuPosition.left }}
                role="menu"
              >
                {([
                  ["png", "当前视图 PNG"],
                  ["pdf", "全部视图 PDF"],
                  ["markdown", "复核稿 Markdown"],
                  ["json", "结构数据 JSON"],
                ] as const).map(([kind, label]) => (
                  <button
                    key={kind}
                    type="button"
                    role="menuitem"
                    className="w-full rounded px-2 py-1.5 text-left text-xs text-foreground hover:bg-accent"
                    onClick={() => void runExport(kind)}
                  >
                    {label}
                  </button>
                ))}
              </div>
            )}
          </div>
          <Button
            type="button"
            size="sm"
            disabled={saving || (!semanticDirty && !layoutDirty)}
            onClick={() => void persist("律师编辑可视化工作区")}
          >
            {saving ? <LoaderCircle className="size-3.5 animate-spin" /> : saveState === "saved" ? <Check className="size-3.5" /> : <Save className="size-3.5" />}
            {saving ? "保存中" : saveState === "saved" ? "已保存" : "保存修改"}
          </Button>
        </div>
      </header>

      {error && (
        <div className="shrink-0 border-b border-destructive/20 bg-destructive/8 px-4 py-2 text-xs text-destructive">
          {error}
        </div>
      )}

      <div className="grid min-h-0 flex-1 grid-cols-1 grid-rows-[auto_minmax(0,1fr)] lg:grid-cols-[220px_minmax(0,1fr)] lg:grid-rows-1 xl:grid-cols-[220px_minmax(0,1fr)_310px]">
        <nav className="flex min-h-0 items-center gap-2 overflow-x-auto border-b border-border bg-surface-muted px-2 py-2 lg:block lg:overflow-y-auto lg:border-b-0 lg:border-r lg:py-3" aria-label="可视化视图">
          <div className="flex shrink-0 items-center gap-2 px-2 lg:mb-2">
            <Sparkles className="size-3.5 text-brand" />
            <span className="text-xs font-semibold">视图</span>
          </div>
          <div className="flex gap-1 lg:block lg:space-y-1">
            {graph.views.map((view) => (
              <button
                key={view.id}
                type="button"
                onClick={() => {
                  setCurrentViewId(view.id);
                  setSelectedNodeId(null);
                }}
                className={cn(
                  "w-auto min-w-28 shrink-0 rounded-md px-2.5 py-2 text-left transition-colors lg:w-full lg:min-w-0",
                  currentView?.id === view.id ? "bg-background text-foreground shadow-sm" : "text-muted-foreground hover:bg-background/70 hover:text-foreground",
                )}
                aria-label={`${view.title}，${viewKindLabel(view)}`}
              >
                <span className="block truncate text-xs font-medium">{view.title}</span>
                <span className="mt-0.5 block text-[10px]">{viewKindLabel(view)}</span>
              </button>
            ))}
          </div>
          <div className="mt-5 hidden border-t border-border px-2 pt-3 lg:block">
            <p className="text-[10px] font-medium text-muted-foreground">事实状态</p>
            <div className="mt-2 space-y-1.5">
              {(["confirmed", "our_claim", "opponent_claim", "disputed", "inferred", "unknown"] as const).map((status) => {
                const visual = statusVisual(status);
                return (
                  <div key={status} className="flex items-center gap-2 text-[10px] text-muted-foreground">
                    <span className="flex size-4 items-center justify-center rounded border text-[9px]" style={{ color: visual.color, borderColor: visual.color, borderStyle: visual.borderStyle }}>
                      {visual.marker}
                    </span>
                    {visual.label}
                  </div>
                );
              })}
            </div>
          </div>
        </nav>

        <main ref={canvasRef} className="relative min-h-0 overflow-hidden bg-background" aria-label={currentView?.title ?? "可视化画布"}>
          <Suspense
            fallback={
              <div className="flex h-full items-center justify-center gap-2 text-sm text-muted-foreground">
                <LoaderCircle className="size-4 animate-spin" />
                正在准备视图…
              </div>
            }
          >
            <VisualizationErrorBoundary resetKey={currentView?.id ?? "empty"}>
              {renderCurrentView()}
            </VisualizationErrorBoundary>
          </Suspense>
        </main>

        {panelOpen && (
          <button
            type="button"
            className="fixed inset-0 z-[150] bg-black/15 xl:hidden"
            aria-label="关闭工作面板遮罩"
            onClick={() => setPanelOpen(false)}
          />
        )}
        <div
          data-testid="visual-panel-drawer"
          className={cn(
            "fixed bottom-0 right-0 top-14 z-[160] min-h-0 w-[min(360px,calc(100vw-20px))] bg-background shadow-2xl xl:static xl:z-auto xl:flex xl:w-auto xl:shadow-none",
            panelOpen ? "flex" : "hidden",
          )}
        >
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="absolute right-2 top-2 z-10 px-2 xl:hidden"
            onClick={() => setPanelOpen(false)}
            aria-label="关闭工作面板"
          >
            <X className="size-4" />
          </Button>
          <Suspense fallback={<aside className="h-full border-l border-border bg-surface-muted" />}>
            {renderSidePanel()}
          </Suspense>
        </div>
      </div>
    </section>
  );
}

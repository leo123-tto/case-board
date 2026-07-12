import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";

import { MarkdownModal } from "@/components/MarkdownModal";
import { SourceDocumentViewerDrawer } from "@/components/SourceDocumentViewerDrawer";
import { SettingsModal, type SettingsTab } from "@/components/SettingsModal";
import { OnboardingWizard } from "@/components/OnboardingWizard";
import { DeepSeekBalanceChip } from "@/components/DeepSeekBalanceChip";
import { FeedbackButton } from "@/components/FeedbackButton";
import { ModuleTabs } from "@/components/ModuleTabs";
// 私人专属功能接缝(双轨发布模型):开源仓返回 [] → 无「独立」顶层 tab。
import { getPrivateTopTabs } from "@/private";
import { HomeView, type UpcomingEvent } from "@/components/HomeView";
import { HomeDropZone } from "@/components/HomeDropZone";
import { isCriminalCase, splitCasesByDomain } from "@/lib/caseDomain";
import { RunningTaskOverlay } from "@/components/RunningTaskOverlay";
import { RunningTaskProvider } from "@/contexts/RunningTaskContext";
import { UpdateAvailableDialog } from "@/components/UpdateAvailableDialog";
import { UpdateSuccessDialog } from "@/components/UpdateSuccessDialog";
import { consumeJustUpdated, type PendingUpdate } from "@/lib/updater";
import { VersionChip } from "@/components/VersionChip";
import { toast, dismissToast, ToastViewport } from "@/components/ui/toast";
import { TransactionModule } from "@/modules/transaction";
import { ToolsModule } from "@/modules/tools";
import { MemoryModule } from "@/modules/memory/MemoryModule";
import type { InterestPrefill } from "@/modules/tools/calculators/InterestCalculator";
import { TeamModule } from "@/modules/team/TeamModule";
import { ExecutionModule } from "@/modules/execution";
import { CaseView } from "@/modules/litigation/components/CaseView";
import { DetachedChatWindow } from "@/modules/litigation/components/chat/DetachedChatWindow";
import { EmptyState } from "@/modules/litigation/components/EmptyState";
import { ProgressBanner } from "@/modules/litigation/components/ProgressBanner";
import { confirmDialog } from "@/lib/dialog";
import { useFeatureFlag } from "@/lib/featureFlags";
import {
  checkForUpdate,
  deleteCase,
  getCaseWithDocs,
  getSettings,
  generateClosingMaterials,
  globalExtractCase,
  importCaseFolder,
  planImportFolder,
  commitImportFolder,
  listCases,
  findFeishuCasePath,
  openInDefaultApp,
  refreshCaseFiles,
  relinkCaseFolder,
  revealInFinder,
  setDocumentDisplayName,
} from "@/lib/api";
import {
  type Case,
  type DocOcrStatusEvent,
  type Document,
  type ImportPlan,
  type ProgressEvent,
  type UpdateInfo,
} from "@/lib/types";
import { SplitImportDialog } from "@/components/SplitImportDialog";

function readChatWindowParams(): {
  caseId: string | null;
  caseName: string | null;
  domain: "civil" | "criminal";
} | null {
  try {
    const params = new URLSearchParams(window.location.search);
    if (params.get("window") === "chat") {
      return {
        caseId: params.get("caseId"),
        caseName: params.get("caseName"),
        domain: params.get("domain") === "criminal" ? "criminal" : "civil",
      };
    }
    // 2026-06-23 v0.3.26.1 Windows fix:WebviewUrl::App("index.html?...")
    // 在 Windows NTFS 上 PathBuf 规范化会把 `?` 后的 query 剥掉,detached 窗口拿不到
    // window=chat 参数 → App.tsx 退到 MainApp,用户看到空界面。改走 Rust 端
    // WebviewWindowBuilder.initialization_script 注入 window.__CHAT_INIT__ 做兜底。
    const init = (window as unknown as {
      __CHAT_INIT__?: {
        caseId?: string | null;
        caseName?: string | null;
        domain?: string;
        detached?: boolean;
      };
    }).__CHAT_INIT__;
    if (init && init.detached) {
      return {
        caseId: init.caseId ?? null,
        caseName: init.caseName ?? null,
        domain: init.domain === "criminal" ? "criminal" : "civil",
      };
    }
    return null;
  } catch {
    return null;
  }
}

function App() {
  const chatWindow = readChatWindowParams();
  if (chatWindow) {
    return (
      <DetachedChatWindow
        caseId={chatWindow.caseId}
        caseName={chatWindow.caseName}
        domain={chatWindow.domain}
      />
    );
  }

  return <MainApp />;
}

function MainApp() {
  /** 全部已入库案件(按 updated_at 倒序) */
  const [cases, setCases] = useState<Case[]>([]);
  /** 当前选中案件 ID */
  const [selectedId, setSelectedId] = useState<string | null>(null);
  /** 当前选中案件的完整数据(case + docs),从 DB 读 */
  const [selectedCase, setSelectedCase] = useState<Case | null>(null);
  const [documents, setDocuments] = useState<Document[]>([]);

  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // 多案件拆分预案(检测到一个文件夹含多个案件时弹确认弹窗)
  const [splitPlan, setSplitPlan] = useState<ImportPlan | null>(null);
  /** 当前打开的文档预览(点击 AI 产物或可读文档时弹) */
  const [previewDoc, setPreviewDoc] = useState<Document | null>(null);
  /** 源文件看板 Phase 1:当前在板内查看器抽屉打开的源文件(MD/原件双视图) */
  const [viewerDoc, setViewerDoc] = useState<Document | null>(null);
  /**
   * V0.3 D1+D2 · 写作模式:当前在 Milkdown 编辑器里打开的文书(null = 看板模式)。
   * 仅 chat_artifact 文书(is_ai_artifact + category∈文书类型)可进编辑器。切案件重置。
   */
  const [editingDoc, setEditingDoc] = useState<Document | null>(null);
  /** 案件分析报告弹窗(点详情页 📖 按钮触发,渲染 case_report_path 的 MD) */
  const [reportModalCase, setReportModalCase] = useState<Case | null>(null);
  /** 报告抽取中(没现成报告时点按钮,触发 globalExtractCase) */
  const [reportLoading, setReportLoading] = useState(false);
  /** 线下归档用结案材料生成中 */
  const [closingMaterialsLoading, setClosingMaterialsLoading] = useState(false);
  /** 2026-05-25 · 工具模块预填(从执行案件「算执行款」跳过来时带数据:本金/起算日/还款记录)*/
  const [toolsRoute, setToolsRoute] = useState<{
    tool: "interest" | "courtfiling" | null;
    interestPrefill: InterestPrefill | null;
    /** 自增 nonce:即使 tool 不变也强制 ToolsModule 重新打开(用于「重复跳转」) */
    nonce: number;
  }>({ tool: null, interestPrefill: null, nonce: 0 });
  /**
   * 2026-05-25 V0.1.8 · 设置 page 是否有未保存改动(从 SettingsModal page 模式上报)。
   * 切别的 tab 时会先 confirm,避免静默丢修改。
   */
  const [settingsDirty, setSettingsDirty] = useState(false);
  /** 2026-05-25 V0.1.8 · 当前 App 版本(从 Tauri API 拿,等同 Cargo.toml CARGO_PKG_VERSION) */
  const [appVer, setAppVer] = useState<string>("");
  /** 2026-05-25 V0.1.8 · 远程版本检测结果(启动时静默 fetch + 用户手动检查会更新) */
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  /** 2026-05-25 V0.1.8 · 是否弹「发现新版本」对话框 */
  const [showUpdateDialog, setShowUpdateDialog] = useState(false);
  // 应用内更新重启后弹一次「升级成功」
  const [justUpdated, setJustUpdated] = useState<PendingUpdate | null>(null);
  /** 后台抽取进度(每个 case_id 对应一份独立进度) */
  const [progress, setProgress] = useState<ProgressEvent | null>(null);
  // 单文档云端 OCR 轮询子状态(独立 state,不混进 progress 以免每拍重算把进度条闪回 0%)
  const [ocrSub, setOcrSub] = useState<DocOcrStatusEvent | null>(null);
  /** 视图模式:home = 案件看板首页, detail = 单案件详情。默认 home。(仅诉讼模块用) */
  const [view, setView] = useState<"home" | "detail">("home");
  /** 是否正在跑 reaggregate_all_cases(详情页"重新计算画像"按钮触发) */
  /**
   * 2026-05-24 b:顶部三模块 tab(诉讼 / 非诉 / 工具)。默认诉讼。
   * 各模块完全独立 — 切到非诉/工具不影响诉讼的 cases/selectedId 等 state。
   */
  // string 而非 ModuleId:私人专属顶层 tab(「独立」)的 id 由接缝动态提供,开源仓为空。
  const [activeModule, setActiveModule] = useState<string>("litigation");
  /**
   * F2(2026-06-18):刚导入案件的 id —— 用来「识别为刑事案件后自动切到刑事 tab」。
   * 刑事案件被 civilCases 过滤掉,导入后若不切 tab 会在诉讼 tab「看不见」;但导入瞬间
   * 罪名等 agg 字段尚未抽出,故记下 id,等抽取完成回调里再判一次刑事并切换(一次性)。
   * 限定只对「刚导入的这一个」生效,避免用户手动切回诉讼时被弹回刑事 tab。
   */
  const justImportedCaseRef = useRef<string | null>(null);
  /** 进度条最小化状态(作者 2026-05-23 晚十:文件多时不挡其他东西) */
  const [progressMinimized, setProgressMinimized] = useState(false);
  /**
   * Onboarding 向导是否打开。
   * 首次启动检测 settings.json 里 cloud_enabled 是否决定过(用是否有 token / endpoint 之类的字段判定),
   * 没决定就强制弹,选完才能用 App。
   * 2026-05-23 作者隐私分流决策 — 见 docs/产品决策与理念.md 第 2 节。
   */
  const [showOnboarding, setShowOnboarding] = useState(false);
  /** 用户显示称呼(用于首页问候) */
  const [userDisplayName, setUserDisplayName] = useState<string | null>(null);
  /**
   * 案件详情页 — 编辑模式开关。
   *
   * P1 (V0.1.13) 起,案件详情页右上角"齿轮"已改成"铅笔":点了进入编辑模式,
   * 字段可改 / 卡片可拖 / 表格行可删(P3 接 UI)。每次切案件自动重置回 false。
   */
  const [isEditMode, setIsEditMode] = useState(false);
  /**
   * 2026-05-24 e:LLM provider 是否走云端 + DeepSeek key 是否就绪。
   * 用来决定 ModuleTabs 右侧是否显示 DeepSeekBalanceChip。
   */
  const [showDeepSeekChip, setShowDeepSeekChip] = useState(false);

  // 首次启动检测是否需要 onboarding + 判断是否显示 DeepSeek chip
  useEffect(() => {
    getSettings()
      .then((s) => {
        setUserDisplayName(s.user_display_name);
        // setup_completed 标志位是 onboarding 唯一可信凭证 —— 完成 wizard 时显式置 true。
        if (!s.setup_completed) {
          setShowOnboarding(true);
        }
        // 2026-05-27 简化:**只要填了 DeepSeek API key 就显示余额 chip**。
        //
        // 之前还要 llm_provider==='cloud' 才显示,但这导致两种 false-negative:
        //   1. 用户选 LLM=local 但仍填了 cloud key 备用 — 看不到余额变化
        //   2. 用户 OCR cloud + LLM local — 实际不调 DeepSeek,但用户可能想看余额
        //   3. chat 功能跑起来后,即便 effective provider 是 local,
        //      用户也可能临时切回 cloud 跑生成任务
        // 老板 2026-05-27 反馈:同事配了 key 但 chip 不显示,核心问题是判定太严格。
        const hasDeepSeekKey =
          !!s.cloud_llm_api_key && s.cloud_llm_api_key.trim().length > 0;
        setShowDeepSeekChip(hasDeepSeekKey);
      })
      .catch((err) => console.error("加载 settings 失败:", err));
  }, []);

  // 2026-05-25 V0.1.8 · 启动:拿当前版本 + 静默检测远程版本(失败不报错)
  useEffect(() => {
    getVersion()
      .then(setAppVer)
      .catch(() => {});
    // 应用内更新重启后:命中则弹「升级成功 + 更新内容」(只弹一次)
    consumeJustUpdated()
      .then((p) => {
        if (p) setJustUpdated(p);
      })
      .catch(() => {});
    // 2026-06-15 私人自用包防误更新:编译期设 VITE_NO_UPDATE_CHECK=1 → 跳过启动自动检查更新,
    // 不再弹「发现新版本」。背景:私人自用包带专属功能(「独立」tab),却和公开版共用同一个
    // lawtools.top/latest.json;公开发版后版本号更高,会把私人版自动更新成公开版、丢掉专属功能
    // (作者就这么误装过)。公开构建不设此变量 → 照常检查,公开用户正常收到更新。
    // 手动点右下角版本 chip 仍可主动检查,不受影响。
    if (import.meta.env.VITE_NO_UPDATE_CHECK !== "1") {
      checkForUpdate()
        .then((info) => {
          setUpdateInfo(info);
          // 2026-06-11 反馈:每个新版本只自动弹一次,不要每次启动都弹
          // (开源用户基于旧版二改的,疯狂弹窗会严重打扰)。弹过的版本号记
          // localStorage;下次远程版本没变就不再弹;发了更新的版本再弹一次。
          // 用户仍可随时点右下角版本 chip 主动查看更新。
          const PROMPTED_KEY = "caseboard.update_prompted_version";
          if (info.has_update && info.latest) {
            let prompted: string | null = null;
            try {
              prompted = localStorage.getItem(PROMPTED_KEY);
            } catch {
              /* localStorage 不可用就退回每次弹 */
            }
            if (prompted !== info.latest) {
              setShowUpdateDialog(true);
              try {
                localStorage.setItem(PROMPTED_KEY, info.latest);
              } catch {
                /* 存不进就下次再弹,无伤 */
              }
            }
          }
        })
        .catch(() => {
          // 静默失败:断网 / CDN 抽风都不打扰
        });
    }
  }, []);

  // 切 tab 包装:从设置 tab 切走时,如果有未保存改动,先 confirm
  const setActiveModuleSafe = useCallback(
    async (target: string) => {
      if (activeModule === "settings" && target !== "settings" && settingsDirty) {
        const ok = await confirmDialog(
          "设置里有未保存的改动,切走会丢失这些改动 — 确定继续吗?",
          { danger: true, okLabel: "继续切换" },
        );
        if (!ok) return;
        setSettingsDirty(false); // 用户确认了,清掉脏标记
      }
      setActiveModule(target);
    },
    [activeModule, settingsDirty],
  );

  // 2026-06-16 · 进入设置时初始落在哪个 tab(默认通用;导入缺 LLM key 深链到大脑)
  const [settingsInitialTab, setSettingsInitialTab] = useState<
    SettingsTab | undefined
  >(undefined);

  // 语义化别名 — 所有"打开设置"的入口走这条(过去是 setShowSettings(true) 弹 modal)
  // 普通打开 → 落默认 tab(通用)
  const openSettings = useCallback(() => {
    setSettingsInitialTab(undefined);
    setActiveModuleSafe("settings");
  }, [setActiveModuleSafe]);
  // 深链到指定 tab(导入缺 key → 大脑)
  const openSettingsTab = useCallback(
    (tab: SettingsTab) => {
      setSettingsInitialTab(tab);
      setActiveModuleSafe("settings");
    },
    [setActiveModuleSafe],
  );

  // 案件详情页「开始立案」检测到环境没装好时,会派发此事件 → 跳到法律工具的
  // 「辅助在线立案」标签页(那里能一键装环境)。用全局事件避免深层 prop 钻透。
  useEffect(() => {
    const handler = () => {
      setToolsRoute((r) => ({ tool: "courtfiling", interestPrefill: null, nonce: r.nonce + 1 }));
      void setActiveModuleSafe("tools");
    };
    window.addEventListener("caseboard:open-filing-env", handler);
    return () => window.removeEventListener("caseboard:open-filing-env", handler);
  }, [setActiveModuleSafe]);

  // onboarding / settings 修改完后,刷新 userDisplayName + DeepSeek chip 判断。
  // 2026-05-27 跟启动逻辑对齐:只要填了 key 就显示 chip(详见上面 useEffect 注释)。
  // 之前这里还在用 isCloud 严格判断,导致同事 onboarding 选 local + 设置里补填 cloud key
  // 后,关掉设置面板触发本回调,chip 就被错误隐藏。
  const refreshUserDisplayName = useCallback(() => {
    getSettings()
      .then((s) => {
        setUserDisplayName(s.user_display_name);
        const hasDeepSeekKey =
          !!s.cloud_llm_api_key && s.cloud_llm_api_key.trim().length > 0;
        setShowDeepSeekChip(hasDeepSeekKey);
      })
      .catch(console.error);
  }, []);

  // 订阅后台抽取进度事件
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    listen<ProgressEvent>("extraction_progress", (event) => {
      // OCR 轮询子状态:单独存,不替换 progress(否则主进度条 percent 每拍重算闪回 0%)
      if (event.payload.stage === "doc_ocr_status") {
        setOcrSub(event.payload);
        return;
      }
      setProgress(event.payload);
      // 任何主进度事件到来都清掉上一份 OCR 子状态(那份只在某文档轮询期间有意义)
      setOcrSub(null);
      // 处理完成后:刷新当前案件(case 表的 agg_* + documents 列表)+ 5 秒后清进度条
      if (event.payload.stage === "completed") {
        // ⭐ 2026-05-23 晚十 修 bug:之前只刷 documents 没刷 case,导致 selectedCase.agg_computed_at 一直空,详情页"正在抽取"占位不消失
        if (selectedId && event.payload.case_id === selectedId) {
          getCaseWithDocs(selectedId)
            .then((r) => {
              setSelectedCase(r.case);
              setDocuments(r.documents);
              // F2:刚导入的案件,抽取完成后罪名等字段就位 → 若识别为刑事且还没切过,平滑切到刑事 tab
              //(一次性:无论是否刑事都清掉标记,避免后续重抽再触发 / 把用户困在刑事 tab)。
              if (justImportedCaseRef.current === r.case.id) {
                justImportedCaseRef.current = null;
                if (isCriminalCase(r.case)) {
                  void setActiveModuleSafe("criminal");
                }
              }
            })
            .catch(() => {});
        }
        // 同时刷案件列表(首页卡片要更新)
        listCases().then(setCases).catch(() => {});
        window.setTimeout(() => setProgress(null), 5000);
      }
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch((e) => console.warn("listen extraction_progress failed", e));
    return () => {
      if (unlisten) unlisten();
    };
    // selectedId 变了也要重新订阅
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId]);

  // 老版本升级 / 新装用户:后端启动时自动创建了本地知识库 → 弹一次提示,
  // 让用户知道「越用越省钱」(法规/案例自动入库 + 本地命中)已开启。
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    listen<string>("local-kb-auto-created", () => {
      toast("已为你创建本地知识库,以后查到的法规 / 案例会自动入库,越用越省钱", "success");
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch((e) => console.warn("listen local-kb-auto-created failed", e));
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  // 启动时从 DB 拉取已有案件(不自动跳转到任何案件,默认停在首页)
  useEffect(() => {
    let cancelled = false;
    listCases()
      .then((all) => {
        if (cancelled) return;
        setCases(all);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // 选中案件变化时拉详情 + 重置编辑模式(切案件别带状态过来)
  useEffect(() => {
    setIsEditMode(false);
    // V0.3 D1+D2 · 切案件重置写作模式,否则 A 案的编辑器带进 B 案
    setEditingDoc(null);
    if (!selectedId) {
      setSelectedCase(null);
      setDocuments([]);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    getCaseWithDocs(selectedId)
      .then((r) => {
        if (cancelled) return;
        setSelectedCase(r.case);
        setDocuments(r.documents);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedId]);

  // 防呆:导入前先检查云端档 API key 是否齐全且验证通过。返回 true=可导入。
  // 2026-05-26 V0.1.11 补强:之前只查 key 非空,导致老用户从旧版升级后 key 填了"1"
  // 没验证通过却仍能导入(然后批量失败)。现在加 verified_at 检查,**未验证一律拦下**。
  // V0.3:本地模型已隐藏 → 只走云端,这里恒按云端校验(与后端 effective_*=cloud 一致,
  // 同时消化老用户 ocr/llm_provider="local" 残留,避免前端漏检→后端却走云端而失败的错位)。
  const validateImportKeys = useCallback(async (): Promise<boolean> => {
    const s = await getSettings();

    type Issue = { label: string; reason: "missing" | "unverified" };
    const issues: Issue[] = [];

    {
      const filled = !!s.mineru_api_key?.trim();
      const verified = !!s.mineru_verified_at;
      if (!filled) {
        issues.push({ label: "MinerU API Token(云端 OCR)", reason: "missing" });
      } else if (!verified) {
        issues.push({ label: "MinerU API Token(云端 OCR)", reason: "unverified" });
      }
    }
    {
      // 2026-06-15/16:按云端后端校验对应的 key,与后端 effective_cloud_llm_backend 三选一对齐
      // (minimax / 通用兼容 glm·mimo·custom / 其余回落 DeepSeek)。各后端 key 字段独立。
      const backend = s.cloud_llm_backend ?? "deepseek";
      const isMinimax = backend === "minimax";
      const isCompat = ["glm", "mimo", "custom"].includes(backend);
      const compatKey =
        backend === "glm"
          ? s.glm_llm_api_key || s.compat_llm_api_key
          : backend === "mimo"
            ? s.mimo_llm_api_key || s.compat_llm_api_key
            : backend === "custom"
              ? s.custom_llm_api_key || s.compat_llm_api_key
              : s.compat_llm_api_key;
      const compatVerifiedAt =
        backend === "glm"
          ? s.glm_llm_verified_at || s.compat_llm_verified_at
          : backend === "mimo"
            ? s.mimo_llm_verified_at || s.compat_llm_verified_at
            : backend === "custom"
              ? s.custom_llm_verified_at || s.compat_llm_verified_at
              : s.compat_llm_verified_at;
      const filled = isMinimax
        ? !!s.minimax_api_key?.trim()
        : isCompat
          ? !!compatKey?.trim()
          : !!s.cloud_llm_api_key?.trim();
      const verified = isMinimax
        ? !!s.minimax_verified_at
        : isCompat
          ? !!compatVerifiedAt
          : !!s.deepseek_verified_at;
      const providerName = isMinimax
        ? "MiniMax"
        : isCompat
          ? { glm: "智谱 GLM", mimo: "小米 MiMo", custom: "自定义模型" }[backend] ??
            "云端模型"
          : "DeepSeek";
      const label = `${providerName} API Key(云端 LLM)`;
      if (!filled) {
        issues.push({ label, reason: "missing" });
      } else if (!verified) {
        issues.push({ label, reason: "unverified" });
      }
    }

    if (issues.length > 0) {
      const lines = issues.map(
        (i) =>
          `${i.label}${i.reason === "missing" ? "(还未填写)" : "(已填写但未通过验证)"}`,
      );
      // toast(z-200 在设置面板之上,不会被盖住)+ 自动打开设置面板引导补填
      toast(
        `无法导入:${lines.join(";")}。已为你打开设置,填好并验证后再导入。`,
        "error",
        7000,
      );
      // 缺的是云端 LLM key(a92ae91 校验),深链到「大脑」tab 直接补填
      openSettingsTab("brain");
      return false;
    }
    return true;
  }, [openSettingsTab]);

  // 单个文件夹 → 单个案件导入(保底路径,或拆分确认后的「合并成 1 个」)。失败给 toast。
  const importSingle = useCallback(async (path: string) => {
    setLoading(true);
    setError(null);
    try {
      const result = await importCaseFolder(path);
      const all = await listCases();
      setCases(all);
      setSelectedId(result.case.id);
      setView("detail");
      // F2:刑事案件文件夹导入 → 切到刑事 tab(否则被 civilCases 过滤掉、在诉讼 tab 看不见)。
      // 记下 id,抽取完成回调再判一次(导入瞬间罪名等字段可能还没抽出);名字含「刑」/罪名时此处即可判。
      justImportedCaseRef.current = result.case.id;
      if (isCriminalCase(result.case)) {
        justImportedCaseRef.current = null;
        void setActiveModuleSafe("criminal");
      }
      toast(
        result.is_existing
          ? `已重新扫描 · 共 ${result.docs.length} 份文档`
          : `已导入 · 共 ${result.docs.length} 份文档`,
        "success",
      );
      if (result.scan_warnings.length > 0) {
        toast(
          `导入已继续，但有 ${result.scan_warnings.length} 项未能完整扫描：${result.scan_warnings.slice(0, 3).join("；")}`,
          "error",
          12000,
        );
      }
    } catch (e) {
      setError(String(e));
      toast(`导入失败:${e}`, "error", 7000);
    } finally {
      setLoading(false);
    }
  }, []);

  // 拖拽 / 选目录后的入口:先做多案件检测,检测到多案就弹拆分确认,否则单案导入。
  const doImport = useCallback(
    async (path: string) => {
      setError(null);
      try {
        const plan = await planImportFolder(path);
        if (plan.multi && plan.cases.length >= 2) {
          setSplitPlan(plan); // 弹拆分确认弹窗,后续走 confirmSplit / mergeAll
          return;
        }
      } catch (e) {
        // 检测失败不阻断:退回单案导入(保底)
        console.warn("plan_import_folder 失败,退回单案导入", e);
      }
      await importSingle(path);
    },
    [importSingle],
  );

  // 拆分确认:按用户勾选的案件批量建案,跳到第一个案件。
  const confirmSplit = useCallback(
    async (
      root: string,
      cases: { dir: string; name: string }[],
      sharedDirs: string[],
    ) => {
      setLoading(true);
      setError(null);
      try {
        const results = await commitImportFolder(root, cases, sharedDirs);
        const all = await listCases();
        setCases(all);
        if (results[0]) {
          setSelectedId(results[0].case.id);
          setView("detail");
          // F2:拆分导入后,若首个案件已可判为刑事则切刑事 tab;否则记 id 等抽取完成再判。
          justImportedCaseRef.current = results[0].case.id;
          if (isCriminalCase(results[0].case)) {
            justImportedCaseRef.current = null;
            void setActiveModuleSafe("criminal");
          }
        }
        setSplitPlan(null);
        toast(`已拆成 ${results.length} 个案件导入`, "success");
        const scanWarnings = [...new Set(results.flatMap((result) => result.scan_warnings))];
        if (scanWarnings.length > 0) {
          toast(
            `拆分导入已继续，但有 ${scanWarnings.length} 项未能完整扫描：${scanWarnings.slice(0, 3).join("；")}`,
            "error",
            12000,
          );
        }
      } catch (e) {
        setError(String(e));
        toast(`拆分导入失败:${e}`, "error", 7000);
      } finally {
        setLoading(false);
      }
    },
    [],
  );

  // 拆分弹窗里选「合并成 1 个案件」:走保底单案导入。
  const mergeAllAsSingle = useCallback(
    async (root: string) => {
      setSplitPlan(null);
      await importSingle(root);
    },
    [importSingle],
  );

  // 点「导入案件」按钮:校验 key → 弹系统选目录器 → 导入。
  const handleImport = useCallback(async () => {
    if (!(await validateImportKeys())) return;
    const selected = await open({
      directory: true,
      multiple: false,
      title: "选择案件文件夹",
    });
    if (typeof selected !== "string") return;
    await doImport(selected);
  }, [validateImportKeys, doImport]);

  // 点飞书日历事件后导入对应文件夹:先按事件标题反查飞书案件池里的本地路径,
  // 反查不到再弹文件夹选择器。(整合外部贡献 PR #9,gcheng-001)
  const handleCalendarImport = useCallback(
    async (eventTitle: string) => {
      if (!(await validateImportKeys())) return;
      try {
        const localPath = await findFeishuCasePath(eventTitle);
        if (localPath) {
          await doImport(localPath);
          return;
        }
      } catch (e) {
        console.warn("findFeishuCasePath failed:", e);
      }
      const selected = await open({
        directory: true,
        multiple: false,
        title: `选择「${eventTitle}」的案件文件夹`,
      });
      if (typeof selected === "string") {
        await doImport(selected);
      }
    },
    [validateImportKeys, doImport],
  );

  // 首页拖拽文件夹进来:校验 key → 直接导入拖入的路径(走和按钮同一条管线)。
  const handleDropImport = useCallback(
    async (path: string) => {
      if (!(await validateImportKeys())) return;
      await doImport(path);
    },
    [validateImportKeys, doImport],
  );

  /**
   * 文档点击行为:文本类弹 markdown 预览,非文本类用系统默认应用打开。
   * 错误时不在主页面打断,console.warn 即可(下次可以加 toast)。
   */
  const handleOpenDoc = useCallback((doc: Document) => {
    // 源文件看板 Phase 1(2026-06-19):真实导入的源文件(非 AI 产物)→ 板内查看器抽屉
    // (「处理后 MD / 原件」双 tab,板内看 PDF/图片、MD 失真时切原件核对)。
    // AI 产物 / 报告 / chat 文书仍走 MarkdownModal —— 它们 MD-native,「原件」tab 无意义。
    if (!doc.is_ai_artifact) {
      setViewerDoc(doc);
      return;
    }
    const isOfficeDoc =
      /\.(docx?|rtf|odt)$/i.test(doc.filename) ||
      /wordprocessingml|msword|opendocument\.text|rtf/i.test(doc.mime_type ?? "");
    if (isOfficeDoc) {
      openInDefaultApp(doc.source_path).catch((e) => {
        console.warn("open_in_default_app failed", e);
        setError(String(e));
      });
      return;
    }
    // 2026-05-31 · 抽取成功的文件(PDF/扫描件/docx 等)点击优先看「处理后的文本(MD)」
    // —— 这正是 AI 实际读到的内容,也方便核对抽取质量;原件仍可用行尾「在 Finder 打开」。
    // 见下方 MarkdownModal 的 previewExtractedPath 逻辑。
    const hasExtracted =
      doc.extraction_status === "done" && !!doc.extracted_text_path;
    // App 内预览能力覆盖的文件类型:
    //   .md/.markdown/.txt   → react-markdown
    //   .html/.htm           → iframe sandbox
    //   .docx/.doc/.rtf/.odt → 上面已交给系统默认应用
    // 其他(.pdf/.png/...)原本走系统默认应用;现在抽取成功的也能 App 内看处理后文本。
    const isPreviewable = /\.(md|markdown|html?|txt)$/i.test(doc.filename);
    if (hasExtracted || isPreviewable) {
      setPreviewDoc(doc);
      return;
    }
    openInDefaultApp(doc.source_path).catch((e) => {
      console.warn("open_in_default_app failed", e);
      setError(String(e));
    });
  }, []);

  const handleRevealDoc = useCallback((doc: Document) => {
    revealInFinder(doc.source_path).catch((e) => {
      console.warn("reveal_in_finder failed", e);
      setError(String(e));
    });
  }, []);

  const handleRevealCase = useCallback(() => {
    if (!selectedCase) return;
    revealInFinder(selectedCase.source_folder).catch((e) => {
      console.warn("reveal case folder failed", e);
      setError(String(e));
    });
  }, [selectedCase]);

  /**
   * 删除当前案件。需要先弹原生 confirm 确认。
   * 删除后切到列表第一个案件,如果列表空了回到 EmptyState。
   */
  const handleDeleteCase = useCallback(async () => {
    if (!selectedCase) return;
    const confirmed = await confirmDialog(
      `确定要从看板删除「${selectedCase.name}」吗?\n\n` +
        `只删 CaseBoard 数据库里的记录,你的原始文件夹「${selectedCase.source_folder}」不会动,以后还可以重新导入。`,
      { danger: true, okLabel: "删除案件" },
    );
    if (!confirmed) return;

    try {
      await deleteCase(selectedCase.id);
      const all = await listCases();
      setCases(all);
      setSelectedId(all.length > 0 ? all[0].id : null);
      toast("已从看板删除(原始文件夹未动)", "success");
    } catch (e) {
      setError(String(e));
    }
  }, [selectedCase]);

  /**
   * 首页右键卡片「删除」:按 id 删任意案件(不依赖当前选中)。同样先弹原生 confirm。
   * 删的若正是当前选中案件,则把选中重置到列表第一个(或清空)。
   */
  const handleDeleteCaseById = useCallback(
    async (id: string) => {
      const target = cases.find((c) => c.id === id);
      if (!target) return;
      const confirmed = await confirmDialog(
        `确定要从看板删除「${target.name}」吗?\n\n` +
          `只删 CaseBoard 数据库里的记录,你的原始文件夹「${target.source_folder}」不会动,以后还可以重新导入。`,
        { danger: true, okLabel: "删除案件" },
      );
      if (!confirmed) return;
      try {
        await deleteCase(id);
        const all = await listCases();
        setCases(all);
        setSelectedId((prev) =>
          prev === id ? (all.length > 0 ? all[0].id : null) : prev,
        );
        toast("已从看板删除(原始文件夹未动)", "success");
      } catch (e) {
        setError(String(e));
        toast(`删除失败:${e}`, "error", 6000);
      }
    },
    [cases],
  );

  /**
   * 首页「多选」批量删除:一次确认 → 逐个删 → 刷新一次。只删库记录,不动原始文件夹。
   */
  const handleDeleteCases = useCallback(
    async (ids: string[]) => {
      if (ids.length === 0) return;
      const names = ids
        .map((id) => cases.find((c) => c.id === id)?.name)
        .filter((n): n is string => !!n);
      const preview =
        names.slice(0, 5).join("、") +
        (names.length > 5 ? ` 等 ${names.length} 个` : "");
      const confirmed = await confirmDialog(
        `确定要从看板删除选中的 ${ids.length} 个案件吗?\n\n${preview}\n\n` +
          `只删 CaseBoard 数据库里的记录,你的原始文件夹不会动,以后还可以重新导入。`,
        { danger: true, okLabel: `删除 ${ids.length} 个案件` },
      );
      if (!confirmed) return;
      let deleted = 0;
      try {
        for (const id of ids) {
          await deleteCase(id);
          deleted += 1;
        }
        toast(`已删除 ${ids.length} 个案件(原始文件夹未动)`, "success");
      } catch (e) {
        setError(String(e));
        toast(
          `批量删除中断:成功 ${deleted}/${ids.length} 个,错误:${e}`,
          "error",
          7000,
        );
      } finally {
        // 无论成功/中断都刷新一次,反映已删的部分
        const all = await listCases();
        setCases(all);
        setSelectedId((prev) =>
          prev && ids.includes(prev) ? (all.length > 0 ? all[0].id : null) : prev,
        );
      }
    },
    [cases],
  );

  /**
   * 2026-05-24 i:打开案件分析报告。
   * - 如果当前案件已经有 case_report_path,直接弹 MarkdownModal
   * - 如果没有(还没跑过全局抽),先调 globalExtractCase → 等完成 → 刷新 case → 弹 Modal
   */
  const handleOpenReport = useCallback(async () => {
    if (!selectedCase) return;
    // 已有报告:直接弹
    if (selectedCase.case_report_path) {
      setReportModalCase(selectedCase);
      return;
    }
    // 没有报告 → 触发抽取
    setReportLoading(true);
    // 长任务态:duration=0 不自动消失,finally 里 dismiss
    const reportToastId = toast("正在生成案件报告(~ 10-30 秒)…", "info", 0);
    try {
      const r = await globalExtractCase(selectedCase.id);
      if (r.error) {
        setError(`报告生成失败:${r.error}`);
        return;
      }
      // 刷新案件拿新 case_report_path
      const fresh = await getCaseWithDocs(selectedCase.id);
      setSelectedCase(fresh.case);
      setDocuments(fresh.documents);
      // 顺便更新 cases 列表里的对应项
      setCases((prev) => prev.map((c) => (c.id === fresh.case.id ? fresh.case : c)));
      if (fresh.case.case_report_path) {
        setReportModalCase(fresh.case);
        if (r.warning) {
          toast(r.warning, "info", 8000);
        }
        toast(`报告生成完成 · ${(r.elapsed_ms / 1000).toFixed(1)} 秒`, "success");
      } else {
        setError("报告生成完成,但未找到报告文件");
      }
    } catch (e) {
      setError(`报告生成失败:${e}`);
    } finally {
      setReportLoading(false);
      dismissToast(reportToastId);
    }
  }, [selectedCase]);

  const handleGenerateClosingMaterials = useCallback(async () => {
    if (!selectedCase || closingMaterialsLoading) return;
    setClosingMaterialsLoading(true);
    const toastId = toast("正在生成结案归档材料要素…", "info", 0);
    try {
      const doc = await generateClosingMaterials(selectedCase.id);
      setPreviewDoc(doc);
      if (selectedId) {
        const r = await getCaseWithDocs(selectedId);
        setSelectedCase(r.case);
        setDocuments(r.documents);
        setCases((prev) => prev.map((c) => (c.id === r.case.id ? r.case : c)));
      }
      toast("已生成结案材料,可直接复制粘贴到线下归档表格", "success");
    } catch (e) {
      toast(`生成结案材料失败:${e}`, "error", 7000);
    } finally {
      setClosingMaterialsLoading(false);
      dismissToast(toastId);
    }
  }, [selectedCase, selectedId, closingMaterialsLoading]);

  /** 是否正在跑刷新源文件(disable 按钮防重复点) */
  const [refreshingFiles, setRefreshingFiles] = useState(false);
  const [referenceMaterialsEnabled] = useFeatureFlag("reference_materials");

  const handleRelinkCase = useCallback(async () => {
    if (!selectedCase || refreshingFiles) return;
    const selected = await open({
      directory: true,
      multiple: false,
      title: "重新选择这个案件的源文件夹",
    });
    if (typeof selected !== "string") return;
    setRefreshingFiles(true);
    try {
      const stats = await relinkCaseFolder(
        selectedCase.id,
        selected,
        referenceMaterialsEnabled,
      );
      const fresh = await getCaseWithDocs(selectedCase.id);
      setSelectedCase(fresh.case);
      setDocuments(fresh.documents);
      setCases((prev) => prev.map((c) => (c.id === fresh.case.id ? fresh.case : c)));
      const parts = [`已重新关联`];
      if (stats.moved > 0) parts.push(`识别移动 ${stats.moved}`);
      if (stats.added > 0) parts.push(`新增 ${stats.added}`);
      if (stats.updated > 0) parts.push(`更新 ${stats.updated}`);
      if (stats.deleted > 0) parts.push(`移除 ${stats.deleted}`);
      toast(parts.join(" · "), "success");
      if (stats.scan_warnings.length > 0) {
        toast(
          `重新关联已继续，但有 ${stats.scan_warnings.length} 项未能完整扫描：${stats.scan_warnings.slice(0, 3).join("；")}`,
          "error",
          12000,
        );
      }
      setError(null);
    } catch (e) {
      setError(`重新关联失败: ${e}`);
    } finally {
      setRefreshingFiles(false);
    }
  }, [selectedCase, refreshingFiles, referenceMaterialsEnabled]);

  /**
   * 2026-05-25 V0.1.5 「🔄 刷新源文件」处理函数。
   *
   * 后端做 diff sync(scan_folder → sync_documents_for_case),立即返回 SyncStats;
   * 如果有 added/updated/deleted,后端会自动 spawn_extraction,前端通过现有的
   * `extraction_progress` 事件订阅看进度 + 完成后自动 reload(跟初次导入复用同一通道)。
   */
  const handleRefreshFiles = useCallback(async () => {
    if (!selectedCase || refreshingFiles) return;
    setRefreshingFiles(true);
    try {
      const stats = await refreshCaseFiles(selectedCase.id, referenceMaterialsEnabled);
      const hasChange =
        stats.added > 0 || stats.updated > 0 || stats.deleted > 0 || stats.moved > 0;
      if (!hasChange) {
        toast(`源文件夹无变化(${stats.unchanged} 份均最新)`, "info");
      } else {
        const parts: string[] = [];
        if (stats.added > 0) parts.push(`新增 ${stats.added}`);
        if (stats.updated > 0) parts.push(`更新 ${stats.updated}`);
        if (stats.deleted > 0) parts.push(`移除 ${stats.deleted}`);
        if (stats.moved > 0) parts.push(`移动 ${stats.moved}`);
        const needsAnalysis = stats.added > 0 || stats.updated > 0 || stats.deleted > 0;
        toast(
          `${parts.join(" · ")}${needsAnalysis ? " · 后台抽取中" : " · 无需重新分析"}`,
          "success",
        );
        // 立刻刷一次文档列表,让前端看到 deleted_at / pending 状态变化
        if (selectedId) {
          try {
            const r = await getCaseWithDocs(selectedId);
            setSelectedCase(r.case);
            setDocuments(r.documents);
          } catch {
            /* 不阻塞 */
          }
        }
      }
      if (stats.scan_warnings.length > 0) {
        toast(
          `刷新已继续，但有 ${stats.scan_warnings.length} 项未能完整扫描：${stats.scan_warnings.slice(0, 3).join("；")}`,
          "error",
          12000,
        );
      }
    } catch (e) {
      setError(`刷新源文件失败: ${e}`);
    } finally {
      setRefreshingFiles(false);
    }
  }, [selectedCase, selectedId, refreshingFiles, referenceMaterialsEnabled]);

  /**
   * 2026-05-27 V0.1.13+ chat artifact 完成后的轻量 reload。
   *
   * 跟 `handleRefreshFiles` 的区别:**不**走 sync_documents_for_case,只重读 DB。
   * 原因:chat artifact 写到 app data(`extracts/<case_id>/chat_artifacts/`),
   * 不在源文件夹里;走 sync 会触发不必要的源文件夹 diff,且 UI 会显示"新增 1"
   * 让人困惑。这里只刷 React 状态,让新 artifact 出现在文档列表。
   */
  const handleReloadCase = useCallback(async () => {
    if (!selectedId) return;
    try {
      const r = await getCaseWithDocs(selectedId);
      setSelectedCase(r.case);
      setDocuments(r.documents);
    } catch {
      /* 不阻塞 */
    }
  }, [selectedId]);

  /**
   * V0.3 D2 · chat 落了 save_artifact 文书后:reload 案件 + **自动进编辑器打开该文书**。
   * docId 空字符串 = 只 reload(后端没回传 id 的兜底,例如老路径)。
   * 用 reload 拿到的 fresh documents 找目标(不能读 setDocuments 后的 state,异步)。
   */
  const handleArtifactCreated = useCallback(
    async (docId: string) => {
      if (!selectedId) return;
      try {
        const r = await getCaseWithDocs(selectedId);
        setSelectedCase(r.case);
        setDocuments(r.documents);
        if (docId) {
          const target = r.documents.find((d) => d.id === docId);
          if (
            target &&
            (target.source === "chat_artifact" || target.source === "chat")
          ) {
            setEditingDoc(target);
          }
        }
      } catch {
        /* 不阻塞 */
      }
    },
    [selectedId],
  );

  /** V0.3 D1+D2 · 在编辑器里打开一份文书(从 MarkdownModal「✏️ 进行编辑」入口) */
  const handleOpenEditor = useCallback((doc: Document) => {
    setPreviewDoc(null);
    setEditingDoc(doc);
  }, []);

  /** V0.3 D1+D2 · 关闭编辑器,回看板模式 */
  const handleCloseEditor = useCallback(() => {
    setEditingDoc(null);
  }, []);

  // 跨平台键盘快捷键
  //   macOS:Cmd+O / Cmd+, / Cmd+R；Windows/Linux:Ctrl+O / Ctrl+, / Ctrl+R
  // 必须在所有 early return 之前(React Hooks 规则:每次 render 调用相同顺序的 hooks)
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (!e.metaKey && !e.ctrlKey) return;
      switch (e.key) {
        case "o":
        case "O":
          e.preventDefault();
          handleImport();
          break;
        case ",":
          e.preventDefault();
          openSettings();
          break;
        case "r":
        case "R":
          // 无论当前是否选中案件,都阻止 WebView 把 Ctrl/Cmd+R 当成整页刷新。
          e.preventDefault();
          if (selectedCase) {
            void handleRefreshFiles();
          }
          break;
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [handleImport, handleRefreshFiles, selectedCase, openSettings]);

  // ========================================================================
  // 所有 hooks 声明完毕,以下可以做条件渲染 / 路由
  // ========================================================================

  // 诉讼模块内部子路由:首页 (HomeView) ↔ 案件详情 (CaseView)
  const pickCase = (caseId: string) => {
    setSelectedId(caseId);
    setView("detail");
  };
  const pickCaseFromHome = (caseId: string) => {
    const target = cases.find((item) => item.id === caseId);
    void setActiveModuleSafe(target && isCriminalCase(target) ? "criminal" : "litigation");
    setSelectedId(caseId);
    setView("detail");
  };
  const handleCaseStatusChanged = useCallback(
    (caseId: string, status: string | null) => {
      const patch = {
        workflow_status: status,
        workflow_status_locked: status == null ? 0 : 1,
      };
      setCases((prev) =>
        prev.map((c) => (c.id === caseId ? { ...c, ...patch } : c)),
      );
      setSelectedCase((prev) =>
        prev?.id === caseId ? { ...prev, ...patch } : prev,
      );
    },
    [],
  );
  const openHomeEvent = (event: UpcomingEvent) => {
    if (!event.caseId) return;
    const target = cases.find((item) => item.id === event.caseId);
    void setActiveModuleSafe(target && isCriminalCase(target) ? "criminal" : "litigation");
    setSelectedId(event.caseId);
    setView("detail");
    if (event.sourceDoc) {
      setViewerDoc(event.sourceDoc);
    }
  };
  const goHome = () => {
    setView("home");
  };

  // 诉讼 / 刑事 共享导入·PDF分类·OCR·全局抽取·case+document 数据层,只按「领域」过滤显示
  //(归类启发式见 src/lib/caseDomain.ts:刑事案件进刑事 tab,其余进诉讼 tab)。
  // selectedId/view 也共享,故详情分支额外校验 selectedId 属于本领域,避免切 tab 串案件。
  const { civil: civilCases, criminal: criminalCases } =
    splitCasesByDomain(cases);

  // 一个案件详情视图的公共 props(诉讼/刑事复用,只换 cases 子集与 domain)。
  const caseViewCommonProps = {
    selectedCase,
    documents,
    loading,
    error,
    onDismissError: () => setError(null),
    onSwitchCase: setSelectedId,
    onGoHome: goHome,
    onOpenDoc: handleOpenDoc,
    onRevealDoc: handleRevealDoc,
    onRevealCase: handleRevealCase,
    isEditMode,
    onToggleEditMode: () => setIsEditMode((v) => !v),
    onDeleteCase: handleDeleteCase,
    onRefreshFiles: handleRefreshFiles,
    onRelinkCase: handleRelinkCase,
    refreshingFiles,
    onOpenReport: handleOpenReport,
    reportLoading,
    onGenerateClosingMaterials: handleGenerateClosingMaterials,
    closingMaterialsLoading,
    onReloadCase: handleReloadCase,
    editingDoc,
    onCloseEditor: handleCloseEditor,
    onArtifactCreated: handleArtifactCreated,
  };

  // 诉讼模块整体渲染:从未导入任何案件→EmptyState / 选中民事案件→CaseView / 否则→全案件 HomeView。
  // 首页两态(EmptyState / HomeView)都包一层 HomeDropZone:拖案件文件夹进来即导入。
  // EmptyState 判据用全量 cases(不是 civilCases):否则只有刑事案件时诉讼 tab 会误显「还没有案件」。
  // 首页是全局入口:民事、刑事、行政等全部案件都要显示。点击刑事案件时再切到刑事详情。
  const litigationBody =
    cases.length === 0 && !loading ? (
      <HomeDropZone onImportPath={handleDropImport}>
        <EmptyState
          onImport={handleImport}
          error={error}
          onOpenSettings={openSettings}
        />
      </HomeDropZone>
    ) : view === "detail" &&
      civilCases.some((c) => c.id === selectedId) ? (
      <CaseView cases={civilCases} domain="civil" {...caseViewCommonProps} />
    ) : (
      <HomeDropZone onImportPath={handleDropImport}>
        <HomeView
          cases={cases}
          userDisplayName={userDisplayName}
          onPickCase={pickCaseFromHome}
          onOpenEvent={openHomeEvent}
          onImport={handleImport}
          onDeleteCase={handleDeleteCaseById}
          onDeleteCases={handleDeleteCases}
          onCaseStatusChanged={handleCaseStatusChanged}
          onImportFolder={handleCalendarImport}
        />
      </HomeDropZone>
    );

  // 刑事模块:复刻诉讼框架,只显示刑事案件;空态文案不同(案件靠自动识别归类,非"从未导入")。
  const criminalBody =
    criminalCases.length === 0 && !loading ? (
      <HomeDropZone onImportPath={handleDropImport}>
        <main className="app-shell app-page-enter flex h-full w-full flex-col items-center justify-center px-6">
          <div className="w-full max-w-md text-center">
            <h1 className="text-2xl font-semibold tracking-tight text-foreground">
              刑事案件
            </h1>
            <p className="mt-3 text-sm text-muted-foreground">
              还没有识别到刑事案件
            </p>
            <p className="mt-3 text-xs leading-relaxed text-muted-foreground/80">
              导入案件文件夹后,系统会按案号(含「刑」)、罪名(含「罪」)、起诉书 / 公诉 /
              被告人等刑事专属信息自动把刑事案件归到这里;民事 / 诉讼案件请在「诉讼」标签查看。
            </p>
            <div className="mt-8 flex justify-center">
              <button
                type="button"
                onClick={handleImport}
                className="inline-flex items-center gap-2 rounded-md bg-foreground px-4 py-2 text-sm font-medium text-background transition-colors hover:bg-foreground/90"
              >
                导入案件文件夹
              </button>
            </div>
          </div>
        </main>
      </HomeDropZone>
    ) : view === "detail" &&
      criminalCases.some((c) => c.id === selectedId) ? (
      <CaseView
        cases={criminalCases}
        domain="criminal"
        {...caseViewCommonProps}
      />
    ) : (
      <HomeDropZone onImportPath={handleDropImport}>
        <HomeView
          cases={criminalCases}
          userDisplayName={userDisplayName}
          onPickCase={pickCase}
          onOpenEvent={openHomeEvent}
          onImport={handleImport}
          onDeleteCase={handleDeleteCaseById}
          onDeleteCases={handleDeleteCases}
          onCaseStatusChanged={handleCaseStatusChanged}
          onImportFolder={handleCalendarImport}
        />
      </HomeDropZone>
    );

  return (
    <div className="app-shell flex h-full w-full flex-col">
      {/* 顶部三模块 tab(诉讼 / 非诉 / 工具)+ 左侧首页按钮 + 右侧 DeepSeek 余额 */}
      <ModuleTabs
        active={activeModule}
        onSwitch={setActiveModuleSafe}
        onGoHome={() => {
          setActiveModuleSafe("litigation");
          setView("home");
          setSelectedId(null);
        }}
        rightSlot={
          <>
            {showDeepSeekChip && <DeepSeekBalanceChip />}
            <FeedbackButton />
          </>
        }
      />

      {/* 模块内容区(flex-1 + min-h-0 让子模块能正常滚动) */}
      <div className="min-h-0 flex-1">
        {activeModule === "litigation" && litigationBody}
        {activeModule === "criminal" && criminalBody}
        {activeModule === "execution" && (
          <ExecutionModule
            onCalculateInterest={(prefill) => {
              setToolsRoute((r) => ({ tool: "interest", interestPrefill: prefill, nonce: r.nonce + 1 }));
              setActiveModuleSafe("tools");
            }}
          />
        )}
        {activeModule === "transaction" && <TransactionModule />}
        {activeModule === "tools" && (
          <ToolsModule
            initialTool={toolsRoute.tool}
            interestPrefill={toolsRoute.interestPrefill}
            routeNonce={toolsRoute.nonce}
          />
        )}
        {activeModule === "memory" && <MemoryModule />}
        {activeModule === "team" && <TeamModule />}
        {activeModule === "settings" && (
          <div className="app-page-enter h-full overflow-auto">
            <SettingsModal
              mode="page"
              initialTab={settingsInitialTab}
              onDirtyChange={setSettingsDirty}
              onClose={refreshUserDisplayName}
              onSaved={refreshUserDisplayName}
            />
          </div>
        )}
        {/* 私人专属顶层 tab(双轨发布模型;开源仓接缝返回 [] → 此分支永不命中) */}
        {getPrivateTopTabs().map(
          (t) =>
            activeModule === t.id && (
              <div
                key={t.id}
                className="app-page-enter h-full overflow-auto px-4 py-5 sm:px-6 xl:px-8 xl:py-6"
              >
                <div className="mx-auto w-full max-w-5xl">{t.render()}</div>
              </div>
            ),
        )}
      </div>

      {/* 全局弹窗 / 浮层 — 跨模块共享 */}
      {/* 源文件看板 Phase 1:源文件查看器抽屉(MD/原件双视图) */}
      {viewerDoc && selectedCase && (
        <SourceDocumentViewerDrawer
          doc={viewerDoc}
          caseFolder={selectedCase.source_folder}
          onClose={() => setViewerDoc(null)}
          onRename={async (docId, name) => {
            try {
              await setDocumentDisplayName(docId, name);
              // 立即更新打开中的查看器标题(reload 拿的是新数组,viewerDoc 仍指旧对象)
              setViewerDoc((prev) =>
                prev && prev.id === docId
                  ? {
                      ...prev,
                      display_name: name,
                      display_name_source: name ? "user" : null,
                    }
                  : prev,
              );
              await handleReloadCase();
            } catch (e) {
              toast(`重命名失败:${e}`, "error");
            }
          }}
        />
      )}
      {previewDoc &&
        (() => {
          // 2026-05-31 · 抽取成功的非文本原件(PDF/扫描件/docx)→ 预览「处理后文本」(extracted_text_path),
          // 不再开原始 PDF;AI 产物 / 原生 .md/.txt 仍看原文件本身(它本就是要展示/编辑/导出的内容)。
          const nativeText = /\.(md|markdown|txt)$/i.test(previewDoc.filename);
          const showExtracted =
            !previewDoc.is_ai_artifact &&
            !nativeText &&
            previewDoc.extraction_status === "done" &&
            !!previewDoc.extracted_text_path;
          const previewPath = showExtracted
            ? previewDoc.extracted_text_path!
            : previewDoc.source_path;
          const previewFilename = showExtracted
            ? `${previewDoc.filename} · 处理后文本`
            : previewDoc.filename;
          return (
            <MarkdownModal
              path={previewPath}
              filename={previewFilename}
              badge={
                previewDoc.is_ai_artifact
                  ? "AI 产物"
                  : showExtracted
                    ? "处理后文本(OCR/抽取)"
                    : (previewDoc.category ?? undefined)
              }
              onClose={() => setPreviewDoc(null)}
          /* 2026-05-27 V0.1.13+:AI 生成的 artifact(LLM 全局抽 / chat)给导出 HTML/Word 能力。
             非 AI 原文件(诉状、合同等)不出导出按钮 — 没意义。 */
          exportMd={
            previewDoc.is_ai_artifact
              ? {
                  mdPath: previewDoc.source_path,
                  title: previewDoc.filename.replace(/\.(md|html?|txt)$/i, ""),
                  // V0.3:只有 save_artifact 正式文书(source='chat_artifact')走「Word(法律格式)」;
                  // 分析类 AI 产物(source='chat')走普通 Word/HTML(法律排版套不上分析报告)。
                  filing:
                    previewDoc.source === "chat_artifact"
                      ? { docId: previewDoc.id }
                      : undefined,
                }
              : undefined
          }
          /* V0.3:AI 写的材料(source='chat' 分析产物 / 'chat_artifact' 起草文书)都能「✏️ 进行编辑」
             → 进 Milkdown 写作模式。**只认这两类 app 自有文档**,不给 scanner 标记的用户原文件
             开编辑(write_editor_doc 原地覆写 source_path,会改用户文件 → 数据丢失)。报告/执行
             模块预览不传 onEdit(只读)。 */
          onEdit={
            previewDoc.source === "chat" ||
            previewDoc.source === "chat_artifact"
              ? () => handleOpenEditor(previewDoc)
              : undefined
          }
            />
          );
        })()}
      {reportModalCase?.case_report_path && (
        <MarkdownModal
          path={reportModalCase.case_report_path}
          filename={`${reportModalCase.name} · 案件分析报告.md`}
          badge="LLM 全局抽"
          onClose={() => setReportModalCase(null)}
          exportCase={{ id: reportModalCase.id, name: reportModalCase.name }}
        />
      )}
      {/* 2026-05-25 V0.1.8 · 左下角版本号 chip + 启动检测发现新版本时弹的更新提示 */}
      {appVer && (
        <VersionChip
          version={appVer}
          updateInfo={updateInfo}
          onCheck={(info) => {
            setUpdateInfo(info);
            // 手动点 chip 三种反馈:有更新弹 dialog / 失败 toast / 已最新 toast
            if (info.has_update) {
              setShowUpdateDialog(true);
            } else if (info.error) {
              toast(`检查更新失败:${info.error}`, "error");
            } else {
              toast(`已是最新版本 v${info.current}`, "success");
            }
          }}
        />
      )}
      {splitPlan && (
        <SplitImportDialog
          plan={splitPlan}
          busy={loading}
          onConfirm={(cs, sd) => confirmSplit(splitPlan.root, cs, sd)}
          onMergeAll={() => mergeAllAsSingle(splitPlan.root)}
          onCancel={() => setSplitPlan(null)}
        />
      )}

      {showUpdateDialog && updateInfo && updateInfo.has_update && (
        <UpdateAvailableDialog
          info={updateInfo}
          onClose={() => setShowUpdateDialog(false)}
        />
      )}
      {justUpdated && (
        <UpdateSuccessDialog
          version={justUpdated.version}
          notes={justUpdated.notes}
          onClose={() => setJustUpdated(null)}
        />
      )}
      {/* 进度条:诉讼 / 刑事 模块详情页 + 当前案件匹配时显示(刑事 tab 同样要有抽取进度,见坑 #20) */}
      {progress &&
        (activeModule === "litigation" || activeModule === "criminal") &&
        view === "detail" &&
        progress.case_id === selectedId && (
          <ProgressBanner
            progress={progress}
            ocrSub={ocrSub && ocrSub.case_id === selectedId ? ocrSub : null}
            minimized={progressMinimized}
            onToggleMinimize={() => setProgressMinimized((v) => !v)}
            onClose={() => {
              setProgress(null);
              setOcrSub(null);
            }}
          />
        )}
      <OnboardingWizard
        open={showOnboarding}
        onComplete={() => {
          setShowOnboarding(false);
          refreshUserDisplayName();
        }}
      />

    </div>
  );
}

/**
 * 2026-05-25 V0.1.7:对外的默认导出加 RunningTaskProvider 包裹,
 * 让全局任务锁状态在 App 任何子组件都能访问(useRunningTask hook)。
 */
function AppWithProviders() {
  return (
    <RunningTaskProvider>
      <App />
      <RunningTaskOverlay />
      <ToastViewport />
    </RunningTaskProvider>
  );
}

export default AppWithProviders;

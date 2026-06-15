import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";

import { MarkdownModal } from "@/components/MarkdownModal";
import { SettingsModal } from "@/components/SettingsModal";
import { OnboardingWizard } from "@/components/OnboardingWizard";
import { DeepSeekBalanceChip } from "@/components/DeepSeekBalanceChip";
import { FeedbackButton } from "@/components/FeedbackButton";
import { ModuleTabs } from "@/components/ModuleTabs";
// 私人专属功能接缝(双轨发布模型):开源仓返回 [] → 无「独立」顶层 tab。
import { getPrivateTopTabs } from "@/private";
import { HomeView } from "@/components/HomeView";
import { HomeDropZone } from "@/components/HomeDropZone";
import { RunningTaskOverlay } from "@/components/RunningTaskOverlay";
import { RunningTaskProvider } from "@/contexts/RunningTaskContext";
import { UpdateAvailableDialog } from "@/components/UpdateAvailableDialog";
import { UpdateSuccessDialog } from "@/components/UpdateSuccessDialog";
import { consumeJustUpdated, type PendingUpdate } from "@/lib/updater";
import { VersionChip } from "@/components/VersionChip";
import { toast, dismissToast, ToastViewport } from "@/components/ui/toast";
import { TransactionModule } from "@/modules/transaction";
import { ToolsModule } from "@/modules/tools";
import type { InterestPrefill } from "@/modules/tools/calculators/InterestCalculator";
import { TeamModule } from "@/modules/team/TeamModule";
import { ExecutionModule } from "@/modules/execution";
import { CaseView } from "@/modules/litigation/components/CaseView";
import { EmptyState } from "@/modules/litigation/components/EmptyState";
import { ProgressBanner } from "@/modules/litigation/components/ProgressBanner";
import { confirmDialog } from "@/lib/dialog";
import {
  checkForUpdate,
  deleteCase,
  getCaseWithDocs,
  getSettings,
  globalExtractCase,
  importCaseFolder,
  planImportFolder,
  commitImportFolder,
  listCases,
  openInDefaultApp,
  refreshCaseFiles,
  revealInFinder,
  findFeishuCasePath,
} from "@/lib/api";
import {
  CLOUD_PROVIDERS,
  type Case,
  type CloudProviderId,
  type DocOcrStatusEvent,
  type Document,
  type ImportPlan,
  type ProgressEvent,
  type UpdateInfo,
} from "@/lib/types";
import { SplitImportDialog } from "@/components/SplitImportDialog";

function App() {
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
  /**
   * V0.3 D1+D2 · 写作模式:当前在 Milkdown 编辑器里打开的文书(null = 看板模式)。
   * 仅 chat_artifact 文书(is_ai_artifact + category∈文书类型)可进编辑器。切案件重置。
   */
  const [editingDoc, setEditingDoc] = useState<Document | null>(null);
  /** 案件分析报告弹窗(点详情页 📖 按钮触发,渲染 case_report_path 的 MD) */
  const [reportModalCase, setReportModalCase] = useState<Case | null>(null);
  /** 报告抽取中(没现成报告时点按钮,触发 globalExtractCase) */
  const [reportLoading, setReportLoading] = useState(false);
  /** 2026-05-25 · 工具模块预填(从执行案件「算执行款」跳过来时带数据:本金/起算日/还款记录)*/
  const [toolsRoute, setToolsRoute] = useState<{
    tool: "interest" | null;
    interestPrefill: InterestPrefill | null;
  }>({ tool: null, interestPrefill: null });
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

  // 语义化别名 — 所有"打开设置"的入口走这条(过去是 setShowSettings(true) 弹 modal)
  const openSettings = useCallback(() => {
    setActiveModuleSafe("settings");
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
    const providerId = (s.cloud_llm_provider ?? "deepseek") as CloudProviderId;
    const legacyMinimax =
      (s.cloud_llm_backend ?? "").trim() === "minimax" && providerId !== "minimax";
    if (legacyMinimax) {
      const filled = !!(s.minimax_api_key?.trim() || s.cloud_llm_api_key?.trim());
      const verified = !!(s.minimax_verified_at || s.deepseek_verified_at);
      if (!filled) {
        issues.push({ label: "MiniMax API Key(云端 LLM)", reason: "missing" });
      } else if (!verified) {
        issues.push({ label: "MiniMax API Key(云端 LLM)", reason: "unverified" });
      }
    } else {
      const provider = CLOUD_PROVIDERS[providerId] ?? CLOUD_PROVIDERS.deepseek;
      const providerKey =
        providerId === "mimo"
          ? s.mimo_api_key || s.cloud_llm_api_key
          : providerId === "glm"
          ? s.glm_api_key || s.cloud_llm_api_key
          : providerId === "custom"
          ? s.custom_api_key || s.cloud_llm_api_key
          : s.deepseek_api_key || s.cloud_llm_api_key;
      const providerVerifiedAt =
        providerId === "mimo"
          ? s.mimo_verified_at || s.deepseek_verified_at
          : providerId === "glm"
          ? s.glm_verified_at || s.deepseek_verified_at
          : providerId === "custom"
          ? s.custom_verified_at || s.deepseek_verified_at
          : s.deepseek_verified_at;
      const filled = !!providerKey?.trim();
      const verified = !!providerVerifiedAt;
      const label = `${provider.label} API Key(云端 LLM)`;
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
      openSettings();
      return false;
    }
    return true;
  }, [openSettings]);

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
      toast(
        result.is_existing
          ? `已重新扫描 · 共 ${result.docs.length} 份文档`
          : `已导入 · 共 ${result.docs.length} 份文档`,
        "success",
      );
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
        }
        setSplitPlan(null);
        toast(`已拆成 ${results.length} 个案件导入`, "success");
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

  // 首页拖拽文件夹进来:校验 key → 直接导入拖入的路径(走和按钮同一条管线)。
  const handleDropImport = useCallback(
    async (path: string) => {
      if (!(await validateImportKeys())) return;
      await doImport(path);
    },
    [validateImportKeys, doImport],
  );

  // 点击飞书日历事件后导入对应文件夹
  const handleCalendarImport = useCallback(
    async (eventTitle: string) => {
      if (!(await validateImportKeys())) return;

      // 先尝试从飞书案件池自动匹配本地路径
      try {
        const localPath = await findFeishuCasePath(eventTitle);
        if (localPath) {
          await doImport(localPath);
          return;
        }
      } catch (e) {
        console.warn("findFeishuCasePath failed:", e);
      }

      // 没有匹配到路径，弹出文件夹选择器
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

  /**
   * 文档点击行为:文本类弹 markdown 预览,非文本类用系统默认应用打开。
   * 错误时不在主页面打断,console.warn 即可(下次可以加 toast)。
   */
  const handleOpenDoc = useCallback((doc: Document) => {
    // 2026-05-31 · 抽取成功的文件(PDF/扫描件/docx 等)点击优先看「处理后的文本(MD)」
    // —— 这正是 AI 实际读到的内容,也方便核对抽取质量;原件仍可用行尾「在 Finder 打开」。
    // 见下方 MarkdownModal 的 previewExtractedPath 逻辑。
    const hasExtracted =
      doc.extraction_status === "done" && !!doc.extracted_text_path;
    // App 内预览能力覆盖的文件类型:
    //   .md/.markdown/.txt   → react-markdown
    //   .html/.htm           → iframe sandbox
    //   .docx/.doc/.rtf/.odt → macOS textutil 抽纯文本
    // 其他(.pdf/.png/...)原本走系统默认应用;现在抽取成功的也能 App 内看处理后文本。
    const isPreviewable = /\.(md|markdown|html?|txt|docx?|rtf|odt)$/i.test(
      doc.filename,
    );
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

  /** 是否正在跑刷新源文件(disable 按钮防重复点) */
  const [refreshingFiles, setRefreshingFiles] = useState(false);

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
      const stats = await refreshCaseFiles(selectedCase.id);
      const hasChange =
        stats.added > 0 || stats.updated > 0 || stats.deleted > 0;
      if (!hasChange) {
        toast(`源文件夹无变化(${stats.unchanged} 份均最新)`, "info");
      } else {
        const parts: string[] = [];
        if (stats.added > 0) parts.push(`新增 ${stats.added}`);
        if (stats.updated > 0) parts.push(`更新 ${stats.updated}`);
        if (stats.deleted > 0) parts.push(`移除 ${stats.deleted}`);
        toast(`${parts.join(" · ")} · 后台抽取中`, "success");
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
    } catch (e) {
      setError(`刷新源文件失败: ${e}`);
    } finally {
      setRefreshingFiles(false);
    }
  }, [selectedCase, selectedId, refreshingFiles]);

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
          if (target) setEditingDoc(target);
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

  // macOS 键盘快捷键
  //   Cmd+O 导入 / Cmd+, 设置 / Cmd+R 重扫
  // 必须在所有 early return 之前(React Hooks 规则:每次 render 调用相同顺序的 hooks)
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (!e.metaKey) return;
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
          if (selectedCase) {
            e.preventDefault();
            handleImport();
          }
          break;
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [handleImport, selectedCase, openSettings]);

  // ========================================================================
  // 所有 hooks 声明完毕,以下可以做条件渲染 / 路由
  // ========================================================================

  // 诉讼模块内部子路由:首页 (HomeView) ↔ 案件详情 (CaseView)
  const pickCase = (caseId: string) => {
    setSelectedId(caseId);
    setView("detail");
  };
  const goHome = () => {
    setView("home");
  };

  // 诉讼模块整体渲染:无案件→EmptyState / 有案件→HomeView 或 CaseView。
  // 首页两态(EmptyState / HomeView)都包一层 HomeDropZone:拖案件文件夹进来即导入。
  const litigationBody =
    cases.length === 0 && !loading ? (
      <HomeDropZone onImportPath={handleDropImport}>
        <EmptyState
          onImport={handleImport}
          error={error}
          onOpenSettings={openSettings}
        />
      </HomeDropZone>
    ) : view === "home" ? (
      <HomeDropZone onImportPath={handleDropImport}>
        <HomeView
          cases={cases}
          userDisplayName={userDisplayName}
          onPickCase={pickCase}
          onImport={handleImport}
          onImportFolder={handleCalendarImport}
        />
      </HomeDropZone>
    ) : (
      <CaseView
        cases={cases}
        selectedCase={selectedCase}
        documents={documents}
        loading={loading}
        error={error}
        onSwitchCase={setSelectedId}
        onGoHome={goHome}
        onOpenDoc={handleOpenDoc}
        onRevealDoc={handleRevealDoc}
        onRevealCase={handleRevealCase}
        isEditMode={isEditMode}
        onToggleEditMode={() => setIsEditMode((v) => !v)}
        onDeleteCase={handleDeleteCase}
        onRefreshFiles={handleRefreshFiles}
        refreshingFiles={refreshingFiles}
        onOpenReport={handleOpenReport}
        reportLoading={reportLoading}
        onReloadCase={handleReloadCase}
        editingDoc={editingDoc}
        onCloseEditor={handleCloseEditor}
        onArtifactCreated={handleArtifactCreated}
      />
    );

  return (
    <div className="flex h-full w-full flex-col bg-background">
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
        {activeModule === "execution" && (
          <ExecutionModule
            onCalculateInterest={(prefill) => {
              setToolsRoute({ tool: "interest", interestPrefill: prefill });
              setActiveModuleSafe("tools");
            }}
          />
        )}
        {activeModule === "transaction" && <TransactionModule />}
        {activeModule === "tools" && (
          <ToolsModule
            initialTool={toolsRoute.tool}
            interestPrefill={toolsRoute.interestPrefill}
          />
        )}
        {activeModule === "team" && <TeamModule />}
        {activeModule === "settings" && (
          <div className="h-full overflow-auto bg-background">
            <SettingsModal
              mode="page"
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
                className="h-full overflow-auto bg-background px-8 py-6"
              >
                <div className="mx-auto w-full max-w-5xl">{t.render()}</div>
              </div>
            ),
        )}
      </div>

      {/* 全局弹窗 / 浮层 — 跨模块共享 */}
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
      {/* 进度条:只在诉讼模块详情页 + 当前案件匹配时显示 */}
      {progress &&
        activeModule === "litigation" &&
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

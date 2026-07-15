import { useCallback, useEffect, useRef, useState } from "react";
import {
  X,
  Loader2,
  ExternalLink,
  Save,
  CheckCircle2,
  XCircle,
  Database,
  FolderOpen,
  Download,
  Upload,
  AlertTriangle,
  Coins,
  RefreshCw,
  Sparkles,
  Trash2,
  Plus,
  Plug,
  Brain,
  Wrench,
  BookText,
  SlidersHorizontal,
  User,
  Palette,
} from "lucide-react";
import { open as dialogOpen, save as dialogSave } from "@tauri-apps/plugin-dialog";
import { confirmDialog } from "@/lib/dialog";
import { todayIsoLocal } from "@/lib/date";

import { Button } from "@/components/ui/button";
import { HoverHint } from "@/components/HoverHint";
import { GroupQrCode } from "@/components/GroupQrCode";
import { KbSemanticIndexCard } from "@/components/KbSemanticIndexCard";
import { DeviceSyncCard } from "@/components/DeviceSyncCard";
import {
  createLocalKb,
  detectKbStatus,
  exportKbToZip,
  getSettings,
  getYuandianBalance,
  getYuandianCreditsOverview,
  importKbFromZip,
  migrateLocalKb,
  pruneYuandianCache,
  openInDefaultApp,
  openUrl,
  parseMcpPaste,
  saveSettings,
  testMcpServer,
  verifyDeepSeekKey,
  verifyMiniMaxKey,
  verifyOpenAICompatKey,
  verifyMinerUKey,
  verifyPaddleVlKey,
  verifyEmbeddingKey,
  verifyYuandianKey,
  type KbConflictStrategy,
  type KbImportResult,
  type KbStatus,
  type CreditsOverview,
  type YuandianBalance,
} from "@/lib/api";
import type { Settings, McpServerConfig } from "@/lib/types";
import { cn } from "@/lib/utils";
import {
  FEATURE_FLAGS,
  getFeatureFlag,
  setFeatureFlag,
  type FeatureFlagName,
} from "@/lib/featureFlags";
import { FONT_SCALE, useFontScale } from "@/lib/uiScale";
import {
  THEMES,
  getThemePreference,
  setThemePreference,
  type ThemeId,
} from "@/lib/theme";

type VerifyStatus = "idle" | "verifying" | "ok" | "fail";
type CompatBackend = "glm" | "mimo" | "kimi" | "custom";
type CompatSettingKey =
  | "glm_llm_endpoint"
  | "glm_llm_model"
  | "glm_llm_api_key"
  | "glm_llm_verified_at"
  | "mimo_llm_endpoint"
  | "mimo_llm_model"
  | "mimo_llm_api_key"
  | "mimo_llm_verified_at"
  | "kimi_llm_endpoint"
  | "kimi_llm_model"
  | "kimi_llm_api_key"
  | "kimi_llm_verified_at"
  | "custom_llm_endpoint"
  | "custom_llm_model"
  | "custom_llm_api_key"
  | "custom_llm_verified_at";
type CompatFieldKind = "endpoint" | "model" | "apiKey" | "verifiedAt";

/** 云端 AI 后端可选项(下拉)。glm/mimo/custom 共用「通用 OpenAI 兼容」配置(compat_llm_*)。 */
const CLOUD_BACKEND_OPTIONS = [
  { id: "deepseek", label: "DeepSeek(默认)" },
  { id: "minimax", label: "MiniMax(M 系列)" },
  { id: "glm", label: "智谱 GLM(OpenAI 兼容)" },
  { id: "mimo", label: "小米 MiMo(OpenAI 兼容)" },
  { id: "kimi", label: "Kimi Coding Plan(OpenAI 兼容)" },
  { id: "custom", label: "自定义(OpenAI 兼容)" },
] as const;

/** OpenAI 兼容后端预设(镜像 Rust llm::providers;切换时预填到 compat_llm_*,均可改)。 */
const COMPAT_PRESETS: Record<
  CompatBackend,
  { label: string; endpoint: string; model: string; applyUrl?: string }
> = {
  glm: {
    label: "智谱 GLM",
    endpoint: "https://open.bigmodel.cn/api/paas/v4/chat/completions",
    model: "glm-4.6",
    applyUrl: "https://open.bigmodel.cn/usercenter/apikeys",
  },
  mimo: {
    label: "小米 MiMo",
    endpoint: "https://token-plan-cn.xiaomimimo.com/v1/chat/completions",
    model: "mimo-v2.5",
  },
  kimi: {
    label: "Kimi Coding Plan",
    endpoint: "https://api.kimi.com/coding/v1/chat/completions",
    model: "kimi-for-coding",
    applyUrl: "https://www.kimi.com/code/console",
  },
  custom: { label: "自定义(OpenAI 兼容)", endpoint: "", model: "" },
};

const COMPAT_FIELD_KEYS: Record<
  CompatBackend,
  {
    endpoint: CompatSettingKey;
    model: CompatSettingKey;
    apiKey: CompatSettingKey;
    verifiedAt: CompatSettingKey;
  }
> = {
  glm: {
    endpoint: "glm_llm_endpoint",
    model: "glm_llm_model",
    apiKey: "glm_llm_api_key",
    verifiedAt: "glm_llm_verified_at",
  },
  mimo: {
    endpoint: "mimo_llm_endpoint",
    model: "mimo_llm_model",
    apiKey: "mimo_llm_api_key",
    verifiedAt: "mimo_llm_verified_at",
  },
  kimi: {
    endpoint: "kimi_llm_endpoint",
    model: "kimi_llm_model",
    apiKey: "kimi_llm_api_key",
    verifiedAt: "kimi_llm_verified_at",
  },
  custom: {
    endpoint: "custom_llm_endpoint",
    model: "custom_llm_model",
    apiKey: "custom_llm_api_key",
    verifiedAt: "custom_llm_verified_at",
  },
};

function isCompatBackend(value: string | null | undefined): value is CompatBackend {
  return value === "glm" || value === "mimo" || value === "kimi" || value === "custom";
}

function compatValue(
  settings: Settings,
  backend: CompatBackend,
  kind: CompatFieldKind,
): string | null {
  const key = COMPAT_FIELD_KEYS[backend][kind];
  const value = settings[key];
  return typeof value === "string" && value.trim() ? value : null;
}

function legacyCompatValue(
  settings: Settings,
  kind: CompatFieldKind,
): string | null {
  const key =
    kind === "apiKey"
      ? "compat_llm_api_key"
      : kind === "verifiedAt"
        ? "compat_llm_verified_at"
        : kind === "model"
          ? "compat_llm_model"
          : "compat_llm_endpoint";
  const value = settings[key];
  return typeof value === "string" && value.trim() ? value : null;
}

function effectiveCompatValue(
  settings: Settings,
  backend: CompatBackend,
  kind: CompatFieldKind,
): string | null {
  return compatValue(settings, backend, kind) || legacyCompatValue(settings, kind);
}

function setStringSetting(
  target: Partial<Settings>,
  key: CompatSettingKey,
  value: string | null,
) {
  target[key] = value as never;
}

/** 设置页底部标签页(按类型归拢散乱配置;详见 docs/设置页重构-分类方案-2026-06-16.md) */
export type SettingsTab =
  | "theme" // 主题:界面配色，独立于功能开关
  | "brain" // 大脑:对话大模型
  | "models" // 功能模型:OCR / Embedding 等调云端 API 的工具型模型
  | "kb" // 知识库:本地法律知识库 + 语义索引
  | "datasource" // 数据源:元典 + 外部 MCP(企查查/万得/北大法宝)
  | "toggles" // 功能开关:首页清爽开关
  | "general"; // 通用:个人信息 / 加群 / 快递100

const SETTINGS_TABS: { id: SettingsTab; label: string; icon: typeof Brain }[] = [
  { id: "general", label: "通用", icon: User },
  { id: "theme", label: "主题", icon: Palette },
  { id: "brain", label: "大脑", icon: Brain },
  { id: "models", label: "功能模型", icon: Wrench },
  { id: "kb", label: "知识库", icon: BookText },
  { id: "datasource", label: "数据源", icon: Database },
  { id: "toggles", label: "功能开关", icon: SlidersHorizontal },
];

interface Props {
  /** modal 模式下必填(用户点 X / 蒙层 / Escape / 保存成功 都调它);page 模式可选 */
  onClose?: () => void;
  /** 2026-05-25 V0.1.8 · 展示形态:modal=弹窗;page=主内容区独立页(去掉 modal shell) */
  mode?: "modal" | "page";
  /** 2026-05-25 V0.1.8 · page 模式上报 dirty 状态,父组件用来在切 tab 时弹未保存提醒 */
  onDirtyChange?: (dirty: boolean) => void;
  /** 2026-05-27 · 保存成功后通知父组件(page 模式不关闭,但 settings 已经落库 ——
   *  父组件需要重判依赖项,比如右上角 DeepSeek 余额 chip 的可见性)。
   *  modal 模式下保存成功直接 onClose,父组件那侧已经会重读 settings,不需要这个。 */
  onSaved?: () => void;
  /** 2026-06-16 · 进入设置时初始落在哪个 tab。默认 "general"(通用);
   *  导入缺 LLM key 跳设置时父组件传 "brain" 深链到大脑。 */
  initialTab?: SettingsTab;
}

/**
 * 用户设置(modal 弹窗 / page 独立页 双形态)。
 *
 * 设计原则(对应 CLAUDE.md 隐私铁律):
 *   - 每个用户填自己的 token,工具不内置任何人的 key
 *   - 顶部有一行明确说明"配置只保存在你本机,不发送任何地方"
 *   - 每个字段附"如何获取/安装"链接
 *   - api_key 用 password input,不在窗口里明文显示
 *
 * 2026-05-25 V0.1.8:加 mode prop。page 模式给「设置 tab」用,modal 模式仍兼容
 * 现有「导入前 token 缺失自动弹」流程,两种形态共用同一份 form 逻辑。
 */
export function SettingsModal({
  onClose,
  mode = "modal",
  onDirtyChange,
  onSaved,
  initialTab,
}: Props) {
  const isPage = mode === "page";
  const handleClose = () => {
    if (onClose) onClose();
  };
  const [qrHover, setQrHover] = useState(false);
  const qrCloseTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const handleQrEnter = useCallback(() => {
    if (qrCloseTimerRef.current) {
      clearTimeout(qrCloseTimerRef.current);
      qrCloseTimerRef.current = null;
    }
    setQrHover(true);
  }, []);
  const handleQrLeave = useCallback(() => {
    qrCloseTimerRef.current = setTimeout(() => setQrHover(false), 200);
  }, []);
  useEffect(
    () => () => {
      if (qrCloseTimerRef.current) clearTimeout(qrCloseTimerRef.current);
    },
    [],
  );
  const [settings, setSettings] = useState<Settings | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false); // page 模式下保存成功显示 toast(modal 模式直接关闭)
  // 2026-05-25 V0.1.8 · 是否有未保存改动(page 模式上报给父组件做切 tab 防呆)
  const [dirty, setDirty] = useState(false);
  // 设置页里的界面开关先暂存，跟后端设置一起在「保存」时落盘。
  // 不能直接用 useFeatureFlag：它会绕过本页 dirty 状态并立即写 localStorage。
  const [featureFlagDraft, setFeatureFlagDraft] = useState<
    Partial<Record<FeatureFlagName, boolean>>
  >(() =>
    Object.fromEntries(
      FEATURE_FLAGS.filter((flag) => flag.location === "settings").map((flag) => [
        flag.name,
        getFeatureFlag(flag.name),
      ]),
    ),
  );
  // 2026-06-16 · 设置页内部标签页。默认「通用」(作者要求点开设置先看通用);
  // 导入缺 LLM key 跳设置时父组件传 initialTab="brain" 深链到大脑(a92ae91 校验的是 LLM key)。
  const [tab, setTab] = useState<SettingsTab>(initialTab ?? "general");

  // 2026-05-25 V0.1.6 · token 在线验证状态
  const [mineruStatus, setMineruStatus] = useState<VerifyStatus>("idle");
  const [mineruMsg, setMineruMsg] = useState<string>("");
  // 2026-06-12 · PaddleOCR VL(AI Studio)访问令牌验证状态
  const [paddleStatus, setPaddleStatus] = useState<VerifyStatus>("idle");
  const [paddleMsg, setPaddleMsg] = useState<string>("");
  const [deepseekStatus, setDeepseekStatus] = useState<VerifyStatus>("idle");
  const [deepseekMsg, setDeepseekMsg] = useState<string>("");
  // 2026-06-15 · MiniMax API key 在线验证状态
  const [minimaxStatus, setMinimaxStatus] = useState<VerifyStatus>("idle");
  const [minimaxMsg, setMinimaxMsg] = useState<string>("");
  // 2026-06-16 · 通用 OpenAI 兼容后端(GLM/MiMo/自定义)在线验证状态
  const [compatStatus, setCompatStatus] = useState<VerifyStatus>("idle");
  const [compatMsg, setCompatMsg] = useState<string>("");
  // 2026-05-25 V0.1.8 · 元典 API key 在线验证状态
  const [yuandianStatus, setYuandianStatus] = useState<VerifyStatus>("idle");
  const [yuandianMsg, setYuandianMsg] = useState<string>("");
  const [embeddingStatus, setEmbeddingStatus] = useState<VerifyStatus>("idle");
  const [embeddingMsg, setEmbeddingMsg] = useState<string>("");

  // settings 加载完后,如果 verified_at 非空,初始化为 "ok"(从 DB 读出来的已验证状态)
  useEffect(() => {
    if (!settings) return;
    if (settings.mineru_verified_at && mineruStatus === "idle") {
      setMineruStatus("ok");
    }
    if (settings.paddle_vl_verified_at && paddleStatus === "idle") {
      setPaddleStatus("ok");
    }
    if (settings.deepseek_verified_at && deepseekStatus === "idle") {
      setDeepseekStatus("ok");
    }
    if (settings.minimax_verified_at && minimaxStatus === "idle") {
      setMinimaxStatus("ok");
    }
    const backend = isCompatBackend(settings.cloud_llm_backend)
      ? settings.cloud_llm_backend
      : null;
    const compatVerifiedAt = backend
      ? effectiveCompatValue(settings, backend, "verifiedAt")
      : null;
    if (compatVerifiedAt && compatStatus === "idle") {
      setCompatStatus("ok");
    }
    if (settings.yuandian_verified_at && yuandianStatus === "idle") {
      setYuandianStatus("ok");
    }
    if (settings.embedding_verified_at && embeddingStatus === "idle") {
      setEmbeddingStatus("ok");
    }
    // 只在初次加载时设
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    settings?.mineru_verified_at,
    settings?.paddle_vl_verified_at,
    settings?.deepseek_verified_at,
    settings?.minimax_verified_at,
    settings?.cloud_llm_backend,
    settings?.compat_llm_verified_at,
    settings?.glm_llm_verified_at,
    settings?.mimo_llm_verified_at,
    settings?.kimi_llm_verified_at,
    settings?.custom_llm_verified_at,
    settings?.yuandian_verified_at,
  ]);

  async function handleVerifyMineru() {
    if (!settings?.mineru_api_key?.trim()) {
      setMineruStatus("fail");
      setMineruMsg("请先填入 Token");
      return;
    }
    setMineruStatus("verifying");
    setMineruMsg("");
    try {
      const r = await verifyMinerUKey(settings.mineru_api_key);
      if (r.ok) {
        setMineruStatus("ok");
        setMineruMsg("");
        updateField("mineru_verified_at", new Date().toISOString());
      } else {
        setMineruStatus("fail");
        setMineruMsg(r.message);
        updateField("mineru_verified_at", null);
      }
    } catch (e) {
      setMineruStatus("fail");
      setMineruMsg(String(e));
      updateField("mineru_verified_at", null);
    }
  }

  async function handleVerifyPaddle() {
    if (!settings?.paddle_vl_api_key?.trim()) {
      setPaddleStatus("fail");
      setPaddleMsg("请先填入访问令牌");
      return;
    }
    setPaddleStatus("verifying");
    setPaddleMsg("");
    try {
      const r = await verifyPaddleVlKey(settings.paddle_vl_api_key);
      if (r.ok) {
        setPaddleStatus("ok");
        setPaddleMsg("");
        updateField("paddle_vl_verified_at", new Date().toISOString());
      } else {
        setPaddleStatus("fail");
        setPaddleMsg(r.message);
        updateField("paddle_vl_verified_at", null);
      }
    } catch (e) {
      setPaddleStatus("fail");
      setPaddleMsg(String(e));
      updateField("paddle_vl_verified_at", null);
    }
  }

  async function handleVerifyDeepSeek() {
    if (!settings?.cloud_llm_api_key?.trim()) {
      setDeepseekStatus("fail");
      setDeepseekMsg("请先填入 API Key");
      return;
    }
    setDeepseekStatus("verifying");
    setDeepseekMsg("");
    try {
      const r = await verifyDeepSeekKey(
        settings.cloud_llm_api_key,
        settings.cloud_llm_endpoint ?? undefined,
      );
      if (r.ok) {
        setDeepseekStatus("ok");
        setDeepseekMsg("");
        updateField("deepseek_verified_at", new Date().toISOString());
      } else {
        setDeepseekStatus("fail");
        setDeepseekMsg(r.message);
        updateField("deepseek_verified_at", null);
      }
    } catch (e) {
      setDeepseekStatus("fail");
      setDeepseekMsg(String(e));
      updateField("deepseek_verified_at", null);
    }
  }

  async function handleVerifyMiniMax() {
    if (!settings?.minimax_api_key?.trim()) {
      setMinimaxStatus("fail");
      setMinimaxMsg("请先填入 API Key");
      return;
    }
    setMinimaxStatus("verifying");
    setMinimaxMsg("");
    try {
      const r = await verifyMiniMaxKey(
        settings.minimax_api_key,
        settings.minimax_endpoint ?? undefined,
      );
      if (r.ok) {
        setMinimaxStatus("ok");
        setMinimaxMsg("");
        updateField("minimax_verified_at", new Date().toISOString());
      } else {
        setMinimaxStatus("fail");
        setMinimaxMsg(r.message);
        updateField("minimax_verified_at", null);
      }
    } catch (e) {
      setMinimaxStatus("fail");
      setMinimaxMsg(String(e));
      updateField("minimax_verified_at", null);
    }
  }

  async function handleVerifyCompat() {
    if (!settings || !isCompatBackend(settings.cloud_llm_backend)) return;
    const keys = COMPAT_FIELD_KEYS[settings.cloud_llm_backend];
    const apiKey = effectiveCompatValue(settings, settings.cloud_llm_backend, "apiKey") || "";
    const endpoint = effectiveCompatValue(settings, settings.cloud_llm_backend, "endpoint") || "";
    const model = effectiveCompatValue(settings, settings.cloud_llm_backend, "model") || "";
    if (!apiKey.trim()) {
      setCompatStatus("fail");
      setCompatMsg("请先填入 API Key");
      return;
    }
    setCompatStatus("verifying");
    setCompatMsg("");
    try {
      const r = await verifyOpenAICompatKey(
        apiKey,
        endpoint,
        model,
      );
      if (r.ok) {
        setCompatStatus("ok");
        setCompatMsg("");
        updateField(keys.verifiedAt, new Date().toISOString());
      } else {
        setCompatStatus("fail");
        setCompatMsg(r.message);
        updateField(keys.verifiedAt, null);
      }
    } catch (e) {
      setCompatStatus("fail");
      setCompatMsg(String(e));
      updateField(keys.verifiedAt, null);
    }
  }

  // 切换云端 AI 后端。deepseek→存 null(默认);minimax→"minimax";glm/mimo/custom→预填兼容配置。
  function handleChangeBackend(value: string) {
    if (value === "minimax") {
      updateField("cloud_llm_backend", "minimax");
      return;
    }
    if (isCompatBackend(value)) {
      const preset = COMPAT_PRESETS[value];
      const keys = COMPAT_FIELD_KEYS[value];
      const patch: Partial<Settings> = { cloud_llm_backend: value };
      if (!settings || !effectiveCompatValue(settings, value, "endpoint")) {
        setStringSetting(patch, keys.endpoint, preset.endpoint || null);
      }
      if (!settings || !effectiveCompatValue(settings, value, "model")) {
        setStringSetting(patch, keys.model, preset.model || null);
      }
      updateFields(patch);
      setCompatStatus("idle");
      setCompatMsg("");
      return;
    }
    updateField("cloud_llm_backend", null); // deepseek / 默认
  }

  async function handleVerifyYuandian() {
    if (!settings?.yuandian_api_key?.trim()) {
      setYuandianStatus("fail");
      setYuandianMsg("请先填入 API Key");
      return;
    }
    setYuandianStatus("verifying");
    setYuandianMsg("");
    try {
      const r = await verifyYuandianKey(settings.yuandian_api_key);
      if (r.ok) {
        setYuandianStatus("ok");
        setYuandianMsg("");
        updateField("yuandian_verified_at", new Date().toISOString());
      } else {
        setYuandianStatus("fail");
        setYuandianMsg(r.message);
        updateField("yuandian_verified_at", null);
      }
    } catch (e) {
      setYuandianStatus("fail");
      setYuandianMsg(String(e));
      updateField("yuandian_verified_at", null);
    }
  }

  async function handleVerifyEmbedding() {
    if (!settings?.embedding_api_key?.trim()) {
      setEmbeddingStatus("fail");
      setEmbeddingMsg("请先填入 API Key");
      return;
    }
    setEmbeddingStatus("verifying");
    setEmbeddingMsg("");
    try {
      const dim = await verifyEmbeddingKey(
        settings.embedding_endpoint ?? "",
        settings.embedding_model ?? "",
        settings.embedding_api_key,
      );
      setEmbeddingStatus("ok");
      setEmbeddingMsg(`✓ 已验证 · 向量维度 ${dim}`);
      updateField("embedding_verified_at", new Date().toISOString());
    } catch (e) {
      setEmbeddingStatus("fail");
      setEmbeddingMsg(String(e));
      updateField("embedding_verified_at", null);
    }
  }

  useEffect(() => {
    let cancelled = false;
    getSettings()
      .then((s) => {
        if (!cancelled) setSettings(s);
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
  }, []);

  useEffect(() => {
    // page 模式 Escape 不关页(切 tab 才离开),只 modal 模式响应
    if (isPage) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") handleClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // handleClose 不放进 deps,因为它依赖 onClose,而 onClose 是新引用每次
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isPage, onClose]);

  // dirty 上报给父组件
  useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);

  async function handleSave() {
    if (!settings) return;
    setSaving(true);
    setError(null);
    try {
      await saveSettings(prepareSettingsForSave(settings));
      for (const [name, value] of Object.entries(featureFlagDraft)) {
        setFeatureFlag(name as FeatureFlagName, value);
      }
      setDirty(false);
      // 2026-05-27 · 两种模式都要通知父组件 settings 已经变了,父组件据此重判依赖项
      // (如 DeepSeek 余额 chip 是否显示)。修复同事场景:onboarding 选"稍后再配置"
      // 进 page 模式补填 key,保存后 chip 不出现 —— 因为 page 模式只显示 toast、不触发
      // onClose,父组件的 showDeepSeekChip 状态从未更新。
      onSaved?.();
      if (isPage) {
        // page 模式:不关闭页面,显示"已保存"提示
        setSaved(true);
        setSaving(false);
        // 3 秒后清掉"已保存"提示
        setTimeout(() => setSaved(false), 3000);
      } else {
        // modal 模式:保存成功 → 自动关闭(作者 2026-05-23 晚九 反馈)
        handleClose();
      }
    } catch (e) {
      setError(String(e));
      setSaving(false);
    }
  }

  function updateField<K extends keyof Settings>(key: K, value: Settings[K]) {
    setSettings((prev) => (prev ? { ...prev, [key]: value } : prev));
    setDirty(true);
  }

  function updateFeatureFlag(name: FeatureFlagName, value: boolean) {
    setFeatureFlagDraft((previous) => ({ ...previous, [name]: value }));
    setDirty(true);
  }

  // 一次更新多个字段(切换云端服务商时:backend + 预填 endpoint/model + 清 key/verified 要原子改)
  function updateFields(patch: Partial<Settings>) {
    setSettings((prev) => (prev ? { ...prev, ...patch } : prev));
    setDirty(true);
  }

  function prepareSettingsForSave(current: Settings): Settings {
    if (!isCompatBackend(current.cloud_llm_backend)) return current;
    const backend = current.cloud_llm_backend;
    const keys = COMPAT_FIELD_KEYS[backend];
    const next: Settings = { ...current };
    if (!compatValue(next, backend, "endpoint")) {
      setStringSetting(
        next,
        keys.endpoint,
        legacyCompatValue(current, "endpoint") || COMPAT_PRESETS[backend].endpoint || null,
      );
    }
    if (!compatValue(next, backend, "model")) {
      setStringSetting(
        next,
        keys.model,
        legacyCompatValue(current, "model") || COMPAT_PRESETS[backend].model || null,
      );
    }
    if (!compatValue(next, backend, "apiKey")) {
      setStringSetting(next, keys.apiKey, legacyCompatValue(current, "apiKey"));
    }
    if (!compatValue(next, backend, "verifiedAt")) {
      setStringSetting(next, keys.verifiedAt, legacyCompatValue(current, "verifiedAt"));
    }
    return next;
  }

  // page 模式:没有蒙层,卡片直接占主区域,scroll 由父容器管;不带 X 按钮
  // modal 模式:蒙层 + max-h 限高 + X 按钮(原有形态)
  // 注意:不能用内嵌函数组件 wrap children,那会让每次 render 重建组件类型 → 子树 unmount + state 丢失
  // 改用条件渲染同一 body JSX,React 会正确 diff
  const body = (
    <>
        {/* 标题栏 */}
        <header className="app-subheader flex items-center justify-between gap-4 border-b px-4 py-3.5 sm:px-5">
          <div>
            <h2
              className={cn(
                "font-semibold text-foreground",
                isPage ? "text-lg" : "text-sm",
              )}
            >
              设置
            </h2>
            <p className="mt-0.5 text-xs text-muted-foreground">
              配置模型、知识库、数据源和可选功能。
            </p>
          </div>
          {isPage ? (
            <Button
              size="sm"
              onClick={handleSave}
              disabled={saving || !settings || !dirty}
              title="保存本页全部设置"
            >
              {saving ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <Save className="size-3.5" />
              )}
              保存
            </Button>
          ) : (
            <Button
              variant="ghost"
              size="icon"
              onClick={handleClose}
              aria-label="关闭"
            >
              <X className="size-4" />
            </Button>
          )}
        </header>

        {/* 内容区 */}
        <div className="flex-1 overflow-auto px-4 py-5 sm:px-5">
          {loading && (
            <div className="flex items-center justify-center py-8">
              <Loader2 className="size-5 animate-spin text-muted-foreground" />
            </div>
          )}
          {!loading && settings && (
            <>
            {/* 2026-06-16 · 标签页导航:按类型归拢散乱配置 */}
            <div
              className="mb-6 flex flex-wrap gap-1.5 border-b border-border pb-3"
              role="tablist"
              aria-label="设置分类"
            >
              {SETTINGS_TABS.map((t) => {
                const Icon = t.icon;
                const active = tab === t.id;
                return (
                  <button
                    key={t.id}
                    type="button"
                    onClick={() => setTab(t.id)}
                    role="tab"
                    aria-selected={active}
                    className={cn(
                      "inline-flex items-center gap-1.5 rounded-md px-3 py-1.5 text-sm font-medium transition-[transform,background-color,color,box-shadow] duration-150 active:scale-[0.97]",
                      active
                        ? "bg-brand-soft text-brand shadow-sm ring-1 ring-brand/15"
                        : "text-muted-foreground hover:bg-accent/80 hover:text-foreground",
                    )}
                  >
                    <Icon className="size-4" />
                    {t.label}
                  </button>
                );
              })}
            </div>
            <div
              className={cn(
                // page 模式:每个功能区各占一半,左右成对(更简洁、少占行);
                // 窗口恒 ≥1024(minWidth),lg 断点始终生效 → 默认就是两列。
                // modal 模式:保持单列堆叠,窄弹窗里两列会挤。
                isPage
                  ? "grid grid-cols-1 items-start gap-x-6 gap-y-6 lg:grid-cols-2"
                  : "space-y-6",
              )}
            >
              {/* ── 通用:界面字号(放最前,字小问题最常见)── */}
              {tab === "general" && <FontScaleCard />}

              {tab === "theme" && <ThemeCard />}

              {/* 默认关闭；打开后才展示配对/同步操作。Mac 与 Windows 共用同一套 LAN 协议。 */}
              {tab === "general" && <DeviceSyncCard />}

              {/* ── 通用:个人信息 ── */}
              {tab === "general" && (
                  <Section title="个人信息" fill>
                    <Field
                      label="称呼"
                      hint="首页问候用,例:刘律师 / 周律师 / 李三"
                    >
                      <input
                        type="text"
                        value={settings.user_display_name ?? ""}
                        onChange={(e) =>
                          updateField(
                            "user_display_name",
                            e.target.value || null,
                          )
                        }
                        placeholder="例:刘律师"
                        className={inputCls}
                      />
                    </Field>
                  </Section>
              )}

              {tab === "general" && (
                  <Section title="首页天气" fill>
                    <Field
                      label="天气城市"
                      hint="自动定位不准确时,手动指定城市名(如:南通)"
                    >
                      <input
                        type="text"
                        value={settings.weather_city ?? ""}
                        onChange={(e) =>
                          updateField(
                            "weather_city",
                            e.target.value || null,
                          )
                        }
                        placeholder="自动定位"
                        className={inputCls}
                      />
                    </Field>
                  </Section>
              )}

              {/* ── 通用:微信扫码加群(缩略图悬停放大;托管 lawtools.top,过期换图不必重新发版) ── */}
              {tab === "general" && (
                  <Section title="微信扫码加群" fill>
                    <div className="flex items-start gap-3" onMouseLeave={handleQrLeave}>
                      <div className="relative shrink-0" onMouseEnter={handleQrEnter}>
                        <GroupQrCode
                          size={60}
                          className="cursor-pointer rounded border border-border"
                        />
                        {qrHover && (
                          <div
                            className="absolute left-0 top-full z-50 mt-2"
                            onMouseEnter={handleQrEnter}
                          >
                            <GroupQrCode
                              size={300}
                              className="rounded-md border border-border shadow-xl"
                            />
                          </div>
                        )}
                      </div>
                      <p className="pt-2 text-xs text-muted-foreground">
                        鼠标悬停二维码放大,微信扫码进群 —— 反馈、提需求、看更新。
                      </p>
                    </div>
                  </Section>
              )}

              {/* V0.3:本地模型已隐藏 → 只走云端。三个 API key(MinerU / DeepSeek / 元典)常显,
                  不再用 cloud_enabled 开关包裹(该字段保留兼容,前端不再读)。 */}
              {/* ── 功能模型:PaddleOCR(云端 OCR,排在 MinerU 前)──
                  2026-06-12:PaddleOCR VL-1.6(AI Studio)。填了 key 即自动成为
                  另一家的备用(失败/超时/额度用完自动切换);也可在下方「云端 OCR 主力」卡切为主力。
                  实测:精度与 MinerU 打平,速度约快一倍,免费 2 万页/天(MinerU 1 千页/天);
                  单文件 >100 页会自动落回 MinerU。 */}
              {tab === "models" && (
                  <Section
                    title="PaddleOCR(云端 OCR)"
                    link={{
                      label: "点这里申请访问令牌",
                      href: "https://aistudio.baidu.com/account/accessToken",
                    }}
                  >
                    <Field
                      label="访问令牌"
                      hint="选为主力时必填；作为备用时选填。免费额度 2 万页/天"
                    >
                      <div className="flex items-center gap-2">
                        <input
                          type="password"
                          value={settings.paddle_vl_api_key ?? ""}
                          onChange={(e) => {
                            updateField(
                              "paddle_vl_api_key",
                              e.target.value || null,
                            );
                            // 改 token 就重置验证状态;清空 token 时主力退回 MinerU
                            if (paddleStatus !== "idle") {
                              setPaddleStatus("idle");
                              setPaddleMsg("");
                              updateField("paddle_vl_verified_at", null);
                            }
                            if (!e.target.value) {
                              updateField("ocr_cloud_primary", null);
                            }
                          }}
                          placeholder="AI Studio 访问令牌"
                          className={cn(inputCls, "flex-1")}
                          autoComplete="off"
                        />
                        <VerifyStatusIcon status={paddleStatus} />
                        <Button
                          type="button"
                          size="sm"
                          variant="outline"
                          className="disabled:cursor-not-allowed"
                          onClick={handleVerifyPaddle}
                          disabled={
                            paddleStatus === "verifying" ||
                            !settings.paddle_vl_api_key?.trim()
                          }
                        >
                          {paddleStatus === "verifying" ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            "验证"
                          )}
                        </Button>
                      </div>
                      {paddleStatus === "fail" && paddleMsg && (
                        <p className="mt-1.5 text-xs text-red-600">
                          ✗ {paddleMsg}
                        </p>
                      )}
                      {paddleStatus === "ok" && (
                        <p className="mt-1.5 text-xs text-green-700">
                          ✓ 已验证通过,可以使用
                        </p>
                      )}
                    </Field>
                  </Section>
              )}

              {/* ── 功能模型:MinerU(云端 OCR)── */}
              {tab === "models" && (
                  <Section
                    title="MinerU"
                    link={{ label: "点这里申请 token", href: "https://mineru.net/apiManage/token" }}
                  >
                    <Field
                      label="API Token"
                      hint="选为主力时必填；作为备用时选填"
                    >
                      <div className="flex items-center gap-2">
                        <input
                          type="password"
                          value={settings.mineru_api_key ?? ""}
                          onChange={(e) => {
                            updateField("mineru_api_key", e.target.value || null);
                            // 改 token 就重置验证状态
                            if (mineruStatus !== "idle") {
                              setMineruStatus("idle");
                              setMineruMsg("");
                              updateField("mineru_verified_at", null);
                            }
                          }}
                          placeholder="eyJ0eXBl... 或 sk-..."
                          className={cn(inputCls, "flex-1")}
                          autoComplete="off"
                        />
                        <VerifyStatusIcon status={mineruStatus} />
                        <Button
                          type="button"
                          size="sm"
                          variant="outline"
                          className="disabled:cursor-not-allowed"
                          onClick={handleVerifyMineru}
                          disabled={
                            mineruStatus === "verifying" ||
                            !settings.mineru_api_key?.trim()
                          }
                        >
                          {mineruStatus === "verifying" ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            "验证"
                          )}
                        </Button>
                      </div>
                      {mineruStatus === "fail" && mineruMsg && (
                        <p className="mt-1.5 text-xs text-red-600">
                          ✗ {mineruMsg}
                        </p>
                      )}
                      {mineruStatus === "ok" && (
                        <p className="mt-1.5 text-xs text-green-700">
                          ✓ 已验证通过,可以使用
                        </p>
                      )}
                    </Field>
                  </Section>
              )}

              {/* ── 功能模型:云端 OCR 主力(主副选择,单独卡,默认就显示)── */}
              {tab === "models" && (
                  <Section
                    title="云端 OCR 主力"
                    desc="只需填写并验证主力 OCR；备用线路选填。备用已配置时，主力失败、排队超时或额度用完会自动切换。"
                  >
                    <Field label="选择主力">
                      <select
                        value={
                          settings.ocr_cloud_primary === "paddle-vl"
                            ? "paddle-vl"
                            : "mineru"
                        }
                        onChange={(e) =>
                          updateField(
                            "ocr_cloud_primary",
                            e.target.value === "paddle-vl" ? "paddle-vl" : null,
                          )
                        }
                        className={inputCls}
                      >
                        <option value="paddle-vl">
                          PaddleOCR 主力,MinerU 备用(推荐 · 更快、额度更高)
                        </option>
                        <option value="mineru">
                          MinerU 主力,PaddleOCR 备用
                        </option>
                      </select>
                      <p className="mt-1.5 rounded-md bg-sky-50 px-2.5 py-1.5 text-caption text-sky-800">
                        建议用 <strong>PaddleOCR 为主、MinerU 备用</strong> ——
                        PaddleOCR 速度更快、免费额度更高(2 万页/天 vs MinerU 1 千页/天),
                        批量导入更不容易卡。
                        {!settings.paddle_vl_api_key?.trim() &&
                          "(需先在上方「PaddleOCR」卡填访问令牌)"}
                      </p>
                    </Field>
                  </Section>
              )}

              {/* ── 数据源:元典法律开放平台(法规/案例/企业检索 + 执行查被执行人)── */}
              {tab === "datasource" && (
              <Section
                title="元典法律开放平台"
                desc="查询法律法规、裁判案例、企业信息的数据源"
                link={{
                  label: "注册后在「个人中心」申请 API key",
                  href: "https://open.chineselaw.com/profile",
                }}
              >
                <Field label="API Key">
                  <div className="flex items-center gap-2">
                    <input
                      type="password"
                      value={settings.yuandian_api_key ?? ""}
                      onChange={(e) => {
                        updateField(
                          "yuandian_api_key",
                          e.target.value || null,
                        );
                        // 改 key 就重置验证状态
                        if (yuandianStatus !== "idle") {
                          setYuandianStatus("idle");
                          setYuandianMsg("");
                          updateField("yuandian_verified_at", null);
                        }
                      }}
                      placeholder="sk_..."
                      className={cn(inputCls, "flex-1")}
                      autoComplete="off"
                    />
                    <VerifyStatusIcon status={yuandianStatus} />
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      className="disabled:cursor-not-allowed"
                      onClick={handleVerifyYuandian}
                      disabled={
                        yuandianStatus === "verifying" ||
                        !settings.yuandian_api_key?.trim()
                      }
                    >
                      {yuandianStatus === "verifying" ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        "验证"
                      )}
                    </Button>
                  </div>
                  {yuandianStatus === "fail" && yuandianMsg && (
                    <p className="mt-1.5 text-xs text-red-600">
                      ✗ {yuandianMsg}
                    </p>
                  )}
                  {yuandianStatus === "ok" && (
                    <p className="mt-1.5 text-xs text-green-700">
                      ✓ 已验证通过,可以使用「查被执行人」等元典功能
                    </p>
                  )}
                </Field>
              </Section>
              )}

              {/* ── 大脑:云端 AI 后端 + DeepSeek / MiniMax(切换后只显示所选后端)── */}
              {tab === "brain" && (
                <>
                  <Section title="AI Soul">
                    <Field
                      label="全局工作风格"
                      hint="写长期偏好和协作习惯,例如回答结构、风险提示口径、默认称呼。不要写具体案件事实。"
                    >
                      <textarea
                        value={settings.ai_soul_md ?? ""}
                        onChange={(e) =>
                          updateField("ai_soul_md", e.target.value || null)
                        }
                        rows={5}
                        placeholder="例:先给结论,再列依据和风险;事实不明确时先追问;法律文书表达正式克制。"
                        className={cn(inputCls, "min-h-[112px] resize-y leading-relaxed")}
                      />
                      <p className="mt-1 text-label text-muted-foreground">
                        AI Soul 会注入案件 AI 助手,但优先级低于系统规则、当前问题、案件材料和工具返回。
                      </p>
                    </Field>
                  </Section>

                  <Section title="云端 AI 后端">
                    <Field label="提供商">
                      <select
                        value={settings.cloud_llm_backend ?? "deepseek"}
                        onChange={(e) => handleChangeBackend(e.target.value)}
                        className={inputCls}
                      >
                        {CLOUD_BACKEND_OPTIONS.map((o) => (
                          <option key={o.id} value={o.id}>
                            {o.label}
                          </option>
                        ))}
                      </select>
                      <p className="mt-1 text-label text-muted-foreground">
                        切换后下面只显示所选后端的配置。DeepSeek、MiniMax、GLM、MiMo、自定义模型
                        都各自独立保存,切换服务商不会互相覆盖。
                      </p>
                    </Field>
                  </Section>

                  {(settings.cloud_llm_backend ?? "deepseek") === "deepseek" && (
                  <Section
                    title="DeepSeek"
                    link={{
                      label: "点这里申请 API Key",
                      href: "https://platform.deepseek.com/api_keys",
                    }}
                  >
                    <Field label="API Key">
                      <div className="flex items-center gap-2">
                        <input
                          type="password"
                          value={settings.cloud_llm_api_key ?? ""}
                          onChange={(e) => {
                            updateField(
                              "cloud_llm_api_key",
                              e.target.value || null,
                            );
                            if (deepseekStatus !== "idle") {
                              setDeepseekStatus("idle");
                              setDeepseekMsg("");
                              updateField("deepseek_verified_at", null);
                            }
                          }}
                          placeholder="sk-..."
                          className={cn(inputCls, "flex-1")}
                          autoComplete="off"
                        />
                        <VerifyStatusIcon status={deepseekStatus} />
                        <Button
                          type="button"
                          size="sm"
                          variant="outline"
                          className="disabled:cursor-not-allowed"
                          onClick={handleVerifyDeepSeek}
                          disabled={
                            deepseekStatus === "verifying" ||
                            !settings.cloud_llm_api_key?.trim()
                          }
                        >
                          {deepseekStatus === "verifying" ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            "验证"
                          )}
                        </Button>
                      </div>
                      {deepseekStatus === "fail" && deepseekMsg && (
                        <p className="mt-1.5 text-xs text-red-600">
                          ✗ {deepseekMsg}
                        </p>
                      )}
                      {deepseekStatus === "ok" && (
                        <p className="mt-1.5 text-xs text-green-700">
                          ✓ 已验证通过,可以使用
                        </p>
                      )}
                    </Field>
                    <Field label="模型档位">
                      <select
                        value={settings.cloud_llm_model ?? "deepseek-v4-flash"}
                        onChange={(e) =>
                          updateField("cloud_llm_model", e.target.value || null)
                        }
                        className={inputCls}
                      >
                        <option value="deepseek-v4-flash">
                          Flash · 便宜快(默认 · 约 Pro 的 1/3 价 · 推荐日常)
                        </option>
                        <option value="deepseek-v4-pro">
                          Pro · 更准更贵(复杂分析/起草可换它)
                        </option>
                        <option value="auto">
                          自动挡 · 简单走 Flash、复杂走 Pro(均衡)
                        </option>
                      </select>
                      <p className="mt-1 text-label text-muted-foreground">
                        全程按这个档位走。Flash 省钱;觉得效果不够就换 Pro 或自动挡。
                      </p>
                    </Field>
                    {/* Endpoint 默认 https://api.deepseek.com,改了反而可能用不了 → 不暴露输入框,
                        cloud_llm_endpoint 留 null,后端按默认走。 */}
                  </Section>
                  )}

                  {(settings.cloud_llm_backend ?? "deepseek") === "minimax" && (
                  <Section
                    title="MiniMax"
                    link={{
                      label: "点这里申请 API Key",
                      href: "https://platform.minimaxi.com/user-center/payment/token-plan",
                    }}
                  >
                    <Field label="API Key">
                      <div className="flex items-center gap-2">
                        <input
                          type="password"
                          value={settings.minimax_api_key ?? ""}
                          onChange={(e) => {
                            updateField(
                              "minimax_api_key",
                              e.target.value || null,
                            );
                            if (minimaxStatus !== "idle") {
                              setMinimaxStatus("idle");
                              setMinimaxMsg("");
                              updateField("minimax_verified_at", null);
                            }
                          }}
                          placeholder="填入 MiniMax 平台的 API Key"
                          className={cn(inputCls, "flex-1")}
                          autoComplete="off"
                        />
                        <VerifyStatusIcon status={minimaxStatus} />
                        <Button
                          type="button"
                          size="sm"
                          variant="outline"
                          className="disabled:cursor-not-allowed"
                          onClick={handleVerifyMiniMax}
                          disabled={
                            minimaxStatus === "verifying" ||
                            !settings.minimax_api_key?.trim()
                          }
                        >
                          {minimaxStatus === "verifying" ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            "验证"
                          )}
                        </Button>
                      </div>
                      {minimaxStatus === "fail" && minimaxMsg && (
                        <p className="mt-1.5 text-xs text-red-600">
                          ✗ {minimaxMsg}
                        </p>
                      )}
                      {minimaxStatus === "ok" && (
                        <p className="mt-1.5 text-xs text-green-700">
                          ✓ 已验证通过,可以使用
                        </p>
                      )}
                    </Field>
                    <Field
                      label="模型档位"
                      hint="按需选择:M2.7 轻量便宜,M2.7-highspeed 速度加倍,M3 强推理(1M 上下文)"
                    >
                      <select
                        value={normalizeMinimaxModel(settings.minimax_model)}
                        onChange={(e) =>
                          updateField("minimax_model", e.target.value)
                        }
                        className={inputCls}
                      >
                        <option value="MiniMax-M2.7">
                          MiniMax-M2.7(轻量档,60 TPS,推荐日常)
                        </option>
                        <option value="MiniMax-M2.7-highspeed">
                          MiniMax-M2.7-highspeed(高速版,100 TPS)
                        </option>
                        <option value="MiniMax-M3">
                          MiniMax-M3(强推理档,1M 上下文,复杂法律分析)
                        </option>
                      </select>
                    </Field>
                    {/* Endpoint 默认 https://api.minimaxi.com;聊天真实路径
                        /v1/text/chatcompletion_v2 由后端自动补 → 不暴露输入框。 */}
                  </Section>
                  )}

                  {/* ── 通用 OpenAI 兼容后端(GLM / MiMo / Kimi / 自定义)── */}
                  {["glm", "mimo", "kimi", "custom"].includes(
                    settings.cloud_llm_backend ?? "",
                  ) &&
                    (() => {
                      const cur = isCompatBackend(settings.cloud_llm_backend)
                        ? settings.cloud_llm_backend
                        : "custom";
                      const preset = COMPAT_PRESETS[cur];
                      const keys = COMPAT_FIELD_KEYS[cur];
                      const apiKey = effectiveCompatValue(settings, cur, "apiKey") ?? "";
                      const model = effectiveCompatValue(settings, cur, "model") ?? "";
                      const endpoint =
                        effectiveCompatValue(settings, cur, "endpoint") ??
                        preset.endpoint;
                      // 改 key/模型/地址 → 清验证态(坑#11:改了要重验)
                      const onConfigChange = () => {
                        if (compatStatus !== "idle") {
                          setCompatStatus("idle");
                          setCompatMsg("");
                          updateField(
                            keys.verifiedAt,
                            null,
                          );
                        }
                      };
                      return (
                        <Section
                          title={preset.label}
                          desc="OpenAI 兼容云端 LLM(模型名 / 接口地址都可改;改了请重新验证)"
                          link={
                            preset.applyUrl
                              ? {
                                  label: "申请 / 查看 API Key",
                                  href: preset.applyUrl,
                                }
                              : undefined
                          }
                        >
                          <Field label="API Key">
                            <div className="flex items-center gap-2">
                              <input
                                type="password"
                                value={apiKey}
                                onChange={(e) => {
                                  updateField(
                                    keys.apiKey,
                                    e.target.value || null,
                                  );
                                  onConfigChange();
                                }}
                                placeholder="填入服务商平台的 API Key"
                                className={cn(inputCls, "flex-1")}
                                autoComplete="off"
                              />
                              <VerifyStatusIcon status={compatStatus} />
                              <Button
                                type="button"
                                size="sm"
                                variant="outline"
                                className="disabled:cursor-not-allowed"
                                onClick={handleVerifyCompat}
                                disabled={
                                  compatStatus === "verifying" ||
                                  !apiKey.trim()
                                }
                              >
                                {compatStatus === "verifying" ? (
                                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                ) : (
                                  "验证"
                                )}
                              </Button>
                            </div>
                            {compatStatus === "fail" && compatMsg && (
                              <p className="mt-1.5 text-xs text-red-600">
                                ✗ {compatMsg}
                              </p>
                            )}
                            {compatStatus === "ok" && (
                              <p className="mt-1.5 text-xs text-green-700">
                                ✓ 已验证通过,可以使用
                              </p>
                            )}
                          </Field>
                          <Field
                            label="模型名"
                            hint={
                              cur === "kimi"
                                ? "标准版适合日常使用；高速版需要对应会员档位"
                                : "具体型号,以服务商控制台为准(如 glm-4.6 / mimo-v2.5)"
                            }
                          >
                            {cur === "kimi" ? (
                              <select
                                value={model || preset.model}
                                onChange={(e) => {
                                  updateField(keys.model, e.target.value);
                                  onConfigChange();
                                }}
                                className={inputCls}
                              >
                                <option value="kimi-for-coding">Kimi for Coding(标准版)</option>
                                <option value="kimi-for-coding-highspeed">
                                  Kimi for Coding HighSpeed(高速版)
                                </option>
                              </select>
                            ) : (
                              <input
                                type="text"
                                value={model}
                                onChange={(e) => {
                                  updateField(keys.model, e.target.value || null);
                                  onConfigChange();
                                }}
                                placeholder="如 glm-4.6"
                                className={inputCls}
                                autoComplete="off"
                              />
                            )}
                          </Field>
                          <Field
                            label="接口地址"
                            hint="OpenAI 兼容的 chat completions 完整地址;只填到 base 会自动补 /v1/chat/completions"
                          >
                            <input
                              type="text"
                              value={endpoint}
                              onChange={(e) => {
                                updateField(
                                  keys.endpoint,
                                  e.target.value || null,
                                );
                                onConfigChange();
                              }}
                              placeholder="https://.../v1/chat/completions"
                              className={inputCls}
                              autoComplete="off"
                            />
                          </Field>
                        </Section>
                      );
                    })()}
                </>
              )}

              {/* ── 功能模型:硅基流动(Embedding 语义检索;留空后端默认 bge-m3 免费)──
                  填了才启用,否则回退关键词选材料。接口地址 / 模型不暴露。 */}
              {tab === "models" && (
              <Section
                title="硅基流动 API"
                desc="Embedding 语义检索 · 云端 API 服务"
                link={{
                  label: "申请 API key",
                  href: "https://cloud.siliconflow.cn/me/account/ak",
                }}
              >
                <Field label="API Key">
                  <div className="flex items-center gap-2">
                    <input
                      type="password"
                      value={settings.embedding_api_key ?? ""}
                      onChange={(e) => {
                        updateField("embedding_api_key", e.target.value || null);
                        if (embeddingStatus !== "idle") {
                          setEmbeddingStatus("idle");
                          setEmbeddingMsg("");
                          updateField("embedding_verified_at", null);
                        }
                      }}
                      placeholder="sk-..."
                      className={cn(inputCls, "flex-1")}
                      autoComplete="off"
                    />
                    <VerifyStatusIcon status={embeddingStatus} />
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      className="disabled:cursor-not-allowed"
                      onClick={handleVerifyEmbedding}
                      disabled={
                        embeddingStatus === "verifying" ||
                        !settings.embedding_api_key?.trim()
                      }
                    >
                      {embeddingStatus === "verifying" ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        "验证"
                      )}
                    </Button>
                  </div>
                  {embeddingStatus === "fail" && embeddingMsg && (
                    <p className="mt-1.5 text-xs text-red-600">✗ {embeddingMsg}</p>
                  )}
                  {embeddingStatus === "ok" && (
                    <p className="mt-1.5 text-xs text-green-700">
                      {embeddingMsg || "✓ 已验证通过"}
                    </p>
                  )}
                </Field>
              </Section>
              )}

              {/* ── 知识库:法律向量检索维护(法条+案例+企业语义索引)── */}
              {tab === "kb" && (
              <KbSemanticIndexCard
                embeddingConfigured={!!settings.embedding_api_key?.trim()}
                autoIndex={settings.kb_semantic_auto_index !== false}
                onAutoChange={(v) => updateField("kb_semantic_auto_index", v)}
              />
              )}

              {/* 快递100 配置已迁到「法律工具 → 快递查询」页内,就近配置(2026-06-16)。 */}

              {/* ── 数据源:元典积分账(本月统计)── */}
              {tab === "datasource" && (
              <YuandianCreditsCard
                hasApiKey={!!settings.yuandian_api_key?.trim()}
                monthlyLimit={settings.yuandian_monthly_credit_limit ?? null}
                onLimitChange={(n) =>
                  updateField("yuandian_monthly_credit_limit", n)
                }
              />
              )}

              {/* ── 知识库:本地知识库三态卡 ── */}
              {tab === "kb" && (
              <LocalKbCard
                kbRoot={settings.local_kb_root ?? null}
                kbEnabled={settings.local_kb_enabled !== false}
                onKbRootChange={(p) => updateField("local_kb_root", p)}
                onKbEnabledChange={(b) => updateField("local_kb_enabled", b)}
              />
              )}

              {/* V0.3:本地模型已隐藏 → 删「各模块走本机/云端」切换器 + 本机模型(ollama)配置段。
                  字段(ocr_provider/llm_provider/ollama_*)保留在后端/types,以后接新本地模型再恢复 UI。 */}

              {/* ── 功能开关:首页清爽开关(featureFlags)── */}
              {tab === "toggles" && (
                <div className={cn(isPage && "lg:col-span-2")}>
                  <FeatureFlagsCard
                    values={featureFlagDraft}
                    onChange={updateFeatureFlag}
                    homeCalendarEnabled={settings.home_calendar_enabled}
                    onHomeCalendarChange={(enabled) =>
                      updateField("home_calendar_enabled", enabled)
                    }
                  />
                </div>
              )}

              {/* ── 数据源:外部工具(MCP)白名单(企查查/万得/北大法宝 等远程 HTTP)──
                  整宽,AI 助手消费外部 MCP server 工具 */}
              {tab === "datasource" && (
              <McpServersCard
                servers={settings.mcp_servers ?? []}
                onChange={(next) => updateField("mcp_servers", next)}
              />
              )}


              {/* 错误展示 */}
              {error && (
                <div className="rounded-md border border-destructive/30 bg-destructive/5 p-3 lg:col-span-2">
                  <p className="text-xs font-medium text-destructive">出错了</p>
                  <p className="mt-1 font-mono text-caption text-muted-foreground">
                    {error}
                  </p>
                </div>
              )}
            </div>
            </>
          )}

          {/* 开源版仅展示项目署名，不在应用设置中嵌入个人或律所身份。 */}
          <div className="mt-2 border-t border-border pt-5 text-center">
            <p className="text-sm font-medium text-foreground">CaseBoard 项目</p>
            <p className="mt-0.5 text-xs text-muted-foreground">
              开源社区维护
            </p>
          </div>
        </div>

        {/* 底部按钮栏 */}
        <footer
          className={cn(
            "app-subheader flex items-center gap-4 border-t px-4 py-3 sm:px-5",
            isPage ? "justify-start" : "justify-between",
          )}
        >
          <span
            className={cn(
              "text-caption",
              saved
                ? "text-green-700 animate-in fade-in-0 duration-200"
                : "text-muted-foreground",
            )}
          >
            {saved
              ? "已保存 · 下次导入案件时生效(已在跑的任务不切换后端)"
              : settings === null
                ? ""
                : dirty
                  ? "有未保存改动 · 别忘了点保存"
                  : "改完点保存"}
          </span>
          {!isPage && (
            <div className="flex gap-2">
              <Button variant="outline" size="sm" onClick={handleClose}>
                取消
              </Button>
              <Button
                size="sm"
                onClick={handleSave}
                disabled={saving || !settings}
              >
                {saving ? (
                  <Loader2 className="size-3.5 animate-spin" />
                ) : (
                  <Save className="size-3.5" />
                )}
                保存
              </Button>
            </div>
          )}
        </footer>
    </>
  );

  if (isPage) {
    return (
      <div className="app-page-enter mx-auto flex h-full w-full max-w-6xl flex-col overflow-hidden">
        {body}
      </div>
    );
  }
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-foreground/20 px-4 py-8 backdrop-blur-sm animate-in fade-in-0 duration-200"
      onClick={handleClose}
    >
      <div
        className="flex max-h-[85vh] w-full max-w-2xl flex-col overflow-hidden rounded-xl border border-border bg-card shadow-2xl animate-in zoom-in-95 fade-in-0 duration-300 ease-out"
        onClick={(e) => e.stopPropagation()}
      >
        {body}
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* 小组件                                                              */
/* ------------------------------------------------------------------ */

const inputCls = cn(
  "h-9 w-full rounded-md border border-border bg-card/85 px-3 text-sm",
  "placeholder:text-muted-foreground/60",
  "transition-[border-color,box-shadow]",
  "focus:border-brand focus:outline-none focus:ring-2 focus:ring-brand/15",
);

/**
 * MiniMax 模型档位归一:历史 settings 里可能是 "minimax-M3"(小写 m)或 null/空,
 * select 的 value 必须精确匹配某个 option.value 才能高亮。旧的 "MiniMax-M2" 已被
 * M2.7 取代,统一归并到 M2.7(后端 null 默认也已对齐 M2.7)。无法识别的值(用户
 * 手填过的)原样返回 —— select 不高亮,用户重选一次即可。
 */
function normalizeMinimaxModel(raw: string | null | undefined): string {
  if (!raw) return "MiniMax-M2.7";
  const lower = raw.trim().toLowerCase();
  if (lower === "minimax-m2.7" || lower === "minimax-m2") return "MiniMax-M2.7";
  if (lower === "minimax-m2.7-highspeed") return "MiniMax-M2.7-highspeed";
  if (lower === "minimax-m3") return "MiniMax-M3";
  return raw;
}

/** 验证状态图标:ok=绿勾 / fail=红叉 / 其他=不显示 */
function VerifyStatusIcon({ status }: { status: VerifyStatus }) {
  if (status === "ok") {
    return <CheckCircle2 className="h-5 w-5 shrink-0 text-green-600" aria-label="已验证" />;
  }
  if (status === "fail") {
    return <XCircle className="h-5 w-5 shrink-0 text-red-500" aria-label="验证失败" />;
  }
  return null;
}

function Section({
  title,
  desc,
  link,
  children,
  fill,
}: {
  title: string;
  desc?: string;
  link?: { label: string; href: string };
  children: React.ReactNode;
  /** true 时撑满网格行高(同一排卡片等高)。默认 false = 自然紧凑高度。 */
  fill?: boolean;
}) {
  return (
    <section className={fill ? "flex h-full flex-col" : undefined}>
      <div className="mb-3 flex items-start justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold text-foreground">{title}</h3>
          {desc && (
            <p className="mt-0.5 line-clamp-1 text-xs text-muted-foreground">
              {desc}
            </p>
          )}
        </div>
        {link && (
          <button
            type="button"
            onClick={() => openUrl(link.href).catch((e) => console.warn("openUrl failed", e))}
            className="inline-flex shrink-0 items-center gap-1.5 rounded-md border border-brand/15 bg-brand-soft/60 px-2.5 py-1 text-xs font-medium text-brand transition-[transform,background-color,border-color] hover:border-brand/25 hover:bg-brand-soft active:scale-[0.97]"
            title={link.href}
          >
            <ExternalLink className="size-3.5" />
            {link.label}
          </button>
        )}
      </div>
      {/* 默认自然高度(配对相近高度卡 + items-start 不留空);fill=true 时撑满行高(同排等高) */}
      <div
        className={cn(
          "surface-card space-y-3 bg-card/74 p-4",
          fill && "flex-1",
        )}
      >
        {children}
      </div>
    </section>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1 block text-xs font-medium text-foreground">
        {label}
      </span>
      {children}
      {hint && (
        <span className="mt-1 block text-caption text-muted-foreground">
          {hint}
        </span>
      )}
    </label>
  );
}

/**
 * 2026-06-16 · 功能开关卡(「功能开关」tab)。
 * 作者偏好清爽界面:新功能默认关,想用再开,逐设备生效(localStorage)。
 * 以后首页新增模块 → 在 src/lib/featureFlags.ts 的 FEATURE_FLAGS 加一条,这里自动出现开关。
 * 只渲染 location==="settings" 的开关;location==="feature" 的(如滴答待办)由对应功能页自己放。
 */
function FeatureFlagsCard({
  values,
  onChange,
  homeCalendarEnabled,
  onHomeCalendarChange,
}: {
  values: Partial<Record<FeatureFlagName, boolean>>;
  onChange: (name: FeatureFlagName, value: boolean) => void;
  homeCalendarEnabled: boolean;
  onHomeCalendarChange: (enabled: boolean) => void;
}) {
  const flags = FEATURE_FLAGS.filter((f) => f.location === "settings");
  return (
    <Section
      title="功能开关"
      desc="可选模块默认关闭，仅影响本机界面。"
    >
      <div className="grid grid-cols-1 gap-2 xl:grid-cols-2">
        <SettingsSwitchRow
          title="首页日程日历"
          description="把开庭/续封、带日期的待办、手动提醒汇总到首页日历。默认关闭,想体验就开,随时可关。"
          on={homeCalendarEnabled}
          onChange={() => onHomeCalendarChange(!homeCalendarEnabled)}
        />
        {flags.map((f) => (
          <FeatureFlagToggle
            key={f.name}
            name={f.name}
            on={values[f.name] ?? f.defaultValue}
            onChange={onChange}
          />
        ))}
      </div>
    </Section>
  );
}

function SettingsSwitchRow({
  title,
  description,
  on,
  onChange,
}: {
  title: string;
  description: string;
  on: boolean;
  onChange: () => void;
}) {
  return (
    <div className="flex min-h-[76px] items-center justify-between gap-3 rounded-lg border border-border bg-card/70 p-3 transition-colors hover:border-brand/15 hover:bg-card">
      <div className="min-w-0">
        <p className="text-sm font-medium text-foreground">{title}</p>
        <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={on}
        aria-label={title}
        onClick={onChange}
        className={cn(
          "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-[transform,background-color,box-shadow] active:scale-[0.96]",
          on ? "bg-brand shadow-[0_0_0_3px_var(--brand-soft)]" : "bg-muted",
        )}
      >
        <span
          className={cn(
            "inline-block size-4 rounded-full bg-white shadow transition-transform",
            on ? "translate-x-4" : "translate-x-0.5",
          )}
        />
      </button>
    </div>
  );
}

function FeatureFlagToggle({
  name,
  on,
  onChange,
}: {
  name: (typeof FEATURE_FLAGS)[number]["name"];
  on: boolean;
  onChange: (name: FeatureFlagName, value: boolean) => void;
}) {
  const meta = FEATURE_FLAGS.find((f) => f.name === name)!;
  return (
    <SettingsSwitchRow
      title={meta.title}
      description={meta.description}
      on={on}
      onChange={() => onChange(name, !on)}
    />
  );
}

/**
 * 2026-06-16 · 界面字号微调卡(「通用」tab)。有用户反映字小 → 全局等比缩放
 * (改根字号,Tailwind rem 单位连字带间距一起放大)。逐设备 localStorage,实时生效。
 */
function FontScaleCard() {
  const [scale, setScale] = useFontScale();
  const pct = Math.round(scale * 100);
  const presets: { label: string; v: number }[] = [
    { label: "小", v: 0.9 },
    { label: "标准", v: 1.0 },
    { label: "大", v: 1.15 },
    { label: "特大", v: 1.3 },
  ];
  return (
    <Section
      title="界面字号"
      desc="整体缩放文字与间距，仅影响本机。"
    >
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <span className="text-xs text-muted-foreground">当前缩放</span>
          <span className="text-sm font-semibold text-foreground">{pct}%</span>
        </div>
        <input
          type="range"
          min={FONT_SCALE.MIN}
          max={FONT_SCALE.MAX}
          step={FONT_SCALE.STEP}
          value={scale}
          onChange={(e) => setScale(parseFloat(e.target.value))}
          className="w-full accent-[var(--brand)]"
          aria-label="界面字号缩放"
        />
        <div className="flex flex-wrap items-center gap-1.5">
          {presets.map((p) => (
            <button
              key={p.label}
              type="button"
              onClick={() => setScale(p.v)}
              className={cn(
                "rounded-md border px-2.5 py-1 text-xs font-medium transition-colors",
                Math.abs(scale - p.v) < 0.001
                  ? "border-brand/30 bg-brand-soft text-brand"
                  : "border-border text-muted-foreground hover:bg-accent hover:text-foreground",
              )}
            >
              {p.label}
            </button>
          ))}
          <button
            type="button"
            onClick={() => setScale(FONT_SCALE.DEFAULT)}
            className="ml-auto rounded-md border border-border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            重置
          </button>
        </div>
        <p className="rounded-md bg-muted/40 px-3 py-2 text-xs text-foreground">
          示例:这行字会随缩放即时变大变小,调到看着舒服为止。
        </p>
      </div>
    </Section>
  );
}

function ThemeCard() {
  const [theme, setTheme] = useState<ThemeId>(() => getThemePreference());

  function chooseTheme(next: ThemeId) {
    setTheme(next);
    setThemePreference(next);
  }

  return (
    <Section
      title="界面主题"
      desc="仅改变本机界面配色，不影响案件数据。默认主题保持不变，后续可继续增加其他主题。"
      fill
    >
      <div className="grid gap-3 sm:grid-cols-2">
        {THEMES.map((item) => {
          const active = item.id === theme;
          return (
            <button
              key={item.id}
              type="button"
              onClick={() => chooseTheme(item.id)}
              aria-pressed={active}
              className={cn(
                "rounded-lg border p-3 text-left transition-colors",
                active
                  ? "border-brand/40 bg-brand-soft/70 ring-1 ring-brand/15"
                  : "border-border bg-card hover:bg-accent/50",
              )}
            >
              <span className="mb-3 flex gap-1.5" aria-hidden="true">
                <span
                  className={cn(
                    "size-5 rounded-full border",
                    item.id === "default"
                      ? "border-slate-300 bg-slate-100"
                      : "border-emerald-800 bg-[#f3f0e4]",
                  )}
                />
                <span
                  className={cn(
                    "size-5 rounded-full",
                    item.id === "default" ? "bg-slate-700" : "bg-emerald-900",
                  )}
                />
                <span
                  className={cn(
                    "size-5 rounded-full",
                    item.id === "default" ? "bg-slate-300" : "bg-emerald-200",
                  )}
                />
              </span>
              <span className="flex items-center justify-between gap-2">
                <span className="text-sm font-semibold">{item.label}</span>
                {active && <CheckCircle2 className="size-4 text-brand" />}
              </span>
              <span className="mt-1 block text-xs text-muted-foreground">
                {item.description}
              </span>
            </button>
          );
        })}
      </div>
    </Section>
  );
}

// =============================================================================
// V0.3.6 · 外部工具(MCP)配置卡(整宽)
// CaseBoard 当 MCP **客户端**,把外部 MCP server 暴露的工具并入 AI 助手 —— 加能力 = 配一个
// server、热加载,不必改 Rust 重出 dmg。当前只支持 stdio 子进程(npx 等);默认空 = 桥接关闭。
// 详 docs/adr/0008。注意:产出的 transport 形状必须跟后端 serde 完全一致,否则整次保存会失败。
// =============================================================================

const mcpTextareaCls = cn(
  "w-full rounded-md border border-border bg-background px-3 py-2 font-mono text-xs leading-relaxed",
  "placeholder:text-muted-foreground/60",
  "transition-[border-color,box-shadow]",
  "focus:outline-none focus:border-foreground focus:ring-1 focus:ring-foreground/20",
);

/** args: 一行一个,去首尾空白 + 丢空行。 */
function argsToText(args: string[]): string {
  return args.join("\n");
}
function textToArgs(t: string): string[] {
  return t
    .split("\n")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}
/** env: 一行 `KEY=VALUE`,首个 `=` 为分隔;无 `=` / 空 key 的行丢弃。 */
function envToText(env: Record<string, string>): string {
  return Object.entries(env)
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
}
function textToEnv(t: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of t.split("\n")) {
    const eq = line.indexOf("=");
    if (eq <= 0) continue; // 无 = 或 = 在首位(空 key)→ 丢弃
    const key = line.slice(0, eq).trim();
    if (!key) continue;
    out[key] = line.slice(eq + 1).trim();
  }
  return out;
}

// 每行一个 client-only 稳定 id(**不进**保存形状,避免污染后端 serde)。仅为 React key 稳定 —
// mid-list 删除时若用数组下标当 key,行内本地编辑缓冲会串到别的行槽位(经典 index-key bug)。
let mcpRowSeq = 0;
function nextMcpRowId(): string {
  mcpRowSeq += 1;
  return `mcp-row-${mcpRowSeq}`;
}

function McpServersCard({
  servers,
  onChange,
}: {
  servers: McpServerConfig[];
  onChange: (next: McpServerConfig[]) => void;
}) {
  // 内部维护 {id, cfg}:id 只为稳定 key。挂载时 seed 一次自 props(本卡在 settings 加载后才
  // 渲染,props 即加载值);此后本卡是 mcp_servers 的唯一改动源,无需 reactive 同步回 props。
  const [rows, setRows] = useState<{ id: string; cfg: McpServerConfig }[]>(() =>
    servers.map((cfg) => ({ id: nextMcpRowId(), cfg })),
  );
  function commit(next: { id: string; cfg: McpServerConfig }[]) {
    setRows(next);
    onChange(next.map((r) => r.cfg));
  }
  function patchRow(id: string, cfg: McpServerConfig) {
    commit(rows.map((r) => (r.id === id ? { ...r, cfg } : r)));
  }
  function addHttpServer() {
    commit([
      ...rows,
      {
        id: nextMcpRowId(),
        cfg: {
          name: "",
          transport: { type: "http", url: "", headers: {} },
          enabled: true,
        },
      },
    ]);
  }
  function addStdioServer() {
    commit([
      ...rows,
      {
        id: nextMcpRowId(),
        cfg: {
          name: "",
          transport: { type: "stdio", command: "npx", args: [], env: {} },
          enabled: true,
        },
      },
    ]);
  }
  function removeRow(id: string) {
    commit(rows.filter((r) => r.id !== id));
  }

  // ---- 智能粘贴识别(把平台接入文档的配置整段粘进来,自动拆成 server)----
  const [pasteText, setPasteText] = useState("");
  const [pasteBusy, setPasteBusy] = useState(false);
  const [pasteMsg, setPasteMsg] = useState<{ kind: "ok" | "warn" | "err"; lines: string[] } | null>(
    null,
  );
  async function recognizePaste() {
    if (!pasteText.trim() || pasteBusy) return;
    setPasteBusy(true);
    setPasteMsg(null);
    try {
      const r = await parseMcpPaste(pasteText);
      const existing = new Set(rows.map((x) => x.cfg.name));
      const fresh = r.servers.filter((s) => !existing.has(s.name));
      const skipped = r.servers.length - fresh.length;
      commit([...rows, ...fresh.map((cfg) => ({ id: nextMcpRowId(), cfg }))]);
      setPasteText("");
      const lines = [
        `已识别 ${r.servers.length} 个 server${skipped > 0 ? `(${skipped} 个同名已存在,跳过)` : ""}，请逐个点「测试连接」确认能用。`,
        ...r.warnings,
      ];
      setPasteMsg({ kind: r.warnings.length > 0 ? "warn" : "ok", lines });
    } catch (e) {
      setPasteMsg({ kind: "err", lines: [String(e)] });
    } finally {
      setPasteBusy(false);
    }
  }

  return (
    <div className="lg:col-span-2">
      <section>
        <div className="mb-3 flex items-start justify-between gap-3">
          <div>
            <h3 className="flex items-center gap-1.5 text-sm font-semibold text-foreground">
              <Plug className="size-4 text-muted-foreground" />
              外部工具（MCP）
            </h3>
            <p className="mt-0.5 text-xs text-muted-foreground">
              让 AI 助手调外部数据平台的工具（加能力不必更新 App）。元典 / 企查查 / 万得 / 北大法宝等
              平台的云端 MCP 都是「远程 HTTP」型：粘服务地址 + 访问令牌即可，无需安装任何环境。
              <br />
              不配 = 关闭，零影响。配错或连不上的 server 会被自动跳过，不影响 AI 助手正常使用。
            </p>
          </div>
          <button
            type="button"
            onClick={() =>
              openUrl("https://github.com/modelcontextprotocol/servers").catch((e) =>
                console.warn("openUrl failed", e),
              )
            }
            className="inline-flex shrink-0 items-center gap-1.5 rounded-md border border-sky-200 bg-sky-50 px-2.5 py-1 text-xs font-medium text-sky-700 transition-colors hover:border-sky-300 hover:bg-sky-100"
            title="modelcontextprotocol/servers"
          >
            <ExternalLink className="size-3.5" />
            看可用 server
          </button>
        </div>

        <div className="space-y-3 rounded-lg border border-border bg-background/50 p-4">
          {/* 智能粘贴:推荐入口,平台文档配置整段粘进来自动识别 */}
          <div className="rounded-md border border-sky-200 bg-sky-50/50 p-3">
            <p className="mb-1.5 text-xs font-medium text-sky-900">
              ⚡ 快捷接入：把平台「接入指南」里的配置整段粘进来（JSON 或 claude mcp add
              命令都认），自动识别填好
            </p>
            <textarea
              rows={3}
              value={pasteText}
              onChange={(e) => setPasteText(e.target.value)}
              placeholder={'例如平台文档里的:\n{ "mcpServers": { "xxx": { "type": "http", "url": "https://...", "headers": { "Authorization": "Bearer 你的密钥" } } } }'}
              className={mcpTextareaCls}
              spellCheck={false}
              autoComplete="off"
            />
            <div className="mt-2 flex items-center gap-2">
              <button
                type="button"
                onClick={recognizePaste}
                disabled={pasteBusy || !pasteText.trim()}
                className="inline-flex items-center gap-1.5 rounded-md bg-sky-600 px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-sky-700 disabled:cursor-not-allowed disabled:opacity-50"
              >
                {pasteBusy ? "识别中…" : "识别并添加"}
              </button>
              <span className="text-caption text-muted-foreground">
                本地解析，不联网；令牌只存本机
              </span>
            </div>
            {pasteMsg && (
              <div
                className={cn(
                  "mt-2 space-y-0.5 text-xs",
                  pasteMsg.kind === "ok" && "text-emerald-700",
                  pasteMsg.kind === "warn" && "text-amber-700",
                  pasteMsg.kind === "err" && "text-red-600",
                )}
              >
                {pasteMsg.lines.map((l, i) => (
                  <p key={i}>{l}</p>
                ))}
              </div>
            )}
          </div>

          {rows.length === 0 && (
            <p className="text-xs text-muted-foreground">
              还没有配置外部工具。把平台给的配置粘到上方识别，或点下方按钮手动添加。
            </p>
          )}

          {rows.map((r) => (
            <McpServerRow
              key={r.id}
              cfg={r.cfg}
              onChange={(c) => patchRow(r.id, c)}
              onRemove={() => removeRow(r.id)}
            />
          ))}

          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={addHttpServer}
              className="inline-flex items-center gap-1.5 rounded-md border border-dashed border-sky-300 bg-sky-50/60 px-3 py-1.5 text-xs font-medium text-sky-700 transition-colors hover:border-sky-400 hover:bg-sky-50"
            >
              <Plus className="size-3.5" />
              添加远程 server（HTTP，推荐）
            </button>
            <button
              type="button"
              onClick={addStdioServer}
              className="inline-flex items-center gap-1.5 rounded-md border border-dashed border-border px-3 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:border-foreground/40 hover:text-foreground"
            >
              <Plus className="size-3.5" />
              添加本地命令（stdio）
            </button>
          </div>
        </div>
      </section>
    </div>
  );
}

/** 单个 MCP server 行。**关键**:args/env 用本行 local state 当「自由文本编辑缓冲」——
 *  直接拿派生 parse(argsToText/envToText)当受控 `value` 会吃键盘(env 里 `KEY` 还没敲到 `=`
 *  就被 textToEnv 丢掉、args 回车空行被 textToArgs 滤掉→光标弹回)。display 用原始字符串,
 *  只在 onChange 时 parse 进保存模型。buffer 仅挂载时 seed 一次(本行后续变化都源自 buffer 自身)。 */
function McpServerRow({
  cfg,
  onChange,
  onRemove,
}: {
  cfg: McpServerConfig;
  onChange: (c: McpServerConfig) => void;
  onRemove: () => void;
}) {
  const isStdio = cfg.transport.type === "stdio";
  const [argsText, setArgsText] = useState(() =>
    cfg.transport.type === "stdio" ? argsToText(cfg.transport.args) : "",
  );
  const [envText, setEnvText] = useState(() =>
    cfg.transport.type === "stdio" ? envToText(cfg.transport.env) : "",
  );
  // http 的 headers 跟 env 同形(KEY=VALUE),同样需要本行编辑缓冲(见组件 doc)
  const [headersText, setHeadersText] = useState(() =>
    cfg.transport.type === "http" ? envToText(cfg.transport.headers ?? {}) : "",
  );

  // ---- 连接测试:真连一次(握手+列工具),结果就地显示;配置一改就归零 ----
  const [test, setTest] = useState<{ s: "idle" | "busy" | "ok" | "err"; msg?: string }>({
    s: "idle",
  });
  useEffect(() => {
    setTest({ s: "idle" });
  }, [cfg]);
  async function runTest() {
    if (test.s === "busy") return;
    setTest({ s: "busy" });
    try {
      const r = await testMcpServer(cfg);
      const names = r.tool_names.slice(0, 5).join("、");
      setTest({
        s: "ok",
        msg: `已连上，发现 ${r.tool_count} 个工具${names ? `：${names}${r.tool_count > 5 ? " …" : ""}` : ""}`,
      });
    } catch (e) {
      setTest({ s: "err", msg: String(e) });
    }
  }
  // name 会拼进 `mcp__<name>__<tool>`(= DeepSeek 函数名);非 [A-Za-z0-9_-] 后端会清洗成 `_`
  // (兜底不让整个 tools 数组被拒),但仍提示用户用规范名,避免不同名清洗后撞车。
  const nameInvalid = cfg.name.length > 0 && !/^[A-Za-z0-9_-]+$/.test(cfg.name);

  return (
    <div
      className={cn(
        "rounded-md border border-border bg-card/60 p-3",
        !cfg.enabled && "opacity-60",
      )}
    >
      <div className="mb-2 flex items-center gap-2">
        <input
          type="text"
          value={cfg.name}
          onChange={(e) => onChange({ ...cfg, name: e.target.value })}
          placeholder="名称（如 filesystem，仅字母数字 _ -）"
          className={cn(inputCls, "flex-1", nameInvalid && "border-amber-400")}
          autoComplete="off"
        />
        <label className="flex shrink-0 items-center gap-1.5 text-xs text-foreground">
          <input
            type="checkbox"
            checked={cfg.enabled}
            onChange={(e) => onChange({ ...cfg, enabled: e.target.checked })}
            className="size-3.5 accent-sky-600"
          />
          启用
        </label>
        <button
          type="button"
          onClick={runTest}
          disabled={test.s === "busy"}
          className="shrink-0 rounded-md border border-sky-200 bg-sky-50 px-2.5 py-1 text-xs font-medium text-sky-700 transition-colors hover:border-sky-300 hover:bg-sky-100 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {test.s === "busy" ? "测试中…" : "测试连接"}
        </button>
        <button
          type="button"
          onClick={onRemove}
          className="shrink-0 rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
          aria-label="删除"
          title="删除这个 server"
        >
          <Trash2 className="size-4" />
        </button>
      </div>

      {nameInvalid && (
        <p className="mb-2 text-caption text-amber-700">
          名称建议只用字母、数字、下划线或连字符（会作为工具前缀）。
        </p>
      )}

      {isStdio ? (
        <div className="space-y-2.5">
          <Field label="命令" hint="可执行程序，例：npx / uvx / node">
            <input
              type="text"
              value={cfg.transport.type === "stdio" ? cfg.transport.command : ""}
              onChange={(e) =>
                cfg.transport.type === "stdio" &&
                onChange({
                  ...cfg,
                  transport: { ...cfg.transport, command: e.target.value },
                })
              }
              placeholder="npx"
              className={inputCls}
              autoComplete="off"
            />
          </Field>
          <Field
            label="参数（一行一个）"
            hint="例：第一行 -y，第二行 @modelcontextprotocol/server-filesystem，第三行 /要授权的目录"
          >
            <textarea
              rows={3}
              value={argsText}
              onChange={(e) => {
                const t = e.target.value;
                setArgsText(t);
                if (cfg.transport.type === "stdio") {
                  onChange({ ...cfg, transport: { ...cfg.transport, args: textToArgs(t) } });
                }
              }}
              placeholder={"-y\n@modelcontextprotocol/server-filesystem\n/Users/你/案件目录"}
              className={mcpTextareaCls}
              spellCheck={false}
            />
          </Field>
          <Field
            label="环境变量（选填，一行一个 KEY=VALUE）"
            hint="放 token 等；只存本机，不进 git / 日志"
          >
            <textarea
              rows={2}
              value={envText}
              onChange={(e) => {
                const t = e.target.value;
                setEnvText(t);
                if (cfg.transport.type === "stdio") {
                  onChange({ ...cfg, transport: { ...cfg.transport, env: textToEnv(t) } });
                }
              }}
              placeholder={"API_KEY=sk-xxxx"}
              className={mcpTextareaCls}
              spellCheck={false}
              autoComplete="off"
            />
          </Field>
        </div>
      ) : (
        <div className="space-y-2.5">
          <Field label="服务地址（URL）" hint="平台接入文档给的 MCP 服务地址，https 开头">
            <input
              type="text"
              value={cfg.transport.type === "http" ? cfg.transport.url : ""}
              onChange={(e) =>
                cfg.transport.type === "http" &&
                onChange({
                  ...cfg,
                  transport: { ...cfg.transport, url: e.target.value },
                })
              }
              placeholder="https://open.平台域名.com/mcp/xxx/stream"
              className={inputCls}
              autoComplete="off"
              spellCheck={false}
            />
          </Field>
          <Field
            label="请求头（一行一个 KEY=VALUE）"
            hint="放访问令牌，例：Authorization=Bearer 你的密钥；只存本机，不进 git / 日志"
          >
            <textarea
              rows={2}
              value={headersText}
              onChange={(e) => {
                const t = e.target.value;
                setHeadersText(t);
                if (cfg.transport.type === "http") {
                  onChange({ ...cfg, transport: { ...cfg.transport, headers: textToEnv(t) } });
                }
              }}
              placeholder={"Authorization=Bearer sk-xxxx"}
              className={mcpTextareaCls}
              spellCheck={false}
              autoComplete="off"
            />
          </Field>
        </div>
      )}

      {test.s === "ok" && <p className="mt-2 text-xs text-emerald-700">✓ {test.msg}</p>}
      {test.s === "err" && (
        <p className="mt-2 text-xs text-red-600">
          ✗ 连接失败：{test.msg}
          <span className="block text-caption text-muted-foreground">
            提示：401 = 令牌不对或已过期（去平台重新生成）；403 = 该服务未开通或已到期；超时 =
            地址不对或网络不通。
          </span>
        </p>
      )}
    </div>
  );
}

// =============================================================================
// V0.2 D7 · 本地知识库三态卡 + 元典积分卡
// =============================================================================

const DEFAULT_KB_PATH = "~/Documents/知识库";

function LocalKbCard({
  kbRoot,
  kbEnabled,
  onKbRootChange,
  onKbEnabledChange,
}: {
  kbRoot: string | null;
  kbEnabled: boolean;
  onKbRootChange: (p: string | null) => void;
  onKbEnabledChange: (b: boolean) => void;
}) {
  const [status, setStatus] = useState<KbStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [busyMsg, setBusyMsg] = useState("");
  const [importResult, setImportResult] = useState<KbImportResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const s = await detectKbStatus();
      setStatus(s);
      setError(null);
    } catch (e) {
      setError(formatErr(e));
    }
  }, []);

  // 打开 Settings + kbRoot/kbEnabled 变化时重新检测
  useEffect(() => {
    refresh();
  }, [refresh, kbRoot, kbEnabled]);

  async function handleCreateDefault() {
    await handleCreate(DEFAULT_KB_PATH);
  }

  async function handleChoosePath() {
    try {
      const picked = await dialogOpen({ directory: true, multiple: false });
      if (typeof picked === "string" && picked.trim()) {
        await handleCreate(picked);
      }
    } catch (e) {
      setError(formatErr(e));
    }
  }

  async function handleMigrate() {
    if (status?.state !== "bound") return;
    setError(null);
    try {
      const picked = await dialogOpen({ directory: true, multiple: false });
      if (typeof picked !== "string" || !picked.trim()) return;
      const ok = await confirmDialog(
        `将当前知识库完整复制到：\n${picked}\n\n复制完成并核对文件数、字节数后才会切换目录；原目录会保留，不会删除。目标目录必须为空。`,
        { title: "一键迁移知识库", okLabel: "开始迁移" },
      );
      if (!ok) return;
      setBusy(true);
      setBusyMsg("正在复制并校验知识库…");
      const result = await migrateLocalKb(picked);
      onKbRootChange(result.target);
      onKbEnabledChange(true);
      setBusyMsg(
        `迁移完成 · ${result.files_copied} 个文件 · ${formatBytes(result.bytes_copied)}；原目录已保留`,
      );
      await refresh();
    } catch (e) {
      setError(formatErr(e));
    } finally {
      setBusy(false);
      window.setTimeout(() => setBusyMsg(""), 7000);
    }
  }

  async function handleCreate(path: string) {
    setBusy(true);
    setBusyMsg("创建中…");
    setError(null);
    try {
      const r = await createLocalKb(path);
      onKbRootChange(path);
      onKbEnabledChange(true);
      setBusyMsg(
        r.reused_existing
          ? `已绑定到已有目录(补 ${r.dirs_created} 个子目录)`
          : `新建成功(${r.dirs_created} 目录 / ${r.files_created} 文件)`,
      );
      await refresh();
    } catch (e) {
      setError(formatErr(e));
    } finally {
      setBusy(false);
      window.setTimeout(() => setBusyMsg(""), 3000);
    }
  }

  async function handleImport() {
    setError(null);
    setImportResult(null);
    try {
      const picked = await dialogOpen({
        directory: false,
        multiple: false,
        filters: [{ name: "CaseBoard KB 资料包", extensions: ["zip"] }],
      });
      if (typeof picked !== "string" || !picked.trim()) return;
      setBusy(true);
      setBusyMsg("导入中…");
      // 默认 OverwriteOlder(智能合并 — 旧的覆盖,新的保留)
      const strategy: KbConflictStrategy = "overwrite_older";
      const r = await importKbFromZip(picked, strategy);
      setImportResult(r);
      setBusyMsg(
        `导入完成:新增 ${r.added} / 跳过 ${r.skipped} / 覆盖 ${r.overwritten}${r.failed ? ` / 失败 ${r.failed}` : ""}`,
      );
      await refresh();
    } catch (e) {
      setError(formatErr(e));
    } finally {
      setBusy(false);
      window.setTimeout(() => setBusyMsg(""), 5000);
    }
  }

  async function handleExport() {
    setError(null);
    try {
      const today = todayIsoLocal();
      const picked = await dialogSave({
        defaultPath: `caseboard-kb-share-${today}.zip`,
        filters: [{ name: "Zip", extensions: ["zip"] }],
      });
      if (typeof picked !== "string" || !picked.trim()) return;
      setBusy(true);
      setBusyMsg("导出中…");
      const r = await exportKbToZip(picked);
      setBusyMsg(
        `导出完成 · ${r.total_items} 条 · ${formatBytes(r.total_size_bytes)}`,
      );
    } catch (e) {
      setError(formatErr(e));
    } finally {
      setBusy(false);
      window.setTimeout(() => setBusyMsg(""), 5000);
    }
  }

  // P2 · 清理过期检索缓存(只清搜索/向量列表,法规/案例/企业全文详情不动)。需二次确认。
  async function handlePruneCache() {
    setError(null);
    const ok = await confirmDialog(
      "清理 30 天前的检索列表缓存(法规/案例关键词检索、语义检索结果)?\n\n法规/法条/案例的全文详情、入库的企业档案都不会动,放心清。",
      { title: "清理过期检索缓存", okLabel: "清理", danger: true },
    );
    if (!ok) return;
    try {
      setBusy(true);
      setBusyMsg("清理中…");
      const r = await pruneYuandianCache(30);
      setBusyMsg(
        r.removed_entries === 0
          ? "没有 30 天前的检索缓存可清"
          : `已清理 ${r.removed_entries} 条过期检索缓存(删 ${r.removed_files} 个文件)`,
      );
      await refresh();
    } catch (e) {
      setError(formatErr(e));
    } finally {
      setBusy(false);
      window.setTimeout(() => setBusyMsg(""), 5000);
    }
  }

  return (
    <Section
      title="本地法律知识库"
      desc="启用后,法律检索优先查本地缓存,只在缺时调元典 — 大幅省积分。"
    >
      {/* 状态条 */}
      <div className="rounded-md border border-border bg-background p-3">
        {status === null && (
          <p className="text-xs text-muted-foreground">
            <Loader2 className="mr-1 inline size-3 animate-spin" />
            检测中…
          </p>
        )}

        {status?.state === "bound" && (
          <div className="space-y-2">
            <div className="flex items-center justify-between gap-2">
              <div className="flex min-w-0 items-center gap-2">
                <Database className="size-4 shrink-0 text-emerald-600" />
                <span className="truncate text-xs font-medium">
                  ✓ 已绑定 <span className="font-mono">{status.root}</span>
                </span>
              </div>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                onClick={refresh}
                title="重新检测"
                disabled={busy}
              >
                <RefreshCw className={cn("size-3.5", busy && "animate-spin")} />
              </Button>
            </div>
            <KbStatsRow status={status} />
            <div className="flex flex-wrap gap-1.5 pt-1">
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={() => openInDefaultApp(status.root)}
                disabled={busy}
              >
                <FolderOpen className="size-3.5" />
                打开目录
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={handleChoosePath}
                disabled={busy}
              >
                <FolderOpen className="size-3.5" />
                更换目录
              </Button>
              <HoverHint hint="复制全部知识库资料到空目录，校验成功后自动切换；原目录保留，不会删除">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={handleMigrate}
                  disabled={busy}
                >
                  <RefreshCw className="size-3.5" />
                  一键迁移
                </Button>
              </HoverHint>
              <HoverHint hint="导入同事的元典缓存资料包,自动查重合并;只合并元典缓存,不碰你的笔记/案件/客户">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={handleImport}
                  disabled={busy}
                >
                  <Upload className="size-3.5" />
                  导入资料包
                </Button>
              </HoverHint>
              <HoverHint
                hint={
                  status.cache_count === 0
                    ? "知识库还没缓存,无内容可导(本功能仅导出元典缓存)"
                    : "仅导出元典缓存(法规/案例/企业查询结果),不含你的笔记/案件/客户信息"
                }
              >
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={handleExport}
                  disabled={busy || status.cache_count === 0}
                >
                  <Download className="size-3.5" />
                  导出资料包
                </Button>
              </HoverHint>
              <HoverHint hint="清理 30 天前的法规/案例检索列表 + 语义检索结果(全文详情、企业档案不动);去冗余、腾空间。需二次确认">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={handlePruneCache}
                  disabled={busy || status.cache_count === 0}
                >
                  <Trash2 className="size-3.5" />
                  清理过期缓存
                </Button>
              </HoverHint>
            </div>
          </div>
        )}

        {status?.state === "unbound" && (
          <div className="space-y-2.5">
            <div className="flex items-center gap-2 text-xs">
              <AlertTriangle className="size-4 shrink-0 text-amber-500" />
              <span className="font-medium">未检测到本地知识库</span>
              {status.configured_root && (
                <span className="text-muted-foreground">
                  · 默认路径 <span className="font-mono">{status.configured_root}</span> 不存在
                </span>
              )}
            </div>
            <p className="text-label text-muted-foreground">
              本地知识库让法律检索先查本地、只在缺时调元典,大幅节省积分。
            </p>
            <div className="flex flex-wrap gap-1.5">
              <Button
                type="button"
                size="sm"
                onClick={handleCreateDefault}
                disabled={busy}
              >
                <Sparkles className="size-3.5" />
                在 {DEFAULT_KB_PATH} 新建
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={handleChoosePath}
                disabled={busy}
              >
                <FolderOpen className="size-3.5" />
                选择其他路径…
              </Button>
              <HoverHint hint="需先新建或选定一个知识库目录再导入。导入的是元典缓存资料包,不含笔记/案件/客户">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={handleImport}
                  disabled={busy}
                >
                  <Upload className="size-3.5" />
                  导入资料包
                </Button>
              </HoverHint>
            </div>
          </div>
        )}

        {status?.state === "permission_denied" && (
          <div className="space-y-2">
            <div className="flex items-center gap-2 text-xs">
              <AlertTriangle className="size-4 shrink-0 text-red-600" />
              <span className="font-medium">
                🔒 <span className="font-mono">{status.root}</span> 存在,但 CaseBoard 无访问权限
              </span>
            </div>
            <p className="text-label text-muted-foreground">
              请检查该目录的系统访问权限、网盘同步状态或安全软件限制；也可以重新选择一个可读写目录。
            </p>
            <div className="flex flex-wrap gap-1.5">
              <Button
                type="button"
                size="sm"
                onClick={handleChoosePath}
                disabled={busy}
              >
                <FolderOpen className="size-3.5" />
                重新选择目录
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={refresh}
                disabled={busy}
              >
                <RefreshCw className="size-3.5" />
                重新检查
              </Button>
            </div>
          </div>
        )}
      </div>

      {/* busy / 错误 / 导入摘要 */}
      {busyMsg && (
        <p className="text-xs text-muted-foreground">
          {busy && <Loader2 className="mr-1 inline size-3 animate-spin" />}
          {busyMsg}
        </p>
      )}
      {error && (
        <p className="text-xs text-red-600">
          <XCircle className="mr-1 inline size-3" />
          {error}
        </p>
      )}
      {importResult && importResult.conflicts.length > 0 && (
        <details className="text-xs text-muted-foreground">
          <summary className="cursor-pointer">查看 {importResult.conflicts.length} 条冲突明细</summary>
          <ul className="mt-1 max-h-32 space-y-0.5 overflow-y-auto pl-3">
            {importResult.conflicts.slice(0, 50).map((c, i) => (
              <li key={i} className="font-mono text-caption">
                <span
                  className={cn(
                    c.action === "failed" && "text-red-600",
                    c.action === "overwrite" && "text-amber-600",
                  )}
                >
                  [{c.action}]
                </span>{" "}
                {c.path} — {c.reason}
              </li>
            ))}
          </ul>
        </details>
      )}

      {/* 高级:路径手填 + 总开关 */}
      <details className="text-xs">
        <summary className="cursor-pointer text-muted-foreground">高级设置</summary>
        <div className="mt-2 space-y-2 rounded border border-border bg-background/50 p-2.5">
          <Field label="知识库路径(手填,支持 ~/)">
            <input
              type="text"
              value={kbRoot ?? ""}
              onChange={(e) => onKbRootChange(e.target.value || null)}
              placeholder="~/Documents/知识库"
              className={cn(inputCls, "font-mono")}
            />
          </Field>
          <label className="flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              checked={kbEnabled}
              onChange={(e) => onKbEnabledChange(e.target.checked)}
              className="size-3.5"
            />
            <span>启用本地优先(关闭后所有检索直接调元典)</span>
          </label>
        </div>
      </details>
    </Section>
  );
}

function KbStatsRow({
  status,
}: {
  status: Extract<KbStatus, { state: "bound" }>;
}) {
  const breakdownText = Object.entries(status.cache_breakdown)
    .filter(([, n]) => n > 0)
    .map(([k, n]) => `${k} ${n}`)
    .join(" / ");
  return (
    <ul className="grid grid-cols-2 gap-x-3 gap-y-0.5 text-label text-muted-foreground">
      <li>
        已检索内容:
        <span className="ml-1 font-medium text-foreground">
          {status.content_count} 篇
        </span>
      </li>
      <li>
        元典缓存:
        <span className="ml-1 font-medium text-foreground">
          {status.cache_count}
        </span>
        {breakdownText && (
          <span className="text-muted-foreground/70"> ({breakdownText})</span>
        )}
      </li>
      <li>
        占用:
        <span className="ml-1 font-medium text-foreground">
          {status.total_size_bytes != null
            ? formatBytes(status.total_size_bytes)
            : "—"}
        </span>
      </li>
      <li className="col-span-2">
        最近写入:
        <span className="ml-1 font-medium text-foreground">
          {status.last_write_at ? formatDateTime(status.last_write_at) : "—"}
        </span>
      </li>
    </ul>
  );
}

function YuandianCreditsCard({
  hasApiKey,
  monthlyLimit,
  onLimitChange,
}: {
  hasApiKey: boolean;
  monthlyLimit: number | null;
  onLimitChange: (n: number | null) => void;
}) {
  const [overview, setOverview] = useState<CreditsOverview | null>(null);
  const [balance, setBalance] = useState<YuandianBalance | null>(null);
  const [loading, setLoading] = useState(false);
  const [balanceError, setBalanceError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setBalanceError(null);
    try {
      try {
        setOverview(await getYuandianCreditsOverview());
      } catch {
        // 本机账从未使用或数据库暂不可用时保持空态。
      }
      if (hasApiKey) {
        try {
          const b = await getYuandianBalance(true);
          setBalance(b);
          if (b?.refresh_error) setBalanceError(b.refresh_error);
        } catch (e) {
          setBalanceError(formatErr(e));
          // 余额失败不影响本机月账；尽力读最后一次缓存。
          try {
            setBalance(await getYuandianBalance(false));
          } catch {
            // 无缓存时保持空态。
          }
        }
      } else {
        setBalance(null);
      }
    } finally {
      setLoading(false);
    }
  }, [hasApiKey]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const cur = overview?.current;
  const totalQueries = (cur?.api_calls ?? 0) + (cur?.kb_hits ?? 0);
  const kbHitRate =
    totalQueries > 0 ? Math.round(((cur?.kb_hits ?? 0) / totalQueries) * 100) : 0;
  // 跨月归 0:当月没用过但历史有数据 → 补显示上月/累计,免得以为数据丢了
  const showHistory =
    (cur?.credits_used ?? 0) === 0 && (overview?.total_credits ?? 0) > 0;
  const comparisonText = (() => {
    if (!hasApiKey) return "配置并保存元典 API Key 后，可免费查询官方剩余积分。";
    if (!balance) return loading ? "正在查询元典 MCP 官方余额…" : "尚未取得余额。";
    const official = balance.official_spent_since_previous ?? 0;
    const local = balance.local_recorded_since_previous ?? 0;
    switch (balance.comparison_status) {
      case "matched":
        return `与上次快照相比，官方余额减少 ${official} 积分，本机记录 ${local} 积分，对账一致。`;
      case "difference": {
        const diff = balance.difference ?? 0;
        return diff > 0
          ? `本区间官方余额减少 ${official}，本机记录 ${local}，相差 ${diff} 积分；通常表示还有其他客户端调用，或存在尚未纳入本机账的接口。`
          : `本区间官方余额减少 ${official}，本机记录 ${local}，本机多记 ${Math.abs(diff)} 积分；可能存在返还、计价变化或刷新时点差。`;
      }
      case "recharged":
        return `余额较上次增加 ${balance.balance_increased_since_previous ?? 0} 积分，可能发生充值或赠送；本区间不比较消耗。`;
      case "local_reset":
        return "本机积分账曾被清理或重置，本区间不作差额判断。";
      default:
        return "已建立官方余额基线；下次刷新时开始对比余额减少量与本机记账。";
    }
  })();
  const fetchedTime = balance
    ? new Date(balance.fetched_at).toLocaleString("zh-CN", { hour12: false })
    : null;

  return (
    <Section
      title="元典积分账"
      desc="MCP 官方余额 + CaseBoard 本机消耗账 + 本地 KB 节省"
    >
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <Stat
          icon={<Coins className="size-4 text-amber-600" />}
          label="元典官方剩余"
          value={balance ? balance.point_balance.toLocaleString("zh-CN") : "—"}
          suffix="积分"
          right={
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={refresh}
              disabled={loading || !hasApiKey}
              title="刷新官方余额和本机账"
            >
              <RefreshCw className={cn("size-3", loading && "animate-spin")} />
            </Button>
          }
        />
        <Stat
          icon={<Database className="size-4 text-sky-600" />}
          label={`本月已用(${cur?.year_month ?? "—"})`}
          value={cur?.credits_used ?? 0}
          suffix="积分"
        />
        <Stat
          icon={<Database className="size-4 text-emerald-600" />}
          label="本地命中节省"
          value={cur?.kb_hits ?? 0}
          suffix={`次 (命中率 ${kbHitRate}%)`}
        />
      </div>
      <p className="text-caption text-muted-foreground">
        {comparisonText}
        {balance?.count_balance
          ? ` 另有 ${balance.count_balance} 次权益余额。`
          : ""}
        {fetchedTime ? ` · 余额更新于 ${fetchedTime}` : ""}
        {balance?.cached ? "（当前显示缓存）" : ""}
      </p>
      {balanceError && (
        <p className="-mt-1 rounded-md bg-amber-50 px-2.5 py-1.5 text-caption text-amber-800 dark:bg-amber-950/30 dark:text-amber-300">
          官方余额刷新失败，已保留本机账和最后一次余额：{balanceError}
        </p>
      )}
      <p className="-mt-1 text-caption text-muted-foreground">
        本月记录 {cur?.api_calls ?? 0} 次在线调用；本机账按实际端点目录价累计。余额差额可能包含其他 MCP/客户端调用、充值赠送及平台计价变化，不会自动改写本机账。
      </p>
      {showHistory && (
        <p className="-mt-1 rounded-md bg-sky-50 px-2.5 py-1.5 text-caption text-sky-700 dark:bg-sky-950/30 dark:text-sky-300">
          本月暂未使用(每月 1 号归零)。
          {overview?.prev_month &&
            ` 上月(${overview.prev_month.year_month})用了 ${overview.prev_month.credits_used} 积分;`}
          {` 历史累计 ${overview?.total_credits ?? 0} 积分 / 帮你省了 ${overview?.total_kb_hits ?? 0} 次外查。`}
        </p>
      )}
      <Field
        label="月度上限(超出后 chat 自动降级,不再调元典)"
        hint="留空 = 不限制"
      >
        <input
          type="number"
          min={0}
          step={10}
          value={monthlyLimit ?? ""}
          onChange={(e) => {
            const v = e.target.value.trim();
            onLimitChange(v === "" ? null : Math.max(0, Number(v)));
          }}
          placeholder="留空 = 不限"
          className={inputCls}
        />
      </Field>
    </Section>
  );
}

function Stat({
  icon,
  label,
  value,
  suffix,
  right,
}: {
  icon: React.ReactNode;
  label: string;
  value: number | string;
  suffix?: string;
  right?: React.ReactNode;
}) {
  return (
    <div className="flex items-start justify-between rounded-md border border-border bg-background p-2.5">
      <div className="flex min-w-0 items-start gap-2">
        <div className="mt-0.5 shrink-0">{icon}</div>
        <div className="min-w-0">
          <p className="truncate text-caption text-muted-foreground">{label}</p>
          <p className="text-sm font-semibold tabular-nums">
            {value}
            {suffix && (
              <span className="ml-1 text-caption font-normal text-muted-foreground">
                {suffix}
              </span>
            )}
          </p>
        </div>
      </div>
      {right}
    </div>
  );
}

function formatErr(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function formatDateTime(iso: string): string {
  try {
    const d = new Date(iso);
    if (isNaN(d.getTime())) return iso;
    return d.toLocaleString("zh-CN", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return iso;
  }
}

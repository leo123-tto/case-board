import { useCallback, useEffect, useState } from "react";
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
} from "lucide-react";
import { open as dialogOpen, save as dialogSave } from "@tauri-apps/plugin-dialog";
import { confirmDialog } from "@/lib/dialog";

import { Button } from "@/components/ui/button";
import { HoverHint } from "@/components/HoverHint";
import { toast } from "@/components/ui/toast";
import {
  createLocalKb,
  detectKbStatus,
  exportKbToZip,
  getSettings,
  getYuandianCreditsOverview,
  importKbFromZip,
  pruneYuandianCache,
  openInDefaultApp,
  openUrl,
  saveSettings,
  testFeishuNotify,
  verifyCloudLlmKey,
  verifyMinerUKey,
  verifyEmbeddingKey,
  verifyYuandianKey,
  type KbConflictStrategy,
  type KbImportResult,
  type KbStatus,
  type CreditsOverview,
} from "@/lib/api";
import type { Settings, McpServerConfig, CloudProviderId } from "@/lib/types";
import { CLOUD_PROVIDERS } from "@/lib/types";
import { cn } from "@/lib/utils";

type VerifyStatus = "idle" | "verifying" | "ok" | "fail";

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
}

function cloudKeyField(providerId: CloudProviderId): keyof Settings {
  if (providerId === "mimo") return "mimo_api_key";
  if (providerId === "glm") return "glm_api_key";
  if (providerId === "custom") return "custom_api_key";
  if (providerId === "minimax") return "minimax_api_key";
  return "deepseek_api_key";
}

function cloudVerifiedField(providerId: CloudProviderId): keyof Settings {
  if (providerId === "mimo") return "mimo_verified_at";
  if (providerId === "glm") return "glm_verified_at";
  if (providerId === "custom") return "custom_verified_at";
  if (providerId === "minimax") return "minimax_verified_at";
  return "deepseek_verified_at";
}

function getCloudKey(settings: Settings, providerId: CloudProviderId): string {
  const specific = settings[cloudKeyField(providerId)];
  const fallback =
    providerId === "minimax" ? settings.minimax_api_key : settings.cloud_llm_api_key;
  return (typeof specific === "string" && specific.trim() ? specific : fallback) ?? "";
}

function getCloudVerifiedAt(settings: Settings, providerId: CloudProviderId): string | null {
  const specific = settings[cloudVerifiedField(providerId)];
  const fallback = providerId === "minimax" ? settings.minimax_verified_at : settings.deepseek_verified_at;
  return (typeof specific === "string" && specific.trim() ? specific : fallback) ?? null;
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
}: Props) {
  const isPage = mode === "page";
  const handleClose = () => {
    if (onClose) onClose();
  };
  const [settings, setSettings] = useState<Settings | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false); // page 模式下保存成功显示 toast(modal 模式直接关闭)
  // 2026-05-25 V0.1.8 · 是否有未保存改动(page 模式上报给父组件做切 tab 防呆)
  const [dirty, setDirty] = useState(false);

  // 2026-05-25 V0.1.6 · token 在线验证状态
  const [mineruStatus, setMineruStatus] = useState<VerifyStatus>("idle");
  const [mineruMsg, setMineruMsg] = useState<string>("");
  const [deepseekStatus, setDeepseekStatus] = useState<VerifyStatus>("idle");
  const [deepseekMsg, setDeepseekMsg] = useState<string>("");
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
    const providerId = (
      settings.cloud_llm_provider ??
      ((settings.cloud_llm_backend ?? "") === "minimax" ? "minimax" : "deepseek")
    ) as CloudProviderId;
    if (getCloudVerifiedAt(settings, providerId) && deepseekStatus === "idle") {
      setDeepseekStatus("ok");
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
    settings?.deepseek_verified_at,
    settings?.mimo_verified_at,
    settings?.glm_verified_at,
    settings?.custom_verified_at,
    settings?.minimax_verified_at,
    settings?.cloud_llm_provider,
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
      await saveSettings(settings);
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

  // page 模式:没有蒙层,卡片直接占主区域,scroll 由父容器管;不带 X 按钮
  // modal 模式:蒙层 + max-h 限高 + X 按钮(原有形态)
  // 注意:不能用内嵌函数组件 wrap children,那会让每次 render 重建组件类型 → 子树 unmount + state 丢失
  // 改用条件渲染同一 body JSX,React 会正确 diff
  const body = (
    <>
        {/* 标题栏 */}
        <header className="flex items-center justify-between gap-4 border-b border-border bg-card/95 px-5 py-3.5 backdrop-blur">
          <div>
            <h2
              className={cn(
                "font-semibold text-foreground",
                isPage ? "text-lg" : "text-base",
              )}
            >
              设置
            </h2>
            <p className="mt-0.5 text-xs text-muted-foreground">
              填你自己的 token。每个用户填自己的,工具不内置任何人的 key。
            </p>
          </div>
          {!isPage && (
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
        <div className="flex-1 overflow-auto px-5 py-5">
          {loading && (
            <div className="flex items-center justify-center py-8">
              <Loader2 className="size-5 animate-spin text-muted-foreground" />
            </div>
          )}
          {!loading && settings && (
            <div
              className={cn(
                // page 模式:每个功能区各占一半,左右成对(更简洁、少占行);
                // 窗口恒 ≥1024(minWidth),lg 断点始终生效 → 默认就是两列。
                // modal 模式:保持单列堆叠,窄弹窗里两列会挤。
                isPage
                  ? "grid grid-cols-1 lg:grid-cols-2 gap-x-5 gap-y-6 items-start"
                  : "space-y-6",
              )}
            >
              {/* 个人信息 */}
              <Section title="个人信息">
                <Field label="称呼" hint="首页问候用,例:刘律师 / 周律师 / 李三">
                  <input
                    type="text"
                    value={settings.user_display_name ?? ""}
                    onChange={(e) =>
                      updateField("user_display_name", e.target.value || null)
                    }
                    placeholder="例:刘律师"
                    className={inputCls}
                  />
                </Field>
              </Section>

              <Section title="飞书同步">
                <label className="flex items-center gap-2 text-sm text-foreground">
                  <input
                    type="checkbox"
                    checked={!!settings.feishu_enabled}
                    onChange={(e) =>
                      updateField("feishu_enabled", e.target.checked)
                    }
                    className="size-4"
                  />
                  启用案件池同步
                </label>
                <Field label="多维表格 App Token">
                  <input
                    type="text"
                    value={settings.feishu_app_token ?? ""}
                    onChange={(e) =>
                      updateField("feishu_app_token", e.target.value || null)
                    }
                    placeholder="app token"
                    className={inputCls}
                    autoComplete="off"
                  />
                </Field>
                <Field label="案件池 Table ID">
                  <input
                    type="text"
                    value={settings.feishu_cases_table_id ?? ""}
                    onChange={(e) =>
                      updateField("feishu_cases_table_id", e.target.value || null)
                    }
                    placeholder="tbl..."
                    className={inputCls}
                    autoComplete="off"
                  />
                </Field>
                <Field label="日历表 Table ID">
                  <input
                    type="text"
                    value={settings.feishu_calendar_table_id ?? ""}
                    onChange={(e) =>
                      updateField("feishu_calendar_table_id", e.target.value || null)
                    }
                    placeholder="tbl...（可选，用于首页日历事件同步）"
                    className={inputCls}
                    autoComplete="off"
                  />
                </Field>
              </Section>

              <Section title="飞书到期推送">
                <label className="flex items-center gap-2 text-sm text-foreground">
                  <input
                    type="checkbox"
                    checked={!!settings.feishu_notify_enabled}
                    onChange={(e) =>
                      updateField("feishu_notify_enabled", e.target.checked)
                    }
                    className="size-4"
                  />
                  启用到期事项推送
                </label>
                <Field label="接收人 User ID">
                  <input
                    type="text"
                    value={settings.feishu_notify_user_id ?? ""}
                    onChange={(e) =>
                      updateField("feishu_notify_user_id", e.target.value || null)
                    }
                    placeholder="ou_xxx（飞书个人页面可获取）"
                    className={inputCls}
                    autoComplete="off"
                  />
                </Field>
                <Field label="提前提醒天数">
                  <input
                    type="number"
                    value={settings.feishu_notify_days_before ?? 7}
                    onChange={(e) =>
                      updateField("feishu_notify_days_before", parseInt(e.target.value) || 7)
                    }
                    placeholder="7"
                    className={inputCls}
                    min={1}
                    max={90}
                  />
                </Field>
                <FeishuNotifyTestButton />
              </Section>

              {/* V0.3:本地模型已隐藏 → 只走云端。三个 API key(MinerU / DeepSeek / 元典)常显,
                  不再用 cloud_enabled 开关包裹(该字段保留兼容,前端不再读)。 */}
              <>
                  <Section
                    title="MinerU"
                    link={{ label: "点这里申请 token", href: "https://mineru.net/apiManage/token" }}
                  >
                    <Field label="API Token">
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
                          placeholder="eyJ0eXBl..."
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

                  {(() => {
                    const providerId = (
                      settings.cloud_llm_provider ??
                      ((settings.cloud_llm_backend ?? "") === "minimax" ? "minimax" : "deepseek")
                    ) as CloudProviderId;
                    const prov = CLOUD_PROVIDERS[providerId] ?? CLOUD_PROVIDERS.deepseek;
                    const hasEndpointInput = providerId === "custom" || providerId === "minimax";
                    const hasTextInput = providerId === "custom" || providerId === "minimax";
                    const providerKey = getCloudKey(settings, providerId);
                    const providerVerifiedAt = getCloudVerifiedAt(settings, providerId);
                    const providerKeyField = cloudKeyField(providerId);
                    const providerVerifiedField = cloudVerifiedField(providerId);

                    const handleProviderChange = (newId: string) => {
                      const id = newId as CloudProviderId;
                      updateField("cloud_llm_provider", id === "deepseek" ? null : id);
                      updateField("cloud_llm_backend", null);
                      if (id === "minimax") {
                        if (!settings.cloud_llm_api_key?.trim() && settings.minimax_api_key?.trim()) {
                          updateField("cloud_llm_api_key", settings.minimax_api_key);
                        }
                        if (!settings.cloud_llm_endpoint?.trim()) {
                          updateField("cloud_llm_endpoint", settings.minimax_endpoint || null);
                        }
                        if (!settings.cloud_llm_model?.trim()) {
                          updateField("cloud_llm_model", settings.minimax_model || null);
                        }
                      } else {
                        updateField("cloud_llm_endpoint", null);
                        updateField("cloud_llm_model", null);
                      }
                      setDeepseekStatus(getCloudVerifiedAt(settings, id) ? "ok" : "idle");
                      setDeepseekMsg("");
                    };

                    return (
                      <Section
                        title="云端大模型"
                        link={
                          prov.keyUrl
                            ? { label: `去 ${prov.label} 拿 API Key`, href: prov.keyUrl }
                            : undefined
                        }
                      >
                        <Field label="提供商">
                          <select
                            value={providerId}
                            onChange={(e) => handleProviderChange(e.target.value)}
                            className={inputCls}
                          >
                            {Object.entries(CLOUD_PROVIDERS).map(([id, p]) => (
                              <option key={id} value={id}>
                                {p.label}
                              </option>
                            ))}
                          </select>
                        </Field>
                        <Field label="API Key">
                          <div className="flex items-center gap-2">
                            <input
                              type="password"
                              value={providerKey}
                              onChange={(e) => {
                                const nextKey = e.target.value || null;
                                updateField(providerKeyField, nextKey as any);
                                updateField("cloud_llm_api_key", nextKey);
                                if (deepseekStatus !== "idle") {
                                  setDeepseekStatus("idle");
                                  setDeepseekMsg("");
                                  updateField(providerVerifiedField, null as any);
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
                              onClick={async () => {
                                if (!providerKey.trim()) {
                                  setDeepseekStatus("fail");
                                  setDeepseekMsg("请先填入 API Key");
                                  return;
                                }
                                setDeepseekStatus("verifying");
                                setDeepseekMsg("");
                                try {
                                  const r = await verifyCloudLlmKey(
                                    providerId,
                                    providerKey,
                                    settings.cloud_llm_endpoint ?? undefined,
                                  );
                                  if (r.ok) {
                                    const verifiedAt = new Date().toISOString();
                                    const nextSettings = {
                                      ...settings,
                                      [providerKeyField]: providerKey,
                                      [providerVerifiedField]: verifiedAt,
                                      cloud_llm_api_key: providerKey,
                                    } as Settings;
                                    setDeepseekStatus("ok");
                                    setDeepseekMsg("");
                                    setSettings(nextSettings);
                                    await saveSettings(nextSettings);
                                    setDirty(false);
                                    onSaved?.();
                                  } else {
                                    setDeepseekStatus("fail");
                                    setDeepseekMsg(r.message);
                                    updateField(providerVerifiedField, null as any);
                                  }
                                } catch (e) {
                                  setDeepseekStatus("fail");
                                  setDeepseekMsg(String(e));
                                  updateField(providerVerifiedField, null as any);
                                }
                              }}
                              disabled={
                                deepseekStatus === "verifying" ||
                                !providerKey.trim()
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
                            <p className="mt-1.5 text-xs text-red-600">✗ {deepseekMsg}</p>
                          )}
                          {(deepseekStatus === "ok" || providerVerifiedAt) && (
                            <p className="mt-1.5 text-xs text-green-700">✓ 已验证通过,可以使用</p>
                          )}
                        </Field>
                          {hasEndpointInput && (
                            <Field
                              label="Endpoint (base URL)"
                              hint={
                                providerId === "minimax"
                                  ? "留空默认 https://api.minimaxi.com"
                                  : "如 https://api.openai.com"
                              }
                            >
                              <input
                                type="text"
                                value={settings.cloud_llm_endpoint ?? ""}
                              onChange={(e) =>
                                updateField("cloud_llm_endpoint", e.target.value || null)
                              }
                              placeholder={
                                providerId === "minimax"
                                  ? "https://api.minimaxi.com"
                                  : "https://api.example.com"
                              }
                              className={inputCls}
                            />
                          </Field>
                        )}
                        <Field label="模型档位">
                          {hasTextInput ? (
                            <input
                              type="text"
                              value={settings.cloud_llm_model ?? ""}
                              onChange={(e) =>
                                updateField("cloud_llm_model", e.target.value || null)
                              }
                              placeholder={
                                providerId === "minimax"
                                  ? "MiniMax-M2"
                                  : "gpt-4o / qwen-plus / ..."
                              }
                              className={inputCls}
                            />
                          ) : (
                            <select
                              value={settings.cloud_llm_model ?? prov.flash}
                              onChange={(e) =>
                                updateField("cloud_llm_model", e.target.value || null)
                              }
                              className={inputCls}
                            >
                              <option value={prov.flash}>快档 · {prov.flash}</option>
                              <option value={prov.pro}>强档 · {prov.pro}</option>
                              {prov.thinking && (
                                <option value={prov.thinking}>思考 · {prov.thinking}</option>
                              )}
                              <option value="auto">自动挡 · 简单走快档、复杂走强档</option>
                            </select>
                          )}
                          <p className="mt-1 text-label text-muted-foreground">
                            {providerId === "minimax"
                              ? "MiniMax 模型名可直接填写。留空默认 MiniMax-M2。"
                              : "全程按这个档位走。觉得效果不够就换强档或自动挡。"}
                          </p>
                          {providerId === "mimo" && (
                            <p className="mt-1 text-label text-muted-foreground">
                              MiMo 上下文窗口比 DeepSeek 小，超大案件(几十份文档)建议切回 DeepSeek。
                            </p>
                          )}
                        </Field>
                      </Section>
                    );
                  })()}
                </>

              {/* 元典法律开放平台 — 法规/案例/企业信息检索 + 执行查被执行人,跟云端 LLM 独立 */}
              <Section
                title="元典法律开放平台"
                desc="查询法律法规、裁判案例、企业信息的数据源"
                link={{
                  label: "申请 API key",
                  href: "https://open.chineselaw.com/",
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

              {/* 硅基流动 — Embedding 语义检索,填了才启用,否则回退关键词选材料。
                  接口地址 / 模型不暴露:留空后端默认硅基流动 bge-m3(免费),不需要改。 */}
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

              {/* 快递100 — 快递查询工具用(寄送达 / 材料追踪),独立可选 */}
              <Section
                title="快递100"
                desc="工具页「快递查询」用"
                link={{
                  label: "点这里申请 customer / key",
                  href: "https://api.kuaidi100.com/",
                }}
              >
                <Field label="customer(授权码)">
                  <input
                    type="text"
                    value={settings.kuaidi100_customer ?? ""}
                    onChange={(e) =>
                      updateField("kuaidi100_customer", e.target.value || null)
                    }
                    placeholder="快递100 后台的 customer"
                    className={inputCls}
                    autoComplete="off"
                  />
                </Field>
                <Field label="key(授权 key)">
                  <input
                    type="password"
                    value={settings.kuaidi100_key ?? ""}
                    onChange={(e) =>
                      updateField("kuaidi100_key", e.target.value || null)
                    }
                    placeholder="快递100 后台的 key"
                    className={inputCls}
                    autoComplete="off"
                  />
                </Field>
              </Section>

              {/* V0.2 D7 · 元典积分账(本月统计) */}
              <YuandianCreditsCard
                monthlyLimit={settings.yuandian_monthly_credit_limit ?? null}
                onLimitChange={(n) =>
                  updateField("yuandian_monthly_credit_limit", n)
                }
              />

              {/* V0.2 D7 · 本地知识库三态卡 */}
              <LocalKbCard
                kbRoot={settings.local_kb_root ?? null}
                kbEnabled={settings.local_kb_enabled !== false}
                onKbRootChange={(p) => updateField("local_kb_root", p)}
                onKbEnabledChange={(b) => updateField("local_kb_enabled", b)}
              />

              {/* V0.3:本地模型已隐藏 → 删「各模块走本机/云端」切换器 + 本机模型(ollama)配置段。
                  字段(ocr_provider/llm_provider/ollama_*)保留在后端/types,以后接新本地模型再恢复 UI。 */}

              {/* V0.3.6 · 外部工具(MCP)白名单 —— 整宽,AI 助手消费外部 MCP server 工具 */}
              <McpServersCard
                servers={settings.mcp_servers ?? []}
                onChange={(next) => updateField("mcp_servers", next)}
              />


              {/* 律师档案管理 */}

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
          )}

          {/* 项目署名 */}
          <div className="mt-2 border-t border-border pt-5 text-center">
            <p className="text-sm font-medium text-foreground">
              CaseBoard · 案件看板
            </p>
            <p className="mt-0.5 text-xs text-muted-foreground">
              开源项目 · PolyForm Noncommercial 1.0.0
            </p>
          </div>
        </div>

        {/* 底部按钮栏 */}
        <footer className="flex items-center justify-between gap-4 border-t border-border bg-card/95 px-5 py-3 backdrop-blur">
          <span
            className={cn(
              "text-caption",
              saved
                ? "text-green-700 animate-in fade-in-0 duration-200"
                : "text-muted-foreground",
            )}
          >
            {saved
              ? "✓ 已保存 · 下次导入案件时生效(已在跑的任务不切换后端)"
              : settings === null
                ? ""
                : dirty
                  ? "● 有未保存改动 · 别忘了点保存"
                  : "改完点保存"}
          </span>
          <div className="flex gap-2">
            {!isPage && (
              <Button variant="outline" size="sm" onClick={handleClose}>
                取消
              </Button>
            )}
            <Button
              size="sm"
              onClick={handleSave}
              disabled={saving || !settings || (isPage && !dirty)}
            >
              {saving ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <Save className="size-3.5" />
              )}
              保存
            </Button>
          </div>
        </footer>
    </>
  );

  if (isPage) {
    return (
      <div className="mx-auto flex h-full w-full max-w-5xl flex-col overflow-hidden">
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
  "h-9 w-full rounded-md border border-border bg-background px-3 text-sm",
  "placeholder:text-muted-foreground/60",
  "transition-[border-color,box-shadow]",
  "focus:outline-none focus:border-foreground focus:ring-1 focus:ring-foreground/20",
);

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
}: {
  title: string;
  desc?: string;
  link?: { label: string; href: string };
  children: React.ReactNode;
}) {
  return (
    <section>
      <div className="mb-3 flex items-start justify-between gap-3">
        <div>
          <h3 className="text-base font-semibold text-foreground">{title}</h3>
          {desc && <p className="mt-0.5 text-xs text-muted-foreground">{desc}</p>}
        </div>
        {link && (
          <button
            type="button"
            onClick={() => openUrl(link.href).catch((e) => console.warn("openUrl failed", e))}
            className="inline-flex shrink-0 items-center gap-1.5 rounded-md border border-sky-200 bg-sky-50 px-2.5 py-1 text-xs font-medium text-sky-700 transition-colors hover:border-sky-300 hover:bg-sky-100"
            title={link.href}
          >
            <ExternalLink className="size-3.5" />
            {link.label}
          </button>
        )}
      </div>
      <div className="space-y-3 rounded-lg border border-border bg-background/50 p-4">
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
  function addServer() {
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
              让 AI 助手调外部 MCP server 的工具（加能力不必更新 App）。当前支持本地命令（stdio）。
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
          {rows.length === 0 && (
            <p className="text-xs text-muted-foreground">
              还没有配置外部工具。点下方按钮添加一个，例如文件系统、网页抓取等 MCP server。
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

          <button
            type="button"
            onClick={addServer}
            className="inline-flex items-center gap-1.5 rounded-md border border-dashed border-border px-3 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:border-foreground/40 hover:text-foreground"
          >
            <Plus className="size-3.5" />
            添加 MCP server
          </button>
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
        <div className="rounded-md border border-amber-200 bg-amber-50 p-2.5 text-xs text-amber-800">
          这是 HTTP 类型的 server，当前版本暂未实现 HTTP 传输，会被自动跳过。如需使用请删除后改配本地命令（stdio）。
        </div>
      )}
    </div>
  );
}

// =============================================================================
// V0.2 D7 · 本地知识库三态卡 + 元典积分卡
// =============================================================================

/** macOS Documents/Desktop 权限被拒时,这个 URL 直接打开系统设置 → 文件与文件夹 */
const MACOS_PRIVACY_FILES_URL =
  "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders";
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
      const today = new Date().toISOString().slice(0, 10);
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
      desc="启用后,法律检索优先查本地缓存,只在缺时调元典 — 大幅省积分。所有元典外查结果会自动反哺本地。"
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
              请到 系统设置 → 隐私与安全 → 文件与文件夹 → CaseBoard → 勾选"文稿"。
            </p>
            <div className="flex flex-wrap gap-1.5">
              <Button
                type="button"
                size="sm"
                onClick={() => openUrl(MACOS_PRIVACY_FILES_URL).catch(() => {})}
              >
                <ExternalLink className="size-3.5" />
                打开系统设置
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
  monthlyLimit,
  onLimitChange,
}: {
  monthlyLimit: number | null;
  onLimitChange: (n: number | null) => void;
}) {
  const [overview, setOverview] = useState<CreditsOverview | null>(null);
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const o = await getYuandianCreditsOverview();
      setOverview(o);
    } catch {
      // 静默 — 元典没用过时也可能是 0,无所谓
    } finally {
      setLoading(false);
    }
  }, []);

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

  return (
    <Section
      title="元典积分账"
      desc="本月已用积分 / 本地 KB 帮你省了多少次外查"
    >
      <div className="grid grid-cols-2 gap-3">
        <Stat
          icon={<Coins className="size-4 text-amber-600" />}
          label={`本月已用(${cur?.year_month ?? "—"})`}
          value={cur?.credits_used ?? 0}
          suffix="积分"
          right={
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={refresh}
              disabled={loading}
              title="刷新"
            >
              <RefreshCw className={cn("size-3", loading && "animate-spin")} />
            </Button>
          }
        />
        <Stat
          icon={<Database className="size-4 text-emerald-600" />}
          label="本地命中节省"
          value={cur?.kb_hits ?? 0}
          suffix={`次 (命中率 ${kbHitRate}%)`}
        />
      </div>
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

function FeishuNotifyTestButton() {
  const [testing, setTesting] = useState(false);
  const handleTest = async () => {
    setTesting(true);
    try {
      const count = await testFeishuNotify();
      if (count > 0) {
        toast(`已推送 ${count} 条到期提醒到飞书`, "success");
      } else {
        toast("暂无需要推送的到期事项", "info");
      }
    } catch (e) {
      toast(`推送测试失败:${e}`, "error", 7000);
    } finally {
      setTesting(false);
    }
  };
  return (
    <Button
      variant="outline"
      size="sm"
      onClick={handleTest}
      disabled={testing}
    >
      {testing ? (
        <Loader2 className="size-3.5 animate-spin" />
      ) : (
        <RefreshCw className="size-3.5" />
      )}
      {testing ? "推送中…" : "测试推送"}
    </Button>
  );
}

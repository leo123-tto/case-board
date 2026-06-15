/**
 * Onboarding 向导(V0.3.6 · 2026-06-03 重做:多页分步,先介绍功能再配置 API)。
 *
 * 结构:5 个功能介绍页 + 1 个 API 配置页(step 0-5)。
 *   0 看案件 / 1 一键出报告 / 2 执行深挖 / 3 AI 助手 / 4 实用工具 / 5 配置 API
 *
 * 配置页(本地模型已隐藏,只走云端):
 *   - 称呼
 *   - DeepSeek(必填 · canFinish 的唯一硬门槛)、MinerU、元典 —— 三个优先填 + 当场验证
 *   - 硅基流动(语义检索增强)、快递100 —— 选填,只给申请链接,可稍后在「设置」补
 *   - 完成 → 硬写 cloud(ocr/llm_provider=cloud + cloud_enabled=true)+ setup_completed=true
 *
 * setup_completed 置 true 后不再弹(App.tsx 据此触发)。重测需在 settings.json 把它改回 false。
 */
import { useEffect, useState } from "react";
import {
  ArrowRight,
  ArrowLeft,
  ExternalLink,
  Loader2,
  CheckCircle2,
  XCircle,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  getSettings,
  saveSettings,
  openUrl,
  verifyMinerUKey,
  verifyDeepSeekKey,
  verifyYuandianKey,
  seedDemoCaseIfEmpty,
} from "@/lib/api";
import type { Settings } from "@/lib/types";

type VerifyStatus = "idle" | "verifying" | "ok" | "fail";

/* ============ 功能介绍页内容(0-4) ============ */
type FeaturePage = {
  badge: string;
  title: string;
  subtitle: string;
  items: { emoji: string; title: string; desc: string }[];
};

const FEATURE_PAGES: FeaturePage[] = [
  {
    badge: "功能 ① · 看案件",
    title: "案件文件夹一拖,关键信息自动呈现",
    subtitle:
      "把案件材料文件夹拖进首页(或点导入),自动分类、OCR、抽取,省去手工录入。",
    items: [
      {
        emoji: "📁",
        title: "一拖即导入",
        desc: "整个案件文件夹拖进首页就开始;扫描件、PDF、Word 都能识别。",
      },
      {
        emoji: "🗂",
        title: "在办案件一览",
        desc: "当事人 / 案号 / 法院 / 标的金额 / 诉讼阶段,一屏看清。",
      },
      {
        emoji: "⏰",
        title: "重要日期提醒",
        desc: "开庭倒计时、保全续封到期,首页醒目提示,不漏期限。",
      },
    ],
  },
  {
    badge: "功能 ② · 一键出报告",
    title: "案件 AI 助手,一键生成各类分析报告",
    subtitle:
      "进案件详情,点一下就出 —— 联网查准法条与类案,不靠记忆瞎编,结论可追溯。",
    items: [
      {
        emoji: "📖",
        title: "案件分析报告",
        desc: "整案画像:事实、争议焦点、证据、风险一份说清。",
      },
      {
        emoji: "⚖️",
        title: "法律依据",
        desc: "围绕诉求联网查准法条 + 类案,整理成依据清单(自动判审理 / 执行阶段)。",
      },
      {
        emoji: "🥊",
        title: "模拟对抗",
        desc: "预演对方可能的抗辩与反驳,提前补强。",
      },
      {
        emoji: "🔍",
        title: "类案检索",
        desc: "找相似判例,看法院裁判口径与对我方诉求的支持可能。",
      },
    ],
  },
  {
    badge: "功能 ③ · 执行案件深挖",
    title: "自动识别执行案件,一键深挖被执行人",
    subtitle:
      "案件进入执行阶段会自动识别,接元典法律开放平台,把被执行人查个底朝天。",
    items: [
      {
        emoji: "🕵️",
        title: "被执行人调查报告",
        desc: "聚合查失信、限高、关联案件,汇成一份调查报告。",
      },
      {
        emoji: "💰",
        title: "财产线索报告",
        desc: "深挖被执行人财产线索、关联公司、对外投资,给执行 / 拒执提供方向。",
      },
      {
        emoji: "🎯",
        title: "立案日 cutoff",
        desc: "按立案时间切分,给拒执罪线索做时间锚点。",
      },
    ],
  },
  {
    badge: "功能 ④ · AI 助手",
    title: "右侧 AI 助手,你的随身法律工作台",
    subtitle:
      "每个案件详情都自带一个 AI 助手,带 20+ 工具,能查、能想、能写、能改、能导出。",
    items: [
      {
        emoji: "📚",
        title: "查各种法律法规 + 类案",
        desc: "联网查准现行法条、找相似判例,引用都标真实出处,不编法条案号。",
      },
      {
        emoji: "🧠",
        title: "陪你分析案情",
        desc: "读本案全部材料,理事实时间线、争议焦点、举证责任、对方抗辩点。",
      },
      {
        emoji: "📝",
        title: "一键写起诉状 / 证据目录",
        desc: "按本案材料起草正式文书,信息不全会先弹选项问你,不瞎写。",
      },
      {
        emoji: "✏️",
        title: "局部编辑 + 导出 Word",
        desc: "「把第二段金额改成…」这类改动直接说,改完导出 Word(法律排版)。",
      },
    ],
  },
  {
    badge: "功能 ⑤ · 实用工具",
    title: "法院短信、快递查询与常用计算器",
    subtitle: "「工具」页里几个高频小工具,办案路上省事。",
    items: [
      {
        emoji: "📨",
        title: "法院短信处理",
        desc: "粘贴「人民法院在线服务 / 一张网」送达短信,自动下载文书并归档进案件。",
      },
      {
        emoji: "📦",
        title: "快递查询 / 跟踪",
        desc: "输单号实时查物流,自动跟踪到签收(接快递100)。",
      },
      {
        emoji: "🧮",
        title: "律师费 / 诉讼费 / 利息计算器",
        desc: "按办法算费用;利息含 LPR 历史 + 五阶段清偿 + 多案合并。",
      },
    ],
  },
];

const TOTAL_STEPS = FEATURE_PAGES.length + 1; // 5 介绍 + 1 配置
const CONFIG_STEP = FEATURE_PAGES.length; // 最后一页 = 配置

export interface OnboardingWizardProps {
  open: boolean;
  onComplete: () => void;
}

export function OnboardingWizard({ open, onComplete }: OnboardingWizardProps) {
  const [step, setStep] = useState(0);
  const [displayName, setDisplayName] = useState("");
  const [settings, setSettings] = useState<Settings | null>(null);
  const [saving, setSaving] = useState(false);

  const [minerKey, setMinerKey] = useState("");
  const [dsKey, setDsKey] = useState("");
  const [dsEndpoint, setDsEndpoint] = useState("https://api.deepseek.com");
  const [yuandianKey, setYuandianKey] = useState("");

  const [mineruStatus, setMineruStatus] = useState<VerifyStatus>("idle");
  const [mineruMsg, setMineruMsg] = useState("");
  const [deepseekStatus, setDeepseekStatus] = useState<VerifyStatus>("idle");
  const [deepseekMsg, setDeepseekMsg] = useState("");
  const [yuandianStatus, setYuandianStatus] = useState<VerifyStatus>("idle");
  const [yuandianMsg, setYuandianMsg] = useState("");

  useEffect(() => {
    if (!open) return;
    getSettings()
      .then((s) => {
        setSettings(s);
        if (s.user_display_name) setDisplayName(s.user_display_name);
        if (s.mineru_api_key) setMinerKey(s.mineru_api_key);
        if (s.cloud_llm_api_key) setDsKey(s.cloud_llm_api_key);
        if (s.cloud_llm_endpoint) setDsEndpoint(s.cloud_llm_endpoint);
        if (s.yuandian_api_key) setYuandianKey(s.yuandian_api_key);
      })
      .catch(console.error);
  }, [open]);

  if (!open) return null;

  // DeepSeek key 是核心,必填才能「开始使用」;MinerU / 元典 推荐填但可「稍后」。
  const canFinish = dsKey.trim().length > 0;

  async function handleVerifyMineru() {
    setMineruStatus("verifying");
    setMineruMsg("");
    try {
      const r = await verifyMinerUKey(minerKey);
      setMineruStatus(r.ok ? "ok" : "fail");
      setMineruMsg(r.message);
    } catch (e) {
      setMineruStatus("fail");
      setMineruMsg(String(e));
    }
  }

  async function handleVerifyDeepSeek() {
    setDeepseekStatus("verifying");
    setDeepseekMsg("");
    try {
      const r = await verifyDeepSeekKey(dsKey, dsEndpoint);
      setDeepseekStatus(r.ok ? "ok" : "fail");
      setDeepseekMsg(r.message);
    } catch (e) {
      setDeepseekStatus("fail");
      setDeepseekMsg(String(e));
    }
  }

  async function handleVerifyYuandian() {
    setYuandianStatus("verifying");
    setYuandianMsg("");
    try {
      const r = await verifyYuandianKey(yuandianKey);
      setYuandianStatus(r.ok ? "ok" : "fail");
      setYuandianMsg(r.message);
    } catch (e) {
      setYuandianStatus("fail");
      setYuandianMsg(String(e));
    }
  }

  async function finish() {
    if (!settings) return;
    setSaving(true);
    try {
      await saveSettings({
        ...settings,
        user_display_name: displayName.trim() || null,
        setup_completed: true,
        // V0.3:暂时只走云端(本地模型隐藏)。这些字段保留,以后接新本地模型再放开选择。
        ocr_provider: "cloud",
        llm_provider: "cloud",
        cloud_enabled: true,
        mineru_api_key: minerKey.trim() || null,
        cloud_llm_api_key: dsKey.trim() || null,
        deepseek_api_key: dsKey.trim() || null,
        cloud_llm_endpoint: dsEndpoint.trim() || null,
        yuandian_api_key: yuandianKey.trim() || null,
        mineru_verified_at:
          mineruStatus === "ok" ? new Date().toISOString() : null,
        deepseek_verified_at:
          deepseekStatus === "ok" ? new Date().toISOString() : null,
        yuandian_verified_at:
          yuandianStatus === "ok" ? new Date().toISOString() : null,
      });
      // 首次完成 onboarding,若 cases 表空,seed 示例案件(非致命)
      try {
        await seedDemoCaseIfEmpty();
      } catch (e) {
        console.warn("seed demo case failed (non-fatal):", e);
      }
      onComplete();
    } catch (e) {
      console.error(e);
      alert(`保存失败: ${e}`);
    } finally {
      setSaving(false);
    }
  }

  const isConfig = step === CONFIG_STEP;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm animate-in fade-in-0 duration-200">
      <div className="flex max-h-[92vh] w-[720px] max-w-[94vw] flex-col overflow-hidden rounded-2xl bg-background shadow-2xl ring-1 ring-border animate-in zoom-in-95 fade-in-0 duration-300 ease-out">
        {/* 进度点 */}
        <div className="flex shrink-0 justify-center gap-2 px-10 pt-7">
          {Array.from({ length: TOTAL_STEPS }, (_, i) => (
            <span
              key={i}
              className={cn(
                "h-1.5 rounded-full transition-all duration-300",
                i === step ? "w-7 bg-foreground" : "w-1.5 bg-border",
              )}
            />
          ))}
        </div>

        {/* 内容(滚动区) */}
        <div className="min-h-0 flex-1 overflow-y-auto px-10 py-7">
          {!isConfig ? (
            <IntroPage page={FEATURE_PAGES[step]} first={step === 0} />
          ) : (
            <ConfigPage
              displayName={displayName}
              setDisplayName={setDisplayName}
              dsKey={dsKey}
              setDsKey={setDsKey}
              deepseekStatus={deepseekStatus}
              deepseekMsg={deepseekMsg}
              onVerifyDeepSeek={handleVerifyDeepSeek}
              resetDeepSeek={() => {
                setDeepseekStatus("idle");
                setDeepseekMsg("");
              }}
              minerKey={minerKey}
              setMinerKey={setMinerKey}
              mineruStatus={mineruStatus}
              mineruMsg={mineruMsg}
              onVerifyMineru={handleVerifyMineru}
              resetMineru={() => {
                setMineruStatus("idle");
                setMineruMsg("");
              }}
              yuandianKey={yuandianKey}
              setYuandianKey={setYuandianKey}
              yuandianStatus={yuandianStatus}
              yuandianMsg={yuandianMsg}
              onVerifyYuandian={handleVerifyYuandian}
              resetYuandian={() => {
                setYuandianStatus("idle");
                setYuandianMsg("");
              }}
            />
          )}
        </div>

        {/* 底部导航 */}
        <div className="flex shrink-0 items-center justify-between gap-3 border-t border-border bg-card/60 px-10 py-4">
          <div>
            {step > 0 && (
              <Button
                variant="ghost"
                onClick={() => setStep((s) => s - 1)}
                disabled={saving}
              >
                <ArrowLeft className="mr-1 h-4 w-4" />
                上一步
              </Button>
            )}
          </div>
          <div className="flex items-center gap-3">
            {!isConfig && (
              <>
                <Button
                  variant="outline"
                  onClick={() => setStep(CONFIG_STEP)}
                  disabled={saving}
                >
                  跳过介绍
                </Button>
                <Button onClick={() => setStep((s) => s + 1)}>
                  下一步
                  <ArrowRight className="ml-1 h-4 w-4" />
                </Button>
              </>
            )}
            {isConfig && !canFinish && (
              <Button variant="outline" onClick={finish} disabled={saving}>
                稍后再配置
              </Button>
            )}
            {isConfig && (
              <Button onClick={finish} disabled={saving || !canFinish}>
                {saving ? (
                  <>
                    <Loader2 className="mr-1 h-4 w-4 animate-spin" /> 保存中
                  </>
                ) : (
                  <>
                    开始使用
                    <ArrowRight className="ml-1 h-4 w-4" />
                  </>
                )}
              </Button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

/* ============ 功能介绍页 ============ */
function IntroPage({ page, first }: { page: FeaturePage; first: boolean }) {
  return (
    <div className="animate-in fade-in-0 slide-in-from-right-2 duration-300">
      {first && (
        <h1 className="mb-1 text-2xl font-bold tracking-tight">
          欢迎使用 CaseBoard
        </h1>
      )}
      <span className="inline-block rounded-full bg-sky-50 px-2.5 py-0.5 text-xs font-medium text-sky-700">
        {page.badge}
      </span>
      <h2 className="mt-3 text-xl font-semibold tracking-tight text-foreground">
        {page.title}
      </h2>
      <p className="mt-1.5 text-sm text-muted-foreground">{page.subtitle}</p>

      <div className="mt-6 space-y-3">
        {page.items.map((it) => (
          <div
            key={it.title}
            className="flex items-start gap-3 rounded-lg border border-border bg-card p-4"
          >
            <span className="text-2xl leading-none">{it.emoji}</span>
            <div className="min-w-0">
              <p className="text-sm font-semibold text-foreground">{it.title}</p>
              <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
                {it.desc}
              </p>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

/* ============ 配置页 ============ */
function ConfigPage(props: {
  displayName: string;
  setDisplayName: (v: string) => void;
  dsKey: string;
  setDsKey: (v: string) => void;
  deepseekStatus: VerifyStatus;
  deepseekMsg: string;
  onVerifyDeepSeek: () => void;
  resetDeepSeek: () => void;
  minerKey: string;
  setMinerKey: (v: string) => void;
  mineruStatus: VerifyStatus;
  mineruMsg: string;
  onVerifyMineru: () => void;
  resetMineru: () => void;
  yuandianKey: string;
  setYuandianKey: (v: string) => void;
  yuandianStatus: VerifyStatus;
  yuandianMsg: string;
  onVerifyYuandian: () => void;
  resetYuandian: () => void;
}) {
  return (
    <div className="animate-in fade-in-0 slide-in-from-right-2 duration-300">
      <h2 className="text-xl font-semibold tracking-tight">最后一步:配置 API</h2>
      <p className="mt-1.5 text-sm text-muted-foreground">
        这些服务基本都免费,DeepSeek 充几块钱就能用很久。
        <span className="text-foreground">不配也能先进来,但对应功能用不了</span>
        —— 之后随时在「设置」里补。
      </p>

      {/* 称呼 */}
      <div className="mt-6">
        <label className="block">
          <span className="text-sm font-semibold">怎么称呼您?</span>
          <p className="mt-0.5 text-xs text-muted-foreground">
            首页问候用,例:刘律师 / 周律师 / 李三。留空就显示"律师"。
          </p>
          <input
            type="text"
            value={props.displayName}
            onChange={(e) => props.setDisplayName(e.target.value)}
            placeholder="例:刘律师"
            className="mt-2 block h-10 w-full rounded-md border border-input bg-background px-3 text-sm shadow-sm focus:outline-none focus:border-foreground focus:ring-1 focus:ring-foreground/20"
          />
        </label>
      </div>

      <p className="mt-7 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        优先配置(核心功能要用)
      </p>
      <div className="mt-2 space-y-4">
        {/* DeepSeek —— 核心,必填 */}
        <SetupSection
          title="DeepSeek API Key(必填 · AI 抽取 / 分析 / 写材料都靠它)"
          help={
            <>
              <ApplyLink
                url="https://platform.deepseek.com/api_keys"
                label="去 DeepSeek 拿 API Key"
              />
              · 充值 10-50 元够律师用几个月
            </>
          }
        >
          <LabeledInput
            label="API Key"
            value={props.dsKey}
            onChange={(v) => {
              props.setDsKey(v);
              if (props.deepseekStatus !== "idle") props.resetDeepSeek();
            }}
            password
            placeholder="sk-..."
          />
          <VerifyRow
            label="验证 API Key"
            status={props.deepseekStatus}
            msg={props.deepseekMsg}
            onClick={props.onVerifyDeepSeek}
            disabled={!props.dsKey.trim()}
          />
        </SetupSection>

        {/* MinerU —— 扫描件 OCR,推荐 */}
        <SetupSection
          title="MinerU API Token(推荐 · 扫描件 / 图片 / 复杂 PDF 转文字)"
          help={
            <>
              <ApplyLink
                url="https://mineru.net/apiManage/token"
                label="去 MinerU 注册拿 token"
              />
              · 免费 1000 份/天 · 不填扫描件无法识别
            </>
          }
        >
          <LabeledInput
            label="API Token"
            value={props.minerKey}
            onChange={(v) => {
              props.setMinerKey(v);
              if (props.mineruStatus !== "idle") props.resetMineru();
            }}
            password
            placeholder="eyJ0eXBl..."
          />
          <VerifyRow
            label="验证 Token"
            status={props.mineruStatus}
            msg={props.mineruMsg}
            onClick={props.onVerifyMineru}
            disabled={!props.minerKey.trim()}
          />
        </SetupSection>

        {/* 元典 —— 执行查询 / 类案 / 法规,推荐 */}
        <SetupSection
          title="元典 API Key(推荐 · 被执行人调查 / 类案 / 法规检索)"
          help={
            <>
              <ApplyLink
                url="https://open.chineselaw.com/profile"
                label="去元典「个人中心」申请"
              />
              · 不填执行深挖、法律依据、类案检索用不了
            </>
          }
        >
          <LabeledInput
            label="API Key"
            value={props.yuandianKey}
            onChange={(v) => {
              props.setYuandianKey(v);
              if (props.yuandianStatus !== "idle") props.resetYuandian();
            }}
            password
            placeholder="sk_..."
          />
          <VerifyRow
            label="验证 API Key"
            status={props.yuandianStatus}
            msg={props.yuandianMsg}
            onClick={props.onVerifyYuandian}
            disabled={!props.yuandianKey.trim()}
          />
        </SetupSection>
      </div>

      {/* 选填:可稍后在设置里配 */}
      <p className="mt-7 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        选填(可稍后在「设置」里配)
      </p>
      <div className="mt-2 rounded-lg border border-dashed border-border bg-card/50 p-4 text-xs text-muted-foreground">
        <div className="flex items-start gap-2">
          <span className="text-base leading-none">🔎</span>
          <p>
            <span className="font-medium text-foreground">硅基流动</span>{" "}
            —— 开启语义检索(按含义找材料,检索更强,免费 BAAI/bge-m3)。
            <ApplyLink
              url="https://cloud.siliconflow.cn/me/account/ak"
              label="去申请"
            />
          </p>
        </div>
        <div className="mt-2 flex items-start gap-2">
          <span className="text-base leading-none">📦</span>
          <p>
            <span className="font-medium text-foreground">快递100</span>{" "}
            —— 工具页快递查询 / 跟踪要用。
            <ApplyLink url="https://api.kuaidi100.com/" label="去申请" />
          </p>
        </div>
      </div>
    </div>
  );
}

/* ============ 复用小组件 ============ */
function ApplyLink({ url, label }: { url: string; label: string }) {
  return (
    <button
      type="button"
      onClick={() =>
        openUrl(url).catch((e) => console.warn("openUrl failed", e))
      }
      className="inline-flex items-center gap-1 text-primary hover:underline"
    >
      {label}
      <ExternalLink className="h-3 w-3" />
    </button>
  );
}

function VerifyRow({
  label,
  status,
  msg,
  onClick,
  disabled,
}: {
  label: string;
  status: VerifyStatus;
  msg: string;
  onClick: () => void;
  disabled: boolean;
}) {
  return (
    <div className="mt-2 flex min-h-[20px] items-center gap-2">
      <Button
        type="button"
        size="sm"
        variant="outline"
        onClick={onClick}
        disabled={status === "verifying" || disabled}
        className={cn(status === "verifying" && "bg-primary/5")}
      >
        {status === "verifying" ? (
          <Loader2 className="mr-1 h-4 w-4 animate-spin" />
        ) : null}
        {label}
      </Button>
      {status === "ok" && (
        <span className="inline-flex items-center gap-1 text-xs text-green-700">
          <CheckCircle2 className="h-4 w-4" /> 已验证
        </span>
      )}
      {status === "fail" && (
        <span className="inline-flex items-center gap-1 text-xs text-red-600">
          <XCircle className="h-4 w-4" /> {msg || "验证失败"}
        </span>
      )}
    </div>
  );
}

function SetupSection({
  title,
  help,
  children,
}: {
  title: string;
  help: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-lg border border-border bg-card p-4">
      <h3 className="text-sm font-semibold">{title}</h3>
      <div className="mt-1 text-xs text-muted-foreground">{help}</div>
      <div className="mt-3">{children}</div>
    </div>
  );
}

function LabeledInput({
  label,
  value,
  onChange,
  placeholder,
  password = false,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  password?: boolean;
}) {
  return (
    <label className="block">
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      <input
        type={password ? "password" : "text"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="mt-1 block w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-sm focus:outline-none focus:border-foreground focus:ring-1 focus:ring-foreground/20"
      />
    </label>
  );
}

/**
 * 飞书联动配置：日历读取整合自外部贡献 PR #9；手机提醒整合自 PR #22。
 *
 * 日历读取复用本机 lark-cli 登录态；每日提醒使用用户自行创建的飞书群机器人 Webhook。
 *
 * 依赖(诚实标明,装不上属正常):
 *   1. 本机装好飞书官方 `lark-cli` 并 `lark-cli login`(macOS / Windows / Linux 都有);
 *   2. (可选)飞书"案件池"多维表格,用于点日历事件反查并导入本地案件目录。
 */
import { useEffect, useState } from "react";
import {
  BellRing,
  CalendarClock,
  Loader2,
  Save,
  CheckCircle2,
  AlertTriangle,
  ChevronDown,
  ChevronRight,
} from "lucide-react";

import {
  fetchFeishuCalendar,
  getSettings,
  listCredentialMetadata,
  revokeCredential,
  saveCredential,
  saveSettings,
  verifyCredential,
} from "@/lib/api";
import type { CredentialMetadata, Settings } from "@/lib/types";
import { toast } from "@/components/ui/toast";
import { CredentialField } from "@/components/settings/CredentialField";

function todayISO(): string {
  const d = new Date();
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

export function FeishuCalendarTool() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [larkPath, setLarkPath] = useState("");
  const [appToken, setAppToken] = useState("");
  const [credentials, setCredentials] = useState<CredentialMetadata[]>([]);
  const [tableId, setTableId] = useState("");
  const [poolOpen, setPoolOpen] = useState(false);
  const [reminderEnabled, setReminderEnabled] = useState(false);
  const [reminderTime, setReminderTime] = useState("09:00");
  const [reminderDays, setReminderDays] = useState(7);
  const [reminderOpen, setReminderOpen] = useState(false);

  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testOk, setTestOk] = useState<boolean | null>(null);
  const [testMsg, setTestMsg] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([getSettings(), listCredentialMetadata().catch(() => [])])
      .then(([s, metadata]) => {
        setSettings(s);
        setCredentials(metadata);
        setEnabled(s.feishu_enabled === true);
        setLarkPath(s.feishu_lark_cli_path ?? "");
        setAppToken(s.feishu_app_token ?? "");
        setTableId(s.feishu_cases_table_id ?? "");
        setReminderEnabled(s.feishu_reminder_enabled === true);
        setReminderTime(s.feishu_reminder_time ?? "09:00");
        setReminderDays(s.feishu_reminder_days ?? 7);
        if (
          (s.feishu_app_token ?? "").trim()
          || (s.feishu_cases_table_id ?? "").trim()
        ) {
          setPoolOpen(true);
        }
        if (
          s.feishu_reminder_enabled === true
          || metadata.some(
            (item) =>
              item.provider_or_connector_id === "feishu.reminder"
              && item.kind === "webhook_secret"
              && item.secret_present,
          )
        ) {
          setReminderOpen(true);
        }
      })
      .catch(() => {});
  }, []);

  const markDirty = () => setDirty(true);
  const webhookCredential = () =>
    credentials.find(
      (item) =>
        item.provider_or_connector_id === "feishu.reminder"
        && item.kind === "webhook_secret",
    ) ?? null;
  const replaceCredential = (next: CredentialMetadata) =>
    setCredentials((previous) => [
      ...previous.filter((item) => item.handle !== next.handle),
      next,
    ]);
  const saveBridgeCredential = async (secretInput: string) => {
    const current = webhookCredential();
    const next = await saveCredential({
      handle: current?.handle ?? null,
      providerOrConnectorId: "feishu.reminder",
      kind: "webhook_secret",
      secretInput,
    });
    replaceCredential(next);
    return next;
  };
  const verifyBridgeCredential = async () => {
    const current = webhookCredential();
    if (!current) throw new Error("凭据尚未配置");
    const next = await verifyCredential(current.handle, current.revision);
    replaceCredential(next);
    return next;
  };
  const revokeBridgeCredential = async (handle: string, revision: number) => {
    const next = await revokeCredential(handle, revision);
    replaceCredential(next);
    return next;
  };

  const handleSave = async () => {
    if (!settings) return;
    if (reminderEnabled && !webhookCredential()?.secret_present) {
      toast("启用飞书手机提醒前，请先填写机器人 Webhook URL", "error");
      setReminderOpen(true);
      return;
    }
    setSaving(true);
    try {
      const next: Settings = {
        ...settings,
        feishu_enabled: enabled,
        feishu_lark_cli_path: larkPath.trim() || null,
        feishu_app_token: appToken.trim() || null,
        feishu_cases_table_id: tableId.trim() || null,
        feishu_reminder_enabled: reminderEnabled,
        feishu_reminder_time: reminderTime,
        feishu_reminder_days: reminderDays,
      };
      await saveSettings(next);
      setSettings(next);
      setDirty(false);
      toast("飞书联动配置已保存", "info");
    } catch (e) {
      toast(`保存失败:${e}`, "error");
    } finally {
      setSaving(false);
    }
  };

  // 测试连接:先存当前配置,再拉今天的飞书日历(透传真实错误,守坑#8)。
  const handleTest = async () => {
    if (reminderEnabled && !webhookCredential()?.secret_present) {
      toast("启用飞书手机提醒前，请先填写机器人 Webhook URL", "error");
      setReminderOpen(true);
      return;
    }
    setTesting(true);
    setTestOk(null);
    setTestMsg(null);
    try {
      if (settings) {
        const next: Settings = {
          ...settings,
          feishu_enabled: true,
          feishu_lark_cli_path: larkPath.trim() || null,
          feishu_app_token: appToken.trim() || null,
          feishu_cases_table_id: tableId.trim() || null,
          feishu_reminder_enabled: reminderEnabled,
          feishu_reminder_time: reminderTime,
          feishu_reminder_days: reminderDays,
        };
        await saveSettings(next);
        setSettings(next);
        setEnabled(true);
        setDirty(false);
      }
      const today = todayISO();
      const events = await fetchFeishuCalendar(today, today);
      setTestOk(true);
      setTestMsg(`连接成功 · 今天有 ${events.length} 个日程`);
    } catch (e) {
      setTestOk(false);
      setTestMsg(String(e));
    } finally {
      setTesting(false);
    }
  };

  return (
    <div className="space-y-5">
      {/* 标题 */}
      <div className="flex items-center gap-2">
        <CalendarClock className="size-5 text-foreground" />
        <h3 className="text-base font-semibold text-foreground">飞书联动</h3>
      </div>

      {/* 两类联动的依赖说明 */}
      <div className="rounded-lg bg-sky-50 px-4 py-3 text-sm text-slate-700">
        <p className="font-medium text-slate-800">两项能力可独立使用:</p>
        <ol className="mt-1.5 list-decimal space-y-1 pl-5 text-[13px] leading-relaxed">
          <li>
            <b>日历读取</b>:本机安装飞书官方 <code className="rounded bg-white px-1">lark-cli</code> 并登录
            (<code className="rounded bg-white px-1">lark-cli login</code>)；CaseBoard 不保存飞书 token。
          </li>
          <li>
            <b>手机提醒</b>:在飞书群添加自定义机器人，把 Webhook URL 填到下方；不需要安装 lark-cli。
          </li>
          <li>
            macOS 会自动找 Homebrew 下的 lark-cli；<b>Windows / Linux</b> 可使用 PATH 或填写完整路径。
          </li>
        </ol>
      </div>

      {/* 总开关 */}
      <label className="flex cursor-pointer items-center gap-3 rounded-lg border border-border bg-card px-4 py-3">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => {
            setEnabled(e.target.checked);
            markDirty();
          }}
          className="size-4"
        />
        <div>
          <p className="text-sm font-medium text-foreground">启用飞书日历</p>
          <p className="text-xs text-muted-foreground">开启后首页显示飞书月历(蓝点=飞书日程 / 黄点=案件节点)</p>
        </div>
      </label>

      {/* lark-cli 路径(可选) */}
      <div className="space-y-1.5">
        <label className="text-sm font-medium text-foreground">
          lark-cli 路径 <span className="text-muted-foreground">(可选)</span>
        </label>
        <input
          type="text"
          value={larkPath}
          onChange={(e) => {
            setLarkPath(e.target.value);
            markDirty();
          }}
          placeholder="留空 = 自动查找。Windows 示例:C:\\Tools\\lark-cli.exe"
          className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-foreground/40"
        />
      </div>

      {/* 飞书手机每日提醒（与日历读取独立，可单独使用） */}
      <div className="rounded-lg border border-border">
        <button
          type="button"
          onClick={() => setReminderOpen((value) => !value)}
          className="flex w-full items-center gap-2 px-4 py-2.5 text-left text-sm font-medium text-foreground"
        >
          {reminderOpen ? <ChevronDown className="size-4" /> : <ChevronRight className="size-4" />}
          <BellRing className="size-4 text-blue-600" />
          手机每日提醒
          <span className="text-xs font-normal text-muted-foreground">(可选 · 飞书群机器人 Webhook)</span>
        </button>
        {reminderOpen && (
          <div className="space-y-3 border-t border-border px-4 py-3">
            <p className="text-xs leading-relaxed text-muted-foreground">
              App 运行时会在设定时间，把未来到期的案件关键日期、未完成待办、时间线事件和个人日程
              发到飞书群，手机端会收到飞书通知。消息包含案件名称和事项内容；不用时保持关闭即可。
            </p>
            <label className="flex cursor-pointer items-center gap-3 rounded-md bg-muted/30 px-3 py-2.5">
              <input
                type="checkbox"
                checked={reminderEnabled}
                onChange={(event) => {
                  setReminderEnabled(event.target.checked);
                  markDirty();
                }}
                className="size-4"
              />
              <div>
                <p className="text-sm font-medium text-foreground">启用每日提醒</p>
                <p className="text-xs text-muted-foreground">默认关闭；每天成功推送一次，App 重启不会重复发送</p>
              </div>
            </label>
            <CredentialField
              label="飞书机器人 Webhook URL"
              status={webhookCredential()}
              onSave={saveBridgeCredential}
              onVerify={verifyBridgeCredential}
              onRevoke={revokeBridgeCredential}
              placeholder="输入新的飞书机器人 Webhook URL"
            />
            <div className="grid grid-cols-2 gap-3">
              <div className="space-y-1.5">
                <label className="text-sm font-medium text-foreground">每日推送时间</label>
                <input
                  type="time"
                  value={reminderTime}
                  onChange={(event) => {
                    setReminderTime(event.target.value || "09:00");
                    markDirty();
                  }}
                  className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-foreground/40"
                />
              </div>
              <div className="space-y-1.5">
                <label className="text-sm font-medium text-foreground">提前提醒天数</label>
                <select
                  value={reminderDays}
                  onChange={(event) => {
                    setReminderDays(Number(event.target.value));
                    markDirty();
                  }}
                  className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-foreground/40"
                >
                  {[1, 3, 5, 7, 14, 30].map((days) => (
                    <option key={days} value={days}>{days} 天</option>
                  ))}
                </select>
              </div>
            </div>
          </div>
        )}
      </div>

      {/* 案件池配置(可选折叠) */}
      <div className="rounded-lg border border-border">
        <button
          type="button"
          onClick={() => setPoolOpen((v) => !v)}
          className="flex w-full items-center gap-2 px-4 py-2.5 text-left text-sm font-medium text-foreground"
        >
          {poolOpen ? <ChevronDown className="size-4" /> : <ChevronRight className="size-4" />}
          案件池多维表格 <span className="text-xs font-normal text-muted-foreground">(可选 · 点日历事件一键导入对应案件)</span>
        </button>
        {poolOpen && (
          <div className="space-y-3 border-t border-border px-4 py-3">
            <p className="text-xs text-muted-foreground">
              在飞书多维表格里建一张"案件池"表,含「案件名称」「本地路径」两列。配好后,
              点首页飞书日历的事件可按标题反查本地案件目录并一键导入。不配则只展示日历、不影响。
            </p>
            <div className="space-y-1.5">
              <label className="text-sm text-foreground">App Token</label>
              <input
                type="text"
                value={appToken}
                onChange={(e) => {
                  setAppToken(e.target.value);
                  markDirty();
                }}
                placeholder="bascn... / 多维表格 URL 里的 app_token"
                className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-foreground/40"
              />
            </div>
            <div className="space-y-1.5">
              <label className="text-sm text-foreground">Table ID</label>
              <input
                type="text"
                value={tableId}
                onChange={(e) => {
                  setTableId(e.target.value);
                  markDirty();
                }}
                placeholder="tbl... / 多维表格 URL 里的 table_id"
                className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-foreground/40"
              />
            </div>
          </div>
        )}
      </div>

      {/* 测试结果 */}
      {testMsg && (
        <div
          className={
            testOk
              ? "flex items-start gap-2 rounded-md bg-emerald-50 px-3 py-2 text-sm text-emerald-700"
              : "flex items-start gap-2 rounded-md bg-red-50 px-3 py-2 text-sm text-red-700"
          }
        >
          {testOk ? (
            <CheckCircle2 className="mt-0.5 size-4 shrink-0" />
          ) : (
            <AlertTriangle className="mt-0.5 size-4 shrink-0" />
          )}
          <span className="break-all">{testMsg}</span>
        </div>
      )}

      {/* 操作按钮 */}
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={handleTest}
          disabled={testing || saving}
          className="inline-flex items-center gap-1.5 rounded-md border border-border bg-background px-3 py-2 text-sm font-medium text-foreground transition-colors hover:bg-accent disabled:opacity-50"
        >
          {testing ? <Loader2 className="size-4 animate-spin" /> : <CalendarClock className="size-4" />}
          测试日历连接
        </button>
        <button
          type="button"
          onClick={handleSave}
          disabled={saving || testing || !dirty}
          className="inline-flex items-center gap-1.5 rounded-md bg-foreground px-3 py-2 text-sm font-medium text-background transition-colors hover:bg-foreground/90 disabled:opacity-50"
        >
          {saving ? <Loader2 className="size-4 animate-spin" /> : <Save className="size-4" />}
          保存配置
        </button>
        {dirty && <span className="text-xs text-muted-foreground">有未保存改动</span>}
      </div>
    </div>
  );
}

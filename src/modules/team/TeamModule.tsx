/**
 * 团队版 Phase 1 · 「团队」tab(LAN 接力同步,docs/提案-团队版-2026-06-10.md §6.3)。
 *
 * 2026-06-10 老板拍板:**所有团队操作都在本页**,设置页不放团队内容——
 * - 未入团:本页直接创建 / 搜索加入(不再跳设置);
 * - 已入团:团队看板 + 「管理」区(团队长 = 配对码/逐人权限/移出/解散;成员 = 成员名单/退出)。
 *
 * 权限:后端 team_view 已按"我的可见范围"过滤;can_edit 驱动真编辑面板(Phase 2):
 * 留备注 → 生成编辑请求接力转交 → 案件所有人 App 应用后生效,
 * 所有人可在「管理」区撤销。数据=本机缓存,不依赖任何人在线。
 * 团队身份的保存防线在后端 save_settings(team 以磁盘为准)。
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ArrowLeft,
  CalendarClock,
  Copy,
  Crown,
  RefreshCw,
  Settings2,
  Users,
} from "lucide-react";

import {
  getSettings,
  teamCreate,
  teamDiscover,
  teamJoin,
  teamKick,
  teamLeave,
  teamRefreshCode,
  teamRevertEdit,
  teamSetPermissions,
  teamStatus,
  teamSubmitEdit,
  teamSyncNow,
  teamView,
} from "@/lib/api";
import { daysFromToday, todayIsoLocal } from "@/lib/date";
import type {
  DiscoveredTeam,
  RosterMember,
  TeamEdit,
  TeamMemberView,
  TeamSnapshotCase,
  TeamStatus,
  TeamView,
} from "@/lib/types";
import {
  STATUS_DEFS,
  type StatusId,
} from "@/modules/litigation/lib/inferStatus";
import { confirmDialog } from "@/lib/dialog";
import { toast } from "@/components/ui/toast";
import { cn } from "@/lib/utils";

const inputCls =
  "w-full rounded-md border border-border bg-background px-2.5 py-1.5 text-xs text-foreground placeholder:text-muted-foreground focus:border-sky-400 focus:outline-none";

/** 快照里的 stage 可能是工作流英文 id(execution)或自由中文,统一转中文标签。 */
function stageLabel(stage: string | null): string | null {
  if (!stage) return null;
  return STATUS_DEFS[stage as StatusId]?.label ?? stage;
}

/** 团队看板排序方式。加新排序 = 这里加一项 + TeamModule 渲染分支里加一个 case。 */
type TeamSortMode = "member" | "latest";
const SORT_MODES: { id: TeamSortMode; label: string }[] = [
  { id: "member", label: "按成员" },
  { id: "latest", label: "按最新进展" },
];

export function TeamModule() {
  const [loading, setLoading] = useState(true);
  const [status, setStatus] = useState<TeamStatus | null>(null);
  const [view, setView] = useState<TeamView | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [showManage, setShowManage] = useState(false);
  /// 打开的团队案件详情:{成员id, 案件id};null = 看板列表
  const [detail, setDetail] = useState<{ memberId: string; caseId: string } | null>(null);
  /// 排序方式(可扩展:以后有需求直接往 SORT_MODES 加一项)
  const [sortMode, setSortMode] = useState<TeamSortMode>("member");

  const reload = useCallback(async () => {
    try {
      const st = await teamStatus();
      if (st.kicked_from) {
        toast(`你已被移出团队「${st.kicked_from}」`, "error");
      }
      setStatus(st);
      setView(st.in_team ? await teamView() : null);
    } catch (e) {
      toast(`读取团队数据失败:${e}`, "error");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  async function syncNow() {
    if (syncing) return;
    setSyncing(true);
    try {
      const r = await teamSyncNow();
      if (r.peers_found === 0) {
        toast("局域网内没发现在线的队友(对方需开着案件看板且同一网络)", "info");
      } else {
        toast(
          `同步完成:在场 ${r.peers_found} 人,更新 ${r.snapshots_merged} 份快照` +
            (r.errors.length ? `,${r.errors.length} 个失败` : ""),
          r.errors.length ? "error" : "success",
        );
      }
      await reload();
    } catch (e) {
      toast(`同步失败:${e}`, "error");
    } finally {
      setSyncing(false);
    }
  }

  const detailOwner =
    detail && view
      ? view.members.find((m) => m.member_id === detail.memberId)
      : null;
  const detailCase =
    detailOwner && detail
      ? detailOwner.cases.find((x) => x.id === detail.caseId)
      : null;

  useEffect(() => {
    if (detail && view && !detailCase) {
      setDetail(null);
    }
  }, [detail, detailCase, view]);

  if (loading) {
    return <div className="p-10 text-sm text-muted-foreground">加载团队数据…</div>;
  }

  if (!status?.in_team || !view) {
    return <TeamOnboard onDone={() => void reload()} />;
  }

  const isLeader = view.my_role === "leader";

  // 详情页:从最新 view 里找(同步后数据自动更新);案件没了(对方删了)自动回列表
  if (detailOwner && detailCase) {
    return (
      <TeamCaseDetail
        c={detailCase}
        owner={detailOwner}
        edits={view.edits}
        myId={view.my_member_id}
        onBack={() => setDetail(null)}
        onChanged={() => void reload()}
      />
    );
  }

  return (
    <div className="app-shell h-full overflow-auto">
      <div className="app-page-enter mx-auto max-w-6xl px-4 py-6 sm:px-6 xl:px-8">
        {/* 顶栏 */}
        <div className="mb-4 flex flex-wrap items-center gap-3">
          <h2 className="flex items-center gap-2 text-lg font-semibold text-foreground">
            <Users className="size-5 text-sky-600" />
            {view.team_name}
          </h2>
          <span className="text-xs text-muted-foreground">{view.members.length} 名成员</span>
          <span
            className={cn(
              "rounded-full px-2 py-0.5 text-caption",
              isLeader ? "bg-amber-50 text-amber-700" : "bg-sky-50 text-sky-700",
            )}
          >
            {isLeader ? "团队长" : "成员"}
          </span>
          <div className="flex-1" />
          <div className="flex overflow-hidden rounded-md border border-border">
            {SORT_MODES.map((sm) => (
              <button
                key={sm.id}
                type="button"
                onClick={() => setSortMode(sm.id)}
                className={cn(
                  "px-2.5 py-1.5 text-xs transition-colors",
                  sortMode === sm.id
                    ? "bg-sky-50 font-medium text-sky-700"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                {sm.label}
              </button>
            ))}
          </div>
          <button
            type="button"
            onClick={() => setShowManage((v) => !v)}
            className={cn(
              "inline-flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-xs font-medium transition-colors",
              showManage
                ? "border-sky-400 bg-sky-50 text-sky-700"
                : "border-border text-muted-foreground hover:text-foreground",
            )}
          >
            <Settings2 className="size-3.5" />
            管理
          </button>
          <button
            type="button"
            onClick={() => void syncNow()}
            disabled={syncing}
            className="inline-flex items-center gap-1.5 rounded-md border border-sky-200 bg-sky-50 px-3 py-1.5 text-xs font-medium text-sky-700 transition-colors hover:bg-sky-100 disabled:opacity-50"
          >
            <RefreshCw className={cn("size-3.5", syncing && "animate-spin")} />
            {syncing ? "同步中…" : "立即同步"}
          </button>
        </div>

        {showManage && status.identity && status.roster && (
          <ManagePanel
            status={status}
            edits={view.edits}
            onChanged={() => void reload()}
          />
        )}

        <TeamKeyDates view={view} />

        {sortMode === "member" ? (
          <div className="space-y-6">
            {view.members.map((m) => (
              <MemberGroup
                key={m.member_id}
                m={m}
                edits={view.edits}
                onChanged={() => void reload()}
                onOpenCase={(caseId) => setDetail({ memberId: m.member_id, caseId })}
              />
            ))}
          </div>
        ) : (
          <LatestFirstGrid
            view={view}
            edits={view.edits}
            onChanged={() => void reload()}
            onOpenCase={(memberId, caseId) => setDetail({ memberId, caseId })}
          />
        )}

        <p className="mt-8 text-caption text-muted-foreground">
          数据来自队友 App 的接力同步(局域网内自动互通),只含案件登记信息,不含任何文档原文。
        </p>
      </div>
    </div>
  );
}

/* ==================================================================== */
/* 未入团:介绍 + 创建 / 加入(所有入口都在本页)                        */
/* ==================================================================== */

function TeamOnboard({ onDone }: { onDone: () => void }) {
  const [busy, setBusy] = useState(false);
  const [teamName, setTeamName] = useState("");
  const [myName, setMyName] = useState("");
  const [scanning, setScanning] = useState(false);
  const [found, setFound] = useState<DiscoveredTeam[] | null>(null);
  const [joinTarget, setJoinTarget] = useState<DiscoveredTeam | null>(null);
  const [code, setCode] = useState("");

  // 默认姓名取设置里的称呼(没有就留空让用户填)
  useEffect(() => {
    getSettings()
      .then((s) => setMyName((prev) => prev || (s.user_display_name ?? "")))
      .catch(() => {});
  }, []);

  async function run(label: string, f: () => Promise<void>) {
    if (busy) return;
    setBusy(true);
    try {
      await f();
    } catch (e) {
      toast(`${label}失败:${e}`, "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="app-shell h-full overflow-auto">
      <div className="app-page-enter mx-auto max-w-3xl px-4 py-8 sm:px-6 xl:py-10">
        <div className="mb-8 text-center">
          <Users className="mx-auto mb-3 size-10 text-sky-500" />
          <h2 className="mb-2 text-lg font-semibold text-foreground">团队看板</h2>
          <p className="text-sm text-muted-foreground">
            同一办公网内同步案件登记信息，不传文档原文。
          </p>
        </div>

        <div className="grid gap-4 sm:grid-cols-2">
          {/* 创建 */}
          <div className="rounded-lg border border-border bg-card p-4">
            <p className="mb-2 text-xs font-semibold text-foreground">创建团队(我是团队长)</p>
            <div className="space-y-2">
              <input
                type="text"
                value={teamName}
                onChange={(e) => setTeamName(e.target.value)}
                placeholder="团队名,如:XX律师团队"
                className={inputCls}
              />
              <input
                type="text"
                value={myName}
                onChange={(e) => setMyName(e.target.value)}
                placeholder="你的姓名(团队里显示)"
                className={inputCls}
              />
              <button
                type="button"
                disabled={busy || !teamName.trim() || !myName.trim()}
                onClick={() =>
                  void run("创建团队", async () => {
                    await teamCreate(teamName, myName);
                    toast("团队已创建!点「管理」看配对码,告诉队友即可加入", "success");
                    onDone();
                  })
                }
                className="rounded-md bg-sky-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-700 disabled:opacity-50"
              >
                创建团队
              </button>
            </div>
          </div>

          {/* 加入 */}
          <div className="rounded-lg border border-border bg-card p-4">
            <p className="mb-2 text-xs font-semibold text-foreground">加入团队</p>
            <div className="space-y-2">
              <input
                type="text"
                value={myName}
                onChange={(e) => setMyName(e.target.value)}
                placeholder="你的姓名(团队里显示)"
                className={inputCls}
              />
              <button
                type="button"
                disabled={scanning}
                onClick={async () => {
                  setScanning(true);
                  setFound(null);
                  setJoinTarget(null);
                  try {
                    const teams = await teamDiscover();
                    setFound(teams);
                    if (teams.length === 0) {
                      toast(
                        "局域网内没发现团队 —— 需要团队长开着案件看板,且和你在同一网络",
                        "info",
                      );
                    }
                  } catch (e) {
                    toast(`搜索失败:${e}`, "error");
                  } finally {
                    setScanning(false);
                  }
                }}
                className="rounded-md border border-sky-200 bg-sky-50 px-3 py-1.5 text-xs font-medium text-sky-700 hover:bg-sky-100 disabled:opacity-50"
              >
                {scanning ? "搜索中(约 3 秒)…" : "搜索局域网内的团队"}
              </button>

              {found && found.length > 0 && (
                <div className="space-y-1.5">
                  {found.map((t) => (
                    <label
                      key={t.team_id}
                      className={cn(
                        "flex cursor-pointer items-center gap-2 rounded-md border p-2 text-xs",
                        joinTarget?.team_id === t.team_id
                          ? "border-sky-400 bg-sky-50"
                          : "border-border",
                      )}
                    >
                      <input
                        type="radio"
                        name="join-team"
                        checked={joinTarget?.team_id === t.team_id}
                        onChange={() => setJoinTarget(t)}
                        className="accent-sky-600"
                      />
                      <span className="font-medium text-foreground">{t.team_name}</span>
                      <span className="text-muted-foreground">
                        {t.online_members} 人在线
                        {t.leader_online ? "" : " · 团队长不在线,暂不能加入"}
                      </span>
                    </label>
                  ))}
                  {joinTarget && (
                    <div className="flex items-center gap-2">
                      <input
                        type="text"
                        value={code}
                        onChange={(e) => setCode(e.target.value)}
                        placeholder="6 位配对码(问团队长要)"
                        maxLength={6}
                        className={cn(inputCls, "w-44")}
                      />
                      <button
                        type="button"
                        disabled={
                          busy ||
                          code.trim().length !== 6 ||
                          !myName.trim() ||
                          !joinTarget.leader_online
                        }
                        onClick={() =>
                          void run("加入团队", async () => {
                            await teamJoin(joinTarget.team_id, code, myName);
                            toast(`已加入「${joinTarget.team_name}」`, "success");
                            onDone();
                          })
                        }
                        className="rounded-md bg-sky-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-700 disabled:opacity-50"
                      >
                        加入
                      </button>
                    </div>
                  )}
                </div>
              )}
            </div>
          </div>
        </div>

        <p className="mt-6 text-center text-caption text-muted-foreground">
          首次使用 macOS 会询问「查找本地网络设备」权限,请点允许,否则发现不了队友。
        </p>
      </div>
    </div>
  );
}

/* ==================================================================== */
/* 管理区(团队长:配对码 + 逐人权限 + 移出 + 解散;成员:名单 + 退出)   */
/* ==================================================================== */

function ManagePanel({
  status,
  edits,
  onChanged,
}: {
  status: TeamStatus;
  edits: TeamEdit[];
  onChanged: () => void;
}) {
  const identity = status.identity!;
  const roster = status.roster!;
  const isLeader = identity.role === "leader";
  const [busy, setBusy] = useState(false);
  const [permFor, setPermFor] = useState<string | null>(null);
  const [pairingCode, setPairingCode] = useState<string | null>(null);

  async function run(label: string, f: () => Promise<void>) {
    if (busy) return;
    setBusy(true);
    try {
      await f();
    } catch (e) {
      toast(`${label}失败:${e}`, "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mb-5 space-y-3 rounded-lg border border-border bg-card/60 p-4">
      {isLeader && (
        <div className="flex flex-wrap items-center gap-2 rounded-md border border-sky-200 bg-sky-50/60 p-3">
          <span className="text-xs text-sky-900">新配对码(仅本次显示):</span>
          <span className="font-mono text-xl font-bold tracking-[0.3em] text-sky-700">
            {pairingCode ?? "点击右侧刷新生成"}
          </span>
          <button
            type="button"
            title="复制配对码"
            onClick={() => {
              void navigator.clipboard.writeText(pairingCode ?? "");
              toast("配对码已复制", "success");
            }}
            className="rounded p-1 text-sky-700 hover:bg-sky-100"
          >
            <Copy className="size-3.5" />
          </button>
          <button
            type="button"
            disabled={busy}
            title="刷新后旧码立即作废"
            onClick={() =>
              void run("刷新配对码", async () => {
                const c = await teamRefreshCode();
                setPairingCode(c);
                toast("配对码已更新,旧码作废", "success");
              })
            }
            className="rounded p-1 text-sky-700 hover:bg-sky-100 disabled:opacity-50"
          >
            <RefreshCw className="size-3.5" />
          </button>
          <span className="w-full text-caption text-sky-800/80">
            配对码<strong>一次性</strong>:有人加入后自动更换,把新码再给下一位。加入时需要你开着
            App(加入后日常同步不需要)。
          </span>
        </div>
      )}

      <div className="space-y-1.5">
        <p className="text-xs font-semibold text-foreground">成员({roster.members.length})</p>
        {roster.members.map((m) => (
          <div key={m.member_id} className="rounded-md border border-border bg-background p-2.5">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-xs font-medium text-foreground">{m.name}</span>
              {m.role === "leader" && <Crown className="size-3.5 text-amber-500" />}
              {m.member_id === identity.member_id && (
                <span className="rounded bg-secondary px-1.5 py-0.5 text-caption">我</span>
              )}
              <span className="text-caption text-muted-foreground">{permSummary(m)}</span>
              <div className="flex-1" />
              {isLeader && m.role !== "leader" && (
                <>
                  <button
                    type="button"
                    onClick={() => setPermFor(permFor === m.member_id ? null : m.member_id)}
                    className="rounded-md border border-border px-2 py-0.5 text-caption text-muted-foreground hover:text-foreground"
                  >
                    权限
                  </button>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() =>
                      void run("移出成员", async () => {
                        const ok = await confirmDialog(
                          `把「${m.name}」移出团队?其 App 会在下次同步时自动退出。`,
                          { danger: true, okLabel: "移出" },
                        );
                        if (!ok) return;
                        await teamKick(m.member_id);
                        toast(`已移出「${m.name}」`, "success");
                        onChanged();
                      })
                    }
                    className="rounded-md border border-destructive/40 px-2 py-0.5 text-caption text-destructive hover:bg-destructive/10 disabled:opacity-50"
                  >
                    移出
                  </button>
                </>
              )}
            </div>
            {isLeader && permFor === m.member_id && (
              <PermEditor
                member={m}
                others={roster.members.filter((x) => x.member_id !== m.member_id)}
                busy={busy}
                onSave={(view, edit) =>
                  void run("保存权限", async () => {
                    await teamSetPermissions(m.member_id, view, edit);
                    setPermFor(null);
                    toast(`「${m.name}」的权限已更新,随下次同步生效`, "success");
                    onChanged();
                  })
                }
              />
            )}
          </div>
        ))}
      </div>

      <MyCaseEdits
        edits={edits}
        myId={identity.member_id}
        busy={busy}
        onRevert={(e) =>
          void run("撤销改动", async () => {
            const ok = await confirmDialog(
              e.field === "workflow_status"
                ? `撤销「${e.editor_name}」对〈${e.case_name}〉的状态修改(恢复原状态)?`
                : `撤销(隐藏)「${e.editor_name}」给〈${e.case_name}〉的备注?`,
              { danger: true, okLabel: "撤销" },
            );
            if (!ok) return;
            await teamRevertEdit(e.id);
            toast("已撤销", "success");
            onChanged();
          })
        }
      />

      <div className="flex justify-end">
        <button
          type="button"
          disabled={busy}
          onClick={() =>
            void run("退出", async () => {
              const ok = await confirmDialog(
                isLeader
                  ? "解散并退出团队?成员端会保留最后一次同步的数据,但不再有团队长可配对/管理。"
                  : "退出团队?本机的团队数据会被清空(自己的案件不受影响)。",
                { danger: true, okLabel: isLeader ? "解散并退出" : "退出团队" },
              );
              if (!ok) return;
              await teamLeave();
              toast("已退出团队", "success");
              onChanged();
            })
          }
          className="rounded-md border border-destructive/40 px-2.5 py-1 text-xs text-destructive hover:bg-destructive/10 disabled:opacity-50"
        >
          {isLeader ? "解散团队" : "退出团队"}
        </button>
      </div>
    </div>
  );
}

/** 队友对**我的**案件的改动记录(已生效可撤销;被拒的也列出供知情)。 */
function MyCaseEdits({
  edits,
  myId,
  busy,
  onRevert,
}: {
  edits: TeamEdit[];
  myId: string;
  busy: boolean;
  onRevert: (e: TeamEdit) => void;
}) {
  const mine = edits.filter(
    (e) => e.target_member_id === myId && (e.status === "applied" || e.status === "rejected"),
  );
  if (mine.length === 0) return null;
  return (
    <div className="space-y-1.5">
      <p className="text-xs font-semibold text-foreground">队友对我案件的改动</p>
      {mine.slice(0, 20).map((e) => (
        <div
          key={e.id}
          className="flex flex-wrap items-center gap-2 rounded-md border border-border bg-background p-2 text-xs"
        >
          <span className="text-foreground">
            {e.editor_name} {e.field === "workflow_status" ? "把状态改为" : "留言"}
            「{e.field === "workflow_status" ? (stageLabel(e.value) ?? e.value) : e.value}」
          </span>
          <span className="text-caption text-muted-foreground">
            〈{e.case_name}〉· {e.created_at.slice(0, 10)}
          </span>
          {e.status === "rejected" && (
            <span className="rounded bg-secondary px-1.5 py-0.5 text-caption text-muted-foreground">
              已拒绝(无权限/案件不存在)
            </span>
          )}
          <div className="flex-1" />
          {e.status === "applied" && (
            <button
              type="button"
              disabled={busy}
              onClick={() => onRevert(e)}
              className="rounded-md border border-border px-2 py-0.5 text-caption text-muted-foreground hover:text-foreground disabled:opacity-50"
            >
              撤销
            </button>
          )}
        </div>
      ))}
    </div>
  );
}

function permSummary(m: RosterMember): string {
  if (m.role === "leader") return "可见全队 · 可管理";
  const view = m.view === null ? "可见全队" : `可见 ${m.view.length} 人`;
  const edit = m.edit.length > 0 ? ` · 可编辑 ${m.edit.length} 人` : "";
  return view + edit;
}

/** 团队长的逐成员权限编辑(可见范围 + 可编辑哪些人)。 */
function PermEditor({
  member,
  others,
  busy,
  onSave,
}: {
  member: RosterMember;
  others: RosterMember[];
  busy: boolean;
  onSave: (view: string[] | null, edit: string[]) => void;
}) {
  const [viewAll, setViewAll] = useState(member.view === null);
  const [viewSel, setViewSel] = useState<Set<string>>(new Set(member.view ?? []));
  const [editSel, setEditSel] = useState<Set<string>>(new Set(member.edit));

  function toggle(set: Set<string>, id: string, apply: (s: Set<string>) => void) {
    const next = new Set(set);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    apply(next);
  }

  return (
    <div className="mt-2 space-y-2 rounded-md bg-secondary/40 p-2.5">
      <div className="text-caption text-muted-foreground">
        可见范围(他在本页能看到谁的案件;自己恒可见):
      </div>
      <div className="flex flex-wrap items-center gap-3 text-xs">
        <label className="flex items-center gap-1.5">
          <input
            type="radio"
            checked={viewAll}
            onChange={() => setViewAll(true)}
            className="accent-sky-600"
          />
          全队
        </label>
        <label className="flex items-center gap-1.5">
          <input
            type="radio"
            checked={!viewAll}
            onChange={() => setViewAll(false)}
            className="accent-sky-600"
          />
          自定义:
        </label>
        {!viewAll &&
          others.map((o) => (
            <label key={o.member_id} className="flex items-center gap-1 text-xs">
              <input
                type="checkbox"
                checked={viewSel.has(o.member_id)}
                onChange={() => toggle(viewSel, o.member_id, setViewSel)}
                className="size-3.5 accent-sky-600"
              />
              {o.name}
            </label>
          ))}
      </div>
      <div className="text-caption text-muted-foreground">
        可编辑(允许他更新谁的案件登记信息;编辑功能即将开放,权限先配好):
      </div>
      <div className="flex flex-wrap gap-3">
        {others.map((o) => (
          <label key={o.member_id} className="flex items-center gap-1 text-xs">
            <input
              type="checkbox"
              checked={editSel.has(o.member_id)}
              onChange={() => toggle(editSel, o.member_id, setEditSel)}
              className="size-3.5 accent-sky-600"
            />
            {o.name}
          </label>
        ))}
      </div>
      <button
        type="button"
        disabled={busy}
        onClick={() => onSave(viewAll ? null : Array.from(viewSel), Array.from(editSel))}
        className="rounded-md bg-sky-600 px-3 py-1 text-xs font-medium text-white hover:bg-sky-700 disabled:opacity-50"
      >
        保存权限
      </button>
    </div>
  );
}

/* ==================================================================== */
/* 看板:全队重要日期 + 按成员分组                                       */
/* ==================================================================== */

/** 全队重要日期横条:聚合可见成员未来的关键日期。 */
function TeamKeyDates({ view }: { view: TeamView }) {
  const upcoming = useMemo(() => {
    const today = todayIsoLocal();
    const items: { date: string; event: string; who: string; caseName: string }[] = [];
    for (const m of view.members) {
      for (const c of m.cases) {
        for (const d of c.key_dates) {
          if (d.date >= today) {
            items.push({ date: d.date, event: d.event, who: m.name, caseName: c.name });
          }
        }
      }
    }
    items.sort((a, b) => a.date.localeCompare(b.date));
    return items.slice(0, 8);
  }, [view]);

  if (upcoming.length === 0) return null;
  return (
    <div className="mb-5 rounded-lg border border-border bg-card/60 p-3">
      <p className="mb-2 flex items-center gap-1.5 text-xs font-semibold text-foreground">
        <CalendarClock className="size-3.5 text-sky-600" />
        全队重要日期
      </p>
      <div className="flex flex-wrap gap-2">
        {upcoming.map((it, i) => {
          const days = daysFromToday(it.date) ?? 0;
          const urgent = days <= 30;
          return (
            <span
              key={i}
              className={cn(
                "rounded-md px-2 py-1 text-caption",
                urgent ? "bg-red-50 text-red-700" : "bg-secondary text-muted-foreground",
              )}
              title={it.caseName}
            >
              {it.date} {it.event} · {it.who}
              {urgent && days >= 0 ? `(${days} 天)` : ""}
            </span>
          );
        })}
      </div>
    </div>
  );
}

function freshness(updatedAt: string | null): { text: string; stale: boolean } {
  if (!updatedAt) return { text: "尚未收到数据", stale: true };
  const ms = Date.now() - new Date(updatedAt).getTime();
  const days = Math.floor(ms / (24 * 3600 * 1000));
  if (days <= 0) return { text: "今天已同步", stale: false };
  return { text: `上次同步 ${days} 天前`, stale: days >= 7 };
}

/** 「按最新进展」:全队可见案件平铺,latest_event 日期新者在前(无进展的排最后)。 */
function LatestFirstGrid({
  view,
  edits,
  onChanged,
  onOpenCase,
}: {
  view: TeamView;
  edits: TeamEdit[];
  onChanged: () => void;
  onOpenCase: (memberId: string, caseId: string) => void;
}) {
  const [editingCase, setEditingCase] = useState<string | null>(null);
  const flat = useMemo(() => {
    const items: { c: TeamSnapshotCase; owner: TeamMemberView }[] = [];
    for (const m of view.members) {
      for (const c of m.cases) {
        items.push({ c, owner: m });
      }
    }
    items.sort((a, b) => {
      const da = a.c.latest_event?.date ?? "";
      const db = b.c.latest_event?.date ?? "";
      if (da !== db) return db.localeCompare(da); // 日期新者在前;无进展("")自然落底
      return a.c.name.localeCompare(b.c.name, "zh");
    });
    return items;
  }, [view]);

  if (flat.length === 0) {
    return (
      <p className="rounded-md border border-dashed border-border p-4 text-xs text-muted-foreground">
        还没有案件数据,等队友同步后再来看。
      </p>
    );
  }
  return (
    <div className="grid gap-2.5 sm:grid-cols-2 lg:grid-cols-3">
      {flat.map(({ c, owner }, i) => (
        <CaseCard
          key={c.id || `${owner.member_id}-${i}`}
          c={c}
          owner={owner}
          edits={edits}
          editing={editingCase === c.id && !!c.id}
          onToggleEdit={() => setEditingCase(editingCase === c.id ? null : c.id)}
          onChanged={onChanged}
          onOpen={() => c.id && onOpenCase(owner.member_id, c.id)}
          showOwner
        />
      ))}
    </div>
  );
}

function MemberGroup({
  m,
  edits,
  onChanged,
  onOpenCase,
}: {
  m: TeamMemberView;
  edits: TeamEdit[];
  onChanged: () => void;
  onOpenCase: (caseId: string) => void;
}) {
  const f = freshness(m.updated_at);
  const [editingCase, setEditingCase] = useState<string | null>(null);
  return (
    <section className={cn(f.stale && !m.is_me && "opacity-60")}>
      <div className="mb-2 flex items-center gap-2">
        <span className="flex size-7 items-center justify-center rounded-full bg-sky-100 text-xs font-semibold text-sky-700">
          {m.name.slice(0, 1)}
        </span>
        <span className="text-sm font-semibold text-foreground">{m.name}</span>
        {m.role === "leader" && <Crown className="size-3.5 text-amber-500" />}
        {m.is_me && <span className="rounded bg-secondary px-1.5 py-0.5 text-caption">我</span>}
        <span className="text-caption text-muted-foreground">
          {m.cases.length} 个案件 · {f.text}
        </span>
      </div>
      {m.cases.length === 0 ? (
        <p className="rounded-md border border-dashed border-border p-3 text-xs text-muted-foreground">
          暂无案件数据
        </p>
      ) : (
        <div className="grid gap-2.5 sm:grid-cols-2 lg:grid-cols-3">
          {m.cases.map((c, i) => (
            <CaseCard
              key={c.id || i}
              c={c}
              owner={m}
              edits={edits}
              editing={editingCase === c.id && !!c.id}
              onToggleEdit={() => setEditingCase(editingCase === c.id ? null : c.id)}
              onChanged={onChanged}
              onOpen={() => c.id && onOpenCase(c.id)}
            />
          ))}
        </div>
      )}
    </section>
  );
}

function CaseCard({
  c,
  owner,
  edits,
  editing,
  onToggleEdit,
  onChanged,
  onOpen,
  showOwner = false,
}: {
  c: TeamSnapshotCase;
  owner: TeamMemberView;
  edits: TeamEdit[];
  editing: boolean;
  onToggleEdit: () => void;
  onChanged: () => void;
  onOpen: () => void;
  showOwner?: boolean;
}) {
  // 这个案件的团队层覆盖:已生效备注(最新 2 条)+ 待生效改动
  const caseEdits = edits.filter((e) => c.id && e.case_id === c.id);
  const notes = caseEdits
    .filter((e) => e.field === "note" && e.status === "applied")
    .slice(0, 2);
  const pending = caseEdits.filter((e) => e.status === "pending");

  // 下一个未来日期(提醒位);最新进展(摘要位,老板需求:显示时间轴最新发生的事)
  const today = todayIsoLocal();
  const nextUpcoming = c.key_dates.find((d) => d.date >= today);
  return (
    <div
      className="cursor-pointer rounded-lg border border-border bg-card p-3 transition-colors hover:border-sky-300"
      onClick={onOpen}
      title="点击查看案件详情"
    >
      <div className="mb-1 flex items-start justify-between gap-2">
        <p className="text-sm font-medium text-foreground">{c.name}</p>
        <span className="flex shrink-0 items-center gap-1">
          {showOwner && (
            <span className="rounded bg-secondary px-1.5 py-0.5 text-caption text-muted-foreground">
              {owner.name}
              {owner.is_me ? "(我)" : ""}
            </span>
          )}
          {c.stage && (
            <span className="rounded bg-sky-50 px-1.5 py-0.5 text-caption text-sky-700">
              {stageLabel(c.stage)}
            </span>
          )}
        </span>
      </div>
      {c.case_no && <p className="text-caption text-muted-foreground">{c.case_no}</p>}
      {c.parties && <p className="text-caption text-muted-foreground">{c.parties}</p>}
      {c.claim_amount != null && c.claim_amount > 0 && (
        <p className="text-caption text-muted-foreground">
          标的 ¥{c.claim_amount.toLocaleString()}
        </p>
      )}
      {c.latest_event ? (
        <p className="mt-1 text-caption font-medium text-foreground">
          最新进展:{c.latest_event.date} {c.latest_event.event}
        </p>
      ) : (
        c.summary && (
          <p className="mt-1 line-clamp-2 text-caption text-muted-foreground">{c.summary}</p>
        )
      )}
      {nextUpcoming && (
        <p className="mt-1 text-caption text-amber-700">
          ⏰ {nextUpcoming.date} {nextUpcoming.event}
        </p>
      )}

      {notes.map((n) => (
        <p key={n.id} className="mt-1 rounded bg-amber-50 px-1.5 py-1 text-caption text-amber-800">
          💬 {n.editor_name}:{n.value}
        </p>
      ))}
      {pending.length > 0 && (
        <p className="mt-1 text-caption text-muted-foreground">
          ⏳ {pending.length} 条改动待对方同步生效
        </p>
      )}

      {owner.can_edit && !owner.is_me && (
        <button
          type="button"
          disabled={!c.id}
          title={c.id ? "更新这个案件的状态 / 留备注" : "对方版本较旧,请其升级后重新同步"}
          onClick={(ev) => {
            ev.stopPropagation();
            onToggleEdit();
          }}
          className={cn(
            "mt-2 rounded border px-2 py-0.5 text-caption transition-colors",
            editing
              ? "border-sky-400 bg-sky-50 text-sky-700"
              : "border-border text-muted-foreground hover:text-foreground",
            !c.id && "opacity-50",
          )}
        >
          编辑
        </button>
      )}
      {editing && (
        <div onClick={(ev) => ev.stopPropagation()}>
          <CaseEditPanel c={c} owner={owner} onChanged={onChanged} />
        </div>
      )}
    </div>
  );
}

/** 编辑面板:留备注(团队是只读镜像,不改本地状态——状态在各自本地诉讼看板改)。 */
function CaseEditPanel({
  c,
  owner,
  onChanged,
  showNote = true,
}: {
  c: TeamSnapshotCase;
  owner: TeamMemberView;
  onChanged: () => void;
  showNote?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");

  async function submit(field: string, value: string, label: string) {
    if (busy || !value.trim()) return;
    setBusy(true);
    try {
      await teamSubmitEdit(owner.member_id, c.id, c.name, field, value.trim());
      toast(`${label}已提交,待「${owner.name}」的 App 同步后生效`, "success");
      setNote("");
      onChanged();
    } catch (e) {
      toast(`提交失败:${e}`, "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mt-2 space-y-2 rounded-md bg-secondary/40 p-2">
      {showNote && (
        <div className="flex items-center gap-2">
          <input
            type="text"
            value={note}
            onChange={(e) => setNote(e.target.value)}
            placeholder="留一条备注(如:下周三前补充证据)"
            className={cn(inputCls, "flex-1")}
            maxLength={200}
          />
          <button
            type="button"
            disabled={busy || !note.trim()}
            onClick={() => void submit("note", note, "备注")}
            className="rounded-md bg-sky-600 px-2.5 py-1 text-xs font-medium text-white hover:bg-sky-700 disabled:opacity-50"
          >
            留言
          </button>
        </div>
      )}
    </div>
  );
}

/* ==================================================================== */
/* 团队案件详情页(0.3.11 老板需求):点案件卡进入,无 AI 助手;          */
/* 顶部备注栏(可见即可写,固定显示)+ 全部登记详情 + 时间轴             */
/* ==================================================================== */

function TeamCaseDetail({
  c,
  owner,
  edits,
  myId,
  onBack,
  onChanged,
}: {
  c: TeamSnapshotCase;
  owner: TeamMemberView;
  edits: TeamEdit[];
  myId: string;
  onBack: () => void;
  onChanged: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  const notes = edits.filter(
    (e) => e.case_id === c.id && e.field === "note" && e.status === "applied",
  );
  const pendingMine = edits.filter(
    (e) => e.case_id === c.id && e.status === "pending" && e.editor_id === myId,
  );
  const f = freshness(owner.updated_at);
  const today = todayIsoLocal();
  const timeline = [...c.key_dates].sort((a, b) => b.date.localeCompare(a.date));

  async function submitNote() {
    if (busy || !note.trim()) return;
    setBusy(true);
    try {
      await teamSubmitEdit(owner.member_id, c.id, c.name, "note", note.trim());
      toast(
        owner.is_me ? "备注已记下" : `备注已提交,待「${owner.name}」的 App 同步后固定显示`,
        "success",
      );
      setNote("");
      onChanged();
    } catch (e) {
      toast(`备注失败:${e}`, "error");
    } finally {
      setBusy(false);
    }
  }

  async function revertNote(e: TeamEdit) {
    const ok = await confirmDialog(`删除「${e.editor_name}」的这条备注?`, {
      danger: true,
      okLabel: "删除",
    });
    if (!ok) return;
    try {
      await teamRevertEdit(e.id);
      toast("备注已删除", "success");
      onChanged();
    } catch (err) {
      toast(`删除失败:${err}`, "error");
    }
  }

  return (
    <div className="app-shell h-full overflow-auto">
      <div className="app-page-enter mx-auto max-w-4xl px-4 py-6 sm:px-6 xl:px-8">
        {/* 头部 */}
        <div className="mb-4 flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={onBack}
            className="inline-flex items-center gap-1 rounded-md border border-border px-2.5 py-1 text-xs text-muted-foreground hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" />
            返回团队
          </button>
          <h2 className="text-lg font-semibold text-foreground">{c.name}</h2>
          {c.stage && (
            <span className="rounded bg-sky-50 px-2 py-0.5 text-xs text-sky-700">
              {stageLabel(c.stage)}
            </span>
          )}
          <span className="text-caption text-muted-foreground">
            {owner.name}
            {owner.is_me ? "(我)" : ""} 的案件 · {f.text}
          </span>
        </div>

        {/* 备注栏(置顶固定) */}
        <div className="mb-4 rounded-lg border border-amber-200 bg-amber-50/50 p-3">
          <p className="mb-2 text-xs font-semibold text-amber-900">📌 团队备注(全队可写,固定显示)</p>
          {notes.length === 0 && pendingMine.filter((e) => e.field === "note").length === 0 && (
            <p className="mb-2 text-caption text-muted-foreground">还没有备注。</p>
          )}
          <div className="space-y-1.5">
            {notes.map((n) => (
              <div
                key={n.id}
                className="flex items-start gap-2 rounded-md bg-white/70 px-2 py-1.5 text-xs text-amber-900"
              >
                <span className="flex-1">
                  <strong>{n.editor_name}</strong>:{n.value}
                  <span className="ml-1 text-caption text-muted-foreground">
                    {n.created_at.slice(5, 10)}
                  </span>
                </span>
                {owner.is_me && (
                  <button
                    type="button"
                    onClick={() => void revertNote(n)}
                    className="shrink-0 text-caption text-muted-foreground hover:text-destructive"
                  >
                    删除
                  </button>
                )}
              </div>
            ))}
            {pendingMine
              .filter((e) => e.field === "note")
              .map((n) => (
                <p key={n.id} className="text-caption text-muted-foreground">
                  ⏳ 我的备注「{n.value}」待对方同步生效
                </p>
              ))}
          </div>
          <div className="mt-2 flex items-center gap-2">
            <input
              type="text"
              value={note}
              onChange={(e) => setNote(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void submitNote()}
              placeholder="写一条备注(如:下周三前补充证据)…"
              maxLength={200}
              className={cn(inputCls, "flex-1 bg-white/80")}
            />
            <button
              type="button"
              disabled={busy || !note.trim()}
              onClick={() => void submitNote()}
              className="rounded-md bg-amber-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-amber-700 disabled:opacity-50"
            >
              留备注
            </button>
          </div>
        </div>

        {/* 基本信息 */}
        <div className="mb-4 rounded-lg border border-border bg-card p-4">
          <p className="mb-2 text-xs font-semibold text-foreground">基本信息</p>
          <div className="grid grid-cols-1 gap-x-6 gap-y-1.5 text-xs sm:grid-cols-2">
            <DetailRow label="案号" value={c.case_no} />
            <DetailRow label="受理法院" value={c.court} />
            <DetailRow label="案由" value={c.cause} />
            <DetailRow label="案件类型" value={c.case_type} />
            <DetailRow label="立案日期" value={c.filed_at} />
            <DetailRow
              label="标的额"
              value={
                c.claim_amount != null && c.claim_amount > 0
                  ? `¥${c.claim_amount.toLocaleString()}`
                  : null
              }
            />
            <DetailRow label="处理结果" value={c.status_detail} />
            <DetailRow label="最近动态" value={c.last_activity} />
          </div>
          {c.summary && (
            <p className="mt-2 rounded-md bg-secondary/50 px-2 py-1.5 text-xs text-muted-foreground">
              {c.summary}
            </p>
          )}
        </div>

        {/* 当事人 */}
        {((c.plaintiffs?.length ?? 0) > 0 ||
          (c.defendants?.length ?? 0) > 0 ||
          (c.third_parties?.length ?? 0) > 0 ||
          c.parties) && (
          <div className="mb-4 rounded-lg border border-border bg-card p-4">
            <p className="mb-2 text-xs font-semibold text-foreground">当事人</p>
            <div className="space-y-1 text-xs">
              {(c.plaintiffs?.length ?? 0) > 0 && (
                <DetailRow label="原告/申请人" value={c.plaintiffs!.join("、")} />
              )}
              {(c.defendants?.length ?? 0) > 0 && (
                <DetailRow label="被告/被申请人" value={c.defendants!.join("、")} />
              )}
              {(c.third_parties?.length ?? 0) > 0 && (
                <DetailRow label="第三人" value={c.third_parties!.join("、")} />
              )}
              {(c.plaintiffs?.length ?? 0) === 0 &&
                (c.defendants?.length ?? 0) === 0 &&
                c.parties && <DetailRow label="当事人" value={c.parties} />}
            </div>
          </div>
        )}

        {/* 执行款(有数据才显示) */}
        {(c.execution_total ?? 0) > 0 && (
          <div className="mb-4 rounded-lg border border-border bg-card p-4">
            <p className="mb-2 text-xs font-semibold text-foreground">执行款</p>
            <div className="grid grid-cols-3 gap-2 text-xs">
              <DetailRow label="应执行" value={`¥${(c.execution_total ?? 0).toLocaleString()}`} />
              <DetailRow
                label="已到账"
                value={`¥${(c.execution_received ?? 0).toLocaleString()}`}
              />
              <DetailRow
                label="未到账"
                value={`¥${(c.execution_remaining ?? 0).toLocaleString()}`}
              />
            </div>
          </div>
        )}

        {/* 时间轴(最新在前;未来日期 ⏰ 标) */}
        {timeline.length > 0 && (
          <div className="mb-4 rounded-lg border border-border bg-card p-4">
            <p className="mb-2 text-xs font-semibold text-foreground">时间轴</p>
            <div className="space-y-1">
              {timeline.map((d, i) => (
                <p
                  key={i}
                  className={cn(
                    "text-xs",
                    d.date > today ? "text-amber-700" : "text-muted-foreground",
                    c.latest_event && d.date === c.latest_event.date && d.event === c.latest_event.event
                      ? "font-medium text-foreground"
                      : "",
                  )}
                >
                  {d.date > today ? "⏰ " : ""}
                  {d.date} {d.event}
                  {c.latest_event &&
                    d.date === c.latest_event.date &&
                    d.event === c.latest_event.event && (
                      <span className="ml-1 rounded bg-sky-50 px-1 text-caption text-sky-700">
                        最新进展
                      </span>
                    )}
                </p>
              ))}
            </div>
          </div>
        )}

        {/* 改状态(有编辑权且不是自己的案件) */}
        {owner.can_edit && !owner.is_me && (
          <div className="mb-4 rounded-lg border border-border bg-card p-4">
            <p className="mb-2 text-xs font-semibold text-foreground">更新状态</p>
            <CaseEditPanel c={c} owner={owner} onChanged={onChanged} showNote={false} />
          </div>
        )}

        <p className="text-caption text-muted-foreground">
          以上为案件登记信息(由 {owner.name} 的 App 自动同步);文档原文、报告与 AI
          助手仅案件所有人本机可用。
        </p>
      </div>
    </div>
  );
}

function DetailRow({ label, value }: { label: string; value: string | null | undefined }) {
  if (!value) return null;
  return (
    <p>
      <span className="text-muted-foreground">{label}:</span>
      <span className="ml-1 text-foreground">{value}</span>
    </p>
  );
}

import { useEffect, useState } from "react";
import { Pencil } from "lucide-react";
import {
  DndContext,
  type DragEndEvent,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";

import { type Case, type CaseInstance, type Document } from "@/lib/types";
import { listCaseInstances } from "@/lib/api";
import { formatYuan } from "@/lib/format";
import { computeCaseSnapshot } from "@/lib/caseSnapshot";
import {
  applyFieldOverrides,
  rowKeyOf,
  subtableFieldPath,
  type SubtableField,
} from "@/lib/userOverrides";
import { useCaseOverrides } from "@/hooks/useCaseOverrides";

import {
  CardSection,
  Dash,
  type DragHandleProps,
  FactRow,
  KeyMetric,
  TablePeople,
  type TablePeopleRow,
} from "./atoms";
import { CaseTimeline } from "./CaseTimeline";
import { EditableField } from "./EditableField";
import { SortableCard } from "./SortableCard";
import { TodosCard } from "@/components/TodosCard";
import { WorkLogTimeline } from "./WorkLogTimeline";

/**
 * 案件画像主视图。
 *
 * P2 (V0.1.13) 起接入 user_overrides overlay:
 *   - 渲染时 LLM 抽出的 agg_* 之上叠加用户改动
 *   - 编辑模式 (isEditMode = true) 时:
 *     · 字段 contentEditable 失焦自动保存
 *     · 卡片右上 EyeOff 隐藏(只删显示)
 *     · 子表行 hover 右侧 × 删除(只删显示)
 *     · LLM 永不覆盖任何这一层(数据隔离在 user_overrides_json 列)
 *
 * P3b 待补:@dnd-kit 拖卡片排序 + 子表 cell inline 编辑(电话/姓名等)
 */
export function CaseSnapshotView({
  caseData,
  documents,
  isEditMode = false,
  domain = "civil",
}: {
  caseData: Case;
  documents: Document[];
  isEditMode?: boolean;
  /** 案件领域(2026-06-17)。"criminal" = 刑事 tab:部分标签按刑事适配(罪名/办案机关/被告人…)。 */
  domain?: "civil" | "criminal";
}) {
  // 刑事 tab 只做「标签级」适配(老板:先复刻框架 + 能做的轻适配,不深改字段管线)。
  const isCriminal = domain === "criminal";
  const ov = useCaseOverrides(caseData.id, caseData.user_overrides_json);

  // 2026-06-11 审级模型:多审级案件([仲裁]→一审→二审→[再审])加载审级实例,
  // ≥2 个审级时渲染「审级历程」卡(最新在上)。重抽后(agg_computed_at 变)自动刷新。
  const [instances, setInstances] = useState<CaseInstance[]>([]);
  useEffect(() => {
    let alive = true;
    listCaseInstances(caseData.id)
      .then((l) => {
        if (alive) setInstances(l);
      })
      .catch(() => {
        /* 加载失败静默 — 卡片不渲染即可 */
      });
    return () => {
      alive = false;
    };
  }, [caseData.id, caseData.agg_computed_at]);

  // LLM snapshot + 用户 overlay
  const rawSnap = computeCaseSnapshot(caseData, documents);
  const snap = applyFieldOverrides(rawSnap, ov.overrides);

  const amountText = snap.claim_amount ? formatYuan(snap.claim_amount) : null;
  // 编辑态显示纯数字字符串(给用户改);非编辑态显示带 ¥ 千位逗号的格式化值(给人看)。
  // 关键修:用户保留 ¥ 前缀改数字时 parseFloat 会失败,UI 看起来"恢复原值"(其实
  // override 已经存了但格式串解析不出来)。编辑态直接给纯数字串就没这困扰。
  const amountEditValue =
    snap.claim_amount != null ? String(snap.claim_amount) : null;
  const partyL = snap.plaintiffs.length
    ? snap.plaintiffs.slice(0, 3).join("、") + (snap.plaintiffs.length > 3 ? `等${snap.plaintiffs.length}人` : "")
    : null;
  const partyR = snap.defendants.length
    ? snap.defendants.slice(0, 3).join("、") + (snap.defendants.length > 3 ? `等${snap.defendants.length}人` : "")
    : null;

  // 行级删除过滤 + rowKey 计算用 rawSnap(advisor 警告:rowKey 必须 stable,
  // 不能用 applyOverrides 后的 snap 算 — 否则用户改 name 后 rowKey 也变,
  // 同行其他 overrides 全部变孤儿)。
  // applyOverrides 不改数组长度 / 顺序,index 对齐安全。
  const partyContactRows: TablePeopleRow[] = snap.party_contacts
    .map((c, i) => {
      const rowKey = rowKeyOf("agg_party_contacts", rawSnap.party_contacts[i]);
      // 第 0 列 "当事人" 直接用 c.role(改 role override 后立即一致,不依赖 adapter
      // 重跑 — caseSnapshot.ts 里 party 字段只是 LLM role 的冗余复制)
      const partyDisplay = c.role || "—";
      const aliases =
        c.aliases && c.aliases.length > 0 ? `(其他身份:${c.aliases.join(" · ")})` : "";
      return {
        rowKey,
        cells: [partyDisplay, c.name, aliases || null, c.phone, c.email],
      };
    })
    .filter((r) => !ov.rowDeleted("agg_party_contacts", r.rowKey!));

  const courtContactRows: TablePeopleRow[] = snap.court_contacts
    .map((c, i) => {
      const rowKey = rowKeyOf("agg_court_contacts", rawSnap.court_contacts[i]);
      return {
        rowKey,
        cells: [c.role || "—", c.name, c.phone],
      };
    })
    .filter((r) => !ov.rowDeleted("agg_court_contacts", r.rowKey!));

  const feeRows: TablePeopleRow[] = snap.fees
    .map((f, i) => {
      const rowKey = rowKeyOf("agg_fees", rawSnap.fees[i]);
      // 金额编辑态给纯数字字符串(用户改"5000"不会被 ¥ / , 噪声拦)
      // 非编辑态用 toLocaleString 加千位逗号(人读)
      const amountCell =
        f.amount != null
          ? isEditMode
            ? String(f.amount)
            : f.amount.toLocaleString("zh-CN")
          : null;
      return {
        rowKey,
        cells: [f.item, amountCell, f.charged_at, f.receipt_no, f.note],
      };
    })
    .filter((r) => !ov.rowDeleted("agg_fees", r.rowKey!));

  // 为子表卡片底部"已删·还原"chip 准备数据(rowKey + 用户友好显示文本)
  const deletedSummary = (
    field: SubtableField,
  ): Array<{ rowKey: string; label: string }> => {
    const keys = ov.overrides.deleted_rows?.[field] ?? [];
    return keys.map((k) => ({ rowKey: k, label: k.replace(/\|/g, " | ") }));
  };

  const preservationRows: TablePeopleRow[] = snap.preservations.map((p) => ({
    rowKey: undefined, // 保全 P3a 不支持删行(snapshot 没暴露 stable key)
    cells: [
      p.target,
      p.amount != null ? `¥ ${p.amount.toLocaleString("zh-CN")}` : null,
      p.started_at,
      p.duration_years != null ? `${p.duration_years}` : null,
      p.expires_at,
    ],
  }));

  // 卡片标题(用于 hidden_sections 匹配)
  const TITLES = {
    BASIC: "案件基本信息",
    TODOS: "待办清单",
    INSTANCES: "审级历程",
    COURT: "办案机关人员",
    PARTY: "当事人联系人",
    FEE: "收费记录",
    TIMELINE: "办案时间轴",
    PRESERVATION: "财产保全",
    WORK_LOG: "工作记录",
  } as const;

  /**
   * 字段编辑能力 helper — 把"绑 path"那一坨样板抽出来。
   * 调用:`<FactRow label="案由" value={snap.cause} {...edit("agg_cause")} />`
   *
   * 自动接 hasOverride + onReset(↺ 恢复按钮) — 哪个字段被改过都能一键回到 LLM 原值。
   */
  const edit = (path: string) => ({
    isEditMode,
    fieldPath: path,
    caseId: caseData.id,
    onEdit: (p: string, v: string | null) => ov.setField(p, v),
    hasOverride: ov.hasFieldOverride(path),
    onReset: () => ov.clearField(path),
  });

  /**
   * 子表单元格编辑 helper — TablePeople 用。
   *
   * 锁定:row-key 字段(name/role/event/date/item/amount)绝不可编辑,
   * 改了会让同行其他 overrides 变孤儿。editableCells 由调用方按 advisor 表传。
   */
  const cellEdit = (field: SubtableField) => ({
    caseId: caseData.id,
    onEditCell: (rowKey: string, inner: string, v: string | null) =>
      ov.setField(subtableFieldPath(field, rowKey, inner), v),
    hasCellOverride: (rowKey: string, inner: string) =>
      ov.hasFieldOverride(subtableFieldPath(field, rowKey, inner)),
    onResetCell: (rowKey: string, inner: string) =>
      ov.clearField(subtableFieldPath(field, rowKey, inner)),
  });

  /* ---------- 6 张卡片渲染器,按 ov.resolveOrder 顺序排版 ---------- */
  const defaultSectionOrder = [
    TITLES.BASIC,
    TITLES.TODOS,
    // ≥2 个审级才显示历程卡(单审级时与基本信息重复);紧跟基本信息,最新审级在上
    ...(instances.length >= 2 ? [TITLES.INSTANCES] : []),
    TITLES.COURT,
    TITLES.PARTY,
    TITLES.FEE,
    TITLES.TIMELINE,
    ...(snap.preservations.length > 0 ? [TITLES.PRESERVATION] : []),
    TITLES.WORK_LOG,
  ];

  const sections: SectionRenderer[] = [
    {
      id: TITLES.BASIC,
      render: (dragHandle) => (
        <CardSection
          title={TITLES.BASIC}
          isEditMode={isEditMode}
          hidden={ov.overrides.hidden_sections?.includes(TITLES.BASIC)}
          onToggleHidden={() => ov.toggleHidden(TITLES.BASIC)}
          dragHandle={dragHandle}
        >
          <dl className="grid grid-cols-1 gap-x-6 gap-y-3 sm:grid-cols-2 md:grid-cols-3">
            <FactRow label="案件编号" value={snap.case_no} mono {...edit("agg_case_no")} />
            <FactRow label="案件类型" value={snap.case_type} />
            <FactRow label="案件名称" value={caseData.name} />
            <FactRow label="承办机关" value={snap.court} {...edit("agg_court")} />
            <FactRow
              label="当前阶段"
              value={snap.case_stage}
              pill={!isEditMode}
              {...edit("case_stage")}
            />
            <FactRow
              label="案件状态"
              value={snap.case_status}
              pill={!isEditMode}
              {...edit("agg_status_text")}
            />
            <FactRow
              label={isCriminal ? "罪名 / 案由" : "案由"}
              value={snap.cause}
              {...edit("agg_cause")}
            />
            <FactRow label="委托人" value={snap.plaintiffs[0] || null} />
            <FactRow
              label={isCriminal ? "被告人 / 对方" : "对方当事人"}
              value={snap.defendants[0] || null}
            />
            <FactRow label="立案日期" value={snap.filed_at} mono {...edit("agg_filed_at")} />
            <FactRow
              label="预计结案日期"
              value={snap.expected_close_at}
              mono
              {...edit("expected_close_at")}
            />
            <FactRow label="备注" value={snap.case_note} {...edit("case_note")} />
          </dl>
        </CardSection>
      ),
    },
    {
      id: TITLES.TODOS,
      render: (dragHandle) => (
        <CardSection
          title={TITLES.TODOS}
          subtitle="手动待办,打钩完成;首页「待办汇总」会汇总各案未完成项"
          isEditMode={isEditMode}
          hidden={ov.overrides.hidden_sections?.includes(TITLES.TODOS)}
          onToggleHidden={() => ov.toggleHidden(TITLES.TODOS)}
          dragHandle={dragHandle}
        >
          <TodosCard caseId={caseData.id} />
        </CardSection>
      ),
    },
    {
      id: TITLES.COURT,
      render: (dragHandle) => (
        <CardSection
          title={TITLES.COURT}
          subtitle={
            isCriminal
              ? "办案机关(公安 / 检察院 / 法院)联系方式(自动从起诉书/判决书/笔录抽)"
              : "法院联系方式(自动从判决书/调解书/笔录抽)"
          }
          isEditMode={isEditMode}
          hidden={ov.overrides.hidden_sections?.includes(TITLES.COURT)}
          onToggleHidden={() => ov.toggleHidden(TITLES.COURT)}
          dragHandle={dragHandle}
        >
          <TablePeople
            headers={["角色", "姓名", "联系电话"]}
            rows={courtContactRows}
            emptyText="未抽到法院联系人(法院文书还没扫到 / 没跑完抽取)"
            isEditMode={isEditMode}
            onDeleteRow={(k) => ov.deleteRow("agg_court_contacts", k)}
            deletedRows={deletedSummary("agg_court_contacts")}
            onUndeleteRow={(k) => ov.undeleteRow("agg_court_contacts", k)}
            editableCells={[
              { colIndex: 0, inner: "role", placeholder: "角色" },
              { colIndex: 1, inner: "name", placeholder: "姓名" },
              { colIndex: 2, inner: "phone", placeholder: "电话" },
            ]}
            {...cellEdit("agg_court_contacts")}
          />
        </CardSection>
      ),
    },
    {
      id: TITLES.PARTY,
      render: (dragHandle) => (
        <CardSection
          title={TITLES.PARTY}
          subtitle="当事人和代理人的电话/邮箱(从起诉状/委托合同抽)"
          isEditMode={isEditMode}
          hidden={ov.overrides.hidden_sections?.includes(TITLES.PARTY)}
          onToggleHidden={() => ov.toggleHidden(TITLES.PARTY)}
          dragHandle={dragHandle}
        >
          <TablePeople
            headers={["当事人 / 角色", "联系人", "其他身份", "联系电话", "邮箱"]}
            rows={partyContactRows}
            emptyText="未抽到当事人联系方式"
            isEditMode={isEditMode}
            onDeleteRow={(k) => ov.deleteRow("agg_party_contacts", k)}
            deletedRows={deletedSummary("agg_party_contacts")}
            onUndeleteRow={(k) => ov.undeleteRow("agg_party_contacts", k)}
            editableCells={[
              { colIndex: 0, inner: "role", placeholder: "角色,如 原告 / 被告" },
              { colIndex: 1, inner: "name", placeholder: "联系人姓名" },
              // col 2 "其他身份"是 aliases 拼接(数组型),不解锁
              { colIndex: 3, inner: "phone", placeholder: "电话" },
              { colIndex: 4, inner: "email", placeholder: "邮箱" },
            ]}
            {...cellEdit("agg_party_contacts")}
          />
        </CardSection>
      ),
    },
    {
      id: TITLES.FEE,
      render: (dragHandle) => (
        <CardSection
          title={TITLES.FEE}
          subtitle="案件受理费/律师代理费/保全费等"
          isEditMode={isEditMode}
          hidden={ov.overrides.hidden_sections?.includes(TITLES.FEE)}
          onToggleHidden={() => ov.toggleHidden(TITLES.FEE)}
          dragHandle={dragHandle}
        >
          <TablePeople
            headers={["收费项目", "金额(元)", "收费时间", "收据号", "备注"]}
            rows={feeRows}
            emptyText="未抽到收费记录"
            isEditMode={isEditMode}
            onDeleteRow={(k) => ov.deleteRow("agg_fees", k)}
            deletedRows={deletedSummary("agg_fees")}
            onUndeleteRow={(k) => ov.undeleteRow("agg_fees", k)}
            editableCells={[
              { colIndex: 0, inner: "item", placeholder: "收费项目" },
              { colIndex: 1, inner: "amount", placeholder: "数字,如 5000" },
              { colIndex: 2, inner: "charged_at", placeholder: "yyyy-mm-dd" },
              { colIndex: 3, inner: "receipt_no", placeholder: "收据号" },
              { colIndex: 4, inner: "note", placeholder: "备注" },
            ]}
            {...cellEdit("agg_fees")}
          />
        </CardSection>
      ),
    },
    {
      id: TITLES.TIMELINE,
      render: (dragHandle) => {
        const ce = cellEdit("agg_key_dates");
        return (
          <CardSection
            title={TITLES.TIMELINE}
            isEditMode={isEditMode}
            hidden={ov.overrides.hidden_sections?.includes(TITLES.TIMELINE)}
            onToggleHidden={() => ov.toggleHidden(TITLES.TIMELINE)}
            dragHandle={dragHandle}
          >
            <CaseTimeline
              caseData={caseData}
              snap={snap}
              rawSnap={rawSnap}
              documents={documents}
              isEditMode={isEditMode}
              caseId={caseData.id}
              onEditCell={ce.onEditCell}
              hasCellOverride={ce.hasCellOverride}
              onResetCell={ce.onResetCell}
              onDeleteRow={(k) => ov.deleteRow("agg_key_dates", k)}
              rowDeleted={(k) => ov.rowDeleted("agg_key_dates", k)}
              deletedRows={deletedSummary("agg_key_dates")}
              onUndeleteRow={(k) => ov.undeleteRow("agg_key_dates", k)}
            />
          </CardSection>
        );
      },
    },
  ];
  if (instances.length >= 2) {
    sections.push({
      id: TITLES.INSTANCES,
      render: (dragHandle) => (
        <CardSection
          title={TITLES.INSTANCES}
          subtitle="各审级的案号/承办机关/当事人称谓(最新审级在上;AI 按各审级文书分别抽取)"
          isEditMode={isEditMode}
          hidden={ov.overrides.hidden_sections?.includes(TITLES.INSTANCES)}
          onToggleHidden={() => ov.toggleHidden(TITLES.INSTANCES)}
          dragHandle={dragHandle}
        >
          <div className="space-y-3">
            {instances.map((ins) => {
              const handlers = parseJsonObjects<{
                name?: string | null;
                role?: string | null;
                phone?: string | null;
              }>(ins.handlers);
              const partyRoles = parseJsonObjects<{
                name?: string | null;
                role?: string | null;
                note?: string | null;
              }>(ins.party_roles);
              const isArb = ins.authority_type === "仲裁委";
              const handlerText =
                handlers
                  .map((h) =>
                    [h.name, h.role && `(${h.role})`].filter(Boolean).join(""),
                  )
                  .filter(Boolean)
                  .join("、") || null;
              return (
                <div
                  key={ins.id}
                  className={
                    "rounded-md border p-4 " +
                    (ins.is_current
                      ? "border-foreground/30 bg-foreground/[0.02]"
                      : "border-border")
                  }
                >
                  <div className="mb-2 flex flex-wrap items-center gap-2">
                    <span
                      className={
                        "rounded px-2 py-0.5 text-xs font-medium " +
                        (ins.is_current
                          ? "bg-foreground text-background"
                          : "bg-muted text-muted-foreground")
                      }
                    >
                      {ins.level}
                      {ins.is_current ? " · 当前" : ""}
                    </span>
                    {ins.note && (
                      <span className="text-xs text-muted-foreground">{ins.note}</span>
                    )}
                  </div>
                  <dl className="grid grid-cols-1 gap-x-6 gap-y-2 sm:grid-cols-2 md:grid-cols-3">
                    <FactRow label="案号" value={ins.case_no} mono />
                    <FactRow
                      label={isArb ? "仲裁机构" : "承办法院"}
                      value={ins.authority}
                    />
                    <FactRow
                      label={isArb ? "仲裁员" : "承办人"}
                      value={handlerText}
                    />
                    <FactRow label="受理日期" value={ins.filed_at} mono />
                    <FactRow label="结果" value={ins.result} />
                  </dl>
                  {partyRoles.length > 0 && (
                    <p className="mt-2 text-xs text-muted-foreground">
                      当事人:
                      {partyRoles
                        .map((p) =>
                          `${p.name || "—"}(${[p.role, p.note]
                            .filter(Boolean)
                            .join(",")})`,
                        )
                        .join(" · ")}
                    </p>
                  )}
                </div>
              );
            })}
          </div>
        </CardSection>
      ),
    });
  }
  if (snap.preservations.length > 0) {
    sections.push({
      id: TITLES.PRESERVATION,
      render: (dragHandle) => (
        <CardSection
          title={TITLES.PRESERVATION}
          isEditMode={isEditMode}
          hidden={ov.overrides.hidden_sections?.includes(TITLES.PRESERVATION)}
          onToggleHidden={() => ov.toggleHidden(TITLES.PRESERVATION)}
          dragHandle={dragHandle}
        >
          <TablePeople
            headers={["保全标的", "金额", "起算日", "期限(年)", "到期日"]}
            rows={preservationRows}
            emptyText=""
          />
        </CardSection>
      ),
    });
  }
  sections.push({
    id: TITLES.WORK_LOG,
    render: (dragHandle) => (
      <CardSection
        title={TITLES.WORK_LOG}
        subtitle="记录律师为案件付出的具体劳动"
        isEditMode={isEditMode}
        hidden={ov.overrides.hidden_sections?.includes(TITLES.WORK_LOG)}
        onToggleHidden={() => ov.toggleHidden(TITLES.WORK_LOG)}
        dragHandle={dragHandle}
      >
        <WorkLogTimeline caseId={caseData.id} />
      </CardSection>
    ),
  });

  // 2026-06-13:立场被改过、但 LLM 画像/报告还没按新立场重抽 → 持久提示(不分编辑模式)。
  // 判据:overlay 后的 our_side ≠ DB 里 LLM 原值;重新分析后 agg_our_side 同步即消失。
  const ourSideStale =
    !!snap.our_side && snap.our_side !== caseData.agg_our_side;

  return (
    <div className="space-y-4">
      {/* 立场已改但未重抽:报告/画像仍是旧立场,提示去重新分析 */}
      {ourSideStale && (
        <div className="rounded-md border border-sky-200 bg-sky-50 px-3 py-2 text-xs text-sky-800">
          ⚠️ 我方代理立场已改为「{snap.our_side}」,但案件画像 / 报告仍按旧立场生成。
          请到下方「原文件 → 重新分析」重跑,报告才会按新立场调整侧重。
        </div>
      )}

      {/* 编辑模式 banner */}
      {isEditMode && (
        <div className="flex items-center gap-2 rounded-md border border-foreground/40 bg-foreground/10 px-3 py-2 text-xs">
          <Pencil className="size-3.5 shrink-0 text-foreground" />
          <span className="text-foreground">
            <strong>编辑模式已开启</strong>
            <span className="ml-2 text-muted-foreground">
              · 点字段改值(失焦自动存) · 鼠标移到表格行右侧 × 删行 ·
              卡片右上「眼睛」隐藏卡片 · 退出请点右上角铅笔
            </span>
          </span>
        </div>
      )}

      {/* Hero:案由 + 案号 + 法院 + vs banner + 关键数字 */}
      <section className="rounded-lg border border-border bg-card px-6 py-5 shadow-sm">
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
          <h2 className="text-xl font-semibold text-foreground">
            {isEditMode ? (
              <EditableField
                key={`${caseData.id}:agg_cause:hero`}
                initialValue={snap.cause}
                editable
                onCommit={(v) => ov.setField("agg_cause", v)}
                ariaLabel="编辑案由"
                editableClassName="text-xl font-semibold"
                hasOverride={ov.hasFieldOverride("agg_cause")}
                onReset={() => ov.clearField("agg_cause")}
              />
            ) : (
              snap.cause || <Dash />
            )}
          </h2>
          {(snap.case_no || isEditMode) && (
            <span className="font-mono text-sm text-muted-foreground">
              {isEditMode ? (
                <EditableField
                  key={`${caseData.id}:agg_case_no:hero`}
                  initialValue={snap.case_no}
                  editable
                  onCommit={(v) => ov.setField("agg_case_no", v)}
                  ariaLabel="编辑案号"
                  editableClassName="font-mono text-sm"
                  hasOverride={ov.hasFieldOverride("agg_case_no")}
                  onReset={() => ov.clearField("agg_case_no")}
                />
              ) : (
                snap.case_no
              )}
            </span>
          )}
        </div>
        <p className="mt-1 text-sm text-muted-foreground">
          {isEditMode ? (
            <EditableField
              key={`${caseData.id}:agg_court:hero`}
              initialValue={snap.court}
              editable
              onCommit={(v) => ov.setField("agg_court", v)}
              ariaLabel="编辑承办机关"
              editableClassName="text-sm"
              hasOverride={ov.hasFieldOverride("agg_court")}
              onReset={() => ov.clearField("agg_court")}
            />
          ) : (
            snap.court || <Dash />
          )}
        </p>

        {/* 我方代理立场 + 当事人对峙(2026-06-13:立场驱动报告侧重/AI 立场/各 chip)*/}
        <div className="mt-4 rounded-md bg-muted/40 px-4 py-3">
          {/* 立场行:编辑态可选,非编辑态淡蓝 badge;未识别提示去确认 */}
          <div className="mb-2 flex flex-wrap items-center gap-2">
            <span className="text-caption uppercase tracking-wider text-muted-foreground">
              我方代理立场
            </span>
            {isEditMode ? (
              <>
                <select
                  value={snap.our_side ?? ""}
                  onChange={(e) => {
                    const v = e.target.value;
                    if (v === "") ov.clearField("agg_our_side");
                    else ov.setField("agg_our_side", v);
                  }}
                  className="rounded border border-border bg-background px-2 py-0.5 text-sm text-foreground"
                  aria-label="选择我方代理立场"
                >
                  <option value="">未确认(跟随 AI 判断)</option>
                  <option value="原告方">原告方</option>
                  <option value="被告方">被告方</option>
                  <option value="第三人">第三人</option>
                  <option value="反诉混合">反诉混合</option>
                </select>
                {ov.hasFieldOverride("agg_our_side") && (
                  <span className="text-xs text-sky-700">
                    已手改 · 改完去下方「原文件 → 重新分析」让报告/画像按新立场重写
                  </span>
                )}
              </>
            ) : snap.our_side ? (
              <span className="rounded bg-sky-50 px-2 py-0.5 text-sm font-medium text-sky-800">
                {snap.our_side}
              </span>
            ) : (
              <span className="text-sm text-muted-foreground">
                未识别 —— 点右上角铅笔进入编辑模式确认我方是原告方还是被告方
              </span>
            )}
          </div>
          {/* 对峙(P3a 仍只读 — array 字段 P3b 接);我方一侧淡蓝标注 */}
          <div className="flex items-center gap-3">
            <div className="min-w-0 flex-1">
              <div className="text-caption uppercase tracking-wider text-muted-foreground">
                {isCriminal ? "被害方 / 控方" : "原告 / 申请人"}
                {snap.our_side === "原告方" && (
                  <span className="ml-1 font-medium text-sky-700">· 我方</span>
                )}
              </div>
              <div className="mt-0.5 truncate text-sm font-medium text-foreground">{partyL || <Dash />}</div>
            </div>
            <span className="shrink-0 font-mono text-xs text-muted-foreground">vs</span>
            <div className="min-w-0 flex-1 text-right">
              <div className="text-caption uppercase tracking-wider text-muted-foreground">
                {snap.our_side === "被告方" && (
                  <span className="mr-1 font-medium text-sky-700">我方 ·</span>
                )}
                {isCriminal ? "被告人 / 犯罪嫌疑人" : "被告 / 被申请人"}
              </div>
              <div className="mt-0.5 truncate text-sm font-medium text-foreground">{partyR || <Dash />}</div>
            </div>
          </div>
        </div>

        {/* 三个关键数字 */}
        <div className="mt-4 grid grid-cols-1 gap-4 sm:grid-cols-3">
          <KeyMetric
            label={isCriminal ? "涉案金额" : "诉讼金额"}
            value={isEditMode ? amountEditValue : amountText}
            mono
            {...edit("agg_claim_amount")}
          />
          <KeyMetric label="立案日期" value={snap.filed_at} mono {...edit("agg_filed_at")} />
          {/* 承办法官 / 承办人:array 字段,P3a 不编辑 */}
          <KeyMetric
            label={isCriminal ? "承办人" : "承办法官"}
            value={snap.judges.join("、") || null}
          />
        </div>
      </section>

      {/* 6 张卡片打包成 sortable list — 编辑模式拖把手才能拖,普通模式仅顺序应用 */}
      <SortableCards
        isEditMode={isEditMode}
        order={ov.resolveOrder(defaultSectionOrder)}
        onReorder={ov.setOrder}
        sections={sections}
      />

      {/* 基础信息(抽取进度) */}
      <p className="text-caption text-muted-foreground">
        基于 {snap.basedOnDocs} 份文档实时聚合
        {snap.computedAt
          ? ` · 后端聚合已完成 ${snap.computedAt.slice(0, 10)}`
          : " · 抽取中,跑更多文档会更准确"}
      </p>
    </div>
  );
}

/* ============================================================ */
/* SortableCards — DndContext + SortableContext 包装,管理拖排序  */
/* ============================================================ */

interface SectionRenderer {
  id: string;
  render: (dragHandle?: DragHandleProps) => React.ReactNode;
}

function SortableCards({
  isEditMode,
  order,
  onReorder,
  sections,
}: {
  isEditMode: boolean;
  /** 用户偏好顺序(resolveOrder 已合并 default + user_section_order) */
  order: string[];
  /** 拖拽结束写回新顺序 */
  onReorder: (newOrder: string[]) => void;
  sections: SectionRenderer[];
}) {
  // PointerSensor 距离 5px 才激活拖拽:防止点字段被误判为拖
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
  );

  const byId = new Map(sections.map((s) => [s.id, s]));
  // order 可能包含 user_section_order 留下的旧 id(snap schema 变了),过滤
  const validOrder = order.filter((id) => byId.has(id));

  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const oldIdx = validOrder.indexOf(String(active.id));
    const newIdx = validOrder.indexOf(String(over.id));
    if (oldIdx === -1 || newIdx === -1) return;
    onReorder(arrayMove(validOrder, oldIdx, newIdx));
  };

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      onDragEnd={handleDragEnd}
    >
      <SortableContext items={validOrder} strategy={verticalListSortingStrategy}>
        <div className="space-y-4">
          {validOrder.map((id) => {
            const s = byId.get(id)!;
            return (
              <SortableCard key={id} id={id} isEditMode={isEditMode}>
                {({ dragHandle }) => s.render(dragHandle)}
              </SortableCard>
            );
          })}
        </div>
      </SortableContext>
    </DndContext>
  );
}

/** JSON 数组字符串 → 对象数组(解析失败/null → [])。case_instances.handlers/party_roles 用。 */
function parseJsonObjects<T>(s: string | null): T[] {
  if (!s) return [];
  try {
    const v = JSON.parse(s);
    return Array.isArray(v) ? (v as T[]) : [];
  } catch {
    return [];
  }
}

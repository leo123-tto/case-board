/**
 * 2026-05-26 V0.1.13 · 案件画像编辑模式 — local overrides + debounce 写盘 hook。
 *
 * 设计要点(防 mid-edit clobber):
 *   - local overrides state 跟外部 caseData.user_overrides_json **解耦**。
 *   - 只在 caseId 变化时 seed 一次 local from server,**不**跟着外部 caseData 重置。
 *     原因:extraction_progress: completed 时 App.tsx 会 setSelectedCase 整个 case 对象,
 *     如果 local seed 跟外部 json 同步,用户正在 type 的字段会被冲掉。
 *   - 切案件 / 退出编辑模式 / unmount 前会 flush pending debounce,确保不丢改动。
 *
 * 一次写入流程:
 *   组件调 setField → 立刻更新 local overrides → 重置 300ms timer →
 *   timer 触发后 updateCaseOverrides 写 SQLite → 写完不 refetch(local 已经是 truth)
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { getCaseWithDocs, updateCaseOverrides } from "@/lib/api";
import {
  clearFieldOverride,
  getFieldOverride,
  isRowDeleted,
  markRowDeleted,
  parseOverrides,
  resolveSectionOrder,
  serializeOverrides,
  setFieldOverride,
  setSectionOrder,
  type SubtableField,
  toggleHiddenSection,
  unmarkRowDeleted,
  type UserOverrides,
} from "@/lib/userOverrides";

const DEBOUNCE_MS = 300;

export interface UseCaseOverridesResult {
  /** 当前 local overlay(渲染时叠加到 snapshot,UI 看到的就是这个) */
  overrides: UserOverrides;

  /** 改一个顶级字段。value = null 表示"用户清空了" */
  setField: (path: string, value: string | null) => void;
  /**
   * 恢复 LLM 原值 — 删掉这个 path 的 override,字段重新跟随重抽变化。
   * 跟 setField(path, null) 不同:null 是"用户主动清空"(算 override),
   * clearField 是"用户撤销自己改过的"(回到 LLM 视图)。
   */
  clearField: (path: string) => void;
  /** 查询某 path 是否被用户改过(EditableField 用来决定是否显示 ↺ 恢复按钮) */
  hasFieldOverride: (path: string) => boolean;
  /** 隐藏 / 取消隐藏一张卡片(按标题区分) */
  toggleHidden: (sectionTitle: string) => void;
  /** 删一行(只删显示,不删 DB) */
  deleteRow: (field: SubtableField, rowKey: string) => void;
  /** 撤销删除一行 */
  undeleteRow: (field: SubtableField, rowKey: string) => void;
  /** 查一行是不是被用户删了 */
  rowDeleted: (field: SubtableField, rowKey: string) => boolean;
  /** 拖动后写入新的卡片顺序 */
  setOrder: (order: string[]) => void;
  /** 解析最终卡片顺序(用户排过的在前,没排过的按 default 追加) */
  resolveOrder: (defaultOrder: string[]) => string[];

  /** 立刻把 pending 的 debounce 强制 flush(切案件 / unmount 前调) */
  flush: () => Promise<void>;
}

/**
 * 案件 overlay hook。caseId 变化时自动 seed local from server JSON。
 *
 * @param caseId 当前案件 id,变化触发 seed
 * @param initialJson 当前案件的 user_overrides_json(只在 caseId 变化时读)
 */
export function useCaseOverrides(
  caseId: string | null,
  initialJson: string | null,
): UseCaseOverridesResult {
  const [overrides, setOverrides] = useState<UserOverrides>(() =>
    parseOverrides(initialJson),
  );

  // pending debounce timer + 最新 overrides ref(flush 时拿)
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latestRef = useRef<UserOverrides>(overrides);
  const caseIdRef = useRef<string | null>(caseId);

  // 写盘 — 把 latestRef 里的最新 overrides 提交到 SQLite
  const writeToDb = useCallback(async (cid: string, o: UserOverrides) => {
    await updateCaseOverrides(cid, serializeOverrides(o));
  }, []);

  // debounce / 卸载路径没有等待者，保留原有的日志型失败处理；显式 flush 则必须把错误交还
  // 给调用方，尤其是“先 flush 再 reset”不能在查询锁拒绝后继续执行专用 reset。
  const writeToDbQuietly = useCallback(async (cid: string, o: UserOverrides) => {
    try {
      await writeToDb(cid, o);
    } catch (e) {
      // 写盘失败不阻塞编辑(下一次 mutation 会再触发写盘);只记 console 不弹错
      console.warn("updateCaseOverrides failed", e);
    }
  }, [writeToDb]);

  // 公共 flush:取消 timer 立刻写
  const flush = useCallback(async () => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    const cid = caseIdRef.current;
    if (cid) {
      await writeToDb(cid, latestRef.current);
    }
  }, [writeToDb]);

  // case 切换时:先 flush 旧 case pending → 再 seed 新 case
  useEffect(() => {
    const prevCid = caseIdRef.current;
    if (prevCid && prevCid !== caseId && timerRef.current) {
      // 强制 flush 上一案件的 pending 改动(不 await,避免阻塞 case 切换)
      clearTimeout(timerRef.current);
      timerRef.current = null;
      void writeToDbQuietly(prevCid, latestRef.current);
    }
    caseIdRef.current = caseId;
    const seeded = parseOverrides(initialJson);
    setOverrides(seeded);
    latestRef.current = seeded;
    // G 修复(2026-06-13):父组件传入的 caseData 可能是陈旧内存对象 —— 用户改完 override
    // (如我方立场)切到别的页面再回来,App 的 selectedCase 没在写盘后刷新,用陈旧 initialJson
    // seed 会把刚保存的改动"显示成丢了"。mount 时主动从 DB 读最新 user_overrides_json 校正:
    // 仅在仍是同一案件、且用户尚未开始本地编辑(timerRef 为空)时覆盖,防 mid-edit clobber。
    if (caseId) {
      const cid = caseId;
      void getCaseWithDocs(cid)
        .then((r) => {
          if (caseIdRef.current === cid && !timerRef.current) {
            const latest = parseOverrides(r.case.user_overrides_json);
            setOverrides(latest);
            latestRef.current = latest;
          }
        })
        .catch(() => {
          /* 读失败就保留 initialJson seed,静默 */
        });
    }
    // 故意只依赖 caseId,**不依赖 initialJson** — 防 mid-edit clobber:
    // 外部 setSelectedCase 刷新 caseData 时本 hook 不会重置 local overrides。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [caseId, writeToDbQuietly]);

  // AI 工具写入同一覆盖层后，只在这一处监听并重读 DB；本地有待保存输入时不抢写。
  useEffect(() => {
    if (!caseId) return;
    let unlisten: UnlistenFn | undefined;
    void listen<{ case_id: string }>("case-snapshot-changed", (event) => {
      if (event.payload.case_id !== caseId || timerRef.current) return;
      const cid = caseId;
      void getCaseWithDocs(cid)
        .then((result) => {
          if (caseIdRef.current !== cid || timerRef.current) return;
          const incoming = parseOverrides(result.case.user_overrides_json);
          if (serializeOverrides(incoming) !== serializeOverrides(latestRef.current)) {
            latestRef.current = incoming;
            setOverrides(incoming);
          }
        })
        .catch((error) => console.warn("refresh AI-updated case snapshot failed", error));
    })
      .then((stop) => {
        unlisten = stop;
      })
      .catch((error) => console.warn("listen case-snapshot-changed failed", error));
    return () => unlisten?.();
  }, [caseId]);

  // unmount 时 flush(用户切走整个 App / 关窗)
  useEffect(() => {
    return () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
        const cid = caseIdRef.current;
        if (cid) void writeToDbQuietly(cid, latestRef.current);
      }
    };
  }, [writeToDbQuietly]);

  // 公共 mutation 包装:更新 state + ref + 重置 debounce timer
  const mutate = useCallback(
    (fn: (o: UserOverrides) => UserOverrides) => {
      // 先基于 ref 同步算出 next,再交给 React 渲染。这样调用方紧接着 await flush()
      // 时一定拿到刚才的值,不会被 React 的异步 state 调度留在旧快照。
      const next = fn(latestRef.current);
      latestRef.current = next;
      setOverrides(next);
      if (timerRef.current) clearTimeout(timerRef.current);
      const cid = caseIdRef.current;
      if (cid) {
        timerRef.current = setTimeout(() => {
          timerRef.current = null;
          void writeToDbQuietly(cid, latestRef.current);
        }, DEBOUNCE_MS);
      }
    },
    [writeToDbQuietly],
  );

  const setField = useCallback(
    (path: string, value: string | null) => {
      mutate((o) => setFieldOverride(o, path, value));
    },
    [mutate],
  );

  const clearField = useCallback(
    (path: string) => {
      mutate((o) => clearFieldOverride(o, path));
    },
    [mutate],
  );

  const hasFieldOverride = useCallback(
    (path: string): boolean => {
      return getFieldOverride(overrides, path) !== undefined;
    },
    [overrides],
  );

  const toggleHidden = useCallback(
    (sectionTitle: string) => {
      mutate((o) => toggleHiddenSection(o, sectionTitle));
    },
    [mutate],
  );

  const deleteRow = useCallback(
    (field: SubtableField, rowKey: string) => {
      mutate((o) => markRowDeleted(o, field, rowKey));
    },
    [mutate],
  );

  const undeleteRow = useCallback(
    (field: SubtableField, rowKey: string) => {
      mutate((o) => unmarkRowDeleted(o, field, rowKey));
    },
    [mutate],
  );

  const rowDeleted = useCallback(
    (field: SubtableField, rowKey: string): boolean => {
      return isRowDeleted(overrides, field, rowKey);
    },
    [overrides],
  );

  const setOrder = useCallback(
    (order: string[]) => {
      mutate((o) => setSectionOrder(o, order));
    },
    [mutate],
  );

  const resolveOrder = useCallback(
    (defaultOrder: string[]): string[] => {
      return resolveSectionOrder(overrides, defaultOrder);
    },
    [overrides],
  );

  return {
    overrides,
    setField,
    clearField,
    hasFieldOverride,
    toggleHidden,
    deleteRow,
    undeleteRow,
    rowDeleted,
    setOrder,
    resolveOrder,
    flush,
  };
}

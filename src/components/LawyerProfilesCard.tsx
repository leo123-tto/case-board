/**
 * 律师档案管理卡片（挂在 SettingsModal 里）。
 *
 * 支持增删改多条律师档案，设为默认。
 * 供法院立案时选择代理人使用。
 */
import { useEffect, useState } from "react";
import { Plus, Trash2, StarOff } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  listLawyerProfiles,
  saveLawyerProfile,
  updateLawyerProfile,
  deleteLawyerProfile,
  setDefaultLawyer,
} from "@/lib/api";
import type { LawyerProfile } from "@/lib/types";

// 编辑中的律师档案（新增/编辑共用）
interface EditingProfile {
  id?: string; // 有 id = 编辑，无 id = 新增
  name: string;
  bar_number: string;
  law_firm: string;
  id_number: string;
  phone: string;
  address: string;
}

export function LawyerProfilesCard() {
  const [profiles, setProfiles] = useState<LawyerProfile[]>([]);
  const [editing, setEditing] = useState<EditingProfile | null>(null);
  const [loading, setLoading] = useState(false);

  async function refresh() {
    try {
      setProfiles(await listLawyerProfiles());
    } catch {
      /* 首次可能还没建表 */
    }
  }

  useEffect(() => {
    refresh();
  }, []);

  function startAdd() {
    setEditing({
      name: "",
      bar_number: "",
      law_firm: "",
      id_number: "",
      phone: "",
      address: "",
    });
  }

  function startEdit(p: LawyerProfile) {
    setEditing({
      id: p.id,
      name: p.name,
      bar_number: p.bar_number ?? "",
      law_firm: p.law_firm ?? "",
      id_number: p.id_number ?? "",
      phone: p.phone ?? "",
      address: p.address ?? "",
    });
  }

  async function handleSave() {
    if (!editing || !editing.name.trim()) return;
    setLoading(true);
    try {
      const payload = {
        name: editing.name.trim(),
        bar_number: editing.bar_number.trim() || null,
        law_firm: editing.law_firm.trim() || null,
        id_number: editing.id_number.trim() || null,
        phone: editing.phone.trim() || null,
        address: editing.address.trim() || null,
        is_default: 0,
      };
      if (editing.id) {
        await updateLawyerProfile(editing.id, payload);
      } else {
        await saveLawyerProfile(payload);
      }
      setEditing(null);
      await refresh();
    } catch (e) {
      alert("保存失败: " + String(e));
    } finally {
      setLoading(false);
    }
  }

  async function handleDelete(id: string) {
    if (!confirm("确定删除该律师档案？")) return;
    try {
      await deleteLawyerProfile(id);
      await refresh();
    } catch (e) {
      alert("删除失败: " + String(e));
    }
  }

  async function handleSetDefault(id: string) {
    try {
      await setDefaultLawyer(id);
      await refresh();
    } catch (e) {
      alert("设置默认失败: " + String(e));
    }
  }

  return (
    <section>
      <div className="mb-3 flex items-start justify-between gap-3">
        <div>
          <h3 className="text-base font-semibold text-foreground">律师档案</h3>
          <p className="mt-0.5 text-xs text-muted-foreground">
            管理代理律师信息，立案时自动填入。支持多律师档案。
          </p>
        </div>
      </div>

      <div className="space-y-3 rounded-lg border border-border bg-background/50 p-4">
        {/* 律师列表 */}
        {profiles.length === 0 && !editing && (
          <p className="text-xs text-muted-foreground">
            还没有律师档案。点下方按钮添加。
          </p>
        )}

        {profiles.map((p) => (
          <div
            key={p.id}
            className="flex items-center justify-between rounded border border-border px-3 py-2"
          >
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2">
                <span className="text-sm font-medium">{p.name}</span>
                {p.is_default ? (
                  <span className="text-xs text-amber-600">⭐ 默认</span>
                ) : null}
              </div>
              <p className="text-xs text-muted-foreground truncate">
                {p.law_firm || "—"} · {p.bar_number || "—"} · {p.phone || "—"}
              </p>
            </div>
            <div className="flex items-center gap-1 shrink-0">
              {!p.is_default && (
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  onClick={() => handleSetDefault(p.id)}
                  title="设为默认"
                >
                  <StarOff className="size-3.5" />
                </Button>
              )}
              <Button
                type="button"
                size="sm"
                variant="ghost"
                onClick={() => startEdit(p)}
              >
                编辑
              </Button>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                className="text-destructive"
                onClick={() => handleDelete(p.id)}
              >
                <Trash2 className="size-3.5" />
              </Button>
            </div>
          </div>
        ))}

        {/* 编辑表单 */}
        {editing && (
          <div className="rounded border border-primary/30 bg-primary/5 p-3 space-y-2">
            <p className="text-xs font-medium text-foreground">
              {editing.id ? "编辑律师档案" : "新增律师档案"}
            </p>
            <div className="grid grid-cols-2 gap-2">
              <FieldInput label="姓名 *" value={editing.name} onChange={(v) => setEditing({ ...editing, name: v })} />
              <FieldInput label="执业证号" value={editing.bar_number} onChange={(v) => setEditing({ ...editing, bar_number: v })} />
              <FieldInput label="律所" value={editing.law_firm} onChange={(v) => setEditing({ ...editing, law_firm: v })} />
              <FieldInput label="身份证号" value={editing.id_number} onChange={(v) => setEditing({ ...editing, id_number: v })} />
              <FieldInput label="电话" value={editing.phone} onChange={(v) => setEditing({ ...editing, phone: v })} />
              <FieldInput label="律所地址" value={editing.address} onChange={(v) => setEditing({ ...editing, address: v })} />
            </div>
            <div className="flex gap-2">
              <Button type="button" size="sm" onClick={handleSave} disabled={loading || !editing.name.trim()}>
                {loading ? "保存中…" : "保存"}
              </Button>
              <Button type="button" size="sm" variant="outline" onClick={() => setEditing(null)}>
                取消
              </Button>
            </div>
          </div>
        )}

        {/* 新增按钮 */}
        {!editing && (
          <button
            type="button"
            onClick={startAdd}
            className="inline-flex items-center gap-1.5 rounded-md border border-dashed border-border px-3 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:border-foreground/40 hover:text-foreground"
          >
            <Plus className="size-3.5" />
            添加律师
          </button>
        )}
      </div>
    </section>
  );
}

function FieldInput({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <div>
      <label className="text-xs text-muted-foreground">{label}</label>
      <input
        className="w-full rounded border border-input bg-background px-2 py-1 text-sm"
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
    </div>
  );
}

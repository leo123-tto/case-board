import { useEffect, useState } from "react";
import { Archive, Check, MessageSquarePlus, Pencil, X } from "lucide-react";

export interface ConversationSwitcherItem {
  id: string;
  title: string;
}

export function ConversationSwitcher({
  conversations,
  currentId,
  onSelect,
  onCreate,
  onRename,
  onArchive,
  disabled = false,
  disableSelection = false,
}: {
  conversations: ConversationSwitcherItem[];
  currentId: string | null;
  onSelect: (conversationId: string) => void;
  onCreate: () => void;
  onRename: (conversationId: string, title: string) => void;
  onArchive: (conversationId: string) => void;
  disabled?: boolean;
  disableSelection?: boolean;
}) {
  const current = conversations.find((item) => item.id === currentId) ?? null;
  const [renaming, setRenaming] = useState(false);
  const [title, setTitle] = useState(current?.title ?? "");

  useEffect(() => {
    setTitle(current?.title ?? "");
    setRenaming(false);
  }, [current?.id, current?.title]);

  const save = () => {
    const trimmed = title.trim();
    if (!current || !trimmed) return;
    onRename(current.id, trimmed);
    setRenaming(false);
  };

  return (
    <div className="border-b border-border bg-card px-2 py-2">
      {renaming && current ? (
        <div className="flex items-center gap-1">
          <input
            aria-label="对话名称"
            value={title}
            maxLength={80}
            autoFocus
            onChange={(event) => setTitle(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") save();
              if (event.key === "Escape") setRenaming(false);
            }}
            className="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1 text-xs outline-none focus:border-brand/50"
          />
          <button type="button" aria-label="保存对话名称" title="保存对话名称" onClick={save} className="rounded p-1 text-brand hover:bg-muted"><Check className="size-3.5" /></button>
          <button type="button" aria-label="取消重命名" title="取消重命名" onClick={() => setRenaming(false)} className="rounded p-1 text-muted-foreground hover:bg-muted"><X className="size-3.5" /></button>
        </div>
      ) : (
        <div className="flex items-center gap-1">
          <select
            aria-label="对话选择"
            value={currentId ?? ""}
            disabled={disableSelection || conversations.length === 0}
            onChange={(event) => onSelect(event.target.value)}
            className="min-w-0 flex-1 truncate rounded-md border border-border bg-background px-2 py-1.5 text-xs outline-none focus:border-brand/50 disabled:opacity-50"
          >
            {conversations.map((conversation) => (
              <option key={conversation.id} value={conversation.id}>{conversation.title}</option>
            ))}
          </select>
          <button type="button" aria-label="新建对话" title="新建对话" disabled={disabled} onClick={onCreate} className="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-50"><MessageSquarePlus className="size-3.5" /></button>
          <button type="button" aria-label="重命名当前对话" title="重命名当前对话" disabled={disabled || !current} onClick={() => setRenaming(true)} className="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-50"><Pencil className="size-3.5" /></button>
          <button type="button" aria-label="归档当前对话" title="归档当前对话" disabled={disabled || !current} onClick={() => current && onArchive(current.id)} className="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-50"><Archive className="size-3.5" /></button>
        </div>
      )}
    </div>
  );
}

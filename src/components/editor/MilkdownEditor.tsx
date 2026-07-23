/**
 * MilkdownEditor —— WYSIWYG-on-Markdown 编辑器的「干净边界」。
 *
 * 设计契约(advisor 定的可逆边界):对外只暴露 `value`(初始 MD)+ `onChange`(改动回调,
 * 吐出最新 MD)。App 其余代码**不碰 Milkdown 内部**。若以后嫌裸编辑器糙要换 Crepe,
 * 只改这一个文件,外部零感知。
 *
 * 技术:Milkdown 7.21.1 组合式(@milkdown/kit)。commonmark + gfm(表格)+ history(撤销/
 * 重做 Cmd+Z)+ listener(序列化回 MD)。底层存储仍是 Markdown,不改格式 —— 满足
 * docx_filing 导出管线 + 老板 WYSIWYG 铁律(见 memory caseboard-wysiwyg-over-markdown)。
 *
 * 载入新文档:由父组件给 `<MilkdownEditor key={docId} .../>` 换 key 强制 remount
 *(defaultValueCtx 只在初始化生效,换 key 最省事可靠)。
 */
import { useRef } from "react";
import {
  Editor,
  rootCtx,
  defaultValueCtx,
  editorViewCtx,
} from "@milkdown/kit/core";
import {
  commonmark,
  toggleStrongCommand,
  toggleEmphasisCommand,
  wrapInHeadingCommand,
  wrapInBulletListCommand,
  wrapInOrderedListCommand,
} from "@milkdown/kit/preset/commonmark";
import { gfm, insertTableCommand } from "@milkdown/kit/preset/gfm";
import { history } from "@milkdown/kit/plugin/history";
import { listener, listenerCtx } from "@milkdown/kit/plugin/listener";
import { callCommand } from "@milkdown/kit/utils";
import {
  Milkdown,
  MilkdownProvider,
  useEditor,
  useInstance,
} from "@milkdown/react";
import { nord } from "@milkdown/theme-nord";
import {
  Bold,
  Heading1,
  Heading2,
  Heading3,
  Italic,
  List,
  ListOrdered,
  Table as TableIcon,
} from "lucide-react";

import { cn } from "@/lib/utils";

import "@milkdown/theme-nord/style.css";
import "./editor.css";

interface Props {
  /** 初始 Markdown 正文(不含 filing 注释头,由调用方剥好) */
  value: string;
  /** 编辑产生新 MD 时回调(markdownUpdated;父组件据此存最新 MD + 算 dirty) */
  onChange: (markdown: string) => void;
  /** 可选选区回调，工作区可据此把选中段落交给 AI；案件旧调用不受影响。 */
  onSelectionChange?: (selection: EditorSelection | null) => void;
}

export interface EditorSelection {
  from: number;
  to: number;
  text: string;
}

/** 真正挂 Milkdown 实例的内层(必须在 MilkdownProvider 内) */
function Inner({ value, onChange, onSelectionChange }: Props) {
  // 用 ref 持有 onChange,避免它变化导致 useEditor 重建编辑器(会丢光标/历史)
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const selectionRef = useRef(onSelectionChange);
  selectionRef.current = onSelectionChange;
  const [instanceLoading, getInstance] = useInstance();

  useEditor((root) =>
    Editor.make()
      .config(nord)
      .config((ctx) => {
        ctx.set(rootCtx, root);
        ctx.set(defaultValueCtx, value);
        ctx.get(listenerCtx).markdownUpdated((_, markdown) => {
          onChangeRef.current(markdown);
        });
      })
      .use(commonmark)
      .use(gfm)
      .use(history)
      .use(listener),
  );

  const reportSelection = () => {
    if (instanceLoading || !selectionRef.current) return;
    getInstance()?.action((ctx) => {
      const { state } = ctx.get(editorViewCtx);
      const { from, to } = state.selection;
      const text = state.doc.textBetween(from, to, "\n");
      selectionRef.current?.(from === to ? null : { from, to, text });
    });
  };

  return (
    <div onMouseUp={reportSelection} onKeyUp={reportSelection}>
      <Milkdown />
    </div>
  );
}

/** 工具条按钮 */
function TbBtn({
  title,
  onClick,
  children,
}: {
  title: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      // mouseDown preventDefault:点工具条不让编辑器失焦,保住选区
      onMouseDown={(e) => e.preventDefault()}
      onClick={onClick}
      className="rounded p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
    >
      {children}
    </button>
  );
}

/** 工具条(必须在 MilkdownProvider 内,用 useInstance 调 ProseMirror 命令) */
function Toolbar() {
  const [loading, getInstance] = useInstance();

  // 跑一条 Milkdown 命令,跑完把焦点还给编辑器
  const run = (action: Parameters<Editor["action"]>[0]) => {
    if (loading) return;
    const editor = getInstance();
    if (!editor) return;
    editor.action(action);
    editor.action((ctx) => {
      ctx.get(editorViewCtx).focus();
    });
  };

  return (
    <div className="flex shrink-0 flex-wrap items-center gap-0.5 border-b border-border bg-card/60 px-3 py-1.5">
      <TbBtn
        title="一级标题(居中大标题 → 一、)"
        onClick={() => run(callCommand(wrapInHeadingCommand.key, 1))}
      >
        <Heading1 className="size-4" />
      </TbBtn>
      <TbBtn
        title="二级标题"
        onClick={() => run(callCommand(wrapInHeadingCommand.key, 2))}
      >
        <Heading2 className="size-4" />
      </TbBtn>
      <TbBtn
        title="三级标题(→ （一）)"
        onClick={() => run(callCommand(wrapInHeadingCommand.key, 3))}
      >
        <Heading3 className="size-4" />
      </TbBtn>
      <span className="mx-1 h-4 w-px bg-border" />
      <TbBtn
        title="加粗(强调正文)"
        onClick={() => run(callCommand(toggleStrongCommand.key))}
      >
        <Bold className="size-4" />
      </TbBtn>
      <TbBtn
        title="斜体"
        onClick={() => run(callCommand(toggleEmphasisCommand.key))}
      >
        <Italic className="size-4" />
      </TbBtn>
      <span className="mx-1 h-4 w-px bg-border" />
      <TbBtn
        title="无序列表"
        onClick={() => run(callCommand(wrapInBulletListCommand.key))}
      >
        <List className="size-4" />
      </TbBtn>
      <TbBtn
        title="有序列表(编号)"
        onClick={() => run(callCommand(wrapInOrderedListCommand.key))}
      >
        <ListOrdered className="size-4" />
      </TbBtn>
      <span className="mx-1 h-4 w-px bg-border" />
      <TbBtn
        title="插入表格(证据目录用)"
        onClick={() => run(callCommand(insertTableCommand.key))}
      >
        <TableIcon className="size-4" />
      </TbBtn>
    </div>
  );
}

/**
 * 对外组件。父组件用 `key={docId}` 控制载入哪份文档。
 */
export function MilkdownEditor({
  value,
  onChange,
  onSelectionChange,
  className,
}: Props & { className?: string }) {
  return (
    <MilkdownProvider>
      <div className={cn("milkdown-editor", className)}>
        <Toolbar />
        <div className="min-h-0 flex-1 overflow-auto">
          <Inner
            value={value}
            onChange={onChange}
            onSelectionChange={onSelectionChange}
          />
        </div>
      </div>
    </MilkdownProvider>
  );
}

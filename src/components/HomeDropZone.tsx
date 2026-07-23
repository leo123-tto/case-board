/**
 * 首页拖拽导入区(2026-06-03)。
 *
 * 把案件文件夹直接拖进首页就开始导入 —— 等同点「导入案件」按钮选目录,
 * 只是省掉系统选择器那一步。
 *
 * 实现要点:
 * - Tauri v2 的 `onDragDropEvent` 是**窗口级**事件(整个 webview 都收得到),
 *   靠「本组件只在诉讼首页挂载、离开即卸载」把作用域收在首页 —— 卸载时 unlisten。
 * - 拖入时盖一层半透明蓝色遮罩提示「松开导入」(pointer-events-none 不挡子内容)。
 * - drop 取 `paths[0]`(拖文件夹 macOS 给单条路径);key 校验/失败提示由上层 onImportPath 负责。
 */
import { useEffect, useState, type ReactNode } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { FolderOpen } from "lucide-react";

export function HomeDropZone({
  onImportPath,
  children,
}: {
  onImportPath: (path: string) => void;
  children: ReactNode;
}) {
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const p = event.payload;
        if (p.type === "enter" || p.type === "over") {
          setDragging(true);
        } else if (p.type === "drop") {
          setDragging(false);
          const path = p.paths[0];
          if (path) onImportPath(path);
        } else {
          // "leave"
          setDragging(false);
        }
      })
      .then((fn) => {
        if (disposed) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((e) => console.warn("listen drag-drop failed", e));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [onImportPath]);

  return (
    <div className="relative h-full w-full">
      {children}
      {dragging && (
        <div className="pointer-events-none absolute inset-0 z-50 flex flex-col items-center justify-center gap-3 border-2 border-dashed border-sky-400 bg-sky-50/85 backdrop-blur-sm animate-in fade-in-0 duration-150">
          <FolderOpen className="size-12 text-sky-500" />
          <p className="text-lg font-semibold text-sky-700">
            松开即可导入这个案件文件夹
          </p>
          <p className="text-sm text-sky-600">
            拖入案件所在的文件夹,自动扫描并抽取材料
          </p>
        </div>
      )}
    </div>
  );
}

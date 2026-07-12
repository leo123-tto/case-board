import { FileText, Loader2, X } from "lucide-react";

/* ------------------------------------------------------------------ */
/* 状态视图                                                            */
/* ------------------------------------------------------------------ */

export function LoadingState() {
  return (
    <div className="flex flex-col items-center justify-center py-20 text-center">
      <Loader2 className="size-6 animate-spin text-muted-foreground" />
      <p className="mt-3 text-sm text-muted-foreground">加载中…</p>
    </div>
  );
}

export function ErrorState({
  message,
  onDismiss,
}: {
  message: string;
  onDismiss?: () => void;
}) {
  return (
    <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-4">
      <div className="flex items-start justify-between gap-3">
        <p className="text-sm font-medium text-destructive">出错了</p>
        {onDismiss && (
          <button
            type="button"
            onClick={onDismiss}
            className="rounded p-1 text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
            aria-label="关闭错误提示"
          >
            <X className="size-4" />
          </button>
        )}
      </div>
      <p className="mt-1 font-mono text-xs text-muted-foreground">{message}</p>
    </div>
  );
}

export function NoDocsHint() {
  return (
    <div className="flex flex-col items-center justify-center py-20 text-center">
      <FileText className="size-6 text-muted-foreground" />
      <p className="mt-3 text-sm text-muted-foreground">
        这个文件夹里没扫到任何文档
      </p>
      <p className="mt-1 text-xs text-muted-foreground/70">
        检查一下选对路径了吗?或者文件都被 _archive 排除了?
      </p>
    </div>
  );
}

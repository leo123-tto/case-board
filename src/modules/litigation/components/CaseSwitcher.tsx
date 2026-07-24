import { useEffect, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";

import { type Case } from "@/lib/types";
import { shortenPath } from "@/lib/format";
import { cn } from "@/lib/utils";

export function CaseSwitcher({
  cases,
  selectedId,
  onSwitch,
}: {
  cases: Case[];
  selectedId: string | null;
  onSwitch: (id: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const selected = cases.find((c) => c.id === selectedId);

  useEffect(() => {
    if (!open) return;
    const closeOutside = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", closeOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [open]);

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="inline-flex items-baseline gap-1.5 text-xl font-semibold tracking-tight text-foreground transition-opacity hover:opacity-70"
      >
        <span>{selected?.name ?? "—"}</span>
        <ChevronDown className="size-4 self-center text-muted-foreground" />
      </button>
      {open && (
        <ul className="absolute left-0 top-full z-20 mt-1 min-w-64 overflow-hidden rounded-md border border-border bg-popover shadow-lg animate-in zoom-in-95 fade-in-0 duration-150 ease-out">
          {cases.map((c) => (
            <li key={c.id}>
              <button
                type="button"
                onClick={() => {
                  onSwitch(c.id);
                  setOpen(false);
                }}
                className={cn(
                  "block w-full px-3 py-2 text-left text-sm transition-colors hover:bg-accent",
                  c.id === selectedId && "bg-accent/50",
                )}
              >
                <div className="font-medium text-foreground">{c.name}</div>
                <div className="mt-0.5 truncate font-mono text-caption text-muted-foreground">
                  {shortenPath(c.source_folder, 2)}
                </div>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

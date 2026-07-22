import { forwardRef, type ComponentPropsWithoutRef } from "react";

import { cn } from "@/lib/utils";

export const HomeFeatureCard = forwardRef<
  HTMLElement,
  ComponentPropsWithoutRef<"section"> & { tone?: "default" | "brand" }
>(function HomeFeatureCard({ tone = "default", className, ...props }, ref) {
  return (
    <section
      ref={ref}
      data-home-feature-card="true"
      className={cn(
        "flex h-80 min-h-0 flex-col overflow-hidden p-5",
        tone === "brand"
          ? "relative rounded-xl border border-brand/15 bg-brand-soft/55 shadow-[inset_0_1px_0_oklch(1_0_0/0.72)]"
          : "surface-card",
        className,
      )}
      {...props}
    />
  );
});

HomeFeatureCard.displayName = "HomeFeatureCard";

export const HomeFeatureCardScrollArea = forwardRef<
  HTMLDivElement,
  ComponentPropsWithoutRef<"div">
>(function HomeFeatureCardScrollArea({ className, ...props }, ref) {
  return (
    <div
      ref={ref}
      data-home-feature-card-scroll="true"
      className={cn("min-h-0 flex-1 overflow-y-auto overscroll-contain", className)}
      {...props}
    />
  );
});

HomeFeatureCardScrollArea.displayName = "HomeFeatureCardScrollArea";

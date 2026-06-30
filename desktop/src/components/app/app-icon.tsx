import { cn } from "@/lib/utils";

export function AppIcon({ className }: { className?: string }) {
  return (
    <img
      alt=""
      className={cn("shrink-0 outline outline-1 outline-black/10 dark:outline-white/10", className)}
      draggable={false}
      src="/app-icon.png"
    />
  );
}

import { CircleUserRound, Minus, Square, X } from "lucide-react";

import { runWindowAction } from "@/components/app/window-actions";
import { Button } from "@/components/ui/button";
import { SidebarTrigger } from "@/components/ui/sidebar";
import type { RailAction } from "@/lib/app-types";

export function AppChrome({ onAction }: { onAction: (action: RailAction) => void }) {
  return (
    <div
      className="flex h-10 shrink-0 select-none items-center bg-background text-foreground"
      data-tauri-drag-region
    >
      <div className="flex items-center gap-2 px-4">
        <SidebarTrigger aria-label="Toggle sidebar" className="bg-secondary" size="icon-xs" />
        <Button aria-label="Account" onClick={() => onAction("details")} size="icon-xs" type="button" variant="ghost">
          <CircleUserRound data-icon="inline-start" />
        </Button>
      </div>
      <div className="min-w-4 flex-1" data-tauri-drag-region />
      <div className="flex h-full">
        <Button
          aria-label="Minimize"
          className="h-full w-11 rounded-none text-muted-foreground hover:bg-secondary hover:text-foreground active:scale-100"
          onClick={() => void runWindowAction("minimize")}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Minus data-icon="inline-start" />
        </Button>
        <Button
          aria-label="Maximize"
          className="h-full w-11 rounded-none text-muted-foreground hover:bg-secondary hover:text-foreground active:scale-100"
          onClick={() => void runWindowAction("toggleMaximize")}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Square data-icon="inline-start" />
        </Button>
        <Button
          aria-label="Close"
          className="h-full w-11 rounded-none text-muted-foreground hover:bg-destructive hover:text-white active:scale-100"
          onClick={() => void runWindowAction("close")}
          size="icon"
          type="button"
          variant="ghost"
        >
          <X data-icon="inline-start" />
        </Button>
      </div>
    </div>
  );
}

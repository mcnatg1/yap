import { type ElementType } from "react";
import {
  FileText,
  Grid2X2,
  HelpCircle,
  Mic,
  Settings2,
  Sparkles,
} from "lucide-react";

import { AppIcon } from "@/components/app/app-icon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import type { RailAction } from "@/lib/app-types";
import { cn } from "@/lib/utils";

export function ProductRail({
  active,
  collapsed,
  onAction,
}: {
  active: RailAction;
  collapsed: boolean;
  onAction: (action: RailAction) => void;
}) {
  return (
    <aside className={cn("flex h-full min-h-0 min-w-0 flex-col bg-background px-[15px] pb-3 pt-4", collapsed && "items-center")}>
      <div className={cn("mb-5 flex items-center gap-2 px-1", collapsed && "justify-center px-0")}>
        <AppIcon className="size-6 rounded-md" />
        {collapsed ? null : (
          <>
            <div className="text-xl font-semibold tracking-tight">Yap</div>
            <Badge className="h-6 bg-accent-soft px-2 text-xs text-accent-ink hover:bg-accent-soft" variant="secondary">Local</Badge>
          </>
        )}
      </div>

      <nav className="flex w-full flex-col gap-1">
        <RailItem active={active === "home"} collapsed={collapsed} icon={Grid2X2} label="Home" onClick={() => onAction("home")} />
        <RailItem
          active={active === "transcribe"}
          collapsed={collapsed}
          icon={Mic}
          label="Transcribe"
          onClick={() => onAction("transcribe")}
        />
        <RailItem
          active={active === "transcripts"}
          collapsed={collapsed}
          icon={FileText}
          label="Transcripts"
          onClick={() => onAction("transcripts")}
        />
        <RailItem active={active === "polish"} collapsed={collapsed} icon={Sparkles} label="Polish" onClick={() => onAction("polish")} />
      </nav>

      <div className="mt-auto flex w-full flex-col gap-1 pt-4">
        <Separator className="mb-3" />
        <RailItem active={active === "details"} collapsed={collapsed} icon={Settings2} label="Settings" onClick={() => onAction("details")} />
        <RailItem active={active === "help"} collapsed={collapsed} icon={HelpCircle} label="Help" onClick={() => onAction("help")} />
      </div>
    </aside>
  );
}

function RailItem({
  active,
  collapsed,
  icon: Icon,
  label,
  onClick,
}: {
  active?: boolean;
  collapsed?: boolean;
  icon: ElementType;
  label: string;
  onClick: () => void;
}) {
  return (
    <Button
      aria-current={active ? "page" : undefined}
      className={cn(
        "h-auto w-full justify-start rounded-lg px-2.5 py-2 text-left text-sm font-semibold text-foreground hover:bg-[var(--rail-hover)]",
        active && "bg-secondary text-foreground",
        collapsed && "justify-center px-2",
      )}
      onClick={onClick}
      title={label}
      type="button"
      variant="ghost"
    >
      <Icon data-icon="inline-start" />
      <span className={cn("truncate", collapsed && "sr-only")}>{label}</span>
    </Button>
  );
}

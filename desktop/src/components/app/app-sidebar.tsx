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
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarSeparator,
} from "@/components/ui/sidebar";
import type { RailAction } from "@/lib/app-types";

const mainNav: { action: RailAction; icon: ElementType; label: string }[] = [
  { action: "home", icon: Grid2X2, label: "Home" },
  { action: "transcribe", icon: Mic, label: "Transcribe" },
  { action: "transcripts", icon: FileText, label: "Transcripts" },
  { action: "polish", icon: Sparkles, label: "Polish" },
];

const footerNav: { action: RailAction; icon: ElementType; label: string }[] = [
  { action: "details", icon: Settings2, label: "Settings" },
  { action: "help", icon: HelpCircle, label: "Help" },
];

export function AppSidebar({
  active,
  onAction,
}: {
  active: RailAction;
  onAction: (action: RailAction) => void;
}) {
  return (
    <Sidebar collapsible="icon">
      <SidebarHeader className="px-3 pb-2 pt-4">
        <div className="flex items-center gap-2 overflow-hidden px-1 group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:px-0">
          <AppIcon className="size-6 shrink-0 rounded-md" />
          <div className="min-w-0 group-data-[collapsible=icon]:hidden">
            <div className="flex items-center gap-2">
              <span className="truncate text-xl font-semibold tracking-tight">Yap</span>
              <Badge
                className="h-6 shrink-0 bg-accent-soft px-2 text-xs text-accent-ink hover:bg-accent-soft"
                variant="secondary"
              >
                Local
              </Badge>
            </div>
          </div>
        </div>
      </SidebarHeader>

      <SidebarContent className="px-2">
        <SidebarMenu>
          {mainNav.map(({ action, icon: Icon, label }) => (
            <SidebarMenuItem key={action}>
              <SidebarMenuButton
                isActive={active === action}
                onClick={() => onAction(action)}
                tooltip={label}
                type="button"
              >
                <Icon data-icon="inline-start" />
                <span>{label}</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          ))}
        </SidebarMenu>
      </SidebarContent>

      <SidebarFooter className="px-2 pb-3">
        <SidebarSeparator className="mb-2" />
        <SidebarMenu>
          {footerNav.map(({ action, icon: Icon, label }) => (
            <SidebarMenuItem key={action}>
              <SidebarMenuButton
                isActive={active === action}
                onClick={() => onAction(action)}
                tooltip={label}
                type="button"
              >
                <Icon data-icon="inline-start" />
                <span>{label}</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          ))}
        </SidebarMenu>
      </SidebarFooter>
    </Sidebar>
  );
}

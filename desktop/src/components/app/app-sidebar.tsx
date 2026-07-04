import { useEffect, useRef, type ElementType } from "react";
import {
  CircleUserRound,
  Grid2X2,
  HelpCircle,
  Mic,
  Settings2,
  Sparkles,
} from "lucide-react";

import { AppIcon } from "@/components/app/app-icon";
import { Button } from "@/components/ui/button";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarSeparator,
  SidebarTrigger,
  useSidebar,
} from "@/components/ui/sidebar";
import type { RailAction } from "@/lib/app-types";

const mainNav: { action: RailAction; icon: ElementType; label: string }[] = [
  { action: "home", icon: Grid2X2, label: "Home" },
  { action: "transcribe", icon: Mic, label: "Transcribe" },
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
  const { state } = useSidebar();
  const brandIconRef = useRef<HTMLDivElement>(null);
  const wordmarkRef = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    let cancelled = false;
    let killTweens: (() => void) | undefined;

    void import("gsap").then(({ default: gsap }) => {
      const icon = brandIconRef.current;
      const wordmark = wordmarkRef.current;

      if (cancelled || !icon || !wordmark) {
        return;
      }

      const collapsed = state === "collapsed";
      killTweens = () => {
        gsap.killTweensOf(icon);
        gsap.killTweensOf(wordmark);
      };
      killTweens();

      gsap.to(icon, {
        duration: 0.14,
        ease: "power2.out",
        overwrite: "auto",
        scale: collapsed ? 0.96 : 1,
      });
      gsap.to(wordmark, {
        autoAlpha: collapsed ? 0 : 1,
        duration: 0.12,
        ease: "power2.out",
        overwrite: "auto",
        x: collapsed ? -6 : 0,
      });
    });

    return () => {
      cancelled = true;
      killTweens?.();
    };
  }, [state]);

  return (
    <Sidebar collapsible="icon">
      <SidebarHeader className="gap-0 px-3 pb-0 pt-4">
        <div className="flex flex-col">
          <div className="flex h-7 items-center gap-2 px-1 group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:px-0">
            <SidebarTrigger aria-label="Toggle sidebar" className="bg-secondary" size="icon-xs" />
            <Button
              aria-label="Account"
              className="text-muted-foreground group-data-[collapsible=icon]:hidden"
              onClick={() => onAction("details")}
              size="icon-xs"
              type="button"
              variant="ghost"
            >
              <CircleUserRound data-icon="inline-start" />
            </Button>
          </div>

          <div className="flex h-[3.75rem] items-center gap-2 overflow-hidden px-1">
            <div ref={brandIconRef} className="size-6 shrink-0 will-change-transform">
              <AppIcon className="size-6 rounded-md" />
            </div>
            <span
              ref={wordmarkRef}
              className="min-w-0 truncate text-xl font-semibold tracking-tight will-change-[opacity,transform]"
            >
              Yap
            </span>
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

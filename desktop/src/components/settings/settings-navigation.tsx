import { SealCheck as BadgeCheck } from "@phosphor-icons/react/SealCheck";
import { Microphone as Mic } from "@phosphor-icons/react/Microphone";
import { HardDrives as Server } from "@phosphor-icons/react/HardDrives";
import type { ComponentType } from "react";

import { cn } from "@/lib/utils";

export type SettingsSection = "general" | "system" | "about";

const settingsSections: {
  id: SettingsSection;
  icon: ComponentType<{ className?: string }>;
  label: string;
}[] = [
  { id: "general", icon: Mic, label: "General" },
  { id: "system", icon: Server, label: "System" },
  { id: "about", icon: BadgeCheck, label: "About" },
];

export function settingsSectionTitle(section: SettingsSection) {
  if (section === "general") return "General";
  if (section === "system") return "System";
  return "About";
}

export function SettingsNavigation({
  onSelect,
  section,
}: {
  onSelect: (section: SettingsSection) => void;
  section: SettingsSection;
}) {
  return (
    <aside className="flex min-h-0 flex-col border-r bg-muted/45 p-5">
      <div className="mb-4 text-xs font-medium uppercase tracking-[0.14em] text-muted-foreground">
        Settings
      </div>
      <nav className="grid gap-1">
        {settingsSections.map((item) => {
          const Icon = item.icon;
          return (
            <button
              className={cn(
                "flex h-11 items-center gap-3 rounded-lg px-3 text-left text-sm font-medium transition-[background-color,color,scale] duration-150 ease-out active:scale-[0.96]",
                section === item.id
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:bg-background/60 hover:text-foreground",
              )}
              key={item.id}
              onClick={() => onSelect(item.id)}
              type="button"
            >
              <Icon className="size-5 shrink-0" />
              {item.label}
            </button>
          );
        })}
      </nav>
      <div className="mt-auto text-xs text-muted-foreground">Yap</div>
    </aside>
  );
}

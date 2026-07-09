import React from "react";
import ReactDOM from "react-dom/client";
import type { ComponentType } from "react";

import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";

const isLiveOverlay = new URLSearchParams(window.location.search).get("window") === "live-overlay";
document.documentElement.dataset.window = isLiveOverlay ? "live-overlay" : "main";

async function bootstrap() {
  if (import.meta.env.VITE_WDIO === "1") {
    await import("@wdio/tauri-plugin");
  }

  const Root = await loadRootComponent();

  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <TooltipProvider>
        <Root />
        <Toaster />
      </TooltipProvider>
    </React.StrictMode>,
  );
}

async function loadRootComponent(): Promise<ComponentType> {
  if (isLiveOverlay) {
    const module = await import("@/components/live/live-overlay-host");
    return module.LiveOverlayHost;
  }

  const module = await import("@/App");
  return module.default;
}

void bootstrap();

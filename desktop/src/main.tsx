import React from "react";
import ReactDOM from "react-dom/client";

import App from "@/App";
import { LiveOverlayHost } from "@/components/live/live-overlay-host";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";

const isLiveOverlay = new URLSearchParams(window.location.search).get("window") === "live-overlay";
document.documentElement.dataset.window = isLiveOverlay ? "live-overlay" : "main";
const Root = isLiveOverlay ? LiveOverlayHost : App;

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <TooltipProvider>
      <Root />
      <Toaster />
    </TooltipProvider>
  </React.StrictMode>,
);

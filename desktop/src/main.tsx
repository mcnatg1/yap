import React from "react";
import ReactDOM from "react-dom/client";

import App from "@/App";
import { LiveOverlayHost } from "@/components/live/live-overlay-host";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";

const Root = new URLSearchParams(window.location.search).get("window") === "live-overlay" ? LiveOverlayHost : App;

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <TooltipProvider>
      <Root />
      <Toaster />
    </TooltipProvider>
  </React.StrictMode>,
);

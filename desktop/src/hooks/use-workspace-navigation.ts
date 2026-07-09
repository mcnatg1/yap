import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";

import { isWorkspaceView, type RailAction, type WorkspaceView } from "@/lib/app-types";

export function useWorkspaceNavigation({
  onOpenDetails,
  onOpenPolish,
}: {
  onOpenDetails: () => void;
  onOpenPolish: () => void;
}) {
  const [activeRail, setActiveRail] = useState<RailAction>("home");
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [railCollapsed, setRailCollapsed] = useState(false);
  const [workspaceView, setWorkspaceView] = useState<WorkspaceView>("home");
  const onOpenDetailsRef = useRef(onOpenDetails);
  const onOpenPolishRef = useRef(onOpenPolish);

  useEffect(() => {
    onOpenDetailsRef.current = onOpenDetails;
    onOpenPolishRef.current = onOpenPolish;
  }, [onOpenDetails, onOpenPolish]);

  const openWorkspace = useCallback((action: RailAction) => {
    setActiveRail(action);

    if (action === "details") {
      setDetailsOpen(true);
      onOpenDetailsRef.current();
      return;
    }
    if (action === "help") {
      setHelpOpen(true);
      return;
    }

    setWorkspaceView(action);
    if (action === "polish") onOpenPolishRef.current();
  }, []);

  const showDetails = useCallback(() => {
    setActiveRail("details");
    setDetailsOpen(true);
  }, []);

  useEffect(() => {
    if (!isTauri()) return;

    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void listen<unknown>("open-workspace", (event) => {
      if (isWorkspaceView(event.payload)) openWorkspace(event.payload);
    }).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlisten = stop;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [openWorkspace]);

  const closeDetails = useCallback(() => {
    setDetailsOpen(false);
    if (activeRail === "details") setActiveRail(workspaceView);
  }, [activeRail, workspaceView]);

  const closeHelp = useCallback(() => {
    setHelpOpen(false);
    if (activeRail === "help") setActiveRail(workspaceView);
  }, [activeRail, workspaceView]);

  const onDetailsOpenChange = useCallback((open: boolean) => {
    if (open) {
      setDetailsOpen(true);
      return;
    }
    closeDetails();
  }, [closeDetails]);

  const onHelpOpenChange = useCallback((open: boolean) => {
    if (open) {
      setHelpOpen(true);
      return;
    }
    closeHelp();
  }, [closeHelp]);

  return {
    activeRail,
    closeDetails,
    detailsOpen,
    helpOpen,
    onDetailsOpenChange,
    onHelpOpenChange,
    openWorkspace,
    railCollapsed,
    setRailCollapsed,
    showDetails,
    workspaceView,
  };
}

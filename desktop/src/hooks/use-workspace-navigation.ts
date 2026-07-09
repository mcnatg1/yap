import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";

import { isWorkspaceView, type RailAction, type WorkspaceView } from "@/lib/app-types";

export type WorkspaceNavigationState = {
  activeRail: RailAction;
  detailsOpen: boolean;
  helpOpen: boolean;
  railCollapsed: boolean;
  workspaceView: WorkspaceView;
};

export type WorkspaceNavigationAction =
  | { type: "closeDetails" }
  | { type: "closeHelp" }
  | { type: "openWorkspace"; action: RailAction }
  | { type: "setDetailsOpen"; open: boolean }
  | { type: "setHelpOpen"; open: boolean }
  | { type: "setRailCollapsed"; collapsed: boolean }
  | { type: "showDetails" };

export const initialWorkspaceNavigationState: WorkspaceNavigationState = {
  activeRail: "home",
  detailsOpen: false,
  helpOpen: false,
  railCollapsed: false,
  workspaceView: "home",
};

export function workspaceNavigationStateForAction(
  state: WorkspaceNavigationState,
  action: WorkspaceNavigationAction,
): WorkspaceNavigationState {
  switch (action.type) {
    case "closeDetails":
      return {
        ...state,
        activeRail: state.activeRail === "details" ? state.workspaceView : state.activeRail,
        detailsOpen: false,
      };
    case "closeHelp":
      return {
        ...state,
        activeRail: state.activeRail === "help" ? state.workspaceView : state.activeRail,
        helpOpen: false,
      };
    case "openWorkspace":
      if (action.action === "details") return { ...state, activeRail: "details", detailsOpen: true };
      if (action.action === "help") return { ...state, activeRail: "help", helpOpen: true };
      return { ...state, activeRail: action.action, workspaceView: action.action };
    case "setDetailsOpen":
      return action.open ? { ...state, detailsOpen: true } : workspaceNavigationStateForAction(state, { type: "closeDetails" });
    case "setHelpOpen":
      return action.open ? { ...state, helpOpen: true } : workspaceNavigationStateForAction(state, { type: "closeHelp" });
    case "setRailCollapsed":
      return { ...state, railCollapsed: action.collapsed };
    case "showDetails":
      return { ...state, activeRail: "details", detailsOpen: true };
  }
}

export function useWorkspaceNavigation({
  onOpenDetails,
  onOpenPolish,
}: {
  onOpenDetails: () => void;
  onOpenPolish: () => void;
}) {
  const [navigation, setNavigation] = useState(initialWorkspaceNavigationState);
  const onOpenDetailsRef = useRef(onOpenDetails);
  const onOpenPolishRef = useRef(onOpenPolish);

  useEffect(() => {
    onOpenDetailsRef.current = onOpenDetails;
    onOpenPolishRef.current = onOpenPolish;
  }, [onOpenDetails, onOpenPolish]);

  const openWorkspace = useCallback((action: RailAction) => {
    setNavigation((state) => workspaceNavigationStateForAction(state, { type: "openWorkspace", action }));
    if (action === "details") onOpenDetailsRef.current();
    if (action === "polish") onOpenPolishRef.current();
  }, []);

  const showDetails = useCallback(() => {
    setNavigation((state) => workspaceNavigationStateForAction(state, { type: "showDetails" }));
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
    setNavigation((state) => workspaceNavigationStateForAction(state, { type: "closeDetails" }));
  }, []);

  const onDetailsOpenChange = useCallback((open: boolean) => {
    setNavigation((state) => workspaceNavigationStateForAction(state, { type: "setDetailsOpen", open }));
  }, []);

  const onHelpOpenChange = useCallback((open: boolean) => {
    setNavigation((state) => workspaceNavigationStateForAction(state, { type: "setHelpOpen", open }));
  }, []);

  const setRailCollapsed = useCallback((collapsed: boolean) => {
    setNavigation((state) => workspaceNavigationStateForAction(state, { type: "setRailCollapsed", collapsed }));
  }, []);

  return {
    activeRail: navigation.activeRail,
    closeDetails,
    detailsOpen: navigation.detailsOpen,
    helpOpen: navigation.helpOpen,
    onDetailsOpenChange,
    onHelpOpenChange,
    openWorkspace,
    railCollapsed: navigation.railCollapsed,
    setRailCollapsed,
    showDetails,
    workspaceView: navigation.workspaceView,
  };
}

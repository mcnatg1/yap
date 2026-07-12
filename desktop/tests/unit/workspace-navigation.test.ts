import { describe, expect, it } from "vitest";

import {
  initialWorkspaceNavigationState,
  workspaceNavigationEffectForIntent,
  workspaceNavigationStateForAction,
  type WorkspaceNavigationState,
} from "@/hooks/use-workspace-navigation";

function state(overrides: Partial<WorkspaceNavigationState> = {}): WorkspaceNavigationState {
  return { ...initialWorkspaceNavigationState, ...overrides };
}

describe("workspace navigation", () => {
  it("selects callbacks only for rail-driven details and polish", () => {
    expect(workspaceNavigationEffectForIntent({ type: "openWorkspace", action: "details" }))
      .toBe("refreshDetails");
    expect(workspaceNavigationEffectForIntent({ type: "openWorkspace", action: "polish" }))
      .toBe("openPolish");
    expect(workspaceNavigationEffectForIntent({ type: "openWorkspace", action: "home" }))
      .toBeUndefined();
    expect(workspaceNavigationEffectForIntent({ type: "showDetails" })).toBeUndefined();
  });

  it("opens details without changing the workspace view", () => {
    expect(
      workspaceNavigationStateForAction(
        state({ activeRail: "home", workspaceView: "home" }),
        { type: "openWorkspace", action: "details" },
      ),
    ).toMatchObject({
      activeRail: "details",
      detailsOpen: true,
      workspaceView: "home",
    });
  });

  it("restores the active rail to the current workspace when details closes", () => {
    expect(
      workspaceNavigationStateForAction(
        state({ activeRail: "details", detailsOpen: true, workspaceView: "polish" }),
        { type: "closeDetails" },
      ),
    ).toMatchObject({
      activeRail: "polish",
      detailsOpen: false,
      workspaceView: "polish",
    });
  });

  it("opens help without changing the workspace view", () => {
    expect(
      workspaceNavigationStateForAction(
        state({ activeRail: "transcribe", workspaceView: "transcribe" }),
        { type: "openWorkspace", action: "help" },
      ),
    ).toMatchObject({
      activeRail: "help",
      helpOpen: true,
      workspaceView: "transcribe",
    });
  });

  it("opens polish as a workspace", () => {
    expect(
      workspaceNavigationStateForAction(
        state({ activeRail: "home", workspaceView: "home" }),
        { type: "openWorkspace", action: "polish" },
      ),
    ).toMatchObject({
      activeRail: "polish",
      workspaceView: "polish",
    });
  });

  it("show details opens setup without changing the workspace view", () => {
    expect(
      workspaceNavigationStateForAction(
        state({ activeRail: "home", workspaceView: "home" }),
        { type: "showDetails" },
      ),
    ).toMatchObject({
      activeRail: "details",
      detailsOpen: true,
      workspaceView: "home",
    });
  });
});

import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

import { projectAppModalState } from "@/components/panels/app-modal-state";
import {
  isHistoryBodySearchPending,
  projectHistorySearchDisplay,
} from "@/components/panels/history-panel";
import { isDevelopmentPolishAvailable } from "@/lib/product-features";
import { createPolishOperationOwner, isPolishDraftCurrent } from "@/polish";

function cspSources(csp: string, directive: string) {
  const value = csp
    .split(";")
    .map((entry) => entry.trim())
    .find((entry) => entry.startsWith(`${directive} `));
  return value?.slice(directive.length + 1).trim().split(/\s+/) ?? [];
}

describe("product surface contracts", () => {
  it("does not report an empty history while transcript bodies are indexing", () => {
    expect(projectHistorySearchDisplay({ hasResults: false, indexingBodies: true }))
      .toBe("indexing");
    expect(projectHistorySearchDisplay({ hasResults: true, indexingBodies: true }))
      .toBe("results");
    expect(projectHistorySearchDisplay({ hasResults: false, indexingBodies: false }))
      .toBe("empty");
  });

  it("derives body-search pending synchronously from query and cache", () => {
    expect(isHistoryBodySearchPending({
      cachedOutputPaths: new Set(["one.txt"]),
      hasPreviewLoader: true,
      outputPaths: ["one.txt", "two.txt"],
      query: "spoken phrase",
    })).toBe(true);
    expect(isHistoryBodySearchPending({
      cachedOutputPaths: new Set(["one.txt", "two.txt"]),
      hasPreviewLoader: true,
      outputPaths: ["one.txt", "two.txt"],
      query: "spoken phrase",
    })).toBe(false);
    expect(isHistoryBodySearchPending({
      cachedOutputPaths: new Set(),
      hasPreviewLoader: true,
      outputPaths: ["one.txt"],
      query: "x",
    })).toBe(false);
  });

  it("gives Settings and Help one mutually exclusive modal owner", () => {
    expect(projectAppModalState("settings")).toEqual({ detailsOpen: true, helpOpen: false });
    expect(projectAppModalState("help")).toEqual({ detailsOpen: false, helpOpen: true });
    expect(projectAppModalState(null)).toEqual({ detailsOpen: false, helpOpen: false });
  });

  it("keeps the direct Ollama Polish path behind an explicit development-only seam", () => {
    expect(isDevelopmentPolishAvailable({ explicitlyEnabled: true, isDevelopment: true }))
      .toBe(true);
    expect(isDevelopmentPolishAvailable({ explicitlyEnabled: true, isDevelopment: false }))
      .toBe(false);
    expect(isDevelopmentPolishAvailable({ explicitlyEnabled: false, isDevelopment: true }))
      .toBe(false);
  });

  it("withholds Copy and Save from running or stale Polish drafts", () => {
    expect(isPolishDraftCurrent({
      currentContext: "C:/one.txt\0light",
      draftContext: "C:/one.txt\0light",
      running: false,
      text: "Current draft",
    })).toBe(true);
    expect(isPolishDraftCurrent({
      currentContext: "C:/one.txt\0clean",
      draftContext: "C:/one.txt\0light",
      running: false,
      text: "Stale draft",
    })).toBe(false);
    expect(isPolishDraftCurrent({
      currentContext: "C:/one.txt\0light",
      draftContext: "C:/one.txt\0light",
      running: true,
      text: "Running draft",
    })).toBe(false);
  });

  it("keeps stale Polish saves from completing or finishing a newer draft", () => {
    const owner = createPolishOperationOwner();
    const firstRun = owner.startRun("C:/one.txt\0light");
    expect(firstRun).toBeDefined();
    const firstDraft = owner.acceptRun(firstRun!);
    expect(firstDraft).toBeDefined();
    expect(Object.isFrozen(firstDraft)).toBe(true);
    expect(owner.finishRun(firstRun!)).toBe(true);

    const staleSave = owner.startSave(firstDraft!);
    expect(staleSave).toBeDefined();
    expect(Object.isFrozen(staleSave)).toBe(true);
    expect(owner.startRun("C:/one.txt\0light")).toBeUndefined();

    owner.invalidate();
    const secondRun = owner.startRun("C:/one.txt\0clean")!;
    const secondDraft = owner.acceptRun(secondRun)!;
    expect(owner.finishRun(secondRun)).toBe(true);
    const currentSave = owner.startSave(secondDraft)!;

    expect(owner.acceptSave(staleSave!)).toBe(false);
    expect(owner.finishSave(staleSave!)).toBe(false);
    expect(owner.isSaving()).toBe(true);
    expect(owner.acceptSave(currentSave)).toBe(true);
    expect(owner.finishSave(currentSave)).toBe(true);
    expect(owner.isSaving()).toBe(false);
  });

  it("allows only the loopback media owner and removes the asset protocol", () => {
    const config = JSON.parse(readFileSync(
      new URL("../../src-tauri/tauri.conf.json", import.meta.url),
      "utf8",
    )) as {
      app: { security: { assetProtocol?: unknown; csp: string; devCsp?: string } };
      bundle: { resources: Record<string, string> };
    };
    const productionConnect = cspSources(config.app.security.csp, "connect-src");
    const developmentConnect = cspSources(config.app.security.devCsp ?? "", "connect-src");

    expect(productionConnect).toEqual([
      "'self'",
      "ipc:",
      "http://ipc.localhost",
      "https://ipc.localhost",
      "http://127.0.0.1:*",
    ]);
    expect(developmentConnect).toEqual(productionConnect);
    expect(cspSources(config.app.security.csp, "media-src")).toContain(
      "http://127.0.0.1:*",
    );
    expect(cspSources(config.app.security.csp, "form-action")).toEqual(["'none'"]);
    expect(cspSources(config.app.security.devCsp ?? "", "form-action")).toEqual(["'none'"]);
    expect(config.app.security.assetProtocol).toBeUndefined();
    expect(config.bundle.resources["../../THIRD_PARTY_NOTICES.md"])
      .toBe("THIRD_PARTY_NOTICES.md");
  });

  it("keeps renderer event authority listen-only in both application windows", () => {
    const readCapability = (name: string) => JSON.parse(readFileSync(
      new URL(`../../src-tauri/capabilities/${name}.json`, import.meta.url),
      "utf8",
    )) as { permissions: string[]; windows: string[] };
    const main = readCapability("default");
    const overlay = readCapability("live-overlay");

    expect(main.windows).toEqual(["main"]);
    expect(main.permissions).toEqual([
      "core:event:allow-listen",
      "core:event:allow-unlisten",
      "core:window:allow-close",
      "core:window:allow-minimize",
      "core:window:allow-start-dragging",
      "core:window:allow-toggle-maximize",
    ]);
    expect(overlay.windows).toEqual(["live-overlay"]);
    expect(overlay.permissions).toEqual([
      "core:event:allow-listen",
      "core:event:allow-unlisten",
    ]);
    expect([...main.permissions, ...overlay.permissions].some((permission) =>
      permission.includes("allow-emit") || permission.endsWith(":default"))).toBe(false);
  });
});

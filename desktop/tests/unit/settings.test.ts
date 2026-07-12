import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock, isTauriMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  isTauriMock: vi.fn(() => true),
  listenMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  isTauri: isTauriMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

import {
  cancelFallbackModelInstall,
  fallbackModelStatus,
  installFallbackModel,
  listenFallbackModelProgress,
  listenFallbackModelStatus,
  openFallbackModelFolder,
  projectServerConnectionTestMessage,
  removeFallbackModel,
  saveServerSettings,
  serverSettings,
  setFallbackModelEnabled,
  testServerConnection,
  verifyFallbackModel,
} from "@/settings";

describe("settings model lifecycle bindings", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    isTauriMock.mockReset();
    isTauriMock.mockReturnValue(true);
    listenMock.mockReset();
  });

  it("invokes the typed fallback model commands", async () => {
    invokeMock.mockResolvedValue({ status: "ready" });

    await fallbackModelStatus();
    await installFallbackModel();
    await installFallbackModel({ force: true });
    await cancelFallbackModelInstall();
    await verifyFallbackModel();
    await removeFallbackModel();
    await setFallbackModelEnabled(false);
    await openFallbackModelFolder();

    expect(invokeMock.mock.calls).toEqual([
      ["fallback_model_status"],
      ["fallback_model_install"],
      ["fallback_model_install", { force: true }],
      ["fallback_model_cancel_install"],
      ["fallback_model_verify"],
      ["fallback_model_remove"],
      ["fallback_model_set_enabled", { enabled: false }],
      ["fallback_model_open_folder"],
    ]);
  });

  it("wraps fallback model events behind typed listeners", async () => {
    const stopProgress = vi.fn();
    const stopStatus = vi.fn();
    listenMock
      .mockResolvedValueOnce(stopProgress)
      .mockResolvedValueOnce(stopStatus);

    const onProgress = vi.fn();
    const onStatus = vi.fn();

    const unlistenProgress = await listenFallbackModelProgress(onProgress);
    const progressEvent = listenMock.mock.calls[0]?.[1];
    progressEvent?.({ payload: { status: "downloading" } });

    const unlistenStatus = await listenFallbackModelStatus(onStatus);
    const statusEvent = listenMock.mock.calls[1]?.[1];
    statusEvent?.({ payload: { status: "ready" } });

    expect(listenMock.mock.calls[0]?.[0]).toBe("fallback-model-progress");
    expect(listenMock.mock.calls[1]?.[0]).toBe("fallback-model-status");
    expect(onProgress).toHaveBeenCalledWith({ status: "downloading" });
    expect(onStatus).toHaveBeenCalledWith({ status: "ready" });
    expect(unlistenProgress).toBe(stopProgress);
    expect(unlistenStatus).toBe(stopStatus);
  });

  it("returns a noop listener outside Tauri", async () => {
    isTauriMock.mockReturnValue(false);

    const unlisten = await listenFallbackModelProgress(vi.fn());

    expect(listenMock).not.toHaveBeenCalled();
    expect(unlisten()).toBeUndefined();
  });

  it("invokes typed server settings and connection commands", async () => {
    const settings = {
      schemaVersion: 1 as const,
      enabled: true,
      baseUrl: "https://server.example",
    };
    invokeMock.mockResolvedValue(settings);

    await serverSettings();
    await saveServerSettings(settings);
    await testServerConnection();

    expect(invokeMock.mock.calls).toEqual([
      ["server_settings"],
      ["set_server_settings", { settings }],
      ["refresh_server_connection"],
    ]);
  });

  it("projects terse inline connection results", () => {
    expect(projectServerConnectionTestMessage("ready")).toBe("Connection ready.");
    expect(projectServerConnectionTestMessage("offline")).toBe("Server is offline.");
    expect(projectServerConnectionTestMessage("sign_in_required")).toBe("Sign-in required.");
  });
});

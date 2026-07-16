import type { Page } from "@playwright/test";


export async function installQueuedServerBridge(
  page: Page,
  serverState: "not_set" | "offline",
) {
  await page.addInitScript((state) => {
    Object.defineProperty(globalThis, "isTauri", { value: true });
    const calls: string[] = [];
    const shortcutCalls: Array<{ args: unknown; command: string }> = [];
    Object.assign(globalThis, { __queuedServerBoundaryTest: { calls, shortcutCalls } });
    let callbackId = 0;
    const serverSnapshot = {
      apiVersion: null,
      capabilities: { batchJobs: false, jobStatus: false, liveStreaming: false },
      checkedAtMs: state === "offline" ? 1 : null,
      errorCode: state === "offline" ? "CONNECTION_FAILED" : null,
      retryAtMs: null,
      state,
    };
    const queuedJob = {
      id: `durable-${state}-job`,
      name: `${state.replace("_", "-")}-interview.wav`,
      pipeline: {
        alignment: "notStarted",
        diarization: "notStarted",
        intake: "done",
        postprocessing: "notStarted",
        preprocessing: "notStarted",
        transcription: "notStarted",
      },
      playbackPath: "http://127.0.0.1:43123/media/queued-proof",
      route: "serverBatch",
      sessionMode: "meeting",
      sessionOrigin: "importedFile",
      sourcePath: `C:\\recordings\\${state}-interview.wav`,
      status: "queued_server",
    };
    let liveSnapshot = {
      captureMode: "pushToTalk",
      hotkey: "Ctrl+Shift+Space",
      pasteHotkey: "Ctrl+Shift+Alt+V",
      route: "localFallback",
      status: "idle",
      visibility: "enabled",
    };

    Object.assign(globalThis, {
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __TAURI_INTERNALS__: {
        convertFileSrc: (path: string) => `asset:${path}`,
        metadata: {
          currentWebview: { label: "main" },
          currentWindow: { label: "main" },
        },
        transformCallback: () => ++callbackId,
        invoke: async (command: string, args?: unknown) => {
          calls.push(command);
          if (command === "plugin:event|listen") return ++callbackId;
          if (command === "plugin:event|unlisten") return undefined;
          if (command === "recording_jobs_snapshot") return [queuedJob];
          if (command === "history_catalog") {
            return { maintenanceWarnings: [], sessions: [] };
          }
          if (command === "setup_status") return {
            engineBinaryStatus: "ready",
            engineReady: true,
            engineStatus: "Ready",
            fallbackEnabled: true,
            model: "test",
            modelInstalled: true,
            root: "C:\\Yap",
          };
          if (command === "fallback_model_status") return {
            id: "nemotron-3.5-asr-streaming-0.6b-1120ms-int8",
            label: "Nemotron",
            modelsDir: "C:\\Yap\\models",
            status: "ready",
          };
          if (command === "server_connection_status" || command === "refresh_server_connection") {
            return serverSnapshot;
          }
          if (command === "server_settings") return {
            baseUrl: state === "offline" ? "https://server.example" : null,
            enabled: state === "offline",
            schemaVersion: 1,
          };
          if (command === "live_status") return liveSnapshot;
          if (command === "record_live_hotkey") {
            shortcutCalls.push({ args, command });
            liveSnapshot = { ...liveSnapshot, hotkey: "Ctrl+Shift+D" };
            return liveSnapshot;
          }
          if (command === "record_live_paste_hotkey") {
            shortcutCalls.push({ args, command });
            liveSnapshot = { ...liveSnapshot, pasteHotkey: "Ctrl+Shift+Alt+P" };
            return liveSnapshot;
          }
          if (command === "reset_live_hotkey") {
            shortcutCalls.push({ args, command });
            liveSnapshot = { ...liveSnapshot, hotkey: "Ctrl+Shift+Space" };
            return liveSnapshot;
          }
          if (command === "reset_live_paste_hotkey") {
            shortcutCalls.push({ args, command });
            liveSnapshot = { ...liveSnapshot, pasteHotkey: "Ctrl+Shift+Alt+V" };
            return liveSnapshot;
          }
          if (command === "list_local_compute_targets") {
            return [{ id: "auto", label: "Auto", selected: true }];
          }
          if (
            command === "list_input_devices" ||
            command === "resolve_owned_live_transcript_paths"
          ) return [];
          if (command === "read_text_file" || command === "read_text_preview") return "";
          return undefined;
        },
      },
    });
  }, serverState);
}

export async function shortcutCalls(page: Page) {
  return page.evaluate(() =>
    (globalThis as unknown as {
      __queuedServerBoundaryTest: {
        shortcutCalls: Array<{ args: unknown; command: string }>;
      };
    }).__queuedServerBoundaryTest.shortcutCalls,
  );
}

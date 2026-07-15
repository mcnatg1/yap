import { expect, test, type Page } from "@playwright/test";

async function installQueuedServerBridge(
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
          if (command === "recording_jobs_completed_transcripts") {
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
          if (command === "list_saved_live_sessions") {
            return { maintenanceWarnings: [], sessions: [] };
          }
          if (
            command === "list_input_devices" ||
            command === "list_recoverable_live_sessions" ||
            command === "resolve_owned_live_transcript_paths"
          ) return [];
          if (command === "read_text_file" || command === "read_text_preview") return "";
          return undefined;
        },
      },
    });
  }, serverState);
}

for (const scenario of [
  { label: "server unset", state: "not_set" },
  { label: "offline server", state: "offline" },
] as const) {
  test(`${scenario.label} keeps durable imported jobs queued without local transcription`, async ({ page }) => {
    await installQueuedServerBridge(page, scenario.state);
    await page.goto("/");
    await page.getByRole("button", { name: "Transcribe", exact: true }).click();

    const name = `${scenario.state.replace("_", "-")}-interview.wav`;
    await expect(page.getByRole("button", { name: `Select ${name}` })).toBeVisible();
    await expect(page.getByText("Waiting in queue", { exact: true })).toBeVisible();
    await expect(page.getByRole("paragraph").filter({ hasText:
      "Queued for your organization's transcription server. It will start when Yap connects.",
    })).toBeVisible();
    await expect(page.getByText("Transcribing", { exact: true })).toHaveCount(0);
    expect(await page.evaluate(() => localStorage.getItem("yap.recordingQueue.v1"))).toBeNull();

    await page.getByRole("button", { name: "Open settings" }).click();
    const settings = page.getByRole("dialog", { name: "Settings" });
    await settings.getByRole("button", { name: "System", exact: true }).click();
    await expect(settings.getByText("Local fallback", { exact: true }).locator("..")).toContainText("Ready");

    const calls = await page.evaluate(() =>
      (globalThis as unknown as { __queuedServerBoundaryTest: { calls: string[] } })
        .__queuedServerBoundaryTest.calls,
    );
    expect(calls).not.toContain("start_transcribe");
    expect(calls).not.toContain("fallback_model_install");
    expect(calls).toContain("recording_jobs_completed_transcripts");
    expect(calls.filter((command) =>
      command.startsWith("recording_job") &&
      command !== "recording_jobs_snapshot" &&
      command !== "recording_jobs_completed_transcripts"
    )).toEqual([]);
  });
}

test("shortcut settings delegate physical recording to native commands and expose per-action reset", async ({ page }) => {
  await installQueuedServerBridge(page, "not_set");
  await page.goto("/");
  await page.getByRole("button", { name: "Open settings" }).click();

  const settings = page.getByRole("dialog", { name: "Settings" });
  const dictationRow = settings.getByText("Dictation shortcut", { exact: true }).locator("xpath=../..");
  const pasteRow = settings.getByText("Paste-last shortcut", { exact: true }).locator("xpath=../..");
  await expect(dictationRow).toContainText("Ctrl+Shift+Space");
  await expect(pasteRow).toContainText("Ctrl+Shift+Alt+V");
  await expect(dictationRow.getByRole("textbox")).toHaveCount(0);
  await expect(pasteRow.getByRole("textbox")).toHaveCount(0);

  await page.keyboard.press("A");
  expect(await shortcutCalls(page)).toEqual([]);

  await dictationRow.getByRole("button", { name: "Record shortcut" }).click();
  await expect(dictationRow).toContainText("Ctrl+Shift+D");

  await pasteRow.getByRole("button", { name: "Record shortcut" }).click();
  await expect(pasteRow).toContainText("Ctrl+Shift+Alt+P");

  await pasteRow.getByRole("button", { name: "Reset" }).click();
  await expect(pasteRow).toContainText("Ctrl+Shift+Alt+V");
  const calls = await shortcutCalls(page);
  expect(calls.map(({ command }) => command)).toEqual([
    "record_live_hotkey",
    "record_live_paste_hotkey",
    "reset_live_paste_hotkey",
  ]);
  expect(calls.slice(0, 2).every(({ args }) =>
    !args || !("hotkey" in (args as Record<string, unknown>))
  )).toBe(true);
});

test("main app renders the home surface", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByText("Welcome back")).toBeVisible();
  await expect(page.getByRole("button", { name: "Home" })).toBeVisible();
});

async function shortcutCalls(page: Page) {
  return page.evaluate(() =>
    (globalThis as unknown as {
      __queuedServerBoundaryTest: {
        shortcutCalls: Array<{ args: unknown; command: string }>;
      };
    }).__queuedServerBoundaryTest.shortcutCalls,
  );
}

test("browser preview keeps its startup status and auth labels", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByText("Preview", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Open settings" }).click();
  await page.getByRole("button", { name: "About", exact: true }).click();
  await expect(page.getByText("Tauri bridge", { exact: true })).toBeVisible();
});

test("production surface hides the development-only Polish workspace", async ({ page }) => {
  await page.goto("/");

  await expect(page.locator('[data-sidebar="menu-button"]').filter({ hasText: /^Polish$/ }))
    .toHaveCount(0);
  await expect(page.getByText("Polish unavailable", { exact: true })).toHaveCount(0);
});

test("Settings and Help remain one mutually exclusive modal surface", async ({ page }) => {
  await page.goto("/");

  const settingsButton = page.locator('[data-sidebar="menu-button"]').filter({ hasText: /^Settings$/ });
  const helpButton = page.locator('[data-sidebar="menu-button"]').filter({ hasText: /^Help$/ });

  await settingsButton.click();
  await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();

  await helpButton.evaluate((button) => {
    button.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  });
  await expect(page.getByRole("dialog", { name: "Help" })).toBeVisible();
  await expect(page.getByRole("dialog", { name: "Settings" })).toHaveCount(0);
  await expect(page.getByRole("dialog")).toHaveCount(1);

  await settingsButton.evaluate((button) => {
    button.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  });
  await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
  await expect(page.getByRole("dialog", { name: "Help" })).toHaveCount(0);
  await expect(page.getByRole("dialog")).toHaveCount(1);
});

test("Transcribe and Help describe the organization server queue", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Transcribe", exact: true }).click();

  await expect(page.getByText("Add recordings to your organization's transcription queue."))
    .toBeVisible();
  await expect(page.getByText("Organization server queue", { exact: true })).toBeVisible();
  await expect(page.getByText("Private on this device", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Drop files to run", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Choose files above to add them to the organization server queue.", { exact: true }))
    .toBeVisible();

  await page.locator('[data-sidebar="menu-button"]').filter({ hasText: /^Help$/ }).click();
  await expect(page.getByRole("dialog", { name: "Help" })).toContainText("Choose files");
  await expect(page.getByRole("dialog", { name: "Help" })).toContainText("organization server queue");
});

test("history body search paints indexing before an empty result", async ({ page }) => {
  await page.addInitScript(() => {
    const recordingsDir = "C:\\Users\\tester\\AppData\\Local\\Yap\\live-recordings";
    localStorage.setItem("yap.transcriptHistory.v1", JSON.stringify(
      Array.from({ length: 100 }, (_, index) => ({
        createdAt: new Date(Date.UTC(2026, 6, 11, 12, index)).toISOString(),
        name: `clip-${String(index).padStart(3, "0")}`,
        outputPath: `${recordingsDir}\\clip-${index}.txt`,
        sourcePath: `${recordingsDir}\\clip-${index}.wav`,
      })),
    ));
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Search past transcripts" }).click();

  const firstFrame = await page.getByRole("textbox", { name: "Search past transcripts" })
    .evaluate((input: HTMLInputElement) => {
      const valueSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      valueSetter?.call(input, "body-only-phrase");
      input.dispatchEvent(new Event("input", { bubbles: true }));
      return input.ownerDocument.body.innerText;
    });

  expect(firstFrame).toContain("Searching transcript text...");
  expect(firstFrame).not.toContain("No recordings match that search.");
});

test("history keeps committed review actions separate from recoverable capture actions", async ({ context, page }) => {
  const committedName = "live-s-18f001122334455-2a-0";
  const recoverableName = "live-s-18f001122334455-2a-1";
  const recordingsDir = "C:\\Users\\tester\\AppData\\Local\\Yap\\live-recordings";
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.addInitScript(({ committedName, recoverableName, recordingsDir }) => {
    const committedSessionId = committedName.slice("live-".length);
    const recoverableSessionId = recoverableName.slice("live-".length);
    const committedCreatedAt = "2026-07-11T12:00:00.000Z";
    const recoverableCreatedAt = "2026-07-11T12:01:00.000Z";
    const committed = {
      captureCommitPath: `${recordingsDir}\\${committedName}.commit.json`,
      createdAt: committedCreatedAt,
      name: committedName,
      outputPath: `${recordingsDir}\\${committedName}.txt`,
      sessionId: committedSessionId,
      sourcePath: `${recordingsDir}\\${committedName}.wav`,
    };
    const recoverable = {
      createdAt: recoverableCreatedAt,
      name: recoverableName,
      outputPath: `${recordingsDir}\\${recoverableName}.wav.part`,
      recoveryState: "recoverable",
      sessionId: recoverableSessionId,
      sourcePath: `${recordingsDir}\\${recoverableName}.wav.part`,
      warning: "Capture stopped before publication.",
    };
    localStorage.setItem("yap.transcriptHistory.v1", JSON.stringify([
      committed,
      recoverable,
    ]));

    Object.defineProperty(globalThis, "isTauri", { value: true });
    let callbackId = 0;
    Object.assign(globalThis, {
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __TAURI_INTERNALS__: {
        metadata: {
          currentWebview: { label: "main" },
          currentWindow: { label: "main" },
        },
        transformCallback: () => ++callbackId,
        invoke: async (command: string) => {
          if (command === "plugin:event|listen") return ++callbackId;
          if (command === "plugin:event|unlisten") return undefined;
          if (command === "list_saved_live_sessions") {
            return {
              maintenanceWarnings: [],
              sessions: [{
                ...committed,
                createdAtMs: Date.parse(committedCreatedAt),
              }],
            };
          }
          if (command === "list_recoverable_live_sessions") {
            return [{
              audioPartialPath: recoverable.sourcePath,
              expiresAtMs: Date.parse(recoverableCreatedAt) + 24 * 60 * 60 * 1_000,
              journalPartialPath: null,
              name: recoverable.name,
              reason: recoverable.warning,
              sessionId: recoverable.sessionId,
            }];
          }
          if (command === "setup_status") {
            return {
              engineBinaryStatus: "ready",
              engineReady: true,
              engineStatus: "Ready",
              fallbackEnabled: true,
              model: "test",
              modelInstalled: true,
              root: recordingsDir,
            };
          }
          if (command === "fallback_model_status") {
            return {
              id: "nemotron-3.5-asr-streaming-0.6b-1120ms-int8",
              label: "Nemotron",
              modelsDir: recordingsDir,
              status: "ready",
            };
          }
          if (command === "server_connection_status") {
            return {
              apiVersion: "1",
              capabilities: { batchJobs: false, jobStatus: false, liveStreaming: false },
              checkedAtMs: 1,
              errorCode: null,
              retryAtMs: null,
              state: "ready",
            };
          }
          if (command === "live_status") {
            return {
              captureMode: "pushToTalk",
              hotkey: "Ctrl+Shift+Space",
              pasteHotkey: "",
              route: "none",
              status: "idle",
              visibility: "enabled",
            };
          }
          if (command === "recording_jobs_snapshot") return [];
          if (command === "recording_jobs_completed_transcripts") {
            return { maintenanceWarnings: [], sessions: [] };
          }
          if (command === "list_input_devices" || command === "resolve_owned_live_transcript_paths") return [];
          if (command === "list_local_compute_targets") return [{ id: "auto", label: "Auto", selected: true }];
          if (command === "read_text_file" || command === "read_text_preview") return "";
          return undefined;
        },
      },
    });
  }, { committedName, recoverableName, recordingsDir });

  await page.goto("/");

  const rows = page.locator("tr[data-history-entry-row]");
  await expect(rows).toHaveCount(2);

  const committedRow = rows.filter({
    has: page.getByRole("button", { name: `Review recording ${committedName}` }),
  });
  await expect(committedRow).toHaveCount(1);
  await committedRow.getByRole("button", { name: `Copy transcript for ${committedName}` }).click();
  await expect(page.getByText("Empty transcript copied")).toBeVisible();

  await committedRow.getByRole("button", { name: `Review recording ${committedName}` }).click();
  await expect(page.getByRole("dialog", { name: committedName })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: committedName })).toHaveCount(0);

  const recoverableRow = rows.filter({ hasText: "Partial" });
  await expect(recoverableRow).toHaveCount(1);
  await expect(recoverableRow.getByText("Partial", { exact: true })).toBeVisible();
  await expect(recoverableRow.getByRole("button", { name: `Copy transcript for ${recoverableName}` })).toHaveCount(0);
  await expect(recoverableRow.getByRole("button", { name: `Review recording ${recoverableName}` })).toHaveCount(0);

  const cardCount = await page.locator('[data-slot="card"]').count();
  await recoverableRow.getByRole("button", { name: `Actions for ${recoverableName}` }).click();
  const menu = page.getByRole("menu");
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem")).toHaveText(["Recover", "Hide", "Delete"]);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator('[data-slot="card"]')).toHaveCount(cardCount);
});

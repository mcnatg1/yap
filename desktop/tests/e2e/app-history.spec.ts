import { expect, test } from "@playwright/test";


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
          if (command === "history_catalog") {
            return {
              maintenanceWarnings: [],
              sessions: [
                {
                  captureCommitPath: committed.captureCommitPath,
                  createdAtMs: Date.parse(committedCreatedAt),
                  name: committed.name,
                  origin: "live",
                  outputPath: committed.outputPath,
                  recoveryState: null,
                  sessionId: committed.sessionId,
                  sourcePath: committed.sourcePath,
                  warning: null,
                },
                {
                  captureCommitPath: null,
                  createdAtMs: Date.parse(recoverableCreatedAt),
                  name: recoverable.name,
                  origin: "live",
                  outputPath: recoverable.outputPath,
                  recoveryState: "recoverable",
                  sessionId: recoverable.sessionId,
                  sourcePath: recoverable.sourcePath,
                  warning: recoverable.warning,
                },
              ],
            };
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

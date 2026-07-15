import { expect, test, type Page } from "@playwright/test";

const mediaDuration = 100.9;

async function installPlaybackBridge(
  page: Page,
  paths: string[],
  restoreDelayMs = 0,
  durationSeconds = mediaDuration,
) {
  await page.addInitScript(({ mediaDuration, paths, restoreDelayMs }) => {
    localStorage.removeItem("yap.recordingQueue.v1");
    Object.defineProperty(globalThis, "isTauri", { value: true });

    const currentTimes = new WeakMap<HTMLMediaElement, number>();
    Object.defineProperty(HTMLMediaElement.prototype, "currentTime", {
      configurable: true,
      get() { return currentTimes.get(this) ?? 0; },
      set(value: number) { currentTimes.set(this, value); },
    });
    Object.defineProperty(HTMLMediaElement.prototype, "duration", {
      configurable: true,
      get() { return mediaDuration; },
    });

    const tokens = new Map<string, string>();
    const jobs = paths.map((recordingPath, index) => {
      const token = (index + 1).toString(16).padStart(64, "0");
      const playbackPath = `http://127.0.0.1:43123/media/${token}`;
      tokens.set(recordingPath, playbackPath);
      return {
        id: `job-native-${index + 1}-${token.slice(-16)}`,
        name: recordingPath.split("\\").pop() ?? recordingPath,
        pipeline: {
          alignment: "notStarted",
          diarization: "notStarted",
          intake: "done",
          postprocessing: "notStarted",
          preprocessing: "notStarted",
          transcription: "notStarted",
        },
        playbackPath,
        route: "serverBatch",
        sessionMode: "meeting",
        sessionOrigin: "importedFile",
        sourcePath: recordingPath,
        status: "queued_server",
      };
    });
    const state = {
      activeRestores: 0,
      highWaterMark: 0,
      released: [] as string[],
      restoreCalls: 0,
      jobs: jobs as Array<Record<string, unknown>>,
      tokens,
    };
    Object.assign(globalThis, { __playbackTest: state });
    let callbackId = 0;
    Object.assign(globalThis, {
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __TAURI_INTERNALS__: {
        convertFileSrc: (path: string) => `asset:${path}`,
        metadata: {
          currentWebview: { label: "main" },
          currentWindow: { label: "main" },
        },
        transformCallback: () => ++callbackId,
        invoke: async (command: string, args: Record<string, unknown> = {}) => {
          if (command === "plugin:event|listen") return ++callbackId;
          if (command === "plugin:event|unlisten") return undefined;
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
          if (command === "live_status") return {
            captureMode: "pushToTalk",
            hotkey: "Ctrl+Shift+Space",
            pasteHotkey: "",
            route: "none",
            status: "idle",
            visibility: "enabled",
          };
          if (command === "list_input_devices") return [];
          if (command === "list_local_compute_targets") return [{ id: "auto", label: "Auto", selected: true }];
          if (command === "history_catalog") return { maintenanceWarnings: [], sessions: [] };
          if (command === "resolve_owned_live_transcript_paths") return [];
          if (command === "read_text_file" || command === "read_text_preview") return "";
          if (command === "recording_jobs_snapshot") return structuredClone(state.jobs);
          if (command === "recording_job_cancel") {
            const index = state.jobs.findIndex((job) => job.id === args.jobId);
            const [cancelled] = index >= 0 ? state.jobs.splice(index, 1) : [];
            return { ...cancelled, status: "cancelled" };
          }
          if (command === "recording_job_retry") {
            return state.jobs.find((job) => job.id === args.jobId);
          }
          if (command === "release_recording_playback") {
            state.released.push(String(args.playbackPath));
            return undefined;
          }
          if (command === "restore_recording_playback_path") {
            state.restoreCalls += 1;
            state.activeRestores += 1;
            state.highWaterMark = Math.max(state.highWaterMark, state.activeRestores);
            if (restoreDelayMs) await new Promise((resolve) => setTimeout(resolve, restoreDelayMs));
            state.activeRestores -= 1;
            const path = String(args.path);
            const token = state.restoreCalls.toString(16).padStart(64, "0");
            const playbackPath = `http://127.0.0.1:43123/media/${token}`;
            state.tokens.set(path, playbackPath);
            const waveformEligible = path.includes("small");
            return {
              byteLength: waveformEligible ? "4096" : "33554433",
              playbackPath,
              waveformEligible,
            };
          }
          return undefined;
        },
      },
    });
  }, { mediaDuration: durationSeconds, paths, restoreDelayMs });
}

async function seedTranscriptHistory(page: Page, count: number) {
  await page.addInitScript((entryCount) => {
    localStorage.setItem(
      "yap.transcriptHistory.v1",
      JSON.stringify(Array.from({ length: entryCount }, (_, index) => ({
        createdAt: new Date(Date.UTC(2026, 0, 1, 0, 0, index)).toISOString(),
        name: `history-${index}`,
        outputPath: `C:\\transcripts\\history-${index}.txt`,
        sourcePath: `C:\\recordings\\history-${index}.wav`,
      }))),
    );
  }, count);
}

test("Rust-projected playback stays available without queue reauthorization", async ({ page }) => {
  await installPlaybackBridge(page, ["C:\\small.wav"], 0, 30);
  await page.goto("/");
  await page.getByRole("button", { name: "Transcribe", exact: true }).click();
  await page.getByRole("button", { name: "Select small.wav" }).click();

  const slider = page.getByRole("slider", { name: "Seek recording small.wav" });
  const audio = page.locator("audio");
  await expect(audio).toHaveAttribute("src", /^http:\/\/127\.0\.0\.1:43123\/media\//);
  await audio.dispatchEvent("loadedmetadata");
  await expect(slider).toHaveAttribute("data-waveform-mode", "lightweight");
  await expect(slider).not.toHaveAttribute("data-waveform-mounted", "true");
  expect(await page.evaluate(() =>
    (globalThis as unknown as { __playbackTest: { restoreCalls: number } })
      .__playbackTest.restoreCalls)).toBe(0);
});

test("lightweight seeking uses visible bounds, exact endpoints, ARIA, and release", async ({ page }) => {
  await installPlaybackBridge(page, ["C:\\large.wav"]);
  await page.goto("/");
  await page.getByRole("button", { name: "Transcribe", exact: true }).click();
  const audio = page.locator("audio");
  const slider = page.getByRole("slider", { name: "Seek recording large.wav" });
  await audio.dispatchEvent("loadedmetadata");
  const track = page.getByTestId("lightweight-seek-track");
  const sliderBox = await slider.boundingBox();
  const trackBox = await track.boundingBox();
  expect(sliderBox).not.toBeNull();
  expect(trackBox).not.toBeNull();

  await page.mouse.click(sliderBox!.x + 1, trackBox!.y + trackBox!.height / 2);
  await expect.poll(() => audio.evaluate((element) => element.currentTime)).toBe(0);
  await page.mouse.click(sliderBox!.x + sliderBox!.width - 1, trackBox!.y + trackBox!.height / 2);
  await expect.poll(() => audio.evaluate((element) => element.currentTime)).toBe(mediaDuration);

  await slider.focus();
  await page.keyboard.press("Home");
  await expect(slider).toHaveAttribute("aria-valuenow", "0");
  await page.keyboard.press("End");
  await expect.poll(() => audio.evaluate((element) => element.currentTime)).toBe(mediaDuration);
  await expect(slider).toHaveAttribute("aria-valuemax", "100");
  await expect(slider).toHaveAttribute("aria-valuenow", "100");
  await expect(slider).toHaveAttribute("aria-valuetext", "1:40 of 1:40");

  await page.getByRole("button", { name: "Remove file" }).click();
  await expect(page.getByRole("button", { name: "Select large.wav" })).toHaveCount(0);
});

test("native ledger projection uses only pre-authorized playback", async ({ page }) => {
  await installPlaybackBridge(
    page,
    Array.from({ length: 20 }, (_, index) => `C:\\large-${index}.wav`),
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Transcribe", exact: true }).click();

  await expect(page.getByRole("button", { name: /^Select large-/ })).toHaveCount(20);
  expect(await page.evaluate(() => localStorage.getItem("yap.recordingQueue.v1"))).toBeNull();
  expect(await page.evaluate(() =>
    (globalThis as unknown as { __playbackTest: { restoreCalls: number } })
      .__playbackTest.restoreCalls)).toBe(0);
});

test("history playback is admitted only after selecting one entry", async ({ page }) => {
  await installPlaybackBridge(page, []);
  await seedTranscriptHistory(page, 200);
  await page.goto("/");

  const selectedName = "history-199";
  await expect(page.getByRole("button", { name: `Review recording ${selectedName}` })).toBeVisible();
  await page.waitForTimeout(100);
  expect(await page.evaluate(() =>
    (globalThis as unknown as { __playbackTest: { restoreCalls: number } })
      .__playbackTest.restoreCalls)).toBe(0);

  await page.getByRole("button", { name: `Review recording ${selectedName}` }).click();
  await expect.poll(() => page.evaluate(() =>
    (globalThis as unknown as { __playbackTest: { restoreCalls: number } })
      .__playbackTest.restoreCalls)).toBe(1);
  expect(await page.evaluate(() => [
    ...(globalThis as unknown as { __playbackTest: { tokens: Map<string, string> } })
      .__playbackTest.tokens.keys(),
  ])).toEqual([`C:\\recordings\\${selectedName}.wav`]);

  await page.keyboard.press("Escape");
  await expect.poll(() => page.evaluate(() =>
    (globalThis as unknown as { __playbackTest: { released: string[] } })
      .__playbackTest.released.length)).toBe(1);
});

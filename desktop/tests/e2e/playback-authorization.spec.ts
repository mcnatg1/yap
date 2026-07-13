import { expect, test, type Page } from "@playwright/test";

import { makeTestToneWav } from "../fixtures/audio-fixture";

const mediaDuration = 100.9;

async function installPlaybackBridge(
  page: Page,
  paths: string[],
  restoreDelayMs = 0,
  durationSeconds = mediaDuration,
) {
  await page.addInitScript(({ mediaDuration, paths, restoreDelayMs }) => {
    localStorage.setItem(
      "yap.recordingQueue.v1",
      JSON.stringify(paths.map((path, index) => ({ id: index + 1, path }))),
    );
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

    const state = {
      activeRestores: 0,
      highWaterMark: 0,
      released: [] as string[],
      restoreCalls: 0,
      tokens: new Map<string, string>(),
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
          if (command === "list_input_devices" || command === "list_recoverable_live_sessions") return [];
          if (command === "list_local_compute_targets") return [{ id: "auto", label: "Auto", selected: true }];
          if (command === "list_saved_live_sessions") return { maintenanceWarnings: [], sessions: [] };
          if (command === "resolve_owned_live_transcript_paths") return [];
          if (command === "read_text_file" || command === "read_text_preview") return "";
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

test("decoded waveform state follows ready and error while native playback stays available", async ({ page }) => {
  const testTone = Buffer.from(makeTestToneWav({ durationMs: 100 }));
  let resolveFirstFetchStarted!: () => void;
  let resolveSecondFetchStarted!: () => void;
  let releaseNativeFetch!: () => void;
  let releaseFirstFetch!: () => void;
  let releaseSecondFetch!: () => void;
  const firstFetchStarted = new Promise<void>((resolve) => { resolveFirstFetchStarted = resolve; });
  const secondFetchStarted = new Promise<void>((resolve) => { resolveSecondFetchStarted = resolve; });
  const nativeFetchRelease = new Promise<void>((resolve) => { releaseNativeFetch = resolve; });
  const firstFetchRelease = new Promise<void>((resolve) => { releaseFirstFetch = resolve; });
  const secondFetchRelease = new Promise<void>((resolve) => { releaseSecondFetch = resolve; });
  let waveformFetches = 0;

  await page.route("http://127.0.0.1:43123/media/**", async (route) => {
    if (route.request().resourceType() !== "fetch") {
      await nativeFetchRelease;
      await route.fulfill({ body: testTone, contentType: "audio/wav" });
      return;
    }

    waveformFetches += 1;
    if (waveformFetches === 1) {
      resolveFirstFetchStarted();
      await firstFetchRelease;
      await route.fulfill({ body: testTone, contentType: "audio/wav" });
      return;
    }

    resolveSecondFetchStarted();
    await secondFetchRelease;
    await route.fulfill({ body: "not audio", contentType: "audio/wav" });
  });

  await installPlaybackBridge(
    page,
    ["C:\\small.wav", "C:\\small-broken.wav", "C:\\large.wav"],
    0,
    30,
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Transcribe", exact: true }).click();
  await page.getByRole("button", { name: "Select small.wav" }).click();

  const slider = page.getByRole("slider", { name: "Seek recording small.wav" });
  const audio = page.locator("audio");
  await expect(slider).toHaveAttribute("data-waveform-mode", "pending");
  await expect(audio).toHaveAttribute("src", /^http:\/\/127\.0\.0\.1:43123\/media\//);
  releaseNativeFetch();
  await audio.dispatchEvent("loadedmetadata");
  await expect(slider).toHaveAttribute("data-waveform-mode", "lightweight");
  await expect(slider).not.toHaveAttribute("data-waveform-mounted", "true");

  await page.getByRole("button", { name: "Play recording small.wav" }).click();
  await firstFetchStarted;
  await expect(slider).toHaveAttribute("data-waveform-mode", "lightweight");
  await expect(slider).not.toHaveAttribute("data-waveform-mounted", "true");
  await expect(page.getByTestId("lightweight-seek-track")).toBeVisible();

  releaseFirstFetch();
  await expect(slider).toHaveAttribute("data-waveform-mode", "decoded");
  await expect(slider).toHaveAttribute("data-waveform-mounted", "true");
  await expect(page.getByTestId("lightweight-seek-track")).toHaveCount(0);
  await slider.focus();
  await page.keyboard.press("End");
  await expect.poll(() => audio.evaluate((element) => element.currentTime)).toBe(30);

  await page.getByRole("button", { name: "Select small-broken.wav" }).click();
  const brokenSlider = page.getByRole("slider", { name: "Seek recording small-broken.wav" });
  await audio.dispatchEvent("loadedmetadata");
  await expect(brokenSlider).toHaveAttribute("data-waveform-mode", "lightweight");

  await page.getByRole("button", { name: "Play recording small-broken.wav" }).click();
  await secondFetchStarted;
  await expect(brokenSlider).toHaveAttribute("data-waveform-mode", "lightweight");
  await expect(brokenSlider).not.toHaveAttribute("data-waveform-mounted", "true");
  await expect(page.getByTestId("lightweight-seek-track")).toBeVisible();

  releaseSecondFetch();
  await expect.poll(() => brokenSlider.evaluate((element) =>
    [...element.children].filter((child) => child.shadowRoot).length)).toBe(0);
  await expect(brokenSlider).toHaveAttribute("data-waveform-mode", "lightweight");
  await expect(brokenSlider).not.toHaveAttribute("data-waveform-mounted", "true");
  await expect(page.getByRole("button", {
    name: /^(?:Play|Pause) recording small-broken\.wav$/,
  })).toBeEnabled();

  await page.getByRole("button", { name: "Select large.wav" }).click();
  const largeSlider = page.getByRole("slider", { name: "Seek recording large.wav" });
  await expect(largeSlider).toHaveAttribute("data-waveform-mode", "lightweight");
  await expect(largeSlider).not.toHaveAttribute("data-waveform-mounted", "true");
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
  await expect.poll(() => page.evaluate(() =>
    (globalThis as unknown as { __playbackTest: { released: string[] } })
      .__playbackTest.released.length)).toBe(1);
});

test("queue restoration stays globally bounded across effect generations", async ({ page }) => {
  await installPlaybackBridge(
    page,
    Array.from({ length: 20 }, (_, index) => `C:\\large-${index}.wav`),
    1_000,
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Transcribe", exact: true }).click();
  await page.getByRole("button", { name: "Remove file" }).first().click();

  await expect.poll(() => page.evaluate(() =>
    (globalThis as unknown as { __playbackTest: { restoreCalls: number } })
      .__playbackTest.restoreCalls)).toBe(20);
  const highWaterMark = await page.evaluate(() =>
    (globalThis as unknown as { __playbackTest: { highWaterMark: number } })
      .__playbackTest.highWaterMark);
  expect(highWaterMark).toBe(4);
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

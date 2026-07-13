import { expect, type Locator, type Page, test } from "@playwright/test";

type Box = {
  height: number;
  width: number;
  x: number;
  y: number;
};

type GeometrySample = {
  controls: Array<{ box: Box; name: string }>;
  island: Box | null;
  phase: string | null;
  requestedStatus: string | null;
  root: Box | null;
  surface: string | null;
  timestamp: number;
  transform: { x: number; y: number } | null;
};

const previewUrl = "/?window=live-overlay&preview=live-overlay";
const rootFrame = { height: 40, width: 260 };
const tolerance = 1;

test.describe.configure({ timeout: 45_000 });

test("live overlay hidden idle state renders no sensor", async ({ page }) => {
  await openOverlayPreview(page, "&visibility=hidden&status=idle");

  await expect(page.getByTestId("live-overlay-root")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Start dictating" })).toHaveCount(0);
});

test("idle sensor stays inside a stable preview frame", async ({ page }) => {
  await openOverlayPreview(page);

  const root = page.getByTestId("live-overlay-root");
  await expect(root).toHaveAttribute("data-overlay-surface", "sensor");
  await expectFrame(root, rootFrame);

  await page.mouse.move(130, 20);
  await waitForAnimationFrames(page, 2);
  await expect(root).toHaveAttribute("data-overlay-surface", "sensor");
  await expectFrame(root, rootFrame);

  await hoverIdleSensor(page);
  await expect(root).toHaveAttribute("data-overlay-surface", "peek");
  await waitForIslandSettled(page.getByTestId("live-overlay-island"));
  await expectFrame(root, rootFrame);
});

test("live overlay respects reduced motion without a blank initializing frame", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openOverlayPreview(page);

  const root = page.getByTestId("live-overlay-root");
  await expectFrame(root, rootFrame);
  await startGeometrySampling(page);

  await hoverIdleSensor(page);
  await expect(root).toHaveAttribute("data-overlay-surface", "peek");

  const island = page.getByTestId("live-overlay-island");
  await expectIslandTranslationY(island, 0);
  await expect(island).toHaveCSS("transition-duration", "0s");

  await setLiveView(page, {
    activeCaptureMode: "pushToTalk",
    captureMode: "pushToTalk",
    level: 0,
    route: "localFallback",
    status: "armed",
  });
  await expect(root).toHaveAttribute("data-overlay-phase", "initializing");
  await expect(root).toHaveAttribute("data-overlay-surface", "initializing");
  await expect(page.getByTestId("live-recording-layout")).toBeVisible();
  await expectIslandTranslationY(island, 0);

  await setLiveView(page, {
    activeCaptureMode: "pushToTalk",
    captureMode: "pushToTalk",
    level: 0.12,
    route: "localFallback",
    status: "speaking",
  });
  const waveform = page.getByTestId("live-waveform");
  await expect(waveform).toBeVisible();
  await expect(waveform.locator("span").first()).toHaveCSS("transition-duration", "0s");
  const before = await waveformBarHeights(waveform);
  await waitForAnimationFrames(page, 3);
  expect(await waveformBarHeights(waveform)).toEqual(before);

  const samples = await stopGeometrySampling(page);
  expectStableTransitionGeometry(samples, "reduced-motion initialization");
  expectNoBlankInitializingFrames(samples);
  expect(samples.filter((sample) => sample.requestedStatus === "armed").every(isSettledAtTop)).toBe(true);
});

test("live overlay state machine preserves geometry and mode ownership", async ({ page }) => {
  await openOverlayPreview(page);

  const root = page.getByTestId("live-overlay-root");
  const island = page.getByTestId("live-overlay-island");
  await expect(root).toHaveAttribute("data-overlay-surface", "sensor");
  await expectFrame(root, rootFrame);
  await startGeometrySampling(page);

  await hoverIdleSensor(page);
  await expect(root).toHaveAttribute("data-overlay-surface", "peek");
  await waitForIslandSettled(island);
  await expect(page.getByRole("button", { name: "Start dictating" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Open scratch" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Open transform" })).toBeVisible();
  await expectNoBodyText(page, "Dictate");
  await expectFrame(root, rootFrame);
  await expectInside(island, [
    page.getByRole("button", { name: "Start dictating" }),
    page.getByRole("button", { name: "Open scratch" }),
    page.getByRole("button", { name: "Open transform" }),
  ]);
  await expectNoClippedChildren(island);
  await expect(root).toHaveScreenshot("live-overlay-hover.png", {
    animations: "disabled",
    maxDiffPixelRatio: 0.04,
  });

  await setLiveView(page, {
    activeCaptureMode: "pushToTalk",
    captureMode: "pushToTalk",
    level: 0,
    route: "localFallback",
    status: "armed",
  });
  await expect(root).toHaveAttribute("data-overlay-phase", "initializing");
  await expect(root).toHaveAttribute("data-overlay-surface", "initializing");
  await expect(page.getByTestId("live-recording-layout")).toBeVisible();
  await expectFrame(root, rootFrame);

  await setLiveView(page, {
    activeCaptureMode: "pushToTalk",
    captureMode: "pushToTalk",
    level: 0.72,
    route: "localFallback",
    status: "speaking",
  });
  await expect(root).toHaveAttribute("data-overlay-phase", "recording");
  const holdWaveform = page.getByTestId("live-waveform");
  await expect(holdWaveform).toBeVisible();
  await expect(page.getByRole("button", { name: "Cancel recording" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Finish recording" })).toHaveCount(0);
  await expectFrame(root, rootFrame);
  await expectInside(island, [holdWaveform]);
  const holdWaveformCenter = centerX(await boxOf(holdWaveform));

  await setLiveView(page, {
    activeCaptureMode: "toggle",
    captureMode: "pushToTalk",
    level: 0.84,
    route: "localFallback",
    status: "speaking",
  });
  const toggleWaveform = page.getByTestId("live-waveform");
  const finish = page.getByRole("button", { name: "Finish recording" });
  await expect(toggleWaveform).toBeVisible();
  await expect(finish).toBeVisible();
  await expect(page.getByRole("button", { name: "Cancel recording" })).toHaveCount(0);
  await expectFrame(root, rootFrame);
  await expectInside(island, [toggleWaveform, finish]);
  await expectNoHorizontalOverlap(toggleWaveform, finish);
  expect(centerX(await boxOf(toggleWaveform))).toBeCloseTo(holdWaveformCenter, tolerance);
  const activeIslandWidth = await waitForStableWidth(island);

  await setLiveView(page, {
    activeCaptureMode: "toggle",
    captureMode: "pushToTalk",
    level: 0,
    route: "localFallback",
    status: "saving",
  });
  await expect(root).toHaveAttribute("data-overlay-phase", "processing");
  await expect(page.getByRole("button", { name: "Finish recording" })).toHaveCount(0);
  await expectFrame(root, rootFrame);
  expect((await boxOf(island)).width).toBeCloseTo(activeIslandWidth, tolerance);

  await setLiveView(page, {
    activeCaptureMode: undefined,
    captureMode: "toggle",
    finalText: "Saved dictation",
    level: 0,
    route: "none",
    status: "idle",
  });
  await expect(root).toHaveAttribute("data-overlay-surface", "success");
  await waitForIslandSettled(island);
  await expect(page.getByText("Saved")).toBeVisible();
  await expectFrame(root, rootFrame);
  await expectNoClippedChildren(island);

  await setLiveView(page, {
    error: "Mic denied",
    finalText: undefined,
    level: 0,
    route: "blocked",
    status: "blocked",
  });
  await expect(root).toHaveAttribute("data-overlay-phase", "feedback");
  await expect(root).toHaveAttribute("data-overlay-surface", "feedback");
  await expect(page.getByRole("button", { name: "Retry dictation" })).toBeVisible();
  await expectFrame(root, rootFrame);
  await expectInside(island, [page.getByRole("button", { name: "Retry dictation" })]);
  await expectNoClippedChildren(island);

  const samples = await stopGeometrySampling(page);
  expectStableTransitionGeometry(samples, "full overlay state machine");
  expectNoBlankInitializingFrames(samples);
  expectProcessingWidthPreserved(samples, activeIslandWidth);
  expect(samples.some(isIntermediateIslandFrame)).toBe(true);
});

test("rapid hover and status reversals settle to the latest surface without jitter", async ({ page }) => {
  await openOverlayPreview(page);

  const root = page.getByTestId("live-overlay-root");
  const island = page.getByTestId("live-overlay-island");
  await expectFrame(root, rootFrame);
  await startGeometrySampling(page);

  await hoverIdleSensor(page);
  await expect(root).toHaveAttribute("data-overlay-surface", "peek");
  await waitForAnimationFrames(page, 1);
  await page.mouse.move(130, 70);
  await waitForAnimationFrames(page, 1);
  await page.mouse.move(130, 3);
  await expect(root).toHaveAttribute("data-overlay-surface", "peek");
  await waitForIslandSettled(island);

  await page.mouse.move(130, 70);
  await expect(root).toHaveAttribute("data-overlay-surface", "sensor");
  await expectFrame(root, rootFrame);
  await hoverIdleSensor(page);
  await expect(root).toHaveAttribute("data-overlay-surface", "peek");
  await waitForIslandSettled(island);

  await dispatchPreviewSequence(page, [
    { activeCaptureMode: "pushToTalk", captureMode: "toggle", level: 0, route: "localFallback", status: "armed" },
    { activeCaptureMode: "pushToTalk", captureMode: "toggle", level: 0.7, route: "localFallback", status: "speaking" },
    { activeCaptureMode: "pushToTalk", captureMode: "toggle", level: 0, route: "localFallback", status: "saving" },
    { activeCaptureMode: "toggle", captureMode: "pushToTalk", level: 0.85, route: "localFallback", status: "speaking" },
    { activeCaptureMode: "toggle", captureMode: "pushToTalk", error: "Transient", level: 0, route: "blocked", status: "blocked" },
    { activeCaptureMode: "toggle", captureMode: "pushToTalk", error: undefined, level: 0.92, route: "localFallback", status: "speaking" },
  ]);

  await expect(root).toHaveAttribute("data-overlay-surface", "recording");
  await expect(root).toHaveAttribute("data-overlay-phase", "recording");
  await expect(page.getByRole("button", { name: "Finish recording" })).toBeVisible();
  await waitForIslandSettled(island);
  await expectFrame(root, rootFrame);

  const samples = await stopGeometrySampling(page);
  expectStableTransitionGeometry(samples, "rapid hover and status reversal");
  expectNoBlankInitializingFrames(samples);
  expect(samples.length).toBeGreaterThanOrEqual(8);
  expect(samples[samples.length - 1]?.surface).toBe("recording");
});

async function openOverlayPreview(page: Page, query = "") {
  await page.setViewportSize({ width: 260, height: 90 });
  await page.mouse.move(130, 80);
  await page.goto(`${previewUrl}${query}`);
}

async function setLiveView(page: Page, detail: Record<string, unknown>) {
  await page.evaluate((nextView) => {
    const state = (window as typeof window & {
      __yapOverlayGeometry?: { requestedStatus: string | null };
    }).__yapOverlayGeometry;
    if (state && typeof nextView.status === "string") state.requestedStatus = nextView.status;
    window.dispatchEvent(new CustomEvent("yap-live-overlay-preview", { detail: nextView }));
  }, detail);
}

async function dispatchPreviewSequence(page: Page, states: Array<Record<string, unknown>>) {
  await page.evaluate(async (nextStates) => {
    const geometryState = (window as typeof window & {
      __yapOverlayGeometry?: { requestedStatus: string | null };
    }).__yapOverlayGeometry;

    for (const nextView of nextStates) {
      if (geometryState && typeof nextView.status === "string") {
        geometryState.requestedStatus = nextView.status;
      }
      window.dispatchEvent(new CustomEvent("yap-live-overlay-preview", { detail: nextView }));
      await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()));
      await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()));
    }
  }, states);
}

async function hoverIdleSensor(page: Page) {
  await page.mouse.move(130, 80);
  await page.mouse.move(130, 3);
}

async function startGeometrySampling(page: Page) {
  await page.evaluate(() => {
    type SamplerState = {
      active: boolean;
      rafId: number;
      requestedStatus: string | null;
      samples: GeometrySample[];
    };
    const browserWindow = window as typeof window & { __yapOverlayGeometry?: SamplerState };
    if (browserWindow.__yapOverlayGeometry?.active) {
      window.cancelAnimationFrame(browserWindow.__yapOverlayGeometry.rafId);
    }

    const state: SamplerState = {
      active: true,
      rafId: 0,
      requestedStatus: null,
      samples: [],
    };
    browserWindow.__yapOverlayGeometry = state;

    const toBox = (rect: DOMRect): Box => ({
      height: rect.height,
      width: rect.width,
      x: rect.x,
      y: rect.y,
    });

    const sample = (timestamp: number) => {
      if (!state.active) return;
      const root = document.querySelector<HTMLElement>("[data-testid='live-overlay-root']");
      const island = document.querySelector<HTMLElement>("[data-testid='live-overlay-island']");
      let transform: GeometrySample["transform"] = null;

      if (island) {
        const computedTransform = window.getComputedStyle(island).transform;
        try {
          const matrix = computedTransform === "none"
            ? new DOMMatrixReadOnly()
            : new DOMMatrixReadOnly(computedTransform);
          transform = { x: matrix.m41, y: matrix.m42 };
        } catch {
          transform = { x: Number.NaN, y: Number.NaN };
        }
      }

      state.samples.push({
        controls: island
          ? Array.from(island.querySelectorAll<HTMLElement>("button, [data-testid='live-waveform']")).map((element) => ({
              box: toBox(element.getBoundingClientRect()),
              name: element.getAttribute("aria-label") ?? element.getAttribute("data-testid") ?? element.tagName,
            }))
          : [],
        island: island ? toBox(island.getBoundingClientRect()) : null,
        phase: root?.dataset.overlayPhase ?? null,
        requestedStatus: state.requestedStatus,
        root: root ? toBox(root.getBoundingClientRect()) : null,
        surface: root?.dataset.overlaySurface ?? null,
        timestamp,
        transform,
      });
      state.rafId = window.requestAnimationFrame(sample);
    };

    state.rafId = window.requestAnimationFrame(sample);
  });
}

async function stopGeometrySampling(page: Page): Promise<GeometrySample[]> {
  await waitForAnimationFrames(page, 2);
  return page.evaluate(() => {
    const browserWindow = window as typeof window & {
      __yapOverlayGeometry?: {
        active: boolean;
        rafId: number;
        samples: GeometrySample[];
      };
    };
    const state = browserWindow.__yapOverlayGeometry;
    if (!state) return [];
    state.active = false;
    window.cancelAnimationFrame(state.rafId);
    return state.samples;
  });
}

async function waitForAnimationFrames(page: Page, count: number) {
  await page.evaluate(async (frameCount) => {
    for (let frame = 0; frame < frameCount; frame += 1) {
      await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()));
    }
  }, count);
}

async function waitForIslandSettled(locator: Locator) {
  await expect(locator).toBeVisible();
  await expect.poll(async () => Math.abs(await islandTranslationY(locator))).toBeLessThanOrEqual(0.25);
}

async function waitForStableWidth(locator: Locator) {
  return locator.evaluate(async (element) => {
    let previousWidth = element.getBoundingClientRect().width;
    let stableFrames = 0;

    for (let frame = 0; frame < 60; frame += 1) {
      await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()));
      const width = element.getBoundingClientRect().width;
      stableFrames = Math.abs(width - previousWidth) <= 0.1 ? stableFrames + 1 : 0;
      previousWidth = width;
      if (stableFrames >= 2) return width;
    }

    throw new Error("Island width did not settle within 60 animation frames");
  });
}

function expectStableTransitionGeometry(samples: GeometrySample[], context: string) {
  expect(samples.length, `${context} should include requestAnimationFrame samples`).toBeGreaterThan(0);
  const violations: string[] = [];

  samples.forEach((sample, index) => {
    const label = `${context} frame ${index} (${sample.surface ?? "unmounted"}/${sample.phase ?? "unmounted"})`;
    if (!sample.root) {
      violations.push(`${label}: root was unmounted`);
      return;
    }

    if (!boxIsFinite(sample.root)) violations.push(`${label}: root geometry was non-finite`);
    if (Math.abs(sample.root.width - rootFrame.width) > tolerance) {
      violations.push(`${label}: root width was ${sample.root.width}`);
    }
    if (Math.abs(sample.root.height - rootFrame.height) > tolerance) {
      violations.push(`${label}: root height was ${sample.root.height}`);
    }
    if (Math.abs(sample.root.y) > tolerance) violations.push(`${label}: root top was ${sample.root.y}`);

    const islandRequired = sample.surface !== "sensor" || (
      sample.requestedStatus !== null && sample.requestedStatus !== "idle"
    );
    if (!sample.island) {
      if (islandRequired) violations.push(`${label}: island was unmounted`);
      return;
    }

    if (!boxIsFinite(sample.island)) violations.push(`${label}: island geometry was non-finite`);
    if (!sample.transform || !Number.isFinite(sample.transform.x) || !Number.isFinite(sample.transform.y)) {
      violations.push(`${label}: island transform was non-finite`);
    }
    if (Math.abs(sample.island.height - rootFrame.height) > tolerance) {
      violations.push(`${label}: island height was ${sample.island.height}`);
    }
    if (Math.abs(centerX(sample.island) - centerX(sample.root)) > tolerance) {
      violations.push(`${label}: island center drifted ${centerX(sample.island) - centerX(sample.root)}px`);
    }
    if (sample.island.y < -rootFrame.height - tolerance || sample.island.y > tolerance) {
      violations.push(`${label}: island top was ${sample.island.y}`);
    }
    if (sample.island.x < sample.root.x - tolerance || rightOf(sample.island) > rightOf(sample.root) + tolerance) {
      violations.push(`${label}: island escaped the root horizontally`);
    }

    sample.controls.forEach(({ box, name }) => {
      if (!boxIsFinite(box)) violations.push(`${label}: ${name} geometry was non-finite`);
      if (name === "live-waveform" && Math.abs(centerX(box) - centerX(sample.root!)) > tolerance) {
        violations.push(`${label}: waveform center drifted ${centerX(box) - centerX(sample.root!)}px`);
      }
      if (
        box.x < sample.island!.x - tolerance ||
        box.y < sample.island!.y - tolerance ||
        rightOf(box) > rightOf(sample.island!) + tolerance ||
        bottomOf(box) > bottomOf(sample.island!) + tolerance
      ) {
        violations.push(`${label}: ${name} was clipped by the island`);
      }
    });

    for (let left = 0; left < sample.controls.length; left += 1) {
      for (let right = left + 1; right < sample.controls.length; right += 1) {
        const first = sample.controls[left];
        const second = sample.controls[right];
        if (boxesOverlap(first.box, second.box)) {
          violations.push(`${label}: ${first.name} overlapped ${second.name}`);
        }
      }
    }
  });

  expect(violations, `${context} geometry violations`).toEqual([]);
}

function expectProcessingWidthPreserved(samples: GeometrySample[], activeWidth: number) {
  const processingSamples = samples.filter((sample) => sample.requestedStatus === "saving");
  expect(processingSamples.length, "processing should include requestAnimationFrame samples").toBeGreaterThan(0);
  expect(
    processingSamples.filter((sample) =>
      !sample.island || Math.abs(sample.island.width - activeWidth) > tolerance,
    ),
    "processing must preserve the active island width on every sampled frame",
  ).toEqual([]);
}

function expectNoBlankInitializingFrames(samples: GeometrySample[]) {
  const armedSamples = samples.filter((sample) => sample.requestedStatus === "armed");
  const firstInitializingIndex = armedSamples.findIndex((sample) => sample.phase === "initializing");

  expect(armedSamples.length, "armed should be sampled as initializing").toBeGreaterThan(0);
  expect(
    firstInitializingIndex,
    "armed should produce a committed initializing sample",
  ).toBeGreaterThanOrEqual(0);

  const preCommitSamples = armedSamples.slice(0, firstInitializingIndex);
  expect(
    preCommitSamples.filter((sample) =>
      !sample.root || !sample.island || sample.phase !== "idle" || sample.surface !== "peek",
    ),
    "only the mounted peek frame may precede the armed commit",
  ).toEqual([]);
  expect(preCommitSamples.length, "armed may include at most one pre-commit sample").toBeLessThanOrEqual(1);

  const initializingSamples = armedSamples.slice(firstInitializingIndex);
  expect(
    initializingSamples.filter((sample) => !sample.root || !sample.island),
    "armed/initializing must never render a blank frame",
  ).toEqual([]);
  expect(
    initializingSamples.filter((sample) => sample.phase !== "initializing"),
    "armed samples must remain initializing after the commit",
  ).toEqual([]);
}

function isSettledAtTop(sample: GeometrySample) {
  return sample.island === null || Math.abs(sample.island.y) <= 0.25;
}

function isIntermediateIslandFrame(sample: GeometrySample) {
  return sample.island !== null && sample.island.y < -0.25 && sample.island.y > -rootFrame.height + 0.25;
}

async function expectFrame(locator: Locator, expected: { height: number; width: number }) {
  const box = await boxOf(locator);
  expect(box.width).toBeCloseTo(expected.width, tolerance);
  expect(box.height).toBeCloseTo(expected.height, tolerance);
}

async function expectNoBodyText(page: Page, text: string) {
  const found = await page.locator("body").evaluate((body, expectedText) => body.textContent?.includes(expectedText) ?? false, text);
  expect(found).toBe(false);
}

async function expectInside(parentLocator: Locator, childLocators: Locator[]) {
  const parent = await boxOf(parentLocator);
  for (const childLocator of childLocators) {
    const child = await boxOf(childLocator);
    expect(child.x).toBeGreaterThanOrEqual(parent.x - tolerance);
    expect(child.y).toBeGreaterThanOrEqual(parent.y - tolerance);
    expect(rightOf(child)).toBeLessThanOrEqual(rightOf(parent) + tolerance);
    expect(bottomOf(child)).toBeLessThanOrEqual(bottomOf(parent) + tolerance);
  }
}

async function expectNoHorizontalOverlap(leftLocator: Locator, rightLocator: Locator) {
  const left = await boxOf(leftLocator);
  const right = await boxOf(rightLocator);
  expect(rightOf(left)).toBeLessThanOrEqual(right.x + tolerance);
}

async function expectNoClippedChildren(parentLocator: Locator) {
  const parent = await boxOf(parentLocator);
  const clipped = await parentLocator.evaluate((parentElement) => {
    const parentBox = parentElement.getBoundingClientRect();
    return Array.from(parentElement.querySelectorAll("button, [data-testid='live-waveform']"))
      .map((element) => {
        const box = element.getBoundingClientRect();
        return {
          bounds: { bottom: box.bottom, left: box.left, right: box.right, top: box.top },
          name: element.getAttribute("aria-label") ?? element.getAttribute("data-testid") ?? element.tagName,
          parent: { bottom: parentBox.bottom, left: parentBox.left, right: parentBox.right, top: parentBox.top },
        };
      })
      .filter(({ bounds, parent: boundsParent }) =>
        bounds.bottom > boundsParent.bottom + 1 ||
        bounds.left < boundsParent.left - 1 ||
        bounds.right > boundsParent.right + 1 ||
        bounds.top < boundsParent.top - 1,
      );
  });
  expect(clipped).toEqual([]);
  expect(parent.width).toBeGreaterThan(0);
  expect(parent.height).toBeGreaterThan(0);
}

async function expectIslandTranslationY(locator: Locator, expectedY: number) {
  expect(await islandTranslationY(locator)).toBeCloseTo(expectedY, tolerance);
}

async function islandTranslationY(locator: Locator) {
  return locator.evaluate((element) => {
    const transform = window.getComputedStyle(element).transform;
    if (transform === "none") return 0;
    return new DOMMatrixReadOnly(transform).m42;
  });
}

async function waveformBarHeights(locator: Locator) {
  return locator.locator("span").evaluateAll((bars) =>
    bars.map((bar) => Math.round(bar.getBoundingClientRect().height * 100) / 100),
  );
}

async function boxOf(locator: Locator): Promise<Box> {
  const box = await locator.boundingBox();
  expect(box).not.toBeNull();
  return box!;
}

function boxIsFinite(box: Box) {
  return [box.height, box.width, box.x, box.y].every(Number.isFinite);
}

function boxesOverlap(first: Box, second: Box) {
  return Math.min(rightOf(first), rightOf(second)) - Math.max(first.x, second.x) > tolerance &&
    Math.min(bottomOf(first), bottomOf(second)) - Math.max(first.y, second.y) > tolerance;
}

function centerX(box: Box) {
  return box.x + box.width / 2;
}

function rightOf(box: Box) {
  return box.x + box.width;
}

function bottomOf(box: Box) {
  return box.y + box.height;
}

import { expect, type Locator, type Page, test } from "@playwright/test";

type Box = {
  height: number;
  width: number;
  x: number;
  y: number;
};

const previewUrl = "/?window=live-overlay&preview=live-overlay";
const tolerance = 1;

test.describe.configure({ timeout: 45_000 });

test("live overlay hidden idle state renders no sensor", async ({ page }) => {
  await page.setViewportSize({ width: 260, height: 90 });
  await page.goto(`${previewUrl}&visibility=hidden&status=idle`);

  await expect(page.getByTestId("live-overlay-root")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Start dictating" })).toHaveCount(0);
});

test("live overlay respects reduced motion", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: 260, height: 90 });
  await page.goto(previewUrl);

  const root = page.getByTestId("live-overlay-root");
  await hoverIdleSensor(page);
  await expect(root).toHaveAttribute("data-overlay-surface", "peek");

  const island = page.getByTestId("live-overlay-island");
  await expectIslandTranslationY(island, 0);
  await expect(island).toHaveCSS("transition-duration", "0s");

  await setLiveView(page, { captureMode: "pushToTalk", level: 0.12, route: "localFallback", status: "speaking" });
  const waveform = page.getByTestId("live-waveform");
  const before = await waveformBarHeights(waveform);
  await page.waitForTimeout(180);
  await expect(waveform).toBeVisible();
  await expect(waveform.locator("span").first()).toHaveCSS("transition-duration", "0s");
  expect(await waveformBarHeights(waveform)).toEqual(before);
});

test("live overlay state machine keeps the island compact and collision-free", async ({ page }) => {
  await page.setViewportSize({ width: 260, height: 90 });
  await page.goto(previewUrl);

  const root = page.getByTestId("live-overlay-root");

  await expect(root).toHaveAttribute("data-overlay-surface", "sensor");
  await expectFrame(root, { height: 8, width: 260 });

  await hoverIdleSensor(page);
  await expect(root).toHaveAttribute("data-overlay-surface", "peek");
  await expect(page.getByRole("button", { name: "Start dictating" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Open scratch" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Open transform" })).toBeVisible();
  await waitForIslandMotion();
  await expectIslandTranslationY(page.getByTestId("live-overlay-island"), 0);
  await expectNoBodyText(page, "Dictate");
  await expectFrame(root, { height: 40, width: 260 });
  await expectFrame(page.getByTestId("live-overlay-island"), { height: 40, width: 150 });
  await expectInside(root, [
    page.getByRole("button", { name: "Start dictating" }),
    page.getByRole("button", { name: "Open scratch" }),
    page.getByRole("button", { name: "Open transform" }),
  ]);
  await expectNoClippedChildren(root);
  await expect(root).toHaveScreenshot("live-overlay-hover.png", {
    animations: "disabled",
    maxDiffPixelRatio: 0.04,
  });

  await page.mouse.move(130, 70);
  await expect(root).toHaveAttribute("data-overlay-surface", "peek");
  await page.mouse.move(75, 3);
  await expect(root).toHaveAttribute("data-overlay-surface", "peek");
  await waitForIslandMotion();
  await expectIslandTranslationY(page.getByTestId("live-overlay-island"), 0);
  await page.mouse.move(75, 70);
  await waitForRetract();
  await expect(root).toHaveAttribute("data-overlay-surface", "sensor");
  await expectFrame(root, { height: 8, width: 260 });
  await hoverIdleSensor(page);
  await expect(root).toHaveAttribute("data-overlay-surface", "peek");
  await waitForIslandMotion();

  await setLiveView(page, { captureMode: "pushToTalk", level: 0, route: "localFallback", status: "armed" });
  await expect(root).toHaveAttribute("data-overlay-phase", "recording");
  await waitForIslandMotion();
  await expectFrame(root, { height: 40, width: 260 });
  await expectFrame(page.getByTestId("live-overlay-island"), { height: 40, width: 112 });
  await expectNoClippedChildren(root);

  const holdFrame = await boxOf(root);
  await setLiveView(page, { captureMode: "pushToTalk", level: 0.72, route: "localFallback", status: "speaking" });
  await expect(root).toHaveAttribute("data-overlay-phase", "recording");
  await expectFrame(root, { height: holdFrame.height, width: holdFrame.width });
  await expectInside(root, [page.getByTestId("live-waveform")]);
  await expect(page.getByRole("button", { name: "Cancel recording" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Finish recording" })).toHaveCount(0);
  await expectNoClippedChildren(root);

  await setLiveView(page, { captureMode: "toggle", level: 0.84, route: "localFallback", status: "speaking" });
  await expect(root).toHaveAttribute("data-overlay-phase", "recording");
  await expectFrame(root, { height: 40, width: 260 });
  await expectFrame(page.getByTestId("live-overlay-island"), { height: 40, width: 112 });

  const waveform = page.getByTestId("live-waveform");
  const finish = page.getByRole("button", { name: "Finish recording" });
  await expect(page.getByRole("button", { name: "Cancel recording" })).toHaveCount(0);
  await expect(finish).toBeVisible();
  await expectInside(root, [waveform, finish]);
  await expectNoHorizontalOverlap(waveform, finish);
  await expectNoClippedChildren(root);

  await finish.click();
  await expect(root).toHaveAttribute("data-overlay-phase", "processing");
  await expect(page.getByRole("button", { name: "Cancel recording" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Finish recording" })).toHaveCount(0);

  await setLiveView(page, { activeCaptureMode: "toggle", captureMode: "pushToTalk", level: 0.84, route: "localFallback", status: "speaking" });
  await finish.click();
  await expect(root).toHaveAttribute("data-overlay-phase", "processing");
  await expect(page.getByRole("button", { name: "Cancel recording" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Finish recording" })).toHaveCount(0);

  await setLiveView(page, { activeCaptureMode: "pushToTalk", captureMode: "toggle", level: 0.72, route: "localFallback", status: "speaking" });
  await expectFrame(root, { height: 40, width: 260 });
  await expectFrame(page.getByTestId("live-overlay-island"), { height: 40, width: 112 });
  await expect(page.getByRole("button", { name: "Cancel recording" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Finish recording" })).toHaveCount(0);

  await setLiveView(page, { activeCaptureMode: "toggle", captureMode: "pushToTalk", level: 0.84, route: "localFallback", status: "speaking" });
  await expectFrame(root, { height: 40, width: 260 });
  await expectFrame(page.getByTestId("live-overlay-island"), { height: 40, width: 112 });
  await expect(page.getByRole("button", { name: "Cancel recording" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Finish recording" })).toBeVisible();

  const handsFreeFrame = await boxOf(root);
  await setLiveView(page, { captureMode: "toggle", level: 0, route: "localFallback", status: "saving" });
  await expect(root).toHaveAttribute("data-overlay-phase", "processing");
  await expectFrame(root, { height: handsFreeFrame.height, width: handsFreeFrame.width });
  await expectInside(root, [page.getByTestId("live-overlay-island")]);
  await expectNoClippedChildren(root);

  await setLiveView(page, { captureMode: "toggle", finalText: "Saved dictation", level: 0, route: "none", status: "idle" });
  await expect(root).toHaveAttribute("data-overlay-surface", "success");
  await waitForIslandMotion();
  await expectFrame(root, { height: 40, width: 260 });
  await expectFrame(page.getByTestId("live-overlay-island"), { height: 40, width: 168 });
  await expect(page.getByText("Saved")).toBeVisible();
  await expectNoClippedChildren(root);

  await setLiveView(page, { error: "Mic denied", finalText: undefined, level: 0, route: "blocked", status: "blocked" });
  await expect(root).toHaveAttribute("data-overlay-phase", "feedback");
  await expectFrame(root, { height: 40, width: 180 });
  await expectInside(root, [page.getByRole("button", { name: "Retry dictation" })]);
  await expectNoClippedChildren(root);
});

test("live overlay tolerates rapid state churn without active-frame jitter", async ({ page }) => {
  await page.setViewportSize({ width: 260, height: 90 });
  await page.goto(previewUrl);

  const root = page.getByTestId("live-overlay-root");
  await hoverIdleSensor(page);
  await expect(root).toHaveAttribute("data-overlay-surface", "peek");
  await waitForIslandMotion();

  const activeStates = [
    { activeCaptureMode: "pushToTalk", captureMode: "toggle", level: 0.12, route: "localFallback", status: "armed" },
    { activeCaptureMode: "pushToTalk", captureMode: "toggle", level: 0.72, route: "localFallback", status: "speaking" },
    { activeCaptureMode: "toggle", captureMode: "pushToTalk", level: 0.84, route: "localFallback", status: "speaking" },
    { activeCaptureMode: "toggle", captureMode: "pushToTalk", level: 0, route: "localFallback", status: "saving" },
    { activeCaptureMode: "pushToTalk", captureMode: "toggle", level: 0.4, route: "localFallback", status: "listening" },
    { activeCaptureMode: "toggle", captureMode: "pushToTalk", level: 0.9, route: "localFallback", status: "speaking" },
  ];

  for (const state of activeStates) {
    await setLiveView(page, state);
    await expectFrame(root, { height: 40, width: 260 });
    await expectFrame(page.getByTestId("live-overlay-island"), { height: 40, width: 112 });
    await expectNoClippedChildren(root);
  }

  await setLiveView(page, { finalText: "Saved dictation", level: 0, route: "none", status: "idle" });
  await expect(root).toHaveAttribute("data-overlay-surface", "success");
  await waitForIslandMotion();
  await expectFrame(root, { height: 40, width: 260 });
  await expectFrame(page.getByTestId("live-overlay-island"), { height: 40, width: 168 });
  await expectNoClippedChildren(root);
});

async function setLiveView(page: Page, detail: Record<string, unknown>) {
  await page.evaluate((nextView) => {
    window.dispatchEvent(new CustomEvent("yap-live-overlay-preview", { detail: nextView }));
  }, detail);
}

async function hoverIdleSensor(page: Page) {
  await page.mouse.move(130, 80);
  await page.getByTestId("live-overlay-root").hover({ position: { x: 130, y: 3 } });
}

async function waitForIslandMotion() {
  await new Promise((resolve) => setTimeout(resolve, 220));
}

async function waitForRetract() {
  await new Promise((resolve) => setTimeout(resolve, 220));
}

async function expectFrame(locator: Locator, expected: { height: number; width: number }) {
  const box = await boxOf(locator);
  expect(box.width).toBeCloseTo(expected.width, tolerance);
  expect(box.height).toBeCloseTo(expected.height, tolerance);
}

async function expectNoBodyText(page: Page, text: string) {
  const found = await page.locator("body").evaluate((body, text) => body.textContent?.includes(text) ?? false, text);
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

async function expectNoClippedChildren(rootLocator: Locator) {
  const root = await boxOf(rootLocator);
  const clipped = await rootLocator.evaluate((root) => {
    const rootBox = root.getBoundingClientRect();
    return Array.from(root.querySelectorAll("button, [data-testid='live-waveform'], [data-testid='live-overlay-island']"))
      .map((element) => {
        const box = element.getBoundingClientRect();
        return {
          bounds: { bottom: box.bottom, left: box.left, right: box.right, top: box.top },
          name: element.getAttribute("aria-label") ?? element.getAttribute("data-testid") ?? element.tagName,
          root: { bottom: rootBox.bottom, left: rootBox.left, right: rootBox.right, top: rootBox.top },
        };
      })
      .filter(({ bounds, root }) =>
        bounds.bottom > root.bottom + 1 ||
        bounds.left < root.left - 1 ||
        bounds.right > root.right + 1 ||
        bounds.top < root.top - 1,
      );
  });
  expect(clipped).toEqual([]);
  expect(root.width).toBeGreaterThan(0);
  expect(root.height).toBeGreaterThan(0);
}

async function expectIslandTranslationY(locator: Locator, expectedY: number) {
  const y = await locator.evaluate((element) => {
    const transform = window.getComputedStyle(element).transform;
    if (transform === "none") return 0;
    return new DOMMatrixReadOnly(transform).m42;
  });
  expect(y).toBeCloseTo(expectedY, tolerance);
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

function rightOf(box: Box) {
  return box.x + box.width;
}

function bottomOf(box: Box) {
  return box.y + box.height;
}

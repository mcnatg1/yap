import { expect, test } from "@playwright/test";

test("live overlay hover reveals attached island actions", async ({ page }) => {
  await page.setViewportSize({ width: 260, height: 90 });
  await page.goto("/?window=live-overlay");

  await page.mouse.move(130, 3);

  await expect(page.getByRole("button", { name: "Start dictating" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Open scratch" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Open transform" })).toBeVisible();
  await expect(page.getByText(/^Dictate/)).toHaveCount(0);
  await expect(page.getByText(/^\d+:\d{2}$/)).toHaveCount(0);
  await expect(page.locator(".live-overlay-root").first()).toHaveScreenshot(
    "live-overlay-hover.png",
    {
      animations: "disabled",
      maxDiffPixelRatio: 0.04,
    },
  );
});

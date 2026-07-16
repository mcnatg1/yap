import { expect, test } from "@playwright/test";


test("main app renders the home surface", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByText("Welcome back")).toBeVisible();
  await expect(page.getByRole("button", { name: "Home" })).toBeVisible();
});


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

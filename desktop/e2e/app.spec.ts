import { expect, test } from "@playwright/test";

test("main app renders the home surface", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByText("Welcome back")).toBeVisible();
  await expect(page.getByRole("button", { name: "Home" })).toBeVisible();
});

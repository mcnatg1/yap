import { expect, test } from "@playwright/test";

test("a rendered Polish owner rejects stale text and locks draft-changing controls during save", async ({ page }) => {
  await page.route("http://127.0.0.1:11434/api/chat", async (route) => {
    await route.fulfill({
      contentType: "application/json",
      json: { message: { content: "Owned polished draft" } },
    });
  });
  await page.goto("/tests/fixtures/polish-panel-owner.html");

  await page.getByRole("button", { name: "Polish", exact: true }).click();
  await expect(page.getByText("Owned polished draft", { exact: true })).toBeVisible();

  await page.getByRole("button", { name: "Mutate draft" }).click();
  const staleSave = page.getByRole("button", { name: "Save", exact: true });
  if (await staleSave.count()) await staleSave.click();
  await expect(page.getByLabel("Save calls")).toHaveText("0");

  await page.getByRole("button", { name: "Polish", exact: true }).click();
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByLabel("Save calls")).toHaveText("1");
  await expect(page.getByRole("radio", { name: "Clean" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Polish again" })).toBeDisabled();
});

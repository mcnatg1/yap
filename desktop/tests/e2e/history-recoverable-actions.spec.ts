import { expect, test } from "@playwright/test";

test("a legacy recoverable row can always be hidden", async ({ page }) => {
  const name = "live-s-18f001122334455-2a-hide";
  const recordingsDir = "C:\\Users\\tester\\AppData\\Local\\Yap\\live-recordings";
  await page.addInitScript(({ name, recordingsDir }) => {
    localStorage.setItem("yap.transcriptHistory.v1", JSON.stringify([{
      createdAt: "2026-07-11T12:01:00.000Z",
      name,
      outputPath: `${recordingsDir}\\${name}.wav.part`,
      recoveryState: "recoverable",
      sourcePath: `${recordingsDir}\\${name}.wav.part`,
      warning: "Capture stopped before publication.",
    }]));
  }, { name, recordingsDir });

  await page.goto("/");
  const row = page.locator("tr[data-history-entry-row]").filter({ hasText: "Partial" });
  await row.getByRole("button", { name: `Actions for ${name}` }).click();
  const menu = page.getByRole("menu");

  await expect(menu.getByRole("menuitem")).toHaveText(["Recover", "Hide", "Delete"]);
  await menu.getByRole("menuitem", { name: "Hide" }).click();
  await expect(row).toHaveCount(0);
});

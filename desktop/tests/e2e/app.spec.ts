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

test("history keeps committed review actions separate from recoverable capture actions", async ({ context, page }) => {
  const committedName = "live-s-18f001122334455-2a-0";
  const recoverableName = "live-s-18f001122334455-2a-1";
  const recordingsDir = "C:\\Users\\tester\\AppData\\Local\\Yap\\live-recordings";
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.addInitScript(({ committedName, recoverableName, recordingsDir }) => {
    localStorage.setItem("yap.transcriptHistory.v1", JSON.stringify([
      {
        captureCommitPath: `${recordingsDir}\\${committedName}.commit.json`,
        createdAt: "2026-07-11T12:00:00.000Z",
        name: committedName,
        outputPath: `${recordingsDir}\\${committedName}.txt`,
        sourcePath: `${recordingsDir}\\${committedName}.wav`,
      },
      {
        createdAt: "2026-07-11T12:01:00.000Z",
        name: recoverableName,
        outputPath: `${recordingsDir}\\${recoverableName}.wav.part`,
        recoveryState: "recoverable",
        sourcePath: `${recordingsDir}\\${recoverableName}.wav.part`,
        warning: "Capture stopped before publication.",
      },
    ]));
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
  await expect(menu.getByRole("menuitem")).toHaveText(["Recover", "Delete"]);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator('[data-slot="card"]')).toHaveCount(cardCount);
});

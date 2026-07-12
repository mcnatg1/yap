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

test("history body search paints indexing before an empty result", async ({ page }) => {
  await page.addInitScript(() => {
    const recordingsDir = "C:\\Users\\tester\\AppData\\Local\\Yap\\live-recordings";
    localStorage.setItem("yap.transcriptHistory.v1", JSON.stringify(
      Array.from({ length: 100 }, (_, index) => ({
        createdAt: new Date(Date.UTC(2026, 6, 11, 12, index)).toISOString(),
        name: `clip-${String(index).padStart(3, "0")}`,
        outputPath: `${recordingsDir}\\clip-${index}.txt`,
        sourcePath: `${recordingsDir}\\clip-${index}.wav`,
      })),
    ));
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Search past transcripts" }).click();

  const firstFrame = await page.getByRole("textbox", { name: "Search past transcripts" })
    .evaluate((input: HTMLInputElement) => {
      const valueSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      valueSetter?.call(input, "body-only-phrase");
      input.dispatchEvent(new Event("input", { bubbles: true }));
      return input.ownerDocument.body.innerText;
    });

  expect(firstFrame).toContain("Searching transcript text...");
  expect(firstFrame).not.toContain("No recordings match that search.");
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

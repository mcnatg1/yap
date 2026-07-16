import { expect, test } from "@playwright/test";

import { installQueuedServerBridge } from "./app-server-bridge";


for (const scenario of [
  { label: "server unset", state: "not_set" },
  { label: "offline server", state: "offline" },
] as const) {
  test(`${scenario.label} keeps durable imported jobs queued without local transcription`, async ({ page }) => {
    await installQueuedServerBridge(page, scenario.state);
    await page.goto("/");
    await page.getByRole("button", { name: "Transcribe", exact: true }).click();

    const name = `${scenario.state.replace("_", "-")}-interview.wav`;
    await expect(page.getByRole("button", { name: `Select ${name}` })).toBeVisible();
    await expect(page.getByText("Waiting in queue", { exact: true })).toBeVisible();
    await expect(page.getByRole("paragraph").filter({ hasText:
      "Queued for your organization's transcription server. It will start when Yap connects.",
    })).toBeVisible();
    await expect(page.getByText("Transcribing", { exact: true })).toHaveCount(0);
    expect(await page.evaluate(() => localStorage.getItem("yap.recordingQueue.v1"))).toBeNull();

    await page.getByRole("button", { name: "Open settings" }).click();
    const settings = page.getByRole("dialog", { name: "Settings" });
    await settings.getByRole("button", { name: "System", exact: true }).click();
    await expect(settings.getByText("Local fallback", { exact: true }).locator("..")).toContainText("Ready");

    const calls = await page.evaluate(() =>
      (globalThis as unknown as { __queuedServerBoundaryTest: { calls: string[] } })
        .__queuedServerBoundaryTest.calls,
    );
    expect(calls).not.toContain("start_transcribe");
    expect(calls).not.toContain("fallback_model_install");
    expect(calls).toContain("history_catalog");
    expect(calls.filter((command) =>
      command.startsWith("recording_job") &&
      command !== "recording_jobs_snapshot"
    )).toEqual([]);
  });
}

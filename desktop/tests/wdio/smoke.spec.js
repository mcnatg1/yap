describe("Yap desktop shell", () => {
  it("opens the main window and exposes the WDIO Tauri bridge", async () => {
    await browser.pause(500);

    expect(typeof browser.tauri.execute).toBe("function");
    const bridge = await browser.execute(() => ({
      hasTauriInternals: typeof window.__TAURI_INTERNALS__?.invoke === "function",
      hasWdioTauri: typeof window.wdioTauri?.execute === "function",
    }));
    expect(bridge.hasTauriInternals).toBe(true);
    expect(bridge.hasWdioTauri).toBe(true);

    const heading = await $("h1");
    await heading.waitForDisplayed();
    expect(await heading.getText()).toContain("Welcome back");
  });
});

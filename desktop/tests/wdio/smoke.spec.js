describe("Yap desktop shell", () => {
  it("opens the main window and exposes the WDIO Tauri bridge", async () => {
    await browser.tauri.switchWindow("main");
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

  it("keeps representative native command families registered", async () => {
    await browser.tauri.switchWindow("main");

    const commands = await browser.tauri.execute(async ({ core }) => ({
      live: await core.invoke("live_status"),
      recordings: await core.invoke("list_saved_live_sessions"),
      server: await core.invoke("server_connection_status"),
      setup: await core.invoke("setup_status"),
    }));

    expect(typeof commands.setup.engineReady).toBe("boolean");
    expect(typeof commands.setup.engineStatus).toBe("string");
    expect([
      "not_set",
      "connecting",
      "ready",
      "offline",
      "sign_in_required",
      "retrying",
      "disabled",
    ]).toContain(commands.server);
    expect(typeof commands.live.status).toBe("string");
    expect(typeof commands.live.visibility).toBe("string");
    expect(Array.isArray(commands.recordings.sessions)).toBe(true);
    expect(Array.isArray(commands.recordings.maintenanceWarnings)).toBe(true);
  });
});

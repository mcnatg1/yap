describe("Yap live overlay window", () => {
  it("opens as a compact system overlay and refuses direct close", async () => {
    await browser.tauri.execute(({ core }) => core.invoke("start_live_session", { activeCaptureMode: "toggle" }));
    await browser.waitUntil(async () => (await browser.tauri.listWindows()).includes("live-overlay"));

    const overlay = await browser.tauri.execute(async ({ core }) => {
      const label = "live-overlay";
      const inner = await core.invoke("plugin:window|inner_size", { label });
      const outer = await core.invoke("plugin:window|outer_size", { label });
      return {
        closable: await core.invoke("plugin:window|is_closable", { label }),
        focused: await core.invoke("plugin:window|is_focused", { label }),
        inner,
        outer,
        visible: await core.invoke("plugin:window|is_visible", { label }),
      };
    });
    expect(overlay.visible).toBe(true);
    expect(overlay.focused).toBe(false);
    expect(overlay.closable).toBe(false);
    expect(overlay.inner.width).toBeLessThanOrEqual(180);
    expect(overlay.inner.height).toBeLessThanOrEqual(60);
    expect(overlay.outer.width).toBeLessThanOrEqual(220);
    expect(overlay.outer.height).toBeLessThanOrEqual(80);

    await browser.tauri.execute(({ core }) => core.invoke("stop_live_session"));
  });
});

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
    ]).toContain(commands.server.state);
    expect(typeof commands.server.capabilities.batchJobs).toBe("boolean");
    expect(typeof commands.server.capabilities.liveStreaming).toBe("boolean");
    expect(typeof commands.server.capabilities.jobStatus).toBe("boolean");
    expect(commands.server.checkedAtMs === null || typeof commands.server.checkedAtMs === "number").toBe(true);
    expect(commands.server.retryAtMs === null || typeof commands.server.retryAtMs === "number").toBe(true);
    expect(typeof commands.live.status).toBe("string");
    expect(typeof commands.live.visibility).toBe("string");
    expect(Array.isArray(commands.recordings.sessions)).toBe(true);
    expect(Array.isArray(commands.recordings.maintenanceWarnings)).toBe(true);
  });

  it("reports an enforced CSP violation for a disallowed remote script", async () => {
    await browser.tauri.switchWindow("main");
    const violation = await browser.executeAsync((done) => {
      const probeUrl = "https://example.invalid/yap-csp-probe.js";
      const script = document.createElement("script");
      let settled = false;

      const finish = (result) => {
        if (settled) return;
        settled = true;
        window.clearTimeout(timeout);
        document.removeEventListener("securitypolicyviolation", onViolation);
        script.remove();
        done(result);
      };
      const onViolation = (event) => {
        const blockedURI = String(event.blockedURI ?? "");
        if (!blockedURI.includes("yap-csp-probe")) return;
        finish({
          blockedURI,
          disposition: event.disposition,
          effectiveDirective: event.effectiveDirective,
        });
      };
      const timeout = window.setTimeout(
        () => finish({ error: "No securitypolicyviolation event was emitted." }),
        3_000,
      );

      document.addEventListener("securitypolicyviolation", onViolation);
      script.src = probeUrl;
      document.head.append(script);
    });

    expect(violation.error).toBeUndefined();
    expect(violation.blockedURI).toContain("yap-csp-probe.js");
    expect(["script-src", "script-src-elem"]).toContain(violation.effectiveDirective);
    expect(violation.disposition).toBe("enforce");
  });
});

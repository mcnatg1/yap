import { createServer } from "node:net";

import { describe, expect, test } from "vitest";

import {
  allocateLoopbackPort,
  parsePlaywrightPort,
  selectPlaywrightPort,
} from "./playwright-port.mjs";

describe("Playwright port ownership", () => {
  test("accepts an explicit valid port", () => {
    expect(parsePlaywrightPort("49152")).toBe(49_152);
  });

  test.each([undefined, "", "0", "65536", "3.5", "not-a-port"])(
    "rejects invalid explicit port %s",
    (value) => {
      expect(() => parsePlaywrightPort(value)).toThrow(/YAP_PLAYWRIGHT_PORT/);
    },
  );

  test("uses an explicit port without allocating another", async () => {
    const allocate = () => Promise.reject(new Error("must not allocate"));
    await expect(selectPlaywrightPort("49153", allocate)).resolves.toBe(49_153);
  });

  test("allocates and releases an available loopback port when unset", async () => {
    const port = await selectPlaywrightPort(undefined, allocateLoopbackPort);
    expect(port).toBeGreaterThan(0);
    expect(port).toBeLessThanOrEqual(65_535);

    const server = createServer();
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen({ host: "127.0.0.1", port, exclusive: true }, resolve);
    });
    await new Promise((resolve, reject) => {
      server.close((error) => (error ? reject(error) : resolve()));
    });
  });
});

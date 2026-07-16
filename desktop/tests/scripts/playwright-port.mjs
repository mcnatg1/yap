import { createServer } from "node:net";

export function parsePlaywrightPort(value) {
  if (typeof value !== "string" || !/^\d+$/.test(value)) {
    throw new Error("YAP_PLAYWRIGHT_PORT must be an integer TCP port.");
  }
  const port = Number(value);
  if (port < 1 || port > 65_535) {
    throw new Error("YAP_PLAYWRIGHT_PORT must be between 1 and 65535.");
  }
  return port;
}

export function allocateLoopbackPort() {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen({ host: "127.0.0.1", port: 0, exclusive: true }, () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        server.close(() => reject(new Error("Unable to allocate a loopback TCP port.")));
        return;
      }
      const { port } = address;
      server.close((error) => (error ? reject(error) : resolve(port)));
    });
  });
}

export async function selectPlaywrightPort(value, allocate = allocateLoopbackPort) {
  return value === undefined ? allocate() : parsePlaywrightPort(value);
}

import { invoke } from "@tauri-apps/api/core";

import type { ServerConnectionState } from "@/lib/app-types";

export function serverConnectionStatus(): Promise<ServerConnectionState> {
  return invoke<ServerConnectionState>("server_connection_status");
}

import { invoke } from "@tauri-apps/api/core";

import type { ServerConnectionState } from "@/lib/app-types";

export const SERVER_SETTINGS_SCHEMA_VERSION = 1 as const;

export type ServerSettings = {
  schemaVersion: typeof SERVER_SETTINGS_SCHEMA_VERSION;
  enabled: boolean;
  baseUrl: string | null;
};

export function serverConnectionStatus(): Promise<ServerConnectionState> {
  return invoke<ServerConnectionState>("server_connection_status");
}

export function serverSettings(): Promise<ServerSettings> {
  return invoke<ServerSettings>("server_settings");
}

export function saveServerSettings(settings: ServerSettings): Promise<ServerSettings> {
  return invoke<ServerSettings>("set_server_settings", { settings });
}

export function testServerConnection(): Promise<ServerConnectionState> {
  return serverConnectionStatus();
}

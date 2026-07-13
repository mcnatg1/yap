import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  ServerConnectionState,
} from "@/lib/app-types";

export const SERVER_SETTINGS_SCHEMA_VERSION = 1 as const;

export type ServerSettings = {
  schemaVersion: typeof SERVER_SETTINGS_SCHEMA_VERSION;
  enabled: boolean;
  baseUrl: string | null;
};

export type ServerCapabilities = {
  batchJobs: boolean;
  liveStreaming: boolean;
  jobStatus: boolean;
};

export type ServerConnectionSnapshot = {
  state: ServerConnectionState;
  checkedAtMs: number | null;
  retryAtMs: number | null;
  apiVersion: string | null;
  capabilities: ServerCapabilities;
  errorCode: string | null;
};

export function serverCanRouteImportedRecording(
  snapshot: ServerConnectionSnapshot | null | undefined,
) {
  return snapshot?.state === "ready" && snapshot.capabilities?.batchJobs === true;
}

export function serverCanRouteLive(snapshot: ServerConnectionSnapshot | null | undefined) {
  return snapshot?.state === "ready" && snapshot.capabilities?.liveStreaming === true;
}

export function serverConnectionStatus(): Promise<ServerConnectionSnapshot> {
  return invoke<ServerConnectionSnapshot>("server_connection_status");
}

export function refreshServerConnection(): Promise<ServerConnectionSnapshot> {
  return invoke<ServerConnectionSnapshot>("refresh_server_connection");
}

export async function listenServerConnection(
  onUpdate: (snapshot: ServerConnectionSnapshot) => void,
): Promise<UnlistenFn> {
  if (!isTauri()) return () => undefined;
  return listen<ServerConnectionSnapshot>("server-connection", (event) => onUpdate(event.payload));
}

export function serverSettings(): Promise<ServerSettings> {
  return invoke<ServerSettings>("server_settings");
}

export function saveServerSettings(settings: ServerSettings): Promise<ServerSettings> {
  return invoke<ServerSettings>("set_server_settings", { settings });
}

export async function testServerConnection(): Promise<ServerConnectionState> {
  return (await refreshServerConnection()).state;
}

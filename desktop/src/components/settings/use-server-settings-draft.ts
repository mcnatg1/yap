import { useEffect, useState } from "react";

import {
  projectServerConnectionTestMessage,
  saveServerSettings,
  serverSettings,
  testServerConnection,
  type ServerSettings,
} from "@/settings";

export type ServerSettingsDraftController = {
  enabled: boolean;
  error: string;
  notice: string;
  pending: boolean;
  save: () => Promise<ServerSettings | null>;
  setUrl: (url: string) => void;
  testConnection: () => Promise<void>;
  toggleEnabled: () => void;
  url: string;
};

function terseSettingsError(error: unknown, fallback: string) {
  const message = typeof error === "string"
    ? error
    : error instanceof Error
      ? error.message
      : "";
  return message.trim().split(/\r?\n/, 1)[0]?.slice(0, 160) || fallback;
}

export function useServerSettingsDraft(open: boolean): ServerSettingsDraftController {
  const [url, setUrl] = useState("");
  const [enabled, setEnabled] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  useEffect(() => {
    if (!open) return;
    let active = true;
    setPending(true);
    setError("");
    setNotice("");
    void serverSettings()
      .then((settings) => {
        if (!active) return;
        setUrl(settings.baseUrl ?? "");
        setEnabled(settings.enabled);
      })
      .catch((loadError: unknown) => {
        if (active) setError(terseSettingsError(loadError, "Could not load server settings."));
      })
      .finally(() => {
        if (active) setPending(false);
      });
    return () => {
      active = false;
    };
  }, [open]);

  async function save() {
    setPending(true);
    setError("");
    setNotice("");
    try {
      const saved = await saveServerSettings({
        schemaVersion: 1,
        enabled,
        baseUrl: url.trim() || null,
      });
      setUrl(saved.baseUrl ?? "");
      setEnabled(saved.enabled);
      setNotice("Saved.");
      return saved;
    } catch (saveError) {
      setError(terseSettingsError(saveError, "Could not save server settings."));
      return null;
    } finally {
      setPending(false);
    }
  }

  async function testConnection() {
    const saved = await save();
    if (!saved || !saved.enabled) return;
    setPending(true);
    setNotice("Checking connection.");
    try {
      setNotice(projectServerConnectionTestMessage(await testServerConnection()));
    } catch (connectionError) {
      setError(terseSettingsError(connectionError, "Connection check failed."));
      setNotice("");
    } finally {
      setPending(false);
    }
  }

  return {
    enabled,
    error,
    notice,
    pending,
    save,
    setUrl,
    testConnection,
    toggleEnabled: () => setEnabled((current) => !current),
    url,
  };
}

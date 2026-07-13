import { isTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";

import {
  serverConnectionLabel,
} from "@/lib/app-types";
import {
  listenServerConnection,
  refreshServerConnection,
  serverConnectionStatus,
  type ServerConnectionSnapshot,
} from "@/server";

const initialServerSnapshot: ServerConnectionSnapshot = {
  state: "not_set",
  checkedAtMs: null,
  retryAtMs: null,
  apiVersion: null,
  capabilities: {
    batchJobs: false,
    liveStreaming: false,
    jobStatus: false,
  },
  errorCode: null,
};

export function useServerConnection() {
  const [serverSnapshot, setServerSnapshot] = useState<ServerConnectionSnapshot>(
    initialServerSnapshot,
  );
  const eventVersionRef = useRef(0);
  const serverState = serverSnapshot.state;
  const serverLabel = serverConnectionLabel(serverState);

  useEffect(() => {
    if (!isTauri()) return;

    let cancelled = false;
    let unlisten: (() => void) | undefined;

    void listenServerConnection((snapshot) => {
      eventVersionRef.current += 1;
      if (!cancelled) setServerSnapshot(snapshot);
    }).then(async (stop) => {
      if (cancelled) {
        stop();
        return;
      }

      unlisten = stop;
      const versionBeforeLoad = eventVersionRef.current;
      try {
        const snapshot = await serverConnectionStatus();
        if (!cancelled && eventVersionRef.current === versionBeforeLoad) {
          setServerSnapshot(snapshot);
        }
      } catch {
        // The settings refresh surface reports command errors; keep the last event truth here.
      }
    }).catch(async () => {
      if (cancelled) return;
      const versionBeforeLoad = eventVersionRef.current;
      try {
        const snapshot = await serverConnectionStatus();
        if (!cancelled && eventVersionRef.current === versionBeforeLoad) {
          setServerSnapshot(snapshot);
        }
      } catch {
        // The settings refresh surface reports command errors.
      }
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const refreshServerState = useCallback(async () => {
    const versionBeforeRefresh = eventVersionRef.current;
    const snapshot = await refreshServerConnection();
    if (eventVersionRef.current === versionBeforeRefresh) setServerSnapshot(snapshot);
    return snapshot;
  }, []);

  return { refreshServerState, serverLabel, serverSnapshot, serverState };
}

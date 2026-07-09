import { useCallback, useState } from "react";

import { serverConnectionLabel, type ServerConnectionState } from "@/lib/app-types";
import { serverConnectionStatus } from "@/server";

export function useServerConnection() {
  const [serverState, setServerState] = useState<ServerConnectionState>("not_set");
  const serverLabel = serverConnectionLabel(serverState);

  const refreshServerState = useCallback(async () => {
    const next = await serverConnectionStatus();
    setServerState(next);
    return next;
  }, []);

  return { refreshServerState, serverLabel };
}

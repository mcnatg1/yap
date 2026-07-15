import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import { toast } from "sonner";

import {
  loadNativeHistoryCatalog,
  type NativeHistoryCatalog,
} from "@/history-catalog";
import {
  savedSessionToTranscriptHistoryEntry,
} from "@/native-history";
import type { TranscriptHistoryEntry } from "@/history-model";
import { listenLiveSessionSaved, type SavedLiveSession } from "@/live";
import { createRecordingJobsRefreshCoordinator } from "@/recording-jobs-refresh";
import type { NativeHistoryReconciliation } from "@/hooks/use-transcript-history";

const historySyncWarning = "Transcript history could not be synced.";
const historySubscriptionWarning = "Transcript history updates could not be monitored. Restart Yap if new transcripts do not appear.";
const hiddenHistoryCleanupWarning = "Hidden transcript cleanup could not be completed.";

type HistoryCatalogEventSubscriptions = {
  listenLiveSaved: (
    handler: (session: SavedLiveSession) => void,
  ) => Promise<() => void>;
  listenRecordingJobsChanged: (handler: () => void) => Promise<() => void>;
};

const nativeHistoryCatalogEventSubscriptions: HistoryCatalogEventSubscriptions = {
  listenLiveSaved: listenLiveSessionSaved,
  listenRecordingJobsChanged: (handler) => listen("recording-jobs-changed", handler),
};

export function projectNativeHistoryCatalog(catalog: NativeHistoryCatalog) {
  return catalog.sessions.map(savedSessionToTranscriptHistoryEntry);
}

export function historyCatalogEntryKey(entry: TranscriptHistoryEntry) {
  return `${entry.origin ?? "legacy"}\0${entry.sessionId ?? ""}\0${entry.outputPath}`;
}

export function selectSavedHistoryCatalogEntry(
  entries: TranscriptHistoryEntry[],
  knownKeys: ReadonlySet<string>,
  initialized: boolean,
  preferredKey?: string,
) {
  const preferred = preferredKey
    ? entries.find((entry) => historyCatalogEntryKey(entry) === preferredKey)
    : undefined;
  if (preferred) return preferred;
  if (!initialized) return undefined;
  return entries.find((entry) => !knownKeys.has(historyCatalogEntryKey(entry)));
}

export function acceptMaintenanceWarnings(
  warnings: string[],
  shownWarnings: Set<string>,
) {
  const firstUnshown = warnings.find((warning) => !shownWarnings.has(warning));
  warnings.forEach((warning) => shownWarnings.add(warning));
  return firstUnshown;
}

export async function subscribeHistoryCatalogEvents(
  onLiveSaved: (session: SavedLiveSession) => void,
  onRecordingJobsChanged: () => void,
  subscriptions: HistoryCatalogEventSubscriptions = nativeHistoryCatalogEventSubscriptions,
) {
  const settled = await Promise.allSettled([
    subscriptions.listenLiveSaved(onLiveSaved),
    subscriptions.listenRecordingJobsChanged(onRecordingJobsChanged),
  ]);
  const disposers = settled.flatMap((subscription) => (
    subscription.status === "fulfilled" ? [subscription.value] : []
  ));
  const failures = settled.flatMap((subscription) => (
    subscription.status === "rejected" ? [subscription.reason] : []
  ));
  return {
    dispose() {
      disposers.splice(0).forEach((dispose) => dispose());
    },
    failures,
  };
}

export async function prepareHistoryCatalogReconciliation(
  captureNativeHistoryReconciliation: () => NativeHistoryReconciliation,
  loadCatalog: () => Promise<NativeHistoryCatalog> = loadNativeHistoryCatalog,
) {
  const apply = captureNativeHistoryReconciliation();
  const catalog = await loadCatalog();
  return {
    apply,
    catalog,
    entries: projectNativeHistoryCatalog(catalog),
  };
}

type HistoryCatalogSyncPorts = {
  captureNativeHistoryReconciliation: () => NativeHistoryReconciliation;
  onSaved: (entry: TranscriptHistoryEntry) => void;
  reconcileHiddenHistory: () => Promise<void>;
};

export function useHistoryCatalogSync({
  captureNativeHistoryReconciliation,
  onSaved,
  reconcileHiddenHistory,
}: HistoryCatalogSyncPorts) {
  const portsRef = useRef({
    captureNativeHistoryReconciliation,
    onSaved,
    reconcileHiddenHistory,
  });
  portsRef.current = {
    captureNativeHistoryReconciliation,
    onSaved,
    reconcileHiddenHistory,
  };

  useEffect(() => {
    if (!isTauri()) return;

    let active = true;
    let initialized = false;
    let startupReady = false;
    let pendingSavedKey: string | undefined;
    let knownKeys = new Set<string>();
    const shownMaintenanceWarnings = new Set<string>();
    let disposeSubscriptions: (() => void) | undefined;

    const coordinator = createRecordingJobsRefreshCoordinator(
      () => prepareHistoryCatalogReconciliation(
        portsRef.current.captureNativeHistoryReconciliation,
      ),
      ({ apply, catalog, entries }) => {
        if (!active) return;
        const maintenanceWarning = acceptMaintenanceWarnings(
          catalog.maintenanceWarnings,
          shownMaintenanceWarnings,
        );
        if (maintenanceWarning) {
          toast.warning(maintenanceWarning);
        }
        const visibleEntries = apply(entries, historySyncWarning);
        if (!visibleEntries) return;
        const saved = selectSavedHistoryCatalogEntry(
          visibleEntries,
          knownKeys,
          initialized,
          pendingSavedKey,
        );
        const catalogKeys = new Set(entries.map(historyCatalogEntryKey));
        if (
          pendingSavedKey
          && catalogKeys.has(pendingSavedKey)
          && !visibleEntries.some((entry) => historyCatalogEntryKey(entry) === pendingSavedKey)
        ) {
          pendingSavedKey = undefined;
        }
        knownKeys = new Set(
          [...entries, ...visibleEntries].map(historyCatalogEntryKey),
        );
        initialized = true;
        if (!saved) return;
        if (historyCatalogEntryKey(saved) === pendingSavedKey) {
          pendingSavedKey = undefined;
        }
        portsRef.current.onSaved(saved);
      },
    );

    const reportRefreshFailure = () => {
      if (active) toast.error(historySyncWarning);
    };
    const requestRefresh = () => {
      if (!startupReady) return;
      void coordinator.refresh().catch(reportRefreshFailure);
    };
    const rememberLiveSave = (session: SavedLiveSession) => {
      const entry = savedSessionToTranscriptHistoryEntry({
        ...session,
        origin: "live",
      });
      pendingSavedKey = historyCatalogEntryKey(entry);
      requestRefresh();
    };

    void (async () => {
      const subscriptions = await subscribeHistoryCatalogEvents(
        rememberLiveSave,
        requestRefresh,
      );
      if (!active) {
        subscriptions.dispose();
        return;
      }
      disposeSubscriptions = subscriptions.dispose;
      if (subscriptions.failures.length) {
        toast.warning(historySubscriptionWarning);
      }
      try {
        await portsRef.current.reconcileHiddenHistory();
      } catch {
        if (active) toast.warning(hiddenHistoryCleanupWarning);
      }
      if (!active) return;
      startupReady = true;
      await coordinator.refresh();
    })().catch(reportRefreshFailure);

    return () => {
      active = false;
      disposeSubscriptions?.();
    };
  }, []);
}

import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { toast } from "sonner";

import {
  acceptedFormats,
  acceptedRecordingDrops,
  isRecordingActive,
  type PlaybackAdmission,
  type QueuedRecordingPath,
  type RecordingJobView,
} from "@/lib/app-types";
import {
  allowRecordingPlaybackPath,
  playbackAdmissionDeadlineMs,
  releaseRecordingPlaybackPaths,
  settlePlaybackAdmissionBeforeDeadline,
} from "@/lib/playback-registry";
import {
  availableQueuedServerSlots,
  createQueuedServerRecordingJobs,
  nextRecordingQueueId,
  readRecordingQueue,
  writeRecordingQueue,
} from "@/recording-queue";

type QueueOwnerPorts = {
  allowPlaybackPath?: (path: string) => Promise<PlaybackAdmission>;
  getQueue: () => RecordingJobView[];
  maxAdmissionConcurrency?: number;
  releasePlaybackPaths?: (paths: Iterable<string>) => Promise<void>;
  setQueue: (queue: RecordingJobView[]) => void;
  warn?: (message: string) => void;
};

type ApprovedRecording = QueuedRecordingPath & PlaybackAdmission;

function queueFullWarning(accepted: number, candidates: number) {
  return accepted
    ? `Queued ${accepted} of ${candidates} recordings. Connect to your organization's transcription server before adding more.`
    : "The organization server queue is full. Connect before adding more recordings.";
}

export function createImportedRecordingQueueOwner({
  allowPlaybackPath = allowRecordingPlaybackPath,
  getQueue,
  maxAdmissionConcurrency = 4,
  releasePlaybackPaths = releaseRecordingPlaybackPaths,
  setQueue,
  warn = (message) => toast.warning(message),
}: QueueOwnerPorts) {
  let active = true;
  let generation = 0;
  let nextId = nextRecordingQueueId(getQueue());
  let pendingSlots = 0;
  let runningAdmissions = 0;
  const pendingPaths = new Set<string>();
  const waitingAdmissions: Array<() => void> = [];

  function runNextAdmission() {
    while (
      runningAdmissions < Math.max(1, maxAdmissionConcurrency)
      && waitingAdmissions.length
    ) {
      runningAdmissions += 1;
      waitingAdmissions.shift()!();
    }
  }

  function scheduleAdmission<T>(work: () => Promise<T>) {
    return new Promise<T>((resolve, reject) => {
      waitingAdmissions.push(() => {
        void work().then(resolve, reject).finally(() => {
          runningAdmissions -= 1;
          runNextAdmission();
        });
      });
      runNextAdmission();
    });
  }

  function invalidatePending() {
    generation += 1;
    pendingPaths.clear();
    pendingSlots = 0;
    nextId = nextRecordingQueueId(getQueue());
  }

  function releaseDiscarded(items: ApprovedRecording[]) {
    if (!items.length) return;
    void releasePlaybackPaths(items.map((item) => item.playbackPath)).catch(() => undefined);
  }

  function finishReservation(item: QueuedRecordingPath, operationGeneration: number) {
    if (!active || generation !== operationGeneration) return false;
    pendingSlots = Math.max(0, pendingSlots - 1);
    pendingPaths.delete(item.path);
    return true;
  }

  function commitApproved(item: ApprovedRecording, operationGeneration: number) {
    if (!finishReservation(item, operationGeneration)) {
      releaseDiscarded([item]);
      return undefined;
    }

    const latest = getQueue();
    const addable = acceptedRecordingDrops(latest.map((entry) => entry.path), [item])
      .slice(0, availableQueuedServerSlots(latest))[0];
    if (!addable) {
      releaseDiscarded([item]);
      return undefined;
    }

    const [newItem] = createQueuedServerRecordingJobs([item]);
    setQueue([...latest, {
      ...newItem,
      playbackByteLength: item.byteLength,
    }].sort((left, right) => left.id - right.id));
    return item.id;
  }

  async function addPaths(paths: string[]) {
    if (!active) return undefined;

    const current = getQueue();
    nextId = Math.max(nextId, nextRecordingQueueId(current));
    const firstId = nextId;
    nextId += paths.length;
    const incoming = paths.map((path, index) => ({ id: firstId + index, path }));
    const acceptedCandidates = acceptedRecordingDrops(
      [...current.map((item) => item.path), ...pendingPaths],
      incoming,
    );
    const availableSlots = Math.max(
      0,
      availableQueuedServerSlots(current) - pendingSlots,
    );
    const accepted = acceptedCandidates.slice(0, availableSlots);

    if (paths.length && !acceptedCandidates.length) {
      warn(`Drop ${acceptedFormats} files.`);
      return undefined;
    }
    if (acceptedCandidates.length > accepted.length) {
      warn(queueFullWarning(accepted.length, acceptedCandidates.length));
    }
    if (!accepted.length) return undefined;

    const operationGeneration = generation;
    const admissionDeadlineAt = Date.now() + playbackAdmissionDeadlineMs;
    pendingSlots += accepted.length;
    for (const item of accepted) pendingPaths.add(item.path);

    const outcomes = await Promise.all(accepted.map((item) => scheduleAdmission(async () => {
      if (!active || generation !== operationGeneration) return undefined;

      const admission = await settlePlaybackAdmissionBeforeDeadline(
        () => allowPlaybackPath(item.path),
        (playbackPath) => releasePlaybackPaths([playbackPath]),
        admissionDeadlineAt,
      );
      if (!admission) {
        finishReservation(item, operationGeneration);
        return false;
      }
      const approved: ApprovedRecording = { ...item, ...admission };
      return commitApproved(approved, operationGeneration);
    })));

    if (!active || generation !== operationGeneration) return undefined;
    if (outcomes.some((outcome) => outcome === false)) {
      warn("Some recordings could not be prepared for playback.");
    }
    const committedIds = outcomes.filter((outcome): outcome is number => (
      typeof outcome === "number"
    ));
    return committedIds[committedIds.length - 1];
  }

  return {
    activate() {
      if (active) return;
      active = true;
      invalidatePending();
    },
    addPaths,
    clear() {
      invalidatePending();
      setQueue([]);
    },
    dispose() {
      if (!active) return;
      active = false;
      invalidatePending();
    },
  };
}

export function useImportedRecordingQueue(onClear: () => void) {
  const [queue, setQueueState] = useState<RecordingJobView[]>(readRecordingQueue);
  const queueRef = useRef(queue);
  const onClearRef = useRef(onClear);
  onClearRef.current = onClear;

  const setQueue = useCallback<Dispatch<SetStateAction<RecordingJobView[]>>>((update) => {
    const next = typeof update === "function" ? update(queueRef.current) : update;
    queueRef.current = next;
    setQueueState(next);
  }, []);
  const ownerRef = useRef<ReturnType<typeof createImportedRecordingQueueOwner> | undefined>(
    undefined,
  );
  if (!ownerRef.current) {
    ownerRef.current = createImportedRecordingQueueOwner({
      getQueue: () => queueRef.current,
      setQueue: (next) => setQueue(next),
    });
  }

  useEffect(() => {
    const owner = ownerRef.current!;
    owner.activate();
    return () => owner.dispose();
  }, []);

  useEffect(() => {
    try {
      writeRecordingQueue(queue);
    } catch (error) {
      console.warn("Queued recordings could not be saved.", error);
      toast.warning("Queued recordings could not be saved.");
    }
  }, [queue]);

  const addPaths = useCallback(
    (paths: string[]) => ownerRef.current!.addPaths(paths),
    [],
  );

  const removeItem = useCallback((id: number) => {
    setQueue((items) => {
      const item = items.find((entry) => entry.id === id);
      if (!item || isRecordingActive(item.status)) return items;
      return items.filter((entry) => entry.id !== id);
    });
  }, [setQueue]);

  const clearQueue = useCallback(() => {
    ownerRef.current!.clear();
    onClearRef.current();
  }, []);

  return {
    addPaths,
    clearQueue,
    queue,
    removeItem,
    setQueue,
  };
}

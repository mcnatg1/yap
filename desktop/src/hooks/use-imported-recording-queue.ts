import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { toast } from "sonner";

import {
  acceptedFormats,
  acceptedRecordingDrops,
  isRecordingActive,
  type RecordingJobView,
} from "@/lib/app-types";
import { allowRecordingPlaybackPath } from "@/lib/playback-registry";
import {
  availableQueuedServerSlots,
  createQueuedServerRecordingJobs,
  nextRecordingQueueId,
  readRecordingQueue,
  writeRecordingQueue,
} from "@/recording-queue";

export function useImportedRecordingQueue(onClear: () => void) {
  const [queue, setQueueState] = useState<RecordingJobView[]>(readRecordingQueue);
  const queueRef = useRef(queue);
  const nextRecordingId = useRef(nextRecordingQueueId(queue));
  const onClearRef = useRef(onClear);
  onClearRef.current = onClear;

  const setQueue = useCallback<Dispatch<SetStateAction<RecordingJobView[]>>>((update) => {
    const next = typeof update === "function" ? update(queueRef.current) : update;
    queueRef.current = next;
    setQueueState(next);
  }, []);

  useEffect(() => {
    try {
      writeRecordingQueue(queue);
    } catch (error) {
      console.warn("Queued recordings could not be saved.", error);
      toast.warning("Queued recordings could not be saved.");
    }
  }, [queue]);

  const addPaths = useCallback(async (paths: string[]) => {
    const firstId = nextRecordingId.current;
    nextRecordingId.current += paths.length;
    const incoming = paths.map((path, index) => ({ id: firstId + index, path }));
    const current = queueRef.current;
    const acceptedCandidates = acceptedRecordingDrops(current.map((item) => item.path), incoming);
    const accepted = acceptedCandidates.slice(0, availableQueuedServerSlots(current));

    if (paths.length && !acceptedCandidates.length) {
      toast.warning(`Drop ${acceptedFormats} files.`);
      return undefined;
    }
    if (acceptedCandidates.length > accepted.length) {
      toast.warning(
        accepted.length
          ? `Queued ${accepted.length} of ${acceptedCandidates.length} recordings. Connect to your organization's transcription server before adding more.`
          : "The organization server queue is full. Connect before adding more recordings.",
      );
    }

    const approved = (
      await Promise.all(
        accepted.map(async (item) => {
          try {
            const admission = await allowRecordingPlaybackPath(item.path);
            return {
              ...item,
              playbackByteLength: admission.byteLength,
              playbackPath: admission.playbackPath,
            };
          } catch {
            return undefined;
          }
        }),
      )
    ).filter((item): item is {
      id: number;
      path: string;
      playbackByteLength: number;
      playbackPath: string;
    } => Boolean(item));

    if (accepted.length && approved.length < accepted.length) {
      toast.warning("Some recordings could not be prepared for playback.");
    }
    if (!approved.length) return undefined;

    const latest = queueRef.current;
    const acceptedApprovedIds = new Set(
      acceptedRecordingDrops(latest.map((item) => item.path), approved).map((item) => item.id),
    );
    const addable = approved
      .filter((item) => acceptedApprovedIds.has(item.id))
      .slice(0, availableQueuedServerSlots(latest));
    if (!addable.length) return undefined;

    const playbackByteLengths = new Map(
      addable.map((item) => [item.id, item.playbackByteLength]),
    );
    const newItems = createQueuedServerRecordingJobs(addable).map((item) => ({
      ...item,
      playbackByteLength: playbackByteLengths.get(item.id),
    }));
    setQueue([...latest, ...newItems]);
    return newItems[newItems.length - 1]?.id;
  }, [setQueue]);

  const removeItem = useCallback((id: number) => {
    setQueue((items) => {
      const item = items.find((entry) => entry.id === id);
      if (!item || isRecordingActive(item.status)) return items;
      return items.filter((entry) => entry.id !== id);
    });
  }, [setQueue]);

  const clearQueue = useCallback(() => {
    setQueue([]);
    onClearRef.current();
  }, [setQueue]);

  return {
    addPaths,
    clearQueue,
    queue,
    removeItem,
    setQueue,
  };
}

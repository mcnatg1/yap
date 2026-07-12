import type { TranscriptHistoryEntry } from "@/history";
import { createInitialPipelineState, type RecordingJobView } from "@/lib/app-types";

export function historyEntryToRecordingJob(
  entry: TranscriptHistoryEntry,
  restoredPlaybackPath?: string,
): RecordingJobView {
  return {
    id: 0,
    intent: "live",
    name: entry.name,
    output: entry.outputPath,
    path: entry.sourcePath,
    playbackPath: restoredPlaybackPath,
    pipeline: {
      ...createInitialPipelineState(),
      intake: "done",
      transcription: "done",
      postprocessing: entry.warning || entry.recoveryState ? "error" : "done",
    },
    route: "localFallback",
    error: entry.warning ?? (entry.recoveryState ? "Partial recording available for recovery." : undefined),
    status: entry.warning || entry.recoveryState ? "partial" : "complete",
  };
}

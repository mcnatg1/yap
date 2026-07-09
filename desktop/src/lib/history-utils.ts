import { historyEntryPlaybackPath, type TranscriptHistoryEntry } from "@/history";
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
    playbackPath: restoredPlaybackPath ?? historyEntryPlaybackPath(entry),
    pipeline: {
      ...createInitialPipelineState(),
      intake: "done",
      transcription: "done",
      postprocessing: entry.warning ? "error" : "done",
    },
    route: "localFallback",
    error: entry.warning,
    status: entry.warning ? "partial" : "complete",
  };
}

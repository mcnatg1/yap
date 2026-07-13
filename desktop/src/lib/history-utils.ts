import { transcriptPathIdentity, type TranscriptHistoryEntry } from "@/history";
import { createInitialPipelineState, type RecordingJobView } from "@/lib/app-types";

export function historyEntryToRecordingJob(
  entry: TranscriptHistoryEntry,
  restoredPlaybackPath?: string,
): RecordingJobView {
  return {
    id: `history:${transcriptPathIdentity(entry.outputPath)}`,
    name: entry.name,
    outputPath: entry.outputPath,
    sourcePath: entry.sourcePath,
    playbackPath: restoredPlaybackPath,
    pipeline: {
      ...createInitialPipelineState(),
      intake: "done",
      transcription: "done",
      postprocessing: entry.warning || entry.recoveryState ? "error" : "done",
    },
    route: "localFallback",
    sessionMode: "dictation",
    sessionOrigin: "liveCapture",
    status: entry.warning || entry.recoveryState ? "partial" : "complete",
  };
}

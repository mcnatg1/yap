import type { TranscriptHistoryEntry } from "@/history";
import { createInitialPipelineState, type RecordingJobView } from "@/lib/app-types";

export function historyEntryToRecordingJob(entry: TranscriptHistoryEntry): RecordingJobView {
  return {
    id: 0,
    intent: "recording",
    name: entry.name,
    output: entry.outputPath,
    path: entry.sourcePath,
    pipeline: {
      ...createInitialPipelineState(),
      intake: "done",
      transcription: "done",
      postprocessing: "done",
    },
    route: "localFallback",
    status: "complete",
  };
}

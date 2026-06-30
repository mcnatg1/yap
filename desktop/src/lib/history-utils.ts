import type { UploadItem } from "@/components/stacked-upload";
import type { TranscriptHistoryEntry } from "@/history";

export function historyEntryToUploadItem(entry: TranscriptHistoryEntry): UploadItem {
  return {
    id: 0,
    name: entry.name,
    output: entry.outputPath,
    path: entry.sourcePath,
    status: "done",
  };
}

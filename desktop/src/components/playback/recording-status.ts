import type { RecordingJobStatus } from "@/lib/recording-job";

export function recordingActivityLabel(status: RecordingJobStatus) {
  switch (status) {
    case "uploading":
      return "Uploading";
    case "server_processing":
      return "Processing on server";
    case "diarization_running":
      return "Finding speakers";
    case "saving":
      return "Saving";
    default:
      return "Working";
  }
}

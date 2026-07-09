import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";

import type { RecordingJobView } from "@/lib/app-types";

type TranscriptTextLoader = (path: string) => Promise<string>;

export function useTranscriptFileActions(loadTranscriptText: TranscriptTextLoader) {
  async function copyTranscript(item: RecordingJobView) {
    if (!item.output) return;

    try {
      const text = await loadTranscriptText(item.output);
      await navigator.clipboard.writeText(text);
      toast.success(text.trim() ? "Transcript copied" : "Empty transcript copied");
    } catch {
      toast.error("Copy failed");
    }
  }

  async function openAppPath(path: string) {
    try {
      await invoke("open_app_path", { path });
      toast.success("Opened file");
    } catch {
      toast.error("Open failed");
    }
  }

  async function revealPath(path: string) {
    try {
      await invoke("reveal_app_path", { path });
    } catch {
      toast.error("Reveal failed");
    }
  }

  async function savePolishedTranscript(item: RecordingJobView, text: string) {
    if (!item.output || !text.trim()) return "";

    try {
      const path = await invoke<string>("write_polished_text", { path: item.output, text });
      toast.success("Polished draft saved");
      return path;
    } catch (error) {
      toast.error("Save failed");
      throw error;
    }
  }

  return {
    copyTranscript,
    openAppPath,
    revealPath,
    savePolishedTranscript,
  };
}

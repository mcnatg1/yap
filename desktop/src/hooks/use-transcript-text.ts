import { invoke, isTauri } from "@tauri-apps/api/core";
import { useCallback, useState } from "react";

import { rememberText } from "@/lib/text-cache";

export function useTranscriptText() {
  const [polishedText, setPolishedText] = useState<Record<string, string>>({});
  const [transcriptText, setTranscriptText] = useState<Record<string, string>>({});

  const clearTranscriptText = useCallback(() => {
    setTranscriptText({});
  }, []);

  const forgetTranscriptText = useCallback((path: string) => {
    setTranscriptText((current) => {
      const { [path]: _deleted, ...next } = current;
      return next;
    });
  }, []);

  const loadTranscriptText = useCallback(async (path: string) => {
    if (Object.prototype.hasOwnProperty.call(transcriptText, path)) return transcriptText[path];
    if (!isTauri()) return "";

    const text = await invoke<string>("read_text_file", { path });
    setTranscriptText((current) => rememberText(current, path, text));
    return text;
  }, [transcriptText]);

  const loadTranscriptPreviewText = useCallback(async (path: string) => {
    if (!isTauri()) return "";
    return invoke<string>("read_text_preview", { maxChars: 600, path });
  }, []);

  const rememberPolishedText = useCallback((outputPath: string, text: string) => {
    setPolishedText((current) => rememberText(current, outputPath, text));
  }, []);

  return {
    clearTranscriptText,
    forgetTranscriptText,
    loadTranscriptPreviewText,
    loadTranscriptText,
    polishedText,
    rememberPolishedText,
    transcriptText,
  };
}

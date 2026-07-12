export type TranscriptTextProjection = {
  state: "empty" | "loading" | "ready";
  text: string;
};

export function projectTranscriptText(text: string | undefined): TranscriptTextProjection {
  if (text === undefined) {
    return {
      state: "loading",
      text: "Loading transcript...",
    };
  }
  if (!text.trim()) {
    return {
      state: "empty",
      text: "Empty transcript.",
    };
  }
  return {
    state: "ready",
    text,
  };
}

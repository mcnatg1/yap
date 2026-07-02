import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type SttErrorCode =
  | "MODEL_MISSING"
  | "MODEL_CORRUPT"
  | "BAD_LANG"
  | "OOM"
  | "AUDIO_DECODE"
  | "SIDECAR_CRASH"
  | "SIDECAR_UNREACHABLE"
  | "FALLBACK_DISABLED"
  | "BUSY"
  | "TIMEOUT";

export type TranscriptResult = {
  input: string;
  output: string;
  error?: string;
};

export type SttFailure = {
  code: string;
  message: string;
};

export type TranscribePhase =
  | "starting"
  | "loading_model"
  | "transcribing"
  | "writing"
  | "done";

export type TranscribeProgressEvent = {
  path: string;
  index: number;
  total: number;
  phase: TranscribePhase | string;
  percent?: number;
  message: string;
};

export type TranscribeFileCompleteEvent = {
  path: string;
  index: number;
  total: number;
  result: TranscriptResult;
};

export type TranscribeBatchCompleteEvent = {
  results: TranscriptResult[];
  succeeded: number;
  failed: number;
};

export type TranscribeListeners = {
  onProgress: (event: TranscribeProgressEvent) => void;
  onFileComplete: (event: TranscribeFileCompleteEvent) => void;
  onComplete: (event: TranscribeBatchCompleteEvent) => void;
  onError: (error: SttFailure) => void;
};

const sttErrorCodes: readonly SttErrorCode[] = [
  "MODEL_MISSING",
  "MODEL_CORRUPT",
  "BAD_LANG",
  "OOM",
  "AUDIO_DECODE",
  "SIDECAR_CRASH",
  "SIDECAR_UNREACHABLE",
  "FALLBACK_DISABLED",
  "BUSY",
  "TIMEOUT",
];

export function isSttErrorCode(value: string): value is SttErrorCode {
  return (sttErrorCodes as readonly string[]).includes(value);
}

export function sttErrorMessage(code: SttErrorCode): string {
  switch (code) {
    case "MODEL_MISSING":
      return "Local fallback model isn't installed yet.";
    case "MODEL_CORRUPT":
      return "Model file failed verification.";
    case "BAD_LANG":
      return "That language isn't supported.";
    case "OOM":
      return "Ran out of memory while transcribing.";
    case "AUDIO_DECODE":
      return "Couldn't read that audio file.";
    case "SIDECAR_CRASH":
      return "Transcription engine crashed.";
    case "SIDECAR_UNREACHABLE":
      return "Transcription engine didn't start.";
    case "FALLBACK_DISABLED":
      return "Local fallback is disabled.";
    case "BUSY":
      return "Transcription is busy — try again in a moment.";
    case "TIMEOUT":
      return "Transcription timed out.";
    default: {
      const exhaustive: never = code;
      return exhaustive;
    }
  }
}

export class SttInvokeError extends Error {
  code: string;
  detail: string;

  constructor(code: string, detail: string) {
    super(isSttErrorCode(code) ? sttErrorMessage(code) : detail || "Transcription failed.");
    this.name = "SttInvokeError";
    this.code = code;
    this.detail = detail;
  }
}

function toFailure(raw: unknown): SttFailure {
  if (raw && typeof raw === "object" && "code" in raw) {
    const failure = raw as { code?: unknown; message?: unknown };
    return {
      code: typeof failure.code === "string" ? failure.code : "",
      message: typeof failure.message === "string" ? failure.message : "",
    };
  }
  return { code: "", message: typeof raw === "string" ? raw : String(raw) };
}

export async function startTranscribe(paths: string[]): Promise<void> {
  try {
    await invoke("start_transcribe", { paths });
  } catch (raw) {
    const failure = toFailure(raw);
    throw new SttInvokeError(failure.code, failure.message);
  }
}

export async function listenTranscribeEvents(listeners: TranscribeListeners): Promise<UnlistenFn> {
  const unsubs = await Promise.all([
    listen<TranscribeProgressEvent>("transcribe-progress", (event) => {
      listeners.onProgress(event.payload);
    }),
    listen<TranscribeFileCompleteEvent>("transcribe-file-complete", (event) => {
      listeners.onFileComplete(event.payload);
    }),
    listen<TranscribeBatchCompleteEvent>("transcribe-complete", (event) => {
      listeners.onComplete(event.payload);
    }),
    listen<SttFailure>("transcribe-error", (event) => {
      listeners.onError(event.payload);
    }),
  ]);

  return () => {
    for (const unsub of unsubs) {
      unsub();
    }
  };
}

export function transcriptFileError(result: TranscriptResult): string | undefined {
  if (!result.error) return undefined;
  return isSttErrorCode(result.error) ? sttErrorMessage(result.error) : result.error;
}

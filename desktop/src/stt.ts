import { invoke } from "@tauri-apps/api/core";

export type SttErrorCode =
  | "MODEL_MISSING"
  | "MODEL_CORRUPT"
  | "MODEL_INSTALL_CANCELLED"
  | "BAD_LANG"
  | "OOM"
  | "AUDIO_DECODE"
  | "SIDECAR_CRASH"
  | "SIDECAR_UNREACHABLE"
  | "SERVER_UNAVAILABLE"
  | "FALLBACK_DISABLED"
  | "BUSY"
  | "TIMEOUT";

type SttFailure = {
  code: string;
  message: string;
};

const sttErrorCodes: readonly SttErrorCode[] = [
  "MODEL_MISSING",
  "MODEL_CORRUPT",
  "MODEL_INSTALL_CANCELLED",
  "BAD_LANG",
  "OOM",
  "AUDIO_DECODE",
  "SIDECAR_CRASH",
  "SIDECAR_UNREACHABLE",
  "SERVER_UNAVAILABLE",
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
    case "MODEL_INSTALL_CANCELLED":
      return "Local fallback install was cancelled.";
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
    case "SERVER_UNAVAILABLE":
      return "Server transcription is unavailable.";
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

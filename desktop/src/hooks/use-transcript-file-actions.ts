import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";

import { transcriptPathIdentity } from "@/history-model";
import type { RecordingJobView } from "@/lib/recording-job";
import type { PolishSaveRequest } from "@/polish";

type TranscriptTextLoader = (path: string) => Promise<string>;
type PolishTextWriter = (outputPath: string, text: string) => Promise<string>;
const polishSaveTailWaitMs = 5_000;
const polishWriteTails = new Map<string, Promise<void>>();

export class PolishSaveCancelledError extends Error {
  override name = "AbortError";

  constructor() {
    super("The polished draft changed before it could be saved.");
  }
}

export class PolishSaveBusyError extends Error {
  constructor() {
    super("A previous save to this transcript is still finishing. Try again shortly.");
  }
}

function assertCurrentPolishSave(request: PolishSaveRequest) {
  if (
    request.signal.aborted
    || !request.isCurrent()
    || !request.outputPath
    || !request.sourceIdentity
    || !Number.isSafeInteger(request.revision)
    || request.revision < 1
    || !request.text.trim()
  ) throw new PolishSaveCancelledError();
}

function waitForSaveTail(
  pending: Promise<void>,
  signal: AbortSignal,
  waitMs: number,
) {
  return new Promise<void>((resolve, reject) => {
    if (signal.aborted) {
      reject(new PolishSaveCancelledError());
      return;
    }

    let timer: ReturnType<typeof setTimeout> | undefined;
    const cleanup = () => {
      signal.removeEventListener("abort", onAbort);
      if (timer !== undefined) clearTimeout(timer);
    };
    const onAbort = () => {
      cleanup();
      reject(new PolishSaveCancelledError());
    };
    signal.addEventListener("abort", onAbort, { once: true });
    timer = setTimeout(() => {
      cleanup();
      reject(new PolishSaveBusyError());
    }, waitMs);
    pending.then(
      () => {
        cleanup();
        resolve();
      },
      (error: unknown) => {
        cleanup();
        reject(error);
      },
    );
  });
}

export async function persistPolishedTranscript(
  request: PolishSaveRequest,
  write: PolishTextWriter,
  waitForPreviousMs = polishSaveTailWaitMs,
) {
  // Yield once so a source change or unmount committed by the initiating event
  // can revoke ownership before native I/O begins.
  await Promise.resolve();
  assertCurrentPolishSave(request);

  const writeKey = transcriptPathIdentity(request.outputPath);
  const previous = polishWriteTails.get(writeKey) ?? Promise.resolve();
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const tail = previous.then(() => gate);
  polishWriteTails.set(writeKey, tail);
  void tail.then(() => {
    if (polishWriteTails.get(writeKey) === tail) {
      polishWriteTails.delete(writeKey);
    }
  });

  try {
    await waitForSaveTail(previous, request.signal, waitForPreviousMs);
    assertCurrentPolishSave(request);
    let path: string;
    try {
      // Once native I/O starts, keep this output path serialized until it
      // settles. Abort can revoke the result, but it cannot cancel invoke().
      path = await write(request.outputPath, request.text);
    } catch (error) {
      assertCurrentPolishSave(request);
      throw error;
    }
    assertCurrentPolishSave(request);
    return path;
  } finally {
    release();
  }
}

export function isPolishSaveCancelled(error: unknown) {
  return error instanceof PolishSaveCancelledError
    || (error instanceof DOMException && error.name === "AbortError");
}

export function useTranscriptFileActions(loadTranscriptText: TranscriptTextLoader) {
  async function copyTranscript(item: RecordingJobView) {
    if (!item.outputPath) return;

    try {
      const text = await loadTranscriptText(item.outputPath);
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

  async function savePolishedTranscript(request: PolishSaveRequest) {
    try {
      const path = await persistPolishedTranscript(
        request,
        (path, text) => invoke<string>("write_polished_text", { path, text }),
      );
      toast.success("Polished draft saved");
      return path;
    } catch (error) {
      if (isPolishSaveCancelled(error)) return "";
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

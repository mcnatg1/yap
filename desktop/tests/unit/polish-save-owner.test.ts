import { describe, expect, it, vi } from "vitest";

import {
  persistPolishedTranscript,
  PolishSaveBusyError,
  PolishSaveCancelledError,
} from "@/hooks/use-transcript-file-actions";
import { createInitialPipelineState, type RecordingJobView } from "@/lib/app-types";
import {
  createPolishOperationOwner,
  createPolishSaveRequest,
  polishSourceIdentity,
} from "@/polish";

const firstItem: RecordingJobView = {
  id: "job-first",
  name: "first.wav",
  outputPath: "C:/first.txt",
  sourcePath: "C:/first.wav",
  pipeline: createInitialPipelineState(),
  route: "serverBatch",
  sessionMode: "meeting",
  sessionOrigin: "importedFile",
  status: "complete",
};

const secondItem: RecordingJobView = {
  ...firstItem,
  id: "job-second",
  name: "second.wav",
  outputPath: "C:/second.txt",
  sourcePath: "C:/second.wav",
};

function deferred<T>() {
  let reject!: (error: unknown) => void;
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    reject = rejectPromise;
    resolve = resolvePromise;
  });
  return { promise, reject, resolve };
}

function ownedDraft(
  owner: ReturnType<typeof createPolishOperationOwner>,
  item: RecordingJobView,
  tone = "light",
  sourceText = "Original transcript",
) {
  const sourceIdentity = polishSourceIdentity(item, sourceText);
  const context = `${sourceIdentity}\0${tone}`;
  const run = owner.startRun(context)!;
  const draft = owner.acceptRun(run)!;
  expect(owner.finishRun(run)).toBe(true);
  return { context, draft, sourceIdentity, sourceText };
}

function ownedSave(
  owner: ReturnType<typeof createPolishOperationOwner>,
  item: RecordingJobView,
  text = "Polished draft",
) {
  const { context, draft, sourceIdentity, sourceText } = ownedDraft(owner, item);
  const token = owner.startSave(draft)!;
  const request = createPolishSaveRequest({
    context,
    item,
    sourceText,
    sourceIdentity,
    text,
    token,
  })!;
  return { draft, request, token };
}

describe("Polish save ownership", () => {
  it("revokes run, draft, and save ownership when same-path source text changes", () => {
    const owner = createPolishOperationOwner();
    const first = ownedDraft(owner, firstItem, "light", "First revision");
    const save = owner.startSave(first.draft)!;

    owner.invalidate();
    const secondIdentity = polishSourceIdentity(firstItem, "Second revision");

    expect(secondIdentity).not.toBe(first.sourceIdentity);
    expect(save.signal.aborted).toBe(true);
    expect(owner.currentDraft(first.context)).toBeUndefined();
    expect(createPolishSaveRequest({
      context: first.context,
      item: firstItem,
      sourceIdentity: first.sourceIdentity,
      sourceText: "Second revision",
      text: "Stale polished draft",
      token: save,
    })).toBeUndefined();
  });

  it("aborts the active run signal when its context is invalidated", () => {
    const owner = createPolishOperationOwner();
    const context = `${polishSourceIdentity(firstItem, "Original")}\0light`;
    const run = owner.startRun(context)!;

    owner.invalidate();

    expect(run.signal.aborted).toBe(true);
    expect(owner.acceptRun(run)).toBeUndefined();
  });

  it("passes run ownership through to the Ollama fetch signal", async () => {
    vi.resetModules();
    vi.doMock("@/lib/product-features", () => ({ developmentPolishAvailable: true }));
    vi.doMock("@/settings", () => ({ polishNumGpuLayers: async () => 0 }));
    const fetchMock = vi.fn((_url: string, init?: RequestInit) => (
      new Promise<Response>((_resolve, reject) => {
        init?.signal?.addEventListener("abort", () => {
          reject(new DOMException("Aborted", "AbortError"));
        }, { once: true });
      })
    ));
    vi.stubGlobal("fetch", fetchMock);
    const { polishTranscript: runTranscript } = await import("@/polish");
    const controller = new AbortController();

    const running = runTranscript({
      signal: controller.signal,
      text: "Original transcript",
      tone: "light",
    });
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    expect(fetchMock.mock.calls[0]?.[1]?.signal).toBe(controller.signal);
    controller.abort();

    await expect(running).rejects.toMatchObject({ name: "AbortError" });
    vi.unstubAllGlobals();
    vi.doUnmock("@/lib/product-features");
    vi.doUnmock("@/settings");
  });

  it("cancels before the native write boundary when the source changes", async () => {
    const owner = createPolishOperationOwner();
    const { request } = ownedSave(owner, firstItem);
    const write = vi.fn(async () => "C:/first-polished.txt");

    const saving = persistPolishedTranscript(request, write);
    owner.invalidate();

    await expect(saving).rejects.toBeInstanceOf(PolishSaveCancelledError);
    expect(write).not.toHaveBeenCalled();
  });

  it("settles as cancelled when a native write resolves after unmount", async () => {
    const owner = createPolishOperationOwner();
    const { request } = ownedSave(owner, firstItem);
    const nativeWrite = deferred<string>();
    const started = deferred<void>();
    const saving = persistPolishedTranscript(request, () => {
      started.resolve();
      return nativeWrite.promise;
    });
    await started.promise;

    owner.invalidate();
    nativeWrite.resolve("C:/first-polished.txt");

    await expect(saving).rejects.toBeInstanceOf(PolishSaveCancelledError);
  });

  it("settles as cancelled when a native write rejects after a source change", async () => {
    const owner = createPolishOperationOwner();
    const { request } = ownedSave(owner, firstItem);
    const nativeWrite = deferred<string>();
    const started = deferred<void>();
    const saving = persistPolishedTranscript(request, () => {
      started.resolve();
      return nativeWrite.promise;
    });
    await started.promise;

    owner.invalidate();
    nativeWrite.reject(new Error("late native failure"));

    await expect(saving).rejects.toBeInstanceOf(PolishSaveCancelledError);
  });

  it("serializes same-output writes until the stale native side effect settles", async () => {
    const owner = createPolishOperationOwner();
    const { request: staleRequest } = ownedSave(owner, firstItem, "Stale draft");
    const staleNativeWrite = deferred<string>();
    const staleStarted = deferred<void>();
    let persisted = "";
    const staleSaving = persistPolishedTranscript(staleRequest, async (_, text) => {
      staleStarted.resolve();
      const path = await staleNativeWrite.promise;
      persisted = text;
      return path;
    });
    await staleStarted.promise;

    owner.invalidate();
    const { request: currentRequest } = ownedSave(owner, firstItem, "Current draft");
    const currentWrite = vi.fn(async (_path: string, text: string) => {
      persisted = text;
      return "C:/current.txt";
    });
    const currentSaving = persistPolishedTranscript(currentRequest, currentWrite);
    await Promise.resolve();
    expect(currentWrite).not.toHaveBeenCalled();

    staleNativeWrite.resolve("C:/stale.txt");
    await expect(staleSaving).rejects.toBeInstanceOf(PolishSaveCancelledError);
    await expect(currentSaving).resolves.toBe("C:/current.txt");
    expect(currentWrite).toHaveBeenCalledTimes(1);
    expect(persisted).toBe("Current draft");
  });

  it("keeps a third write behind an active write when the middle waiter aborts", async () => {
    const owner = createPolishOperationOwner();
    const { request: firstRequest } = ownedSave(owner, firstItem, "First draft");
    const firstNativeWrite = deferred<string>();
    const firstStarted = deferred<void>();
    let persisted = "";
    const firstSaving = persistPolishedTranscript(firstRequest, async (_, text) => {
      firstStarted.resolve();
      const path = await firstNativeWrite.promise;
      persisted = text;
      return path;
    });
    await firstStarted.promise;

    owner.invalidate();
    const { request: middleRequest } = ownedSave(owner, firstItem, "Middle draft");
    const middleWrite = vi.fn(async () => "C:/middle.txt");
    const middleSaving = persistPolishedTranscript(middleRequest, middleWrite);
    await Promise.resolve();
    await Promise.resolve();

    owner.invalidate();
    const { request: currentRequest } = ownedSave(owner, firstItem, "Current draft");
    const currentWrite = vi.fn(async (_path: string, text: string) => {
      persisted = text;
      return "C:/current.txt";
    });
    const currentSaving = persistPolishedTranscript(currentRequest, currentWrite);

    await expect(middleSaving).rejects.toBeInstanceOf(PolishSaveCancelledError);
    expect(middleWrite).not.toHaveBeenCalled();
    expect(currentWrite).not.toHaveBeenCalled();

    firstNativeWrite.resolve("C:/first.txt");
    await expect(firstSaving).rejects.toBeInstanceOf(PolishSaveCancelledError);
    await expect(currentSaving).resolves.toBe("C:/current.txt");
    expect(currentWrite).toHaveBeenCalledTimes(1);
    expect(persisted).toBe("Current draft");
  });

  it("lets a newer save supersede an older save before native I/O", async () => {
    const owner = createPolishOperationOwner();
    const { draft, request: staleRequest } = ownedSave(owner, firstItem);
    const staleWrite = vi.fn(async () => "C:/stale.txt");
    const staleSaving = persistPolishedTranscript(staleRequest, staleWrite);

    const currentToken = owner.startSave(draft)!;
    const sourceText = "Original transcript";
    const sourceIdentity = polishSourceIdentity(firstItem, sourceText);
    const currentRequest = createPolishSaveRequest({
      context: `${sourceIdentity}\0light`,
      item: firstItem,
      sourceText,
      sourceIdentity,
      text: "Newer draft",
      token: currentToken,
    })!;

    await expect(staleSaving).rejects.toBeInstanceOf(PolishSaveCancelledError);
    await expect(persistPolishedTranscript(
      currentRequest,
      async () => "C:/current.txt",
    )).resolves.toBe("C:/current.txt");
    expect(staleWrite).not.toHaveBeenCalled();
  });

  it("does not let a hung cancelled save lock the next source", async () => {
    const owner = createPolishOperationOwner();
    const hungItem = { ...firstItem, outputPath: "C:/hung-first.txt" };
    const { request } = ownedSave(owner, hungItem);
    const started = deferred<void>();
    const hungSaving = persistPolishedTranscript(request, () => {
      started.resolve();
      return new Promise<string>(() => undefined);
    });
    await started.promise;

    owner.invalidate();
    let staleSettled = false;
    void hungSaving.finally(() => {
      staleSettled = true;
    });

    const next = ownedSave(owner, secondItem, "Second draft");
    await expect(persistPolishedTranscript(
      next.request,
      async () => "C:/second-polished.txt",
    )).resolves.toBe("C:/second-polished.txt");
    expect(staleSettled).toBe(false);
  });

  it("fails a later same-output save after a bounded wait without racing writes", async () => {
    const owner = createPolishOperationOwner();
    const sameOutputItem = { ...firstItem, outputPath: "C:/bounded-save.txt" };
    const { request: staleRequest } = ownedSave(owner, sameOutputItem, "Stale draft");
    const started = deferred<void>();
    void persistPolishedTranscript(staleRequest, () => {
      started.resolve();
      return new Promise<string>(() => undefined);
    });
    await started.promise;

    owner.invalidate();
    const { request: currentRequest } = ownedSave(owner, sameOutputItem, "Current draft");
    const currentWrite = vi.fn(async () => "C:/current.txt");

    await expect(persistPolishedTranscript(
      currentRequest,
      currentWrite,
      10,
    )).rejects.toBeInstanceOf(PolishSaveBusyError);
    expect(currentWrite).not.toHaveBeenCalled();
  });
});

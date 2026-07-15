import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { isRecordingFinished, type RecordingJobView } from "@/lib/recording-job";
import {
  createPolishOperationOwner,
  createPolishSaveRequest,
  isPolishDraftCurrent,
  polishSourceIdentity,
  polishTranscript,
  type PolishDraftToken,
  type PolishSaveRequest,
  type PolishTone,
} from "@/polish";

export type PolishRunDetails = {
  model: string;
  tokensPerSecond?: number;
  totalSeconds?: number;
};

type OwnedPolishDraft = {
  text: string;
  token: PolishDraftToken;
};

function compactStatus({
  hasPolishedText,
  originalText,
  ready,
  running,
}: {
  hasPolishedText: boolean;
  originalText?: string;
  ready: boolean;
  running: boolean;
}) {
  if (!ready) return "Select a finished transcript to polish.";
  if (running) return "Polishing your transcript…";
  if (hasPolishedText) return "Polished draft ready.";
  if (originalText) return "Ready when you are.";
  return "Loading transcript…";
}

export function usePolishDraft({
  item,
  onPolished,
  onSave,
  originalText,
  polishedText,
}: {
  item?: RecordingJobView;
  onPolished: (outputPath: string, text: string) => void;
  onSave: (request: PolishSaveRequest) => Promise<string>;
  originalText?: string;
  polishedText?: string;
}) {
  const ready = Boolean(item?.outputPath && isRecordingFinished(item.status));
  const [tone, setTone] = useState<PolishTone>("light");
  const [running, setRunning] = useState(false);
  const [saving, setSaving] = useState(false);
  const [runDetails, setRunDetails] = useState<PolishRunDetails | null>(null);
  const [savedPath, setSavedPath] = useState("");
  const [ownedDraft, setOwnedDraft] = useState<OwnedPolishDraft>();
  const currentSourceText = originalText ?? "";
  const currentSourceIdentity = item ? polishSourceIdentity(item, currentSourceText) : "";
  const currentContext = currentSourceIdentity ? `${currentSourceIdentity}\0${tone}` : "";
  const currentContextRef = useRef(currentContext);
  currentContextRef.current = currentContext;
  const polishedTextRef = useRef(polishedText);
  polishedTextRef.current = polishedText;
  const operationOwnerRef = useRef(createPolishOperationOwner());

  useLayoutEffect(() => {
    operationOwnerRef.current.invalidate();
    setSaving(false);
  }, [currentContext]);

  useLayoutEffect(() => () => {
    operationOwnerRef.current.invalidate();
  }, []);

  const hasPolishedText = Boolean(
    ownedDraft
    && operationOwnerRef.current.currentDraft(currentContext) === ownedDraft.token
    && polishedText === ownedDraft.text
    && isPolishDraftCurrent({
      currentContext,
      draftContext: ownedDraft.token.context,
      running,
      text: polishedText,
    }),
  );
  const currentPolishedText = hasPolishedText ? polishedText : undefined;
  const canPolish = ready && originalText !== undefined && !running && !saving;
  const statusLine = compactStatus({ hasPolishedText, originalText, ready, running });

  useEffect(() => {
    setOwnedDraft(undefined);
    setRunDetails(null);
    setRunning(false);
    setSavedPath("");
  }, [currentContext]);

  async function runPolish() {
    if (!item?.outputPath || originalText === undefined || running || saving) return;

    const outputPath = item.outputPath;
    const source = originalText;
    const requestedContext = currentContext;
    const requestedTone = tone;
    const operation = operationOwnerRef.current.startRun(requestedContext);
    if (!operation) return;
    setRunning(true);
    setOwnedDraft(undefined);
    setRunDetails(null);
    setSavedPath("");

    try {
      const result = await polishTranscript({
        signal: operation.signal,
        text: source,
        tone: requestedTone,
      });
      const nextDraft = operationOwnerRef.current.acceptRun(operation);
      if (!nextDraft) return;

      onPolished(outputPath, result.text);
      setOwnedDraft({ text: result.text, token: nextDraft });
      setRunDetails({
        model: result.model.replace("gemma4:", ""),
        tokensPerSecond: result.tokensPerSecond,
        totalSeconds: result.totalSeconds,
      });
    } catch (error) {
      if (!operation.signal.aborted && operationOwnerRef.current.isRunCurrent(operation)) {
        toast.error(error instanceof Error ? error.message : String(error));
      }
    } finally {
      if (operationOwnerRef.current.finishRun(operation)) {
        setRunning(false);
      }
    }
  }

  async function copyPolished() {
    if (!currentPolishedText) return;

    try {
      await navigator.clipboard.writeText(currentPolishedText);
      toast.success("Polished draft copied");
    } catch {
      toast.error("Copy failed");
    }
  }

  async function savePolished() {
    if (!item || !currentPolishedText || !ownedDraft || saving || running) return;
    if (
      currentContextRef.current !== ownedDraft.token.context
      || polishedTextRef.current !== ownedDraft.text
      || operationOwnerRef.current.currentDraft(currentContextRef.current) !== ownedDraft.token
    ) return;

    const saveOperation = operationOwnerRef.current.startSave(ownedDraft.token);
    if (!saveOperation || !operationOwnerRef.current.acceptSave(saveOperation)) return;
    const request = createPolishSaveRequest({
      context: currentContext,
      item,
      sourceText: currentSourceText,
      sourceIdentity: currentSourceIdentity,
      text: ownedDraft.text,
      token: saveOperation,
    });
    if (!request) {
      operationOwnerRef.current.finishSave(saveOperation);
      return;
    }
    setSaving(true);
    try {
      const path = await onSave(request);
      if (
        path
        && saveOperation.isCurrent()
        && currentContextRef.current === saveOperation.draft.context
      ) setSavedPath(path);
    } catch {
      // onSave surfaces save errors via toast.
    } finally {
      if (operationOwnerRef.current.finishSave(saveOperation)) {
        if (currentContextRef.current !== saveOperation.draft.context) {
          operationOwnerRef.current.invalidate();
        }
        setSaving(false);
      }
    }
  }

  return {
    canPolish,
    copyPolished,
    currentPolishedText,
    hasPolishedText,
    ready,
    runDetails,
    runPolish,
    running,
    savedPath,
    savePolished,
    saving,
    setTone,
    statusLine,
    tone,
  };
}

import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { CaretDown as ChevronDown } from "@phosphor-icons/react/CaretDown";
import { Copy } from "@phosphor-icons/react/Copy";
import { FloppyDisk as Save } from "@phosphor-icons/react/FloppyDisk";
import { Sparkle as Sparkles } from "@phosphor-icons/react/Sparkle";
import { toast } from "sonner";

import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ButtonGroup } from "@/components/ui/button-group";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { basename, isRecordingFinished, type RecordingJobView } from "@/lib/app-types";
import { developmentPolishAvailable } from "@/lib/product-features";
import {
  createPolishSaveRequest,
  createPolishOperationOwner,
  isPolishDraftCurrent,
  polishSourceIdentity,
  polishToneHints,
  polishToneLabels,
  polishTranscript,
  type PolishDraftToken,
  type PolishSaveRequest,
  type PolishTone,
} from "@/polish";

type RunDetails = {
  model: string;
  tokensPerSecond?: number;
  totalSeconds?: number;
};

type OwnedPolishDraft = {
  text: string;
  token: PolishDraftToken;
};

type PolishPanelProps = {
  item?: RecordingJobView;
  onLoadText: (path: string) => Promise<string>;
  onOpenHelp?: () => void;
  onPolished: (outputPath: string, text: string) => void;
  onSave: (request: PolishSaveRequest) => Promise<string>;
  originalText?: string;
  polishedText?: string;
};

function PreviewColumn({ empty, title, value }: { empty: string; title: string; value?: string }) {
  return (
    <div className="min-w-0">
      <div className="border-b p-3">
        <p className="text-xs font-semibold text-muted-foreground">{title}</p>
      </div>
      <ScrollArea className="h-[220px]">
        <div className="p-4">
          {value?.trim() ? (
            <pre className="whitespace-pre-wrap break-words text-[15px] leading-7 text-foreground">{value}</pre>
          ) : (
            <p className="text-sm leading-6 text-muted-foreground">{empty}</p>
          )}
        </div>
      </ScrollArea>
    </div>
  );
}

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

export function PolishPanel(props: PolishPanelProps) {
  if (!developmentPolishAvailable) {
    return (
      <Card className="surface-workspace-inset min-w-0 bg-card py-0">
        <CardHeader className="p-4 sm:p-5">
          <Badge className="w-fit" variant="secondary">
            <Sparkles data-icon="inline-start" />
            Polish
          </Badge>
          <CardTitle className="mt-3 text-2xl">Polish unavailable</CardTitle>
          <CardDescription>Local transcript cleanup is still in development.</CardDescription>
        </CardHeader>
      </Card>
    );
  }

  return <DevelopmentPolishPanel {...props} />;
}

function DevelopmentPolishPanel({
  item,
  onOpenHelp,
  onPolished,
  onSave,
  originalText,
  polishedText,
}: PolishPanelProps) {
  const ready = Boolean(item?.outputPath && isRecordingFinished(item.status));
  const [tone, setTone] = useState<PolishTone>("light");
  const [running, setRunning] = useState(false);
  const [saving, setSaving] = useState(false);
  const [runDetails, setRunDetails] = useState<RunDetails | null>(null);
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

  function handleToneKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key !== "Enter" || !canPolish || hasPolishedText) return;

    event.preventDefault();
    void runPolish();
  }

  return (
    <Card className="surface-workspace-inset min-w-0 bg-card py-0">
      <CardHeader className="p-4 sm:p-5">
        <div className="min-w-0">
          <Badge className="w-fit" variant={ready ? "default" : "secondary"}>
            <Sparkles data-icon="inline-start" />
            Polish
          </Badge>
          <CardTitle className="mt-3 text-2xl">{ready ? "Ready to refine" : "Waiting on a transcript"}</CardTitle>
          <CardDescription className="break-words">
            {item ? item.name : "Select or transcribe a recording to start from real text."}
          </CardDescription>
          {!ready && onOpenHelp ? (
            <Button
              className="mt-2 h-auto px-0 text-muted-foreground"
              onClick={onOpenHelp}
              size="sm"
              type="button"
              variant="link"
            >
              How this works
            </Button>
          ) : null}
        </div>
      </CardHeader>
      <CardContent className="grid gap-4 p-4 sm:p-5">
        <div className="grid gap-2" onKeyDown={handleToneKeyDown}>
          <ToggleGroup
            className="grid grid-cols-3"
            disabled={running || saving}
            onValueChange={(value) => {
              if (value) setTone(value as PolishTone);
            }}
            type="single"
            value={tone}
          >
            {(Object.entries(polishToneLabels) as [PolishTone, string][]).map(([value, label]) => (
              <ToggleGroupItem key={value} value={value}>
                {label}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
          <p className="text-sm leading-6 text-muted-foreground">
            {polishToneHints[tone]}
            {canPolish && !hasPolishedText ? " Press Enter to run Polish." : null}
          </p>
          {ready && !hasPolishedText && onOpenHelp ? (
            <Button
              className="h-auto w-fit px-0 text-muted-foreground"
              onClick={onOpenHelp}
              size="sm"
              type="button"
              variant="link"
            >
              Learn more
            </Button>
          ) : null}
        </div>

        {hasPolishedText ? (
          <ButtonGroup
            aria-label="Polished draft actions"
            className="w-full sm:w-auto [&>[data-slot=button]]:flex-1 sm:[&>[data-slot=button]]:flex-none"
          >
            <Button onClick={() => void copyPolished()} size="sm" type="button">
              <Copy data-icon="inline-start" />
              Copy
            </Button>
            <Button disabled={saving} onClick={() => void savePolished()} size="sm" type="button" variant="secondary">
              {saving ? <Spinner data-icon="inline-start" /> : <Save data-icon="inline-start" />}
              Save
            </Button>
            <Button disabled={!canPolish} onClick={() => void runPolish()} size="sm" type="button" variant="ghost">
              {running ? <Spinner data-icon="inline-start" /> : <Sparkles data-icon="inline-start" />}
              Polish again
            </Button>
          </ButtonGroup>
        ) : (
          <Button className="w-full sm:w-auto" disabled={!canPolish} onClick={() => void runPolish()} type="button">
            {running ? <Spinner data-icon="inline-start" /> : <Sparkles data-icon="inline-start" />}
            Polish
          </Button>
        )}

        <div className="min-w-0 overflow-hidden rounded-lg border bg-[var(--surface-transcript)] lg:grid lg:grid-cols-2 lg:divide-x">
          <PreviewColumn
            title="Original"
            value={originalText}
            empty={ready ? "Loading transcript preview." : "No transcript selected."}
          />
          <PreviewColumn
            title="Polished"
            value={currentPolishedText}
            empty="Run Polish to create a cleaned draft."
          />
        </div>

        <div className="flex flex-col gap-2">
          <p className="text-sm text-muted-foreground">{statusLine}</p>
          {ready ? (
            <Collapsible>
              <CollapsibleTrigger asChild>
                <Button
                  className="h-auto w-fit gap-1 px-0 text-muted-foreground hover:text-foreground"
                  size="sm"
                  type="button"
                  variant="link"
                >
                  Details
                  <ChevronDown className="size-4 transition-transform [[data-state=open]_&]:rotate-180" />
                </Button>
              </CollapsibleTrigger>
              <CollapsibleContent className="text-sm leading-6 text-muted-foreground">
                {runDetails ? (
                  <p>
                    {[
                      runDetails.totalSeconds ? `${runDetails.totalSeconds}s` : "",
                      runDetails.tokensPerSecond ? `${runDetails.tokensPerSecond} tok/s` : "",
                      runDetails.model,
                      "On this device",
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </p>
                ) : (
                  <p>Polish runs locally on this device.</p>
                )}
              </CollapsibleContent>
            </Collapsible>
          ) : null}
        </div>

        {savedPath ? (
          <Alert>
            <Save />
            <AlertDescription>
              Saved to <span className="font-medium text-foreground">{basename(savedPath)}</span>
            </AlertDescription>
          </Alert>
        ) : null}
      </CardContent>
    </Card>
  );
}

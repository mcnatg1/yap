import { Copy } from "@phosphor-icons/react/Copy";
import { FloppyDisk as Save } from "@phosphor-icons/react/FloppyDisk";
import { Sparkle as Sparkles } from "@phosphor-icons/react/Sparkle";
import type { KeyboardEvent } from "react";

import { PolishDetails, PolishPreviewColumn } from "@/components/polish/polish-preview";
import { usePolishDraft } from "@/components/polish/use-polish-draft";
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
import { Spinner } from "@/components/ui/spinner";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { basename, type RecordingJobView } from "@/lib/app-types";
import { developmentPolishAvailable } from "@/lib/product-features";
import {
  polishToneHints,
  polishToneLabels,
  type PolishSaveRequest,
  type PolishTone,
} from "@/polish";

type PolishPanelProps = {
  item?: RecordingJobView;
  onOpenHelp?: () => void;
  onPolished: (outputPath: string, text: string) => void;
  onSave: (request: PolishSaveRequest) => Promise<string>;
  originalText?: string;
  polishedText?: string;
};

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
  const draft = usePolishDraft({
    item,
    onPolished,
    onSave,
    originalText,
    polishedText,
  });

  function handleToneKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key !== "Enter" || !draft.canPolish || draft.hasPolishedText) return;
    event.preventDefault();
    void draft.runPolish();
  }

  return (
    <Card className="surface-workspace-inset min-w-0 bg-card py-0">
      <CardHeader className="p-4 sm:p-5">
        <div className="min-w-0">
          <Badge className="w-fit" variant={draft.ready ? "default" : "secondary"}>
            <Sparkles data-icon="inline-start" />
            Polish
          </Badge>
          <CardTitle className="mt-3 text-2xl">
            {draft.ready ? "Ready to refine" : "Waiting on a transcript"}
          </CardTitle>
          <CardDescription className="break-words">
            {item ? item.name : "Select or transcribe a recording to start from real text."}
          </CardDescription>
          {!draft.ready && onOpenHelp ? (
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
            disabled={draft.running || draft.saving}
            onValueChange={(value) => {
              if (value) draft.setTone(value as PolishTone);
            }}
            type="single"
            value={draft.tone}
          >
            {(Object.entries(polishToneLabels) as [PolishTone, string][]).map(([value, label]) => (
              <ToggleGroupItem key={value} value={value}>
                {label}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
          <p className="text-sm leading-6 text-muted-foreground">
            {polishToneHints[draft.tone]}
            {draft.canPolish && !draft.hasPolishedText ? " Press Enter to run Polish." : null}
          </p>
          {draft.ready && !draft.hasPolishedText && onOpenHelp ? (
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

        {draft.hasPolishedText ? (
          <ButtonGroup
            aria-label="Polished draft actions"
            className="w-full sm:w-auto [&>[data-slot=button]]:flex-1 sm:[&>[data-slot=button]]:flex-none"
          >
            <Button onClick={() => void draft.copyPolished()} size="sm" type="button">
              <Copy data-icon="inline-start" />
              Copy
            </Button>
            <Button
              disabled={draft.saving}
              onClick={() => void draft.savePolished()}
              size="sm"
              type="button"
              variant="secondary"
            >
              {draft.saving ? <Spinner data-icon="inline-start" /> : <Save data-icon="inline-start" />}
              Save
            </Button>
            <Button
              disabled={!draft.canPolish}
              onClick={() => void draft.runPolish()}
              size="sm"
              type="button"
              variant="ghost"
            >
              {draft.running ? <Spinner data-icon="inline-start" /> : <Sparkles data-icon="inline-start" />}
              Polish again
            </Button>
          </ButtonGroup>
        ) : (
          <Button
            className="w-full sm:w-auto"
            disabled={!draft.canPolish}
            onClick={() => void draft.runPolish()}
            type="button"
          >
            {draft.running ? <Spinner data-icon="inline-start" /> : <Sparkles data-icon="inline-start" />}
            Polish
          </Button>
        )}

        <div className="min-w-0 overflow-hidden rounded-lg border bg-[var(--surface-transcript)] lg:grid lg:grid-cols-2 lg:divide-x">
          <PolishPreviewColumn
            empty={draft.ready ? "Loading transcript preview." : "No transcript selected."}
            title="Original"
            value={originalText}
          />
          <PolishPreviewColumn
            empty="Run Polish to create a cleaned draft."
            title="Polished"
            value={draft.currentPolishedText}
          />
        </div>

        <PolishDetails
          ready={draft.ready}
          runDetails={draft.runDetails}
          statusLine={draft.statusLine}
        />

        {draft.savedPath ? (
          <Alert>
            <Save />
            <AlertDescription>
              Saved to <span className="font-medium text-foreground">{basename(draft.savedPath)}</span>
            </AlertDescription>
          </Alert>
        ) : null}
      </CardContent>
    </Card>
  );
}

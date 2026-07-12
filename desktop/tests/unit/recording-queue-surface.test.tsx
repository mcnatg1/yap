import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { QueuePanel } from "@/components/panels/queue-panel";
import { TooltipProvider } from "@/components/ui/tooltip";
import { createInitialPipelineState, type RecordingJobView } from "@/lib/app-types";

const source = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");

describe("imported recording queue surface", () => {
  it("renders no local execution action or synthetic queue progress without a connector", () => {
    const item: RecordingJobView = {
      error: "Server unavailable",
      id: 1,
      intent: "recording",
      name: "meeting.wav",
      path: "C:/meeting.wav",
      pipeline: createInitialPipelineState(),
      route: "serverBatch",
      status: "failed",
    };
    const legacyExecutionProps = {
      completed: 0,
      elapsedSeconds: 0,
      hasRunnable: true,
      onRetry: vi.fn(),
      onRun: vi.fn(),
      queueProgress: 0,
      running: false,
    };

    const html = renderToStaticMarkup(
      <TooltipProvider>
        <QueuePanel
          {...legacyExecutionProps}
          onClear={vi.fn()}
          onRemove={vi.fn()}
          onReveal={vi.fn()}
          onSelect={vi.fn()}
          queue={[item]}
        />
      </TooltipProvider>,
    );

    expect(html).not.toContain("Transcribe</button>");
    expect(html).not.toContain("Queue progress");
  });

  it("contains no local-batch compatibility path in the owned frontend surface", () => {
    const ownedSources = [
      source("../../src/App.tsx"),
      source("../../src/components/panels/queue-panel.tsx"),
      source("../../src/components/stacked-upload.tsx"),
      source("../../src/lib/app-types.ts"),
      source("../../src/lib/setup-model-state.ts"),
    ].join("\n");

    expect(ownedSources).not.toMatch(/queued_local_fallback|local_transcribing/);
    expect(ownedSources).not.toMatch(/startTranscribe|transcribeItems|runQueue|retryItem/);
    expect(source("../../src/App.tsx")).toContain("useImportedRecordingQueue");
  });
});

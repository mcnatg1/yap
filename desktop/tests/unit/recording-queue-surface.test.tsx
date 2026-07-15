import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { QueuePanel } from "@/components/panels/queue-panel";
import { TooltipProvider } from "@/components/ui/tooltip";
import { createInitialPipelineState, type RecordingJobView } from "@/lib/recording-job";

const source = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");

describe("imported recording queue surface", () => {
  it("renders no local execution action or synthetic queue progress without a connector", () => {
    const item: RecordingJobView = {
      error: "Server unavailable",
      id: "job-meeting",
      name: "meeting.wav",
      sourcePath: "C:/meeting.wav",
      pipeline: createInitialPipelineState(),
      route: "serverBatch",
      sessionMode: "meeting",
      sessionOrigin: "importedFile",
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
          legacyDiscardAllowed={false}
          onClear={vi.fn()}
          onDiscardLegacyQueue={vi.fn()}
          onRemove={vi.fn()}
          onReveal={vi.fn()}
          onRetryMigration={vi.fn()}
          onSelect={vi.fn()}
          queue={[item]}
          migrationPending={false}
        />
      </TooltipProvider>,
    );

    expect(html).not.toContain("Transcribe</button>");
    expect(html).not.toContain("Queue progress");
  });

  it("contains no local-batch compatibility path in the owned frontend surface", () => {
    expect(source("../../src/App.tsx")).not.toMatch(/startTranscribe|transcribeItems|runQueue/);
    expect(source("../../src/App.tsx")).toContain("useRecordingJobs");
    expect(source("../../src/hooks/use-imported-recording-queue.ts"))
      .not.toMatch(/queued_local_fallback|local_transcribing/);
  });

  it("keeps cancellation reachable while a remote job is active", () => {
    const item: RecordingJobView = {
      id: "job-uploading",
      name: "meeting.wav",
      sourcePath: "C:/meeting.wav",
      pipeline: createInitialPipelineState(),
      route: "serverBatch",
      sessionMode: "meeting",
      sessionOrigin: "importedFile",
      status: "uploading",
    };

    const html = renderToStaticMarkup(
      <TooltipProvider>
        <QueuePanel
          legacyDiscardAllowed={false}
          migrationPending={false}
          onClear={vi.fn()}
          onDiscardLegacyQueue={vi.fn()}
          onRemove={vi.fn()}
          onReveal={vi.fn()}
          onRetryMigration={vi.fn()}
          onSelect={vi.fn()}
          queue={[item]}
        />
      </TooltipProvider>,
    );

    expect(html).toContain('aria-label="Cancel recording"');
    expect(html).not.toMatch(/aria-label="Cancel recording"[^>]*disabled/);
  });

  it("offers confirmed legacy discard only when a migration failure allows it", () => {
    const renderFailure = (legacyDiscardAllowed: boolean) => renderToStaticMarkup(
      <TooltipProvider>
        <QueuePanel
          legacyDiscardAllowed={legacyDiscardAllowed}
          migrationError="Queued recording migration needs attention"
          migrationPending={false}
          onClear={vi.fn()}
          onDiscardLegacyQueue={vi.fn()}
          onRemove={vi.fn()}
          onReveal={vi.fn()}
          onRetryMigration={vi.fn()}
          onSelect={vi.fn()}
          queue={[]}
        />
      </TooltipProvider>,
    );

    expect(renderFailure(false)).not.toContain("Discard old queue");
    expect(renderFailure(true)).toContain("Discard old queue");

    const panelSource = source("../../src/components/panels/queue-panel.tsx");
    expect(panelSource).toContain("Discard the old queue?");
    expect(panelSource).toContain("onClick={onDiscardLegacyQueue}");
  });

  it("wires the guarded discard owner from the recording jobs hook into the queue panel", () => {
    const appSource = source("../../src/App.tsx");
    expect(appSource).toContain("discardLegacyQueue,");
    expect(appSource).toContain("legacyDiscardAllowed,");
    expect(appSource).toContain("legacyDiscardAllowed={legacyDiscardAllowed}");
    expect(appSource).toContain(
      'onDiscardLegacyQueue={() => reportRecordingAction(discardLegacyQueue, "Could not discard old queue")}',
    );
  });
});

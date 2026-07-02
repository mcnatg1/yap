import { invoke, isTauri } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import { AppChrome } from "@/components/app/app-chrome";
import { AppSidebar } from "@/components/app/app-sidebar";
import { openDevtools } from "@/components/app/window-actions";
import { CommandCenter } from "@/components/command-center";
import { HelpSheet, SettingsSheet } from "@/components/panels/app-sheets";
import { DropHero } from "@/components/panels/drop-hero";
import { HomePanel } from "@/components/panels/home-panel";
import { HistoryPanel } from "@/components/panels/history-panel";
import { PolishPanel } from "@/components/panels/polish-panel";
import { QueuePanel } from "@/components/panels/queue-panel";
import { TranscriptPanel } from "@/components/panels/transcript-panel";
import { WorkspaceHeader } from "@/components/panels/workspace-header";
import { type UploadItem } from "@/components/stacked-upload";
import { TranscriptPreviewDialog } from "@/components/transcript-preview-dialog";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { useElapsedSeconds } from "@/hooks/use-elapsed-seconds";
import {
  readTranscriptHistory,
  recordTranscriptHistory,
  removeTranscriptHistory,
  writeTranscriptHistory,
  type TranscriptHistoryEntry,
} from "@/history";
import {
  acceptedFormats,
  audioExtensions,
  audioExts,
  basename,
  extension,
  type RailAction,
  type WorkspaceView,
  groupHistoryByDay,
  workspaceCopy,
} from "@/lib/app-types";
import { historyEntryToUploadItem } from "@/lib/history-utils";
import { cn } from "@/lib/utils";
import {
  listenTranscribeEvents,
  SttInvokeError,
  startTranscribe,
  transcriptFileError,
  type TranscribeBatchCompleteEvent,
  type TranscribeFileCompleteEvent,
  type TranscribeProgressEvent,
} from "@/stt";

type SetupStatus = {
  model: string;
  root: string;
  pythonReady: boolean;
  scriptReady: boolean;
  python: string;
  engineReady: boolean;
  engineBinaryStatus: string;
  usingFallback: boolean;
  engineStatus: string;
};

export default function App() {
  const [queue, setQueue] = useState<UploadItem[]>([]);
  const [nextId, setNextId] = useState(1);
  const [dragging, setDragging] = useState(false);
  const [running, setRunning] = useState(false);
  const [runningSince, setRunningSince] = useState<number>();
  const [status, setStatus] = useState("Starting");
  const [model, setModel] = useState("Cohere Transcribe");
  const [auth, setAuth] = useState("Checking");
  const [engineBinaryStatus, setEngineBinaryStatus] = useState("Checking");
  const [selectedId, setSelectedId] = useState<number>();
  const [activeRail, setActiveRail] = useState<RailAction>("home");
  const [workspaceView, setWorkspaceView] = useState<WorkspaceView>("home");
  const [railCollapsed, setRailCollapsed] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [commandOpen, setCommandOpen] = useState(false);
  const [transcriptText, setTranscriptText] = useState<Record<string, string>>({});
  const [polishedText, setPolishedText] = useState<Record<string, string>>({});
  const [history, setHistory] = useState<TranscriptHistoryEntry[]>(() => readTranscriptHistory());
  const [selectedHistoryOutput, setSelectedHistoryOutput] = useState<string>();
  const [previewEntry, setPreviewEntry] = useState<TranscriptHistoryEntry>();
  const [previewText, setPreviewText] = useState("");
  const pathToItemId = useRef<Map<string, number>>(new Map());

  const hasRunnable = useMemo(
    () => queue.some((item) => item.status === "queued" || item.status === "error"),
    [queue],
  );
  const completed = queue.filter((item) => item.status === "done").length;
  const queueProgress = queue.length ? Math.round((completed / queue.length) * 100) : 0;
  const runningItem = queue.find((item) => item.status === "running");
  const elapsedSeconds = useElapsedSeconds(runningSince);
  const selectedHistoryEntry = history.find((entry) => entry.outputPath === selectedHistoryOutput);
  const selectedHistoryItem = selectedHistoryEntry ? historyEntryToUploadItem(selectedHistoryEntry) : undefined;
  const selectedItem =
    queue.find((item) => item.id === selectedId) ??
    selectedHistoryItem ??
    [...queue].reverse().find((item) => item.status === "done") ??
    (history[0] ? historyEntryToUploadItem(history[0]) : undefined) ??
    queue[0];
  const workspace = workspaceCopy[workspaceView];
  const showQueue = workspaceView === "transcribe";
  const showHistory = workspaceView === "transcripts";
  const showTranscript = workspaceView === "transcribe" || workspaceView === "transcripts" || workspaceView === "polish";
  const showPolish = workspaceView === "polish";

  useEffect(() => {
    if (!isTauri()) return;

    let unlisten: (() => void) | undefined;

    void listenTranscribeEvents({
      onProgress: (event) => {
        updateItemProgress(event);
      },
      onFileComplete: (event) => {
        applyFileResult(event);
      },
      onComplete: (event) => {
        finishBatch(event);
      },
      onError: (error) => {
        setRunning(false);
        setRunningSince(undefined);
        setStatus("Needs attention");
        setAuth(error.message.includes("Hugging Face") ? "Run hf auth login" : "Check runner output");
        toast.error(error.message || "Transcription failed");
      },
    }).then((stop) => {
      unlisten = stop;
    });

    return () => {
      unlisten?.();
    };
  }, []);

  function updateQueueItem(path: string, updater: (item: UploadItem) => UploadItem) {
    setQueue((items) =>
      items.map((entry) => (entry.path === path ? updater(entry) : entry)),
    );
  }

  function updateItemProgress(event: TranscribeProgressEvent) {
    updateQueueItem(event.path, (entry) => ({
      ...entry,
      status: "running",
      progressPhase: event.phase,
      progressPercent: event.percent,
      progressMessage: event.message,
      error: undefined,
    }));
    setStatus(event.message);
    const itemId = pathToItemId.current.get(event.path);
    if (itemId !== undefined) setSelectedId(itemId);
  }

  function applyFileResult(event: TranscribeFileCompleteEvent) {
    const { result } = event;
    const itemId = pathToItemId.current.get(event.path);

    if (result.error) {
      const message = transcriptFileError(result) ?? "Transcription failed.";
      updateQueueItem(event.path, (entry) => ({
        ...entry,
        status: "error",
        error: message,
        progressPhase: undefined,
        progressPercent: undefined,
        progressMessage: undefined,
      }));
      toast.error(`${basename(event.path)}: ${message}`);
      return;
    }

    updateQueueItem(event.path, (entry) => ({
      ...entry,
      output: result.output,
      status: "done",
      error: undefined,
      progressPhase: "done",
      progressPercent: 100,
      progressMessage: "Transcript saved",
    }));
    recordHistoryEntries([
      {
        createdAt: new Date().toISOString(),
        name: basename(event.path),
        outputPath: result.output,
        sourcePath: event.path,
      },
    ]);
    void loadTranscriptText(result.output).catch(() => undefined);
    if (itemId !== undefined) setSelectedId(itemId);
  }

  function finishBatch(event: TranscribeBatchCompleteEvent) {
    setStatus(event.failed ? "Needs attention" : "Ready");
    setAuth("Authorized");
    if (event.succeeded) {
      toast.success(`Transcribed ${event.succeeded} file${event.succeeded === 1 ? "" : "s"}`);
    }
    setRunning(false);
    setRunningSince(undefined);
    pathToItemId.current.clear();
  }

  useEffect(() => {
    void loadStatus();

    if (!isTauri()) {
      setStatus("Preview");
      setAuth("Tauri bridge");
      return;
    }

    const unlistenDrag = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "enter") setDragging(true);
      if (event.payload.type === "leave" || event.payload.type === "drop") setDragging(false);
      if (event.payload.type === "drop") addPaths(event.payload.paths);
    });

    return () => {
      void unlistenDrag.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    function onKeyDown(event: globalThis.KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setCommandOpen((open) => !open);
      }

      if ((event.ctrlKey || event.metaKey) && event.shiftKey && event.key.toLowerCase() === "i") {
        event.preventDefault();
        void openDevtools();
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  useEffect(() => {
    if (selectedHistoryOutput) return;

    if (!queue.length) {
      setSelectedId(undefined);
      return;
    }

    if (!selectedId || !queue.some((item) => item.id === selectedId)) {
      setSelectedId(queue[queue.length - 1].id);
    }
  }, [queue, selectedId, selectedHistoryOutput]);

  useEffect(() => {
    if (selectedHistoryOutput && !history.some((entry) => entry.outputPath === selectedHistoryOutput)) {
      setSelectedHistoryOutput(undefined);
    }
  }, [history, selectedHistoryOutput]);

  useEffect(() => {
    if (selectedItem?.output && !transcriptText[selectedItem.output]) {
      void loadTranscriptText(selectedItem.output).catch(() => toast.error("Preview unavailable"));
    }
  }, [selectedItem?.output, transcriptText]);

  useEffect(() => {
    if (workspaceView !== "home" || !isTauri()) return;

    const todayEntries = groupHistoryByDay(history).find((group) => group.label === "Today")?.entries ?? [];
    for (const entry of todayEntries.slice(0, 10)) {
      if (!transcriptText[entry.outputPath]) {
        void loadTranscriptText(entry.outputPath).catch(() => undefined);
      }
    }
  }, [history, transcriptText, workspaceView]);

  async function loadStatus() {
    if (!isTauri()) return;

    try {
      const setup = await invoke<SetupStatus>("setup_status");
      setModel(setup.model.replace("CohereLabs/", "").replace("ZoOtMcNoOt/", ""));
      setStatus(
        setup.engineReady || (setup.pythonReady && setup.scriptReady)
          ? setup.engineStatus
          : "Setup missing",
      );
      setAuth(setup.pythonReady ? "Authorized" : setup.python);
      setEngineBinaryStatus(setup.engineBinaryStatus);
    } catch (error) {
      setStatus("Setup check failed");
      setAuth(String(error));
    }
  }

  function addPaths(paths: string[]) {
    setQueue((current) => {
      const existing = new Set(current.map((item) => item.path));
      const accepted = paths.filter((path) => audioExts.has(extension(path)) && !existing.has(path));
      if (paths.length && !accepted.length) {
        toast.warning(`Drop ${acceptedFormats} files.`);
        return current;
      }

      const newItems = accepted.map((path, index) => ({
        id: nextId + index,
        path,
        name: basename(path),
        status: "queued" as const,
      }));
      setNextId((id) => id + newItems.length);
      if (newItems.length) {
        setActiveRail("transcribe");
        setWorkspaceView("transcribe");
        setSelectedHistoryOutput(undefined);
        setSelectedId(newItems[newItems.length - 1].id);
      }
      return [...current, ...newItems];
    });
  }

  function goToTranscribe() {
    setActiveRail("transcribe");
    setWorkspaceView("transcribe");
  }

  async function pickFiles() {
    goToTranscribe();

    if (!isTauri()) {
      toast.info("Preview only");
      return;
    }

    try {
      const selected = await openDialog({
        multiple: true,
        title: "Choose recordings",
        filters: [{ name: "Audio and video", extensions: audioExtensions }],
      });
      if (Array.isArray(selected)) addPaths(selected);
      else if (selected) addPaths([selected]);
    } catch (error) {
      toast.error(`Picker failed: ${String(error)}`);
    }
  }

  function handleRailAction(action: RailAction) {
    setActiveRail(action);

    if (action === "details") {
      setDetailsOpen(true);
      void loadStatus();
      return;
    }
    if (action === "help") {
      setHelpOpen(true);
      return;
    }

    setWorkspaceView(action);

    if (action === "polish") {
      setStatus(selectedItem?.status === "done" ? "Transcript ready" : "Transcribe a file first");
    }
  }

  async function transcribeItems(pending: UploadItem[]) {
    if (!pending.length || running || !isTauri()) return;

    pathToItemId.current = new Map(pending.map((item) => [item.path, item.id]));
    setRunning(true);
    setRunningSince(Date.now());
    setStatus(`Transcribing 0/${pending.length}`);

    for (const [index, item] of pending.entries()) {
      if (index === 0) setSelectedId(item.id);
      setQueue((items) =>
        items.map((entry) =>
          entry.id === item.id
            ? {
                ...entry,
                status: index === 0 ? "running" : "queued",
                error: undefined,
                progressPhase: index === 0 ? "starting" : undefined,
                progressPercent: index === 0 ? 0 : undefined,
                progressMessage: index === 0 ? "Preparing…" : undefined,
              }
            : entry,
        ),
      );
    }

    try {
      await startTranscribe(pending.map((item) => item.path));
    } catch (error) {
      const failure = error instanceof SttInvokeError ? error : undefined;
      const message = failure?.message ?? String(error || "Transcription failed");
      setRunning(false);
      setRunningSince(undefined);
      pathToItemId.current.clear();
      setStatus("Needs attention");
      toast.error(message);
    }
  }

  async function runQueue() {
    const pending = queue.filter((item) => item.status === "queued" || item.status === "error");
    await transcribeItems(pending);
  }

  async function retryItem(id: number) {
    const item = queue.find((entry) => entry.id === id);
    if (!item || item.status !== "error" || running) return;

    setSelectedId(id);
    await transcribeItems([{ ...item, status: "queued", error: undefined }]);
  }

  function removeItem(id: number) {
    setQueue((items) => items.filter((item) => item.id !== id));
  }

  function selectQueueItem(id: number) {
    setSelectedHistoryOutput(undefined);
    setSelectedId(id);
  }

  function clearQueue() {
    if (!running) {
      setQueue([]);
      setTranscriptText({});
    }
  }

  async function loadTranscriptText(path: string) {
    if (transcriptText[path]) return transcriptText[path];
    if (!isTauri()) return "";

    const text = await invoke<string>("read_text_file", { path });
    setTranscriptText((current) => ({ ...current, [path]: text }));
    return text;
  }

  async function copyTranscript(item: UploadItem) {
    if (!item.output) return;

    try {
      const text = await loadTranscriptText(item.output);
      await navigator.clipboard.writeText(text || item.output);
      toast.success(text ? "Transcript copied" : "Path copied");
    } catch {
      toast.error("Copy failed");
    }
  }

  async function openTranscript(path: string) {
    try {
      await openPath(path);
      toast.success("Opened transcript");
    } catch {
      toast.error("Open failed");
    }
  }

  async function revealPath(path: string) {
    try {
      await revealItemInDir(path);
    } catch {
      toast.error("Reveal failed");
    }
  }

  async function savePolishedTranscript(item: UploadItem, text: string) {
    if (!item.output || !text.trim()) return "";

    try {
      const path = await invoke<string>("write_polished_text", { path: item.output, text });
      toast.success("Polished draft saved");
      return path;
    } catch (error) {
      toast.error("Save failed");
      throw error;
    }
  }

  function recordHistoryEntries(entries: TranscriptHistoryEntry[]) {
    if (!entries.length) return;

    setHistory((current) => {
      const next = entries.reduce(recordTranscriptHistory, current);
      try {
        writeTranscriptHistory(next);
      } catch {
        // ponytail: transcripts are already saved; history can be rebuilt manually if localStorage is full.
      }
      return next;
    });
  }

  function removeHistoryEntry(outputPath: string) {
    setHistory((current) => {
      const next = removeTranscriptHistory(current, outputPath);
      try {
        writeTranscriptHistory(next);
      } catch {
        // ponytail: removal is UI-only if localStorage refuses the write.
      }
      return next;
    });
    if (selectedHistoryOutput === outputPath) setSelectedHistoryOutput(undefined);
    toast.success("Removed from history");
  }

  function selectHistoryEntry(entry: TranscriptHistoryEntry) {
    setSelectedId(undefined);
    setSelectedHistoryOutput(entry.outputPath);
    setActiveRail("transcripts");
    setWorkspaceView("transcripts");
  }

  async function previewHistoryEntry(entry: TranscriptHistoryEntry) {
    selectHistoryEntry(entry);
    setPreviewEntry(entry);
    setPreviewText("");

    try {
      setPreviewText(await loadTranscriptText(entry.outputPath));
    } catch {
      setPreviewText("Preview unavailable. Open the transcript file from the actions menu.");
    }
  }

  const workspaceLeftPane = (
    <>
      {showQueue ? (
        <QueuePanel
          completed={completed}
          elapsedSeconds={elapsedSeconds}
          hasRunnable={hasRunnable}
          onClear={clearQueue}
          onRemove={removeItem}
          onRetry={(id) => void retryItem(id)}
          onReveal={(path) => void revealPath(path)}
          onRun={() => void runQueue()}
          onSelect={selectQueueItem}
          queue={queue}
          queueProgress={queueProgress}
          running={running}
          runningItem={runningItem}
          selectedId={selectedId}
        />
      ) : null}

      {showHistory ? (
        <HistoryPanel
          entries={history}
          onCopy={(entry) => void copyTranscript(historyEntryToUploadItem(entry))}
          onLoadPreviewText={(entry) => loadTranscriptText(entry.outputPath)}
          onOpen={(entry) => void openTranscript(entry.outputPath)}
          onOpenHelp={() => handleRailAction("help")}
          onPreview={(entry) => void previewHistoryEntry(entry)}
          onRemove={removeHistoryEntry}
          onReveal={(entry) => void revealPath(entry.outputPath)}
          onSelect={selectHistoryEntry}
          selectedOutputPath={selectedHistoryOutput ?? selectedItem?.output}
        />
      ) : null}

      {showPolish ? (
        <PolishPanel
          item={selectedItem}
          onLoadText={loadTranscriptText}
          onOpenHelp={() => handleRailAction("help")}
          onPolished={(outputPath, text) => {
            setPolishedText((current) => ({ ...current, [outputPath]: text }));
            toast.success("Polished draft ready");
          }}
          onSave={savePolishedTranscript}
          originalText={selectedItem?.output ? transcriptText[selectedItem.output] : undefined}
          polishedText={selectedItem?.output ? polishedText[selectedItem.output] : undefined}
        />
      ) : null}
    </>
  );
  const workspaceTranscriptPane = showTranscript ? (
    <div className="h-full min-w-0">
      <TranscriptPanel
        elapsedSeconds={elapsedSeconds}
        item={selectedItem}
        onCopy={copyTranscript}
        onOpen={(path) => void openTranscript(path)}
        onOpenHelp={() => handleRailAction("help")}
        onRetry={(id) => void retryItem(id)}
        onReveal={(path) => void revealPath(path)}
        running={running}
        text={selectedItem?.output ? transcriptText[selectedItem.output] : undefined}
      />
    </div>
  ) : null;
  const workspaceMain = (
    <div
      className={cn(
        "grid w-full min-w-0 gap-5",
        workspaceView === "transcribe" || workspaceView === "polish" || workspaceView === "transcripts"
          ? "grid-cols-[minmax(0,1fr)_minmax(320px,0.78fr)]"
          : "grid-cols-1",
      )}
    >
      {workspaceLeftPane}
      {workspaceTranscriptPane}
    </div>
  );
  const appWorkspace = (
    <section className="surface-workspace scrollbar-none h-full min-h-0 w-full min-w-0 flex-1 overflow-x-hidden overflow-y-auto bg-card p-[15px]">
      <WorkspaceHeader
        auth={auth}
        description={workspace.description}
        historyCount={history.length}
        onOpenCommand={() => setCommandOpen(true)}
        onOpenDetails={() => handleRailAction("details")}
        onOpenHelp={() => handleRailAction("help")}
        status={status}
        title={workspace.title}
      />

      {workspaceView === "home" ? (
        <HomePanel
          history={history}
          onOpenTranscribe={goToTranscribe}
          onPickFiles={() => void pickFiles()}
          onSelectEntry={selectHistoryEntry}
          onViewAll={() => handleRailAction("transcripts")}
          previewSnippet={(entry) => transcriptText[entry.outputPath]}
          queueCount={queue.length}
          running={running}
        />
      ) : (
        <>
          {workspaceView === "transcribe" ? (
            <DropHero
              dragging={dragging}
              onDragLeave={() => setDragging(false)}
              onDragOver={(event) => {
                event.preventDefault();
                setDragging(true);
              }}
              onDrop={(event) => {
                event.preventDefault();
                setDragging(false);
                if (!isTauri()) toast.info("Preview only");
              }}
              onOpenHelp={() => handleRailAction("help")}
              onPickFiles={() => void pickFiles()}
            />
          ) : null}

          <section className="mt-7 w-full min-w-0">
            {workspaceMain}
          </section>
        </>
      )}
    </section>
  );

  return (
    <SidebarProvider
      className="h-screen overflow-hidden bg-background text-foreground"
      onOpenChange={(open) => setRailCollapsed(!open)}
      open={!railCollapsed}
    >
      <AppSidebar active={activeRail} onAction={handleRailAction} />
      <SidebarInset className="flex min-h-0 flex-col overflow-hidden">
        <AppChrome onAction={handleRailAction} />
        <div className="min-h-0 w-full min-w-0 flex-1 overflow-hidden bg-background pb-[15px] pr-[15px] pt-0">
          {appWorkspace}
        </div>
      </SidebarInset>
      <SettingsSheet
        auth={auth}
        engineBinaryStatus={engineBinaryStatus}
        model={model}
        onOpenChange={(open) => {
          setDetailsOpen(open);
          if (!open && activeRail === "details") setActiveRail(workspaceView);
        }}
        open={detailsOpen}
        status={status}
      />
      <HelpSheet
        onOpenChange={(open) => {
          setHelpOpen(open);
          if (!open && activeRail === "help") setActiveRail(workspaceView);
        }}
        open={helpOpen}
      />
      <CommandCenter
        history={history}
        onAction={handleRailAction}
        onOpenChange={setCommandOpen}
        onPickFiles={() => void pickFiles()}
        onPreview={(entry) => void previewHistoryEntry(entry)}
        open={commandOpen}
      />
      <TranscriptPreviewDialog
        entry={previewEntry}
        onCopy={(entry) => void copyTranscript(historyEntryToUploadItem(entry))}
        onOpen={(entry) => void openTranscript(entry.outputPath)}
        onOpenChange={(open) => {
          if (!open) setPreviewEntry(undefined);
        }}
        onReveal={(entry) => void revealPath(entry.outputPath)}
        text={previewText}
      />
    </SidebarProvider>
  );
}

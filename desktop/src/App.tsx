import { invoke, isTauri } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  BadgeCheck,
  Copy,
  Cpu,
  FileAudio2,
  FileText,
  FolderOpen,
  FolderOutput,
  Grid2X2,
  HelpCircle,
  LockKeyhole,
  Minus,
  RotateCw,
  Save,
  Search,
  Settings2,
  Sparkles,
  Square,
  Trash2,
  UploadCloud,
  X,
} from "lucide-react";
import { type DragEvent, type ElementType, type KeyboardEvent, useEffect, useMemo, useState } from "react";
import { Bar, BarChart, CartesianGrid, XAxis } from "recharts";

import { StackedUpload, type UploadItem } from "@/components/stacked-upload";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb";
import { Button } from "@/components/ui/button";
import { ButtonGroup } from "@/components/ui/button-group";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from "@/components/ui/chart";
import { Checkbox } from "@/components/ui/checkbox";
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  CommandShortcut,
} from "@/components/ui/command";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Empty, EmptyDescription, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import {
  readTranscriptHistory,
  recordTranscriptHistory,
  removeTranscriptHistory,
  writeTranscriptHistory,
  type TranscriptHistoryEntry,
} from "@/history";
import { cn } from "@/lib/utils";
import { defaultPolishModel, polishToneLabels, polishTranscript, type PolishTone } from "@/polish";

type SetupStatus = {
  model: string;
  root: string;
  pythonReady: boolean;
  scriptReady: boolean;
  python: string;
};

type TranscriptResult = {
  input: string;
  output: string;
};

type RailAction = "home" | "recordings" | "transcripts" | "polish" | "details" | "help";
type WorkspaceView = "home" | "recordings" | "transcripts" | "polish";

const workspaceCopy: Record<WorkspaceView, { eyebrow: string; title: string; description: string }> = {
  home: {
    eyebrow: "Today",
    title: "Ready when you are.",
    description: "Drop recordings, transcribe them locally, and review the latest result in one place.",
  },
  recordings: {
    eyebrow: "Recordings",
    title: "Your recording queue.",
    description: "Manage the files waiting for transcription and restart anything that needs another pass.",
  },
  transcripts: {
    eyebrow: "Transcripts",
    title: "Review the finished text.",
    description: "Open, copy, or reveal the saved transcript for the selected recording.",
  },
  polish: {
    eyebrow: "Polish",
    title: "Clean up the selected transcript.",
    description: "Use the selected result as the working draft before adding richer rewrite tools.",
  },
};

const audioExtensions = ["mp3", "m4a", "wav", "mp4", "flac", "ogg", "webm"];
const audioExts = new Set(audioExtensions.map((format) => `.${format}`));
const acceptedFormats = "MP3, M4A, WAV, MP4, FLAC, OGG, WEBM";
const historyChartConfig = {
  transcripts: {
    label: "Transcripts",
    color: "var(--primary)",
  },
} satisfies ChartConfig;

function basename(path: string) {
  return path.split(/[\\/]/).pop() ?? path;
}

function extension(path: string) {
  const name = basename(path);
  const dot = name.lastIndexOf(".");
  return dot === -1 ? "" : name.slice(dot).toLowerCase();
}

function historyEntryToUploadItem(entry: TranscriptHistoryEntry): UploadItem {
  return {
    id: 0,
    name: entry.name,
    output: entry.outputPath,
    path: entry.sourcePath,
    status: "done",
  };
}

export default function App() {
  const [queue, setQueue] = useState<UploadItem[]>([]);
  const [nextId, setNextId] = useState(1);
  const [dragging, setDragging] = useState(false);
  const [running, setRunning] = useState(false);
  const [status, setStatus] = useState("Starting");
  const [model, setModel] = useState("Cohere Transcribe");
  const [auth, setAuth] = useState("Checking");
  const [selectedId, setSelectedId] = useState<number>();
  const [activeRail, setActiveRail] = useState<RailAction>("home");
  const [workspaceView, setWorkspaceView] = useState<WorkspaceView>("home");
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [commandOpen, setCommandOpen] = useState(false);
  const [transcriptText, setTranscriptText] = useState<Record<string, string>>({});
  const [polishedText, setPolishedText] = useState<Record<string, string>>({});
  const [history, setHistory] = useState<TranscriptHistoryEntry[]>(() => readTranscriptHistory());
  const [selectedHistoryOutput, setSelectedHistoryOutput] = useState<string>();
  const [previewEntry, setPreviewEntry] = useState<TranscriptHistoryEntry>();
  const [previewText, setPreviewText] = useState("");

  const hasRunnable = useMemo(
    () => queue.some((item) => item.status === "queued" || item.status === "error"),
    [queue],
  );
  const completed = queue.filter((item) => item.status === "done").length;
  const selectedHistoryEntry = history.find((entry) => entry.outputPath === selectedHistoryOutput);
  const selectedHistoryItem = selectedHistoryEntry ? historyEntryToUploadItem(selectedHistoryEntry) : undefined;
  const selectedItem =
    queue.find((item) => item.id === selectedId) ??
    selectedHistoryItem ??
    [...queue].reverse().find((item) => item.status === "done") ??
    (history[0] ? historyEntryToUploadItem(history[0]) : undefined) ??
    queue[0];
  const workspace = workspaceCopy[workspaceView];
  const breadcrumbResource = workspaceView === "home" ? undefined : selectedItem?.name;
  const showQueue = workspaceView === "home" || workspaceView === "recordings";
  const showHistory = workspaceView === "transcripts";
  const showTranscript = workspaceView === "home" || workspaceView === "transcripts" || workspaceView === "polish";
  const showPolish = workspaceView === "polish";

  useEffect(() => {
    loadStatus();

    if (isTauri()) {
      getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === "enter") setDragging(true);
        if (event.payload.type === "leave" || event.payload.type === "drop") setDragging(false);
        if (event.payload.type === "drop") addPaths(event.payload.paths);
      });
      return;
    }

    setStatus("Preview");
    setAuth("Tauri bridge");
  }, []);

  useEffect(() => {
    function onKeyDown(event: globalThis.KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setCommandOpen((open) => !open);
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
      void loadTranscriptText(selectedItem.output).catch(() => setStatus("Preview unavailable"));
    }
  }, [selectedItem?.output, transcriptText]);

  async function loadStatus() {
    if (!isTauri()) return;

    try {
      const setup = await invoke<SetupStatus>("setup_status");
      setModel(setup.model.replace("CohereLabs/", "").replace("ZoOtMcNoOt/", ""));
      setStatus(setup.pythonReady && setup.scriptReady ? "Ready" : "Setup missing");
      setAuth(setup.pythonReady ? "Authorized" : setup.python);
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
        setStatus(`Drop ${acceptedFormats} files.`);
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
        setSelectedHistoryOutput(undefined);
        setSelectedId(newItems[newItems.length - 1].id);
      }
      return [...current, ...newItems];
    });
  }

  async function pickFiles() {
    if (!isTauri()) {
      setStatus("Preview only");
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
      setStatus(`Picker failed: ${String(error)}`);
    }
  }

  function handleRailAction(action: RailAction) {
    setActiveRail(action);

    if (action === "details") {
      setDetailsOpen(true);
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

  async function runQueue() {
    const pending = queue.filter((item) => item.status === "queued" || item.status === "error");
    if (!pending.length || running) return;

    setRunning(true);
    setStatus("Transcribing locally");
    setSelectedId(pending[0].id);
    setQueue((items) =>
      items.map((item) =>
        pending.some((pendingItem) => pendingItem.id === item.id)
          ? { ...item, status: "running", error: undefined }
          : item,
      ),
    );

    try {
      const results = await invoke<TranscriptResult[]>("transcribe_files", {
        paths: pending.map((item) => item.path),
      });
      const outputs = new Map(results.map((result) => [result.input, result.output]));
      const texts: Record<string, string> = {};

      for (const result of results) {
        try {
          texts[result.output] = await invoke<string>("read_text_file", { path: result.output });
        } catch {
          // ponytail: transcript can still be revealed if eager preview read fails.
        }
      }

      setQueue((items) =>
        items.map((item) =>
          outputs.has(item.path) ? { ...item, output: outputs.get(item.path), status: "done" } : item,
        ),
      );
      recordHistoryEntries(
        pending.flatMap((item) => {
          const output = outputs.get(item.path);
          return output
            ? [
                {
                  createdAt: new Date().toISOString(),
                  name: item.name,
                  outputPath: output,
                  sourcePath: item.path,
                },
              ]
            : [];
        }),
      );
      setTranscriptText((current) => ({ ...current, ...texts }));
      setStatus("Ready");
      setAuth("Authorized");
    } catch (error) {
      const message = String(error || "Transcription failed");
      setQueue((items) =>
        items.map((item) =>
          pending.some((pendingItem) => pendingItem.id === item.id)
            ? { ...item, status: "error", error: message }
            : item,
        ),
      );
      setStatus("Needs attention");
      setAuth(message.includes("Hugging Face") ? "Run hf auth login" : "Check runner output");
    } finally {
      setRunning(false);
    }
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
      setStatus(text ? "Transcript copied" : "Path copied");
    } catch {
      setStatus("Copy failed");
    }
  }

  async function openTranscript(path: string) {
    try {
      await openPath(path);
      setStatus("Opened transcript");
    } catch {
      setStatus("Open failed");
    }
  }

  async function revealPath(path: string) {
    try {
      await revealItemInDir(path);
    } catch {
      setStatus("Reveal failed");
    }
  }

  async function savePolishedTranscript(item: UploadItem, text: string) {
    if (!item.output || !text.trim()) return "";

    try {
      const path = await invoke<string>("write_polished_text", { path: item.output, text });
      setStatus("Polished draft saved");
      return path;
    } catch (error) {
      setStatus("Save failed");
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
    setStatus("Removed from history");
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

  return (
    <main className="min-h-screen overflow-x-hidden bg-background text-foreground">
      <AppChrome />
      <div className="mx-auto flex w-full max-w-[1480px] min-w-0 gap-4 p-3 pt-0 sm:p-4 sm:pt-0">
        <ProductRail active={activeRail} auth={auth} model={model} onAction={handleRailAction} status={status} />

        <section className="w-full min-w-0 flex-1 overflow-hidden rounded-[28px] border bg-card/95 p-4 shadow-[0_20px_70px_rgba(32,28,20,0.08)] sm:p-6 lg:p-8">
          <header className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-start">
            <div className="min-w-0">
              <div className="mb-4 flex items-center gap-3 lg:hidden">
                <AppIcon className="size-10 rounded-xl shadow-sm" />
                <div>
                  <div className="text-lg font-semibold">Yap</div>
                  <div className="text-sm text-muted-foreground">Private local transcription</div>
                </div>
              </div>
              <WorkspaceBreadcrumb
                current={workspace.eyebrow}
                onHome={() => handleRailAction("home")}
                resource={breadcrumbResource}
              />
              <h1 className="mt-2 text-3xl font-semibold tracking-tight sm:text-4xl">{workspace.title}</h1>
              <p className="mt-3 max-w-2xl text-sm leading-6 text-muted-foreground">{workspace.description}</p>
            </div>

            <div className="grid w-full min-w-0 grid-cols-[repeat(3,minmax(0,1fr))_auto_auto] items-center gap-1 rounded-2xl bg-secondary p-1 lg:flex lg:w-auto lg:gap-2 lg:rounded-full">
              <Metric icon={FileAudio2} label={`${queue.length} file${queue.length === 1 ? "" : "s"}`} />
              <Metric icon={FileText} label={`${history.length} saved`} />
              <Metric icon={LockKeyhole} label={auth === "Authorized" ? "Local" : status} />
              <Button aria-label="Open command menu" onClick={() => setCommandOpen(true)} size="icon-sm" type="button" variant="outline">
                <Search />
              </Button>
              <Button aria-label="Open setup details" onClick={() => handleRailAction("details")} size="icon-sm" type="button" variant="outline">
                <Settings2 />
              </Button>
            </div>
          </header>

          {workspaceView === "home" ? (
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
                if (!isTauri()) setStatus("Preview only");
              }}
              onPickFiles={() => void pickFiles()}
            />
          ) : null}

          <section
            className={cn(
              "mt-7 grid w-full min-w-0 gap-5",
              workspaceView === "home" || workspaceView === "polish" || workspaceView === "transcripts"
                ? "xl:grid-cols-[minmax(0,1fr)_minmax(360px,0.78fr)]"
                : "xl:grid-cols-1",
            )}
          >
            {showQueue ? (
              <Card className="min-w-0 border-[#eee8de] bg-card py-0 shadow-none">
                <CardHeader className="p-4 sm:p-5">
                  <div className="min-w-0">
                    <Badge className="w-fit" variant="outline">Today</Badge>
                    <CardTitle className="mt-2 flex items-center gap-2 text-xl">
                      Queue
                      <Badge className="tabular-nums" variant="secondary">
                        {queue.length}
                      </Badge>
                    </CardTitle>
                    <CardDescription>
                      {completed
                        ? `${completed} transcript${completed === 1 ? "" : "s"} ready`
                        : "Drop recordings and transcribe them in place"}
                    </CardDescription>
                  </div>
                  <CardAction className="col-span-full col-start-1 row-span-1 row-start-2 w-full justify-self-stretch sm:col-span-1 sm:col-start-2 sm:row-span-2 sm:row-start-1 sm:w-auto sm:justify-self-end">
                    <ButtonGroup
                      aria-label="Queue actions"
                      className="w-full sm:w-auto [&>[data-slot=button]]:flex-1 sm:[&>[data-slot=button]]:flex-none"
                    >
                      <AlertDialog>
                        <AlertDialogTrigger asChild>
                          <Button disabled={running || !queue.length} size="sm" type="button" variant="outline">
                            <Trash2 data-icon="inline-start" />
                            Clear
                          </Button>
                        </AlertDialogTrigger>
                        <AlertDialogContent>
                          <AlertDialogHeader>
                            <AlertDialogTitle>Clear the queue?</AlertDialogTitle>
                            <AlertDialogDescription>
                              This removes the queued files from Yap. Saved transcript files and history stay untouched.
                            </AlertDialogDescription>
                          </AlertDialogHeader>
                          <AlertDialogFooter>
                            <AlertDialogCancel>Cancel</AlertDialogCancel>
                            <AlertDialogAction
                              className="bg-destructive text-white hover:bg-destructive/90 focus-visible:ring-destructive/20"
                              onClick={clearQueue}
                            >
                              Clear queue
                            </AlertDialogAction>
                          </AlertDialogFooter>
                        </AlertDialogContent>
                      </AlertDialog>
                      <Button disabled={running || !hasRunnable} onClick={runQueue} size="sm" type="button">
                        {running ? (
                          <RotateCw data-icon="inline-start" className="animate-spin" />
                        ) : (
                          <Sparkles data-icon="inline-start" />
                        )}
                        Transcribe
                      </Button>
                    </ButtonGroup>
                  </CardAction>
                </CardHeader>
                <Separator />
                <CardContent className="p-4 sm:p-5">
                  <StackedUpload
                    items={queue}
                    onRemove={removeItem}
                    onReveal={(path) => void revealPath(path)}
                    onSelect={selectQueueItem}
                    selectedId={selectedId}
                  />
                </CardContent>
              </Card>
            ) : null}

            {showHistory ? (
              <HistoryList
                entries={history}
                onCopy={(entry) => void copyTranscript(historyEntryToUploadItem(entry))}
                onOpen={(entry) => void openTranscript(entry.outputPath)}
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
                onPolished={(outputPath, text) => {
                  setPolishedText((current) => ({ ...current, [outputPath]: text }));
                  setStatus("Polished draft ready");
                }}
                onSave={savePolishedTranscript}
                originalText={selectedItem?.output ? transcriptText[selectedItem.output] : undefined}
                polishedText={selectedItem?.output ? polishedText[selectedItem.output] : undefined}
              />
            ) : null}

            {showTranscript ? (
              <div className="min-w-0">
                <TranscriptPanel
                  item={selectedItem}
                  onCopy={copyTranscript}
                  onOpen={(path) => void openTranscript(path)}
                  onReveal={(path) => void revealPath(path)}
                  running={running}
                  text={selectedItem?.output ? transcriptText[selectedItem.output] : undefined}
                />
              </div>
            ) : null}
          </section>
        </section>
      </div>
      <DetailsSheet
        auth={auth}
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
    </main>
  );
}

async function runWindowAction(action: "minimize" | "toggleMaximize" | "close") {
  if (!isTauri()) return;

  const window = getCurrentWindow();
  try {
    if (action === "minimize") await window.minimize();
    if (action === "toggleMaximize") await window.toggleMaximize();
    if (action === "close") await window.close();
  } catch {
    // ponytail: preview/dev without window permissions should not break the UI.
  }
}

function CommandCenter({
  history,
  onAction,
  onOpenChange,
  onPickFiles,
  onPreview,
  open,
}: {
  history: TranscriptHistoryEntry[];
  onAction: (action: RailAction) => void;
  onOpenChange: (open: boolean) => void;
  onPickFiles: () => void;
  onPreview: (entry: TranscriptHistoryEntry) => void;
  open: boolean;
}) {
  function run(action: () => void) {
    onOpenChange(false);
    action();
  }

  return (
    <CommandDialog
      description="Search transcripts and jump around Yap."
      onOpenChange={onOpenChange}
      open={open}
      title="Yap command menu"
    >
      <CommandInput placeholder="Search commands or transcripts..." />
      <CommandList>
        <CommandEmpty>No results found.</CommandEmpty>
        <CommandGroup heading="Actions">
          <CommandItem onSelect={() => run(onPickFiles)}>
            <UploadCloud />
            Add recordings
            <CommandShortcut>Files</CommandShortcut>
          </CommandItem>
          <CommandItem onSelect={() => run(() => onAction("home"))}>
            <Grid2X2 />
            Go home
          </CommandItem>
          <CommandItem onSelect={() => run(() => onAction("recordings"))}>
            <FileAudio2 />
            Recording queue
          </CommandItem>
          <CommandItem onSelect={() => run(() => onAction("transcripts"))}>
            <FileText />
            Transcripts
          </CommandItem>
          <CommandItem onSelect={() => run(() => onAction("polish"))}>
            <Sparkles />
            Polish
          </CommandItem>
        </CommandGroup>
        <CommandSeparator />
        <CommandGroup heading="System">
          <CommandItem onSelect={() => run(() => onAction("details"))}>
            <Settings2 />
            Setup details
          </CommandItem>
          <CommandItem onSelect={() => run(() => onAction("help"))}>
            <HelpCircle />
            Help
          </CommandItem>
        </CommandGroup>
        {history.length ? (
          <>
            <CommandSeparator />
            <CommandGroup heading="Recent transcripts">
              {history.slice(0, 12).map((entry) => (
                <CommandItem
                  key={entry.outputPath}
                  onSelect={() => run(() => onPreview(entry))}
                  value={`${entry.name} ${entry.sourcePath} ${entry.outputPath}`}
                >
                  <FileText />
                  <span className="truncate">{entry.name}</span>
                  <CommandShortcut>{formatHistoryDate(entry.createdAt)}</CommandShortcut>
                </CommandItem>
              ))}
            </CommandGroup>
          </>
        ) : null}
      </CommandList>
    </CommandDialog>
  );
}

function TranscriptPreviewDialog({
  entry,
  onCopy,
  onOpen,
  onOpenChange,
  onReveal,
  text,
}: {
  entry?: TranscriptHistoryEntry;
  onCopy: (entry: TranscriptHistoryEntry) => void;
  onOpen: (entry: TranscriptHistoryEntry) => void;
  onOpenChange: (open: boolean) => void;
  onReveal: (entry: TranscriptHistoryEntry) => void;
  text: string;
}) {
  return (
    <Dialog onOpenChange={onOpenChange} open={Boolean(entry)}>
      <DialogContent className="max-h-[86vh] overflow-hidden sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{entry?.name ?? "Transcript preview"}</DialogTitle>
          <DialogDescription className="truncate">{entry?.outputPath ?? "Local transcript"}</DialogDescription>
        </DialogHeader>
        <ScrollArea className="max-h-[58vh] rounded-md border bg-muted">
          <pre className="whitespace-pre-wrap break-words p-4 text-sm leading-6">
            {text || "Loading transcript..."}
          </pre>
        </ScrollArea>
        {entry ? (
          <DialogFooter>
            <Button onClick={() => onCopy(entry)} type="button" variant="outline">
              <Copy data-icon="inline-start" />
              Copy
            </Button>
            <Button onClick={() => onOpen(entry)} type="button" variant="outline">
              <FileText data-icon="inline-start" />
              Open
            </Button>
            <Button onClick={() => onReveal(entry)} type="button">
              <FolderOpen data-icon="inline-start" />
              Reveal
            </Button>
          </DialogFooter>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

function AppChrome() {
  return (
    <div
      className="flex h-10 select-none items-center border-b border-border/70 bg-background/95 text-foreground"
      data-tauri-drag-region
    >
      <div className="flex min-w-0 items-center gap-2 px-3" data-tauri-drag-region>
        <AppIcon className="size-5 rounded-md" />
        <span className="truncate text-sm font-semibold" data-tauri-drag-region>
          Yap
        </span>
      </div>
      <div className="min-w-4 flex-1" data-tauri-drag-region />
      <div className="flex h-full">
        <Button
          aria-label="Minimize"
          className="h-full w-11 rounded-none text-muted-foreground hover:bg-secondary hover:text-foreground"
          onClick={() => void runWindowAction("minimize")}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Minus />
        </Button>
        <Button
          aria-label="Maximize"
          className="h-full w-11 rounded-none text-muted-foreground hover:bg-secondary hover:text-foreground"
          onClick={() => void runWindowAction("toggleMaximize")}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Square />
        </Button>
        <Button
          aria-label="Close"
          className="h-full w-11 rounded-none text-muted-foreground hover:bg-destructive hover:text-white"
          onClick={() => void runWindowAction("close")}
          size="icon"
          type="button"
          variant="ghost"
        >
          <X />
        </Button>
      </div>
    </div>
  );
}

function WorkspaceBreadcrumb({
  current,
  onHome,
  resource,
}: {
  current: string;
  onHome: () => void;
  resource?: string;
}) {
  return (
    <Breadcrumb>
      <BreadcrumbList>
        <BreadcrumbItem>
          <BreadcrumbLink asChild>
            <button className="font-medium" onClick={onHome} type="button">
              Yap
            </button>
          </BreadcrumbLink>
        </BreadcrumbItem>
        <BreadcrumbSeparator />
        <BreadcrumbItem>
          {resource ? (
            <span className="font-normal text-muted-foreground">{current}</span>
          ) : (
            <BreadcrumbPage>{current}</BreadcrumbPage>
          )}
        </BreadcrumbItem>
        {resource ? (
          <>
            <BreadcrumbSeparator />
            <BreadcrumbItem className="min-w-0">
              <BreadcrumbPage className="max-w-[min(56vw,420px)] truncate">{resource}</BreadcrumbPage>
            </BreadcrumbItem>
          </>
        ) : null}
      </BreadcrumbList>
    </Breadcrumb>
  );
}

function AppIcon({ className }: { className?: string }) {
  return (
    <img
      alt=""
      className={cn("shrink-0", className)}
      draggable={false}
      src="/favicon.png"
    />
  );
}

function ProductRail({
  active,
  auth,
  model,
  onAction,
  status,
}: {
  active: RailAction;
  auth: string;
  model: string;
  onAction: (action: RailAction) => void;
  status: string;
}) {
  return (
    <aside className="hidden min-h-[calc(100vh-64px)] w-[238px] shrink-0 flex-col rounded-[28px] bg-background p-3 lg:flex">
      <nav className="mt-2 flex flex-col gap-1">
        <RailItem active={active === "home"} icon={Grid2X2} label="Home" onClick={() => onAction("home")} />
        <RailItem
          active={active === "recordings"}
          icon={FileAudio2}
          label="Recordings"
          onClick={() => onAction("recordings")}
        />
        <RailItem
          active={active === "transcripts"}
          icon={FileText}
          label="Transcripts"
          onClick={() => onAction("transcripts")}
        />
        <RailItem active={active === "polish"} icon={Sparkles} label="Polish" onClick={() => onAction("polish")} />
      </nav>

      <div className="mt-auto flex flex-col gap-1">
        <RailItem
          active={active === "details"}
          icon={LockKeyhole}
          label={auth === "Authorized" ? "Local mode" : status}
          onClick={() => onAction("details")}
        />
        <RailItem active={active === "details"} icon={Settings2} label={model} onClick={() => onAction("details")} />
        <RailItem active={active === "help"} icon={HelpCircle} label="Help" onClick={() => onAction("help")} />
      </div>
    </aside>
  );
}

function RailItem({
  active,
  icon: Icon,
  label,
  onClick,
}: {
  active?: boolean;
  icon: ElementType;
  label: string;
  onClick: () => void;
}) {
  return (
    <Button
      aria-current={active ? "page" : undefined}
      className={cn(
        "h-auto w-full justify-start px-3 py-3 text-left font-semibold text-muted-foreground hover:bg-secondary/70 hover:text-foreground",
        active && "bg-secondary text-foreground",
      )}
      onClick={onClick}
      type="button"
      variant="ghost"
    >
      <Icon />
      <span className="truncate">{label}</span>
    </Button>
  );
}

function Metric({ icon: Icon, label }: { icon: ElementType; label: string }) {
  return (
    <Badge className="max-w-full gap-2 rounded-full px-2 py-2 text-sm font-semibold tabular-nums sm:px-3" variant="secondary">
      <Icon data-icon="inline-start" />
      <span className="whitespace-nowrap max-[359px]:sr-only">{label}</span>
    </Badge>
  );
}

function DropHero({
  dragging,
  onDragLeave,
  onDragOver,
  onDrop,
  onPickFiles,
}: {
  dragging: boolean;
  onDragLeave: () => void;
  onDragOver: (event: DragEvent<HTMLElement>) => void;
  onDrop: (event: DragEvent<HTMLElement>) => void;
  onPickFiles: () => void;
}) {
  return (
    <section
      className={cn(
        "mt-7 w-full max-w-full overflow-hidden rounded-[28px] border bg-[#17120e] text-white shadow-[inset_0_1px_0_rgba(255,255,255,0.18)] transition-[border-color,box-shadow] duration-200",
        dragging && "border-primary shadow-lg shadow-primary/15",
      )}
      onDragLeave={onDragLeave}
      onDragOver={onDragOver}
      onDrop={onDrop}
    >
      <div className="relative min-h-[260px] w-full max-w-full bg-[linear-gradient(110deg,#17120e_0%,#6f3c24_42%,#034f46_100%)] p-6 sm:p-10">
        <div className="absolute inset-0 bg-[linear-gradient(90deg,rgba(0,0,0,0.2),transparent_65%)]" />
        <div className="relative flex w-full min-w-0 max-w-3xl flex-col gap-5">
          <Badge className="w-fit border-white/20 bg-white/12 text-white hover:bg-white/12" variant="outline">
            <LockKeyhole data-icon="inline-start" />
            Private on this device
          </Badge>
          <div>
            <h2 className="max-w-full break-words font-serif text-3xl leading-tight tracking-normal sm:text-5xl">
              Drop recordings. Get polished text.
            </h2>
            <p className="mt-4 max-w-full text-base leading-7 text-white/82 sm:max-w-xl">
              Bring in audio or video files and Yap will save transcripts locally, ready to review.
            </p>
          </div>
          <div className="flex min-w-0 flex-wrap items-center gap-3">
            <Button
              className="max-w-full bg-white text-[#221d18] hover:bg-white/90"
              onClick={onPickFiles}
              type="button"
              variant="secondary"
            >
              <UploadCloud data-icon="inline-start" />
              Drop files here
            </Button>
            <span className="max-w-full break-words text-sm font-medium text-white/72">{acceptedFormats}</span>
          </div>
        </div>
      </div>
    </section>
  );
}

function formatHistoryDate(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Saved";

  return new Intl.DateTimeFormat(undefined, {
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    month: "short",
  }).format(date);
}

function localDayKey(date: Date) {
  return [
    date.getFullYear(),
    String(date.getMonth() + 1).padStart(2, "0"),
    String(date.getDate()).padStart(2, "0"),
  ].join("-");
}

function recentHistoryActivity(entries: TranscriptHistoryEntry[]) {
  const today = new Date();
  const days = Array.from({ length: 7 }, (_, index) => {
    const date = new Date(today.getFullYear(), today.getMonth(), today.getDate() - (6 - index));
    return {
      day: new Intl.DateTimeFormat(undefined, { weekday: "short" }).format(date),
      key: localDayKey(date),
      transcripts: 0,
    };
  });
  const byDay = new Map(days.map((day) => [day.key, day]));

  for (const entry of entries) {
    const date = new Date(entry.createdAt);
    const day = Number.isNaN(date.getTime()) ? undefined : byDay.get(localDayKey(date));
    if (day) day.transcripts += 1;
  }

  return days;
}

function historyEntryTime(entry: TranscriptHistoryEntry) {
  const time = Date.parse(entry.createdAt);
  return Number.isFinite(time) ? time : 0;
}

function HistoryList({
  entries,
  onCopy,
  onOpen,
  onPreview,
  onRemove,
  onReveal,
  onSelect,
  selectedOutputPath,
}: {
  entries: TranscriptHistoryEntry[];
  onCopy: (entry: TranscriptHistoryEntry) => void;
  onOpen: (entry: TranscriptHistoryEntry) => void;
  onPreview: (entry: TranscriptHistoryEntry) => void;
  onRemove: (outputPath: string) => void;
  onReveal: (entry: TranscriptHistoryEntry) => void;
  onSelect: (entry: TranscriptHistoryEntry) => void;
  selectedOutputPath?: string;
}) {
  const [dateFilter, setDateFilter] = useState("");
  const [showFullPaths, setShowFullPaths] = useState(false);
  const [sortNewestFirst, setSortNewestFirst] = useState(true);

  const visibleEntries = useMemo(
    () =>
      entries
        .filter((entry) => !dateFilter || localDayKey(new Date(entry.createdAt)) === dateFilter)
        .sort((a, b) => (sortNewestFirst ? historyEntryTime(b) - historyEntryTime(a) : historyEntryTime(a) - historyEntryTime(b))),
    [dateFilter, entries, sortNewestFirst],
  );

  return (
    <Card className="min-w-0 border-[#eee8de] bg-card py-0 shadow-none">
      <CardHeader className="p-4 sm:p-5">
        <div className="min-w-0">
          <Badge className="w-fit" variant="outline">Local library</Badge>
          <CardTitle className="mt-2 flex items-center gap-2 text-xl">
            History
            <Badge className="tabular-nums" variant="secondary">
              {entries.length}
            </Badge>
          </CardTitle>
          <CardDescription>Saved transcripts stay on this computer.</CardDescription>
        </div>
        {entries.length && dateFilter ? (
          <CardAction className="col-span-full col-start-1 row-span-1 row-start-2 w-full justify-self-stretch sm:col-span-1 sm:col-start-2 sm:row-span-2 sm:row-start-1 sm:w-auto sm:justify-self-end">
            <Button onClick={() => setDateFilter("")} size="sm" type="button" variant="outline">
              Clear date
            </Button>
          </CardAction>
        ) : null}
      </CardHeader>
      <Separator />
      <CardContent className="grid gap-4 p-4 sm:p-5">
        {entries.length ? (
          <>
            <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto_auto_auto] lg:items-start">
              <HistoryActivityChart entries={entries} />
              <label className="grid gap-1 text-xs font-medium text-muted-foreground">
                Saved date
                <input
                  className="h-9 rounded-md border bg-background px-3 text-sm text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
                  onChange={(event) => setDateFilter(event.target.value)}
                  type="date"
                  value={dateFilter}
                />
              </label>
              <Button onClick={() => setSortNewestFirst((value) => !value)} size="sm" type="button" variant="outline">
                {sortNewestFirst ? "Newest first" : "Oldest first"}
              </Button>
              <label className="flex min-h-9 items-center gap-2 rounded-md border bg-background px-3 text-sm font-medium">
                <Checkbox checked={showFullPaths} onCheckedChange={(checked) => setShowFullPaths(checked === true)} />
                <span>Full paths</span>
              </label>
            </div>
            <div className="overflow-hidden rounded-md border bg-background">
              <ScrollArea className="h-[420px]">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Transcript</TableHead>
                      <TableHead>Saved</TableHead>
                      <TableHead>Source</TableHead>
                      <TableHead className="text-right">Actions</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {visibleEntries.length ? visibleEntries.map((entry) => {
                  const selected = entry.outputPath === selectedOutputPath;

                  function selectFromKeyboard(event: KeyboardEvent<HTMLTableRowElement>) {
                    if (event.key === "Enter" || event.key === " ") {
                      event.preventDefault();
                      onSelect(entry);
                    }
                  }

                  return (
                    <ContextMenu key={entry.outputPath}>
                      <ContextMenuTrigger asChild>
                        <TableRow
                          className="cursor-pointer outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
                          data-state={selected ? "selected" : undefined}
                          onClick={() => onSelect(entry)}
                          onKeyDown={selectFromKeyboard}
                          role="button"
                          tabIndex={0}
                        >
                          <TableCell>
                            <div className="max-w-[260px] truncate font-medium">{entry.name}</div>
                            <div className="max-w-[260px] truncate text-xs text-muted-foreground">
                              {showFullPaths ? entry.outputPath : basename(entry.outputPath)}
                            </div>
                          </TableCell>
                          <TableCell>{formatHistoryDate(entry.createdAt)}</TableCell>
                          <TableCell>
                            <div className="max-w-[300px] truncate text-muted-foreground">
                              {showFullPaths ? entry.sourcePath : basename(entry.sourcePath)}
                            </div>
                          </TableCell>
                          <TableCell className="text-right">
                            <div className="flex justify-end">
                              <ButtonGroup aria-label={`Actions for ${basename(entry.sourcePath)}`}>
                                <Button
                                  onClick={(event) => {
                                    event.stopPropagation();
                                    onPreview(entry);
                                  }}
                                  size="xs"
                                  type="button"
                                  variant="outline"
                                >
                                  <FileText data-icon="inline-start" />
                                  Preview
                                </Button>
                                <Button
                                  onClick={(event) => {
                                    event.stopPropagation();
                                    onCopy(entry);
                                  }}
                                  size="xs"
                                  type="button"
                                  variant="outline"
                                >
                                  <Copy data-icon="inline-start" />
                                  Copy
                                </Button>
                                <Button
                                  onClick={(event) => {
                                    event.stopPropagation();
                                    onReveal(entry);
                                  }}
                                  size="xs"
                                  type="button"
                                >
                                  <FolderOpen data-icon="inline-start" />
                                  Reveal
                                </Button>
                              </ButtonGroup>
                            </div>
                          </TableCell>
                        </TableRow>
                      </ContextMenuTrigger>
                      <HistoryContextMenu
                        entry={entry}
                        onCopy={onCopy}
                        onOpen={onOpen}
                        onPreview={onPreview}
                        onRemove={onRemove}
                        onReveal={onReveal}
                      />
                    </ContextMenu>
                  );
                }) : (
                  <TableRow>
                    <TableCell className="h-24 text-center text-muted-foreground" colSpan={4}>
                      No transcripts on that date.
                    </TableCell>
                  </TableRow>
                )}
                  </TableBody>
                </Table>
              </ScrollArea>
            </div>
          </>
        ) : (
          <Empty className="min-h-[260px]">
            <EmptyMedia>
              <FileText />
            </EmptyMedia>
            <div>
              <EmptyTitle>No saved transcripts yet</EmptyTitle>
              <EmptyDescription>Finished transcriptions will appear here.</EmptyDescription>
            </div>
          </Empty>
        )}
      </CardContent>
    </Card>
  );
}

function HistoryActivityChart({ entries }: { entries: TranscriptHistoryEntry[] }) {
  const data = useMemo(() => recentHistoryActivity(entries), [entries]);

  return (
    <div className="min-w-0 rounded-lg border bg-muted/30 p-3">
      <div className="mb-2 flex items-center justify-between gap-3">
        <span className="text-sm font-semibold">Last 7 days</span>
        <Badge variant="secondary">{entries.length} saved</Badge>
      </div>
      <ChartContainer config={historyChartConfig} className="h-[140px] w-full aspect-auto">
        <BarChart accessibilityLayer data={data}>
          <CartesianGrid vertical={false} />
          <XAxis dataKey="day" tickLine={false} tickMargin={8} axisLine={false} />
          <ChartTooltip content={<ChartTooltipContent hideLabel />} />
          <Bar dataKey="transcripts" fill="var(--color-transcripts)" radius={[4, 4, 0, 0]} />
        </BarChart>
      </ChartContainer>
    </div>
  );
}

function HistoryContextMenu({
  entry,
  onCopy,
  onOpen,
  onPreview,
  onRemove,
  onReveal,
}: {
  entry: TranscriptHistoryEntry;
  onCopy: (entry: TranscriptHistoryEntry) => void;
  onOpen: (entry: TranscriptHistoryEntry) => void;
  onPreview: (entry: TranscriptHistoryEntry) => void;
  onRemove: (outputPath: string) => void;
  onReveal: (entry: TranscriptHistoryEntry) => void;
}) {
  return (
    <ContextMenuContent>
      <ContextMenuItem onSelect={() => onPreview(entry)}>
        <FileText />
        Preview
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => onCopy(entry)}>
        <Copy />
        Copy transcript
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => onOpen(entry)}>
        <FileText />
        Open file
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => onReveal(entry)}>
        <FolderOpen />
        Reveal in Explorer
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem onSelect={() => onRemove(entry.outputPath)} variant="destructive">
        <Trash2 />
        Remove from history
      </ContextMenuItem>
    </ContextMenuContent>
  );
}

function PolishPanel({
  item,
  onLoadText,
  onPolished,
  onSave,
  originalText,
  polishedText,
}: {
  item?: UploadItem;
  onLoadText: (path: string) => Promise<string>;
  onPolished: (outputPath: string, text: string) => void;
  onSave: (item: UploadItem, text: string) => Promise<string>;
  originalText?: string;
  polishedText?: string;
}) {
  const ready = item?.status === "done";
  const [tone, setTone] = useState<PolishTone>("light");
  const [running, setRunning] = useState(false);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState("");
  const [stats, setStats] = useState("");
  const [savedPath, setSavedPath] = useState("");
  const hasPolishedText = Boolean(polishedText?.trim());
  const canPolish = ready && Boolean(item?.output) && !running;

  async function runPolish() {
    if (!item?.output || running) return;

    setRunning(true);
    setMessage("");
    setStats("");
    setSavedPath("");

    try {
      const source = originalText ?? (await onLoadText(item.output));
      const result = await polishTranscript({ text: source, tone });
      onPolished(item.output, result.text);
      setStats(
        [
          result.tokensPerSecond ? `${result.tokensPerSecond} tok/s CPU` : "",
          result.totalSeconds ? `${result.totalSeconds}s` : "",
          result.model.replace("gemma4:", ""),
        ]
          .filter(Boolean)
          .join(" · "),
      );
      setMessage("Polished draft ready");
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setRunning(false);
    }
  }

  async function copyPolished() {
    if (!polishedText) return;

    try {
      await navigator.clipboard.writeText(polishedText);
      setMessage("Polished draft copied");
    } catch {
      setMessage("Copy failed");
    }
  }

  async function savePolished() {
    if (!item || !polishedText || saving) return;

    setSaving(true);
    setMessage("");
    try {
      const path = await onSave(item, polishedText);
      setSavedPath(path);
      setMessage("Saved polished draft");
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Card className="min-w-0 border-[#eee8de] bg-card py-0 shadow-none">
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
        </div>
        <CardAction className="col-span-full col-start-1 row-span-1 row-start-2 w-full justify-self-stretch sm:col-span-1 sm:col-start-2 sm:row-span-2 sm:row-start-1 sm:w-auto sm:justify-self-end">
          <ButtonGroup
            aria-label="Polish actions"
            className="w-full sm:w-auto [&>[data-slot=button]]:flex-1 sm:[&>[data-slot=button]]:flex-none"
          >
            <Button disabled={!canPolish} onClick={() => void runPolish()} size="sm" type="button">
              {running ? (
                <RotateCw data-icon="inline-start" className="animate-spin" />
              ) : (
                <Sparkles data-icon="inline-start" />
              )}
              Polish
            </Button>
            <Button disabled={!hasPolishedText} onClick={() => void copyPolished()} size="sm" type="button" variant="outline">
              <Copy data-icon="inline-start" />
              Copy
            </Button>
            <Button disabled={!hasPolishedText || saving} onClick={() => void savePolished()} size="sm" type="button" variant="outline">
              {saving ? <RotateCw data-icon="inline-start" className="animate-spin" /> : <Save data-icon="inline-start" />}
              Save
            </Button>
          </ButtonGroup>
        </CardAction>
      </CardHeader>
      <Separator />
      <CardContent className="grid gap-4 p-4 sm:p-5">
        <ToggleGroup
          className="grid grid-cols-3"
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

        <div className="grid gap-3 md:grid-cols-2">
          <StatusRow
            icon={FileText}
            label="Selected draft"
            value={ready ? (originalText ? "Transcript loaded" : "Loading transcript") : "Transcribe a recording first"}
            wrap
          />
          <StatusRow icon={Cpu} label="Polish model" value={`${defaultPolishModel} · CPU only`} wrap />
          <StatusRow icon={Sparkles} label="Result" value={message || (ready ? "Ready for cleanup" : "No finished transcript selected")} wrap />
          <StatusRow icon={BadgeCheck} label="Speed" value={stats || "Measured at about 19 tok/s CPU"} wrap />
        </div>

        {savedPath ? (
          <Alert>
            <Save />
            <AlertDescription>
              Saved to <span className="font-medium text-foreground">{basename(savedPath)}</span>
            </AlertDescription>
          </Alert>
        ) : null}

        <div className="grid min-w-0 gap-3 lg:grid-cols-2">
          <TextPreview title="Original" value={originalText} empty={ready ? "Loading transcript preview." : "No transcript selected."} />
          <TextPreview title="Polished" value={polishedText} empty="Run Polish to create a cleaned draft." />
        </div>
      </CardContent>
    </Card>
  );
}

function TextPreview({ empty, title, value }: { empty: string; title: string; value?: string }) {
  return (
    <Card className="min-w-0 gap-0 bg-[#fffdf8] py-0 shadow-none">
      <CardHeader className="border-b p-3">
        <CardTitle className="text-xs font-semibold uppercase text-muted-foreground">{title}</CardTitle>
      </CardHeader>
      <CardContent className="p-0">
        <ScrollArea className="h-[220px]">
          <div className="p-4">
            {value?.trim() ? (
              <pre className="whitespace-pre-wrap break-words text-sm leading-6 text-foreground">{value}</pre>
            ) : (
              <p className="text-sm leading-6 text-muted-foreground">{empty}</p>
            )}
          </div>
        </ScrollArea>
      </CardContent>
    </Card>
  );
}

function TranscriptPanel({
  item,
  onCopy,
  onOpen,
  onReveal,
  running,
  text,
}: {
  item?: UploadItem;
  onCopy: (item: UploadItem) => void;
  onOpen: (path: string) => void;
  onReveal: (path: string) => void;
  running: boolean;
  text?: string;
}) {
  const title = item?.status === "done" ? "Transcript ready" : item ? "Transcript workspace" : "No transcript yet";
  const output = item?.output;

  return (
    <Card className="min-h-[420px] min-w-0 border-[#eee8de] bg-card py-0 shadow-none xl:sticky xl:top-5 xl:min-h-[calc(100vh-180px)]">
      <CardHeader className="p-4 sm:p-5">
        <div className="min-w-0">
          <Badge className="w-fit" variant="outline">
            <FileText data-icon="inline-start" />
            Transcript
          </Badge>
          <CardTitle className="mt-3 text-2xl">{title}</CardTitle>
          <CardDescription className="truncate">
            {item ? item.name : "Drop audio to start"}
          </CardDescription>
        </div>
        {output ? (
          <CardAction className="col-span-full col-start-1 row-span-1 row-start-2 w-full justify-self-stretch sm:col-span-1 sm:col-start-2 sm:row-span-2 sm:row-start-1 sm:w-auto sm:justify-self-end">
            <ButtonGroup
              aria-label="Transcript actions"
              className="w-full sm:w-auto [&>[data-slot=button]]:flex-1 sm:[&>[data-slot=button]]:flex-none"
            >
              <Button onClick={() => void onCopy(item)} size="sm" type="button" variant="outline">
                <Copy data-icon="inline-start" />
                Copy transcript
              </Button>
              <Button onClick={() => onOpen(output)} size="sm" type="button" variant="outline">
                <FileText data-icon="inline-start" />
                Open
              </Button>
              <Button onClick={() => onReveal(output)} size="sm" type="button">
                <FolderOpen data-icon="inline-start" />
                Reveal
              </Button>
            </ButtonGroup>
          </CardAction>
        ) : null}
      </CardHeader>
      <Separator />
      <CardContent className="flex min-h-0 flex-1 flex-col gap-4 p-4 sm:p-5">
        <Alert className="bg-muted">
          <FileText />
          <div className="min-w-0">
            <AlertTitle>{item?.name ?? "Nothing selected"}</AlertTitle>
            <AlertDescription className="mt-1 truncate">
              {output ?? item?.path ?? "Files stay local until you drop them here."}
            </AlertDescription>
          </div>
        </Alert>

        <ScrollArea className="min-h-[240px] flex-1 rounded-lg border bg-[#fffdf8]">
          <div className="flex min-h-[240px] flex-col justify-center gap-4 p-5">
            {item?.status === "done" ? (
              <>
                <div className="flex items-center gap-2">
                  <Badge>
                    <BadgeCheck data-icon="inline-start" />
                    Saved
                  </Badge>
                  <span className="truncate text-sm text-muted-foreground">{basename(output ?? "")}</span>
                </div>
                {text ? (
                  <pre className="max-h-[420px] overflow-auto whitespace-pre-wrap break-words text-sm leading-6 text-foreground">
                    {text}
                  </pre>
                ) : (
                  <Alert>
                    <FileText />
                    <AlertDescription>
                      Loading transcript preview. You can still open or reveal the saved file.
                    </AlertDescription>
                  </Alert>
                )}
              </>
            ) : item?.status === "error" ? (
              <>
                <Alert variant="destructive">
                  <HelpCircle />
                  <AlertDescription>{item.error}</AlertDescription>
                </Alert>
              </>
            ) : item ? (
              <>
                <Badge variant="secondary">{running ? "Transcribing locally" : "Queued"}</Badge>
                <p className="text-sm leading-6 text-muted-foreground">
                  This panel switches to the finished transcript as soon as the local run completes.
                </p>
              </>
            ) : (
              <Empty className="border-0 bg-transparent">
                <EmptyMedia>
                  <FileText />
                </EmptyMedia>
                <div>
                  <EmptyTitle>Drop audio to create a transcript</EmptyTitle>
                  <EmptyDescription>Yap saves the transcript locally when the source is protected.</EmptyDescription>
                </div>
              </Empty>
            )}
          </div>
        </ScrollArea>
      </CardContent>
    </Card>
  );
}

function DetailsSheet({
  auth,
  model,
  onOpenChange,
  open,
  status,
}: {
  auth: string;
  model: string;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  status: string;
}) {
  return (
    <Drawer onOpenChange={onOpenChange} open={open}>
      <DrawerContent>
        <DrawerHeader>
          <DrawerTitle>Setup Details</DrawerTitle>
          <DrawerDescription>Local runner and output settings.</DrawerDescription>
        </DrawerHeader>
        <div className="mt-6 flex flex-col gap-3">
          <StatusRow icon={BadgeCheck} label="Status" value={status} />
          <StatusRow icon={Sparkles} label="Model" value={model} />
          <StatusRow icon={Cpu} label="Runner" value="RTX local runner" />
          <StatusRow icon={LockKeyhole} label="Auth" value={auth} />
          <StatusRow icon={FolderOutput} label="Output" value="Source folder, local fallback" />
        </div>
      </DrawerContent>
    </Drawer>
  );
}

function HelpSheet({ onOpenChange, open }: { onOpenChange: (open: boolean) => void; open: boolean }) {
  return (
    <Drawer onOpenChange={onOpenChange} open={open}>
      <DrawerContent>
        <DrawerHeader>
          <DrawerTitle>Help</DrawerTitle>
          <DrawerDescription>Quick map of the working controls.</DrawerDescription>
        </DrawerHeader>
        <div className="mt-6 flex flex-col gap-3">
          <StatusRow icon={UploadCloud} label="Add files" value="Drag files in, or click Drop files here." wrap />
          <StatusRow icon={Sparkles} label="Transcribe" value="Saves beside the source when allowed, otherwise to local Yap transcripts." wrap />
          <StatusRow icon={Copy} label="Copy" value="Copies transcript text after a file finishes." wrap />
          <StatusRow icon={FolderOpen} label="Reveal" value="Shows the saved transcript in File Explorer." wrap />
        </div>
      </DrawerContent>
    </Drawer>
  );
}

function StatusRow({
  icon: Icon,
  label,
  value,
  wrap,
}: {
  icon: ElementType;
  label: string;
  value: string;
  wrap?: boolean;
}) {
  return (
    <Card className="gap-0 py-0 shadow-none">
      <CardContent className="flex items-center gap-3 p-3">
        <div className="grid size-9 shrink-0 place-items-center rounded-md bg-muted text-muted-foreground">
          <Icon className="size-4" />
        </div>
        <div className="min-w-0">
          <div className="text-xs font-medium text-muted-foreground">{label}</div>
          <div className={cn("text-sm font-semibold", wrap ? "leading-5" : "truncate")}>{value}</div>
        </div>
      </CardContent>
    </Card>
  );
}

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
  MoreHorizontal,
  PanelLeftClose,
  PanelLeftOpen,
  Save,
  Search,
  Settings2,
  Sparkles,
  Square,
  Trash2,
  UploadCloud,
  CircleUserRound,
  X,
} from "lucide-react";
import { type DragEvent, type ElementType, type KeyboardEvent, useEffect, useMemo, useState } from "react";

import { StackedUpload, type UploadItem } from "@/components/stacked-upload";
import { Alert, AlertDescription } from "@/components/ui/alert";
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
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Empty, EmptyDescription, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import {
  Item,
  ItemContent,
  ItemDescription,
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item";
import { Kbd, KbdGroup } from "@/components/ui/kbd";
import {
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Progress } from "@/components/ui/progress";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import {
  Drawer,
  DrawerClose,
  DrawerContent,
  DrawerDescription,
  DrawerFooter,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { TooltipProvider } from "@/components/ui/tooltip";
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

const workspaceCopy: Record<WorkspaceView, { title: string; description: string }> = {
  home: {
    title: "Drop recordings",
    description: "Add audio or video files, transcribe locally, and review the text here.",
  },
  recordings: {
    title: "Recording queue",
    description: "Files waiting to transcribe or rerun.",
  },
  transcripts: {
    title: "Transcripts",
    description: "Past transcriptions, grouped by day.",
  },
  polish: {
    title: "Polish",
    description: "Clean up the selected transcript before you copy or export it.",
  },
};

const audioExtensions = ["mp3", "m4a", "wav", "mp4", "flac", "ogg", "webm"];
const audioExts = new Set(audioExtensions.map((format) => `.${format}`));
const acceptedFormats = "MP3, M4A, WAV, MP4, FLAC, OGG, WEBM";

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

  const hasRunnable = useMemo(
    () => queue.some((item) => item.status === "queued" || item.status === "error"),
    [queue],
  );
  const completed = queue.filter((item) => item.status === "done").length;
  const queueProgress = queue.length ? Math.round((completed / queue.length) * 100) : 0;
  const selectedHistoryEntry = history.find((entry) => entry.outputPath === selectedHistoryOutput);
  const selectedHistoryItem = selectedHistoryEntry ? historyEntryToUploadItem(selectedHistoryEntry) : undefined;
  const selectedItem =
    queue.find((item) => item.id === selectedId) ??
    selectedHistoryItem ??
    [...queue].reverse().find((item) => item.status === "done") ??
    (history[0] ? historyEntryToUploadItem(history[0]) : undefined) ??
    queue[0];
  const workspace = workspaceCopy[workspaceView];
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

  const workspaceLeftPane = (
    <>
      {showQueue ? (
        <Card className="h-full min-w-0 bg-card py-0">
          <CardHeader className="p-4 sm:p-5">
            <div className="min-w-0">
              <CardTitle className="flex items-center gap-2 text-xl">
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
                    <Spinner data-icon="inline-start" />
                  ) : (
                    <Sparkles data-icon="inline-start" />
                  )}
                  Transcribe
                </Button>
              </ButtonGroup>
            </CardAction>
          </CardHeader>
          <CardContent className="p-4 sm:p-5">
            {queue.length ? (
              <Field className="mb-4 gap-2">
                <div className="flex items-center justify-between gap-3">
                  <FieldLabel>Queue progress</FieldLabel>
                  <FieldDescription>
                    {completed} of {queue.length}
                  </FieldDescription>
                </div>
                <Progress value={queueProgress} />
              </Field>
            ) : null}
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
    </>
  );
  const workspaceTranscriptPane = showTranscript ? (
    <div className="h-full min-w-0">
      <TranscriptPanel
        item={selectedItem}
        onCopy={copyTranscript}
        onOpen={(path) => void openTranscript(path)}
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
        workspaceView === "home" || workspaceView === "polish" || workspaceView === "transcripts"
          ? "grid-cols-[minmax(0,1fr)_minmax(320px,0.78fr)]"
          : "grid-cols-1",
      )}
    >
      {workspaceLeftPane}
      {workspaceTranscriptPane}
    </div>
  );
  const appWorkspace = (
    <section className="scrollbar-none h-full min-h-0 w-full min-w-0 flex-1 overflow-x-hidden overflow-y-auto rounded-[28px] border bg-card p-[15px] shadow-none">
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0">
          <h1 className="text-2xl font-semibold tracking-tight">{workspace.title}</h1>
          <p className="mt-1.5 max-w-xl text-sm leading-6 text-muted-foreground">{workspace.description}</p>
        </div>

        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <PrivacyStatus auth={auth} status={status} />
          {history.length ? (
            <Badge className="rounded-full px-3 py-1.5 text-sm font-semibold tabular-nums" variant="secondary">
              {history.length} saved
            </Badge>
          ) : null}
          <Button
            aria-label="Open command menu"
            className="px-2"
            onClick={() => setCommandOpen(true)}
            size="sm"
            type="button"
            variant="outline"
          >
            <Search data-icon="inline-start" />
            <span className="hidden xl:inline">Search</span>
            <KbdGroup className="hidden xl:inline-flex">
              <Kbd>Ctrl</Kbd>
              <Kbd>K</Kbd>
            </KbdGroup>
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

      <section className="mt-7 w-full min-w-0">
        {workspaceMain}
      </section>
    </section>
  );

  return (
    <TooltipProvider>
      <main className="flex h-screen flex-col overflow-hidden bg-background text-foreground">
      <AppChrome
        onAction={handleRailAction}
        collapsed={railCollapsed}
        onToggleRail={() => setRailCollapsed((collapsed) => !collapsed)}
      />
      <div className="min-h-0 w-full min-w-0 flex-1 overflow-hidden bg-background pb-[15px] pr-[15px] pt-0">
        <ResizablePanelGroup className="h-full min-h-0 bg-background" key={railCollapsed ? "rail-collapsed" : "rail-expanded"} orientation="horizontal">
          <ResizablePanel
            className="bg-background"
            defaultSize={railCollapsed ? "5.5%" : "15%"}
            maxSize={railCollapsed ? "5.5%" : "22%"}
            minSize={railCollapsed ? "5.5%" : "14%"}
          >
            <ProductRail
              active={activeRail}
              collapsed={railCollapsed}
              onAction={handleRailAction}
            />
          </ResizablePanel>
          <ResizableHandle className={cn("z-10 -mx-1 bg-transparent", railCollapsed && "opacity-0")} withHandle={!railCollapsed} />
          <ResizablePanel className="bg-background" defaultSize={railCollapsed ? "94.5%" : "83%"} minSize="60%">
            {appWorkspace}
          </ResizablePanel>
        </ResizablePanelGroup>
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
    </TooltipProvider>
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

async function openDevtools() {
  if (!isTauri()) return;

  try {
    await invoke("open_devtools");
  } catch {
    // ponytail: browser preview should not care about native devtools.
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

function AppChrome({
  onAction,
  collapsed,
  onToggleRail,
}: {
  onAction: (action: RailAction) => void;
  collapsed: boolean;
  onToggleRail: () => void;
}) {
  return (
    <div
      className="flex h-10 select-none items-center bg-background text-foreground"
      data-tauri-drag-region
    >
      <div className="flex items-center gap-2 px-4">
        <Button
          aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          className="bg-secondary"
          onClick={onToggleRail}
          size="icon-xs"
          title={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          type="button"
          variant="ghost"
        >
          {collapsed ? <PanelLeftOpen /> : <PanelLeftClose />}
        </Button>
        <Button aria-label="Account" onClick={() => onAction("details")} size="icon-xs" type="button" variant="ghost">
          <CircleUserRound />
        </Button>
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

function AppIcon({ className }: { className?: string }) {
  return (
    <img
      alt=""
      className={cn("shrink-0", className)}
      draggable={false}
      src="/app-icon.png"
    />
  );
}

function ProductRail({
  active,
  collapsed,
  onAction,
}: {
  active: RailAction;
  collapsed: boolean;
  onAction: (action: RailAction) => void;
}) {
  return (
    <aside className={cn("flex h-full min-h-0 min-w-0 flex-col bg-background px-[15px] pb-3 pt-4", collapsed && "items-center")}>
      <div className={cn("mb-5 flex items-center gap-2 px-1", collapsed && "justify-center px-0")}>
        <AppIcon className="size-6 rounded-md" />
        {collapsed ? null : (
          <>
            <div className="text-xl font-semibold tracking-tight">Yap</div>
            <Badge className="h-6 bg-accent-soft px-2 text-xs text-accent-ink hover:bg-accent-soft" variant="secondary">Local</Badge>
          </>
        )}
      </div>

      <nav className="flex w-full flex-col gap-1">
        <RailItem active={active === "home"} collapsed={collapsed} icon={Grid2X2} label="Home" onClick={() => onAction("home")} />
        <RailItem
          active={active === "recordings"}
          collapsed={collapsed}
          icon={FileAudio2}
          label="Recordings"
          onClick={() => onAction("recordings")}
        />
        <RailItem
          active={active === "transcripts"}
          collapsed={collapsed}
          icon={FileText}
          label="Transcripts"
          onClick={() => onAction("transcripts")}
        />
        <RailItem active={active === "polish"} collapsed={collapsed} icon={Sparkles} label="Polish" onClick={() => onAction("polish")} />
      </nav>

      <div className="mt-auto flex w-full flex-col gap-1 border-t pt-4">
        <RailItem active={active === "details"} collapsed={collapsed} icon={Settings2} label="Settings" onClick={() => onAction("details")} />
        <RailItem active={active === "help"} collapsed={collapsed} icon={HelpCircle} label="Help" onClick={() => onAction("help")} />
      </div>
    </aside>
  );
}

function RailItem({
  active,
  collapsed,
  icon: Icon,
  label,
  onClick,
}: {
  active?: boolean;
  collapsed?: boolean;
  icon: ElementType;
  label: string;
  onClick: () => void;
}) {
  return (
    <Button
      aria-current={active ? "page" : undefined}
      className={cn(
        "h-auto w-full justify-start rounded-lg px-2.5 py-2 text-left text-sm font-semibold text-foreground hover:bg-[var(--rail-hover)]",
        active && "bg-secondary text-foreground",
        collapsed && "justify-center px-2",
      )}
      onClick={onClick}
      title={label}
      type="button"
      variant="ghost"
    >
      <Icon />
      <span className={cn("truncate", collapsed && "sr-only")}>{label}</span>
    </Button>
  );
}

function PrivacyStatus({ auth, status }: { auth: string; status: string }) {
  const label = auth === "Authorized" ? "Ready" : status;
  const checking = auth === "Checking" || status === "Starting";

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button className="rounded-full px-3 font-semibold" size="sm" type="button" variant="secondary">
          <LockKeyhole data-icon="inline-start" />
          {checking ? <Skeleton className="h-4 w-14 rounded-full" /> : label}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end">
        <PopoverHeader>
          <PopoverTitle>{checking ? "Checking setup" : label}</PopoverTitle>
          <PopoverDescription>
            {auth === "Authorized" ? "Files stay on this device. Transcripts save locally." : status}
          </PopoverDescription>
        </PopoverHeader>
      </PopoverContent>
    </Popover>
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
        "mt-5 w-full rounded-xl border-2 border-dashed bg-[var(--surface-transcript)] transition-[border-color,background-color,box-shadow] duration-200",
        dragging ? "border-primary bg-[var(--primary-soft)] shadow-sm" : "border-border",
      )}
      onDragLeave={onDragLeave}
      onDragOver={onDragOver}
      onDrop={onDrop}
    >
      <div className="flex min-h-[168px] flex-col items-center justify-center gap-4 px-6 py-8 text-center">
        <div className="flex size-12 items-center justify-center rounded-full bg-secondary">
          <UploadCloud className="size-6 text-primary" />
        </div>
        <div className="max-w-md">
          <h2 className="text-lg font-semibold tracking-tight">Drop recordings here</h2>
          <p className="mt-1.5 text-sm leading-6 text-muted-foreground">
            Or choose files to transcribe locally. {acceptedFormats}.
          </p>
        </div>
        <div className="flex flex-wrap items-center justify-center gap-3">
          <Button onClick={onPickFiles} type="button">
            <UploadCloud data-icon="inline-start" />
            Choose files
          </Button>
          <Badge className="border-primary/20 bg-[var(--primary-soft)] text-primary hover:bg-[var(--primary-soft)]" variant="outline">
            <LockKeyhole data-icon="inline-start" />
            Private on this device
          </Badge>
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

function historyEntryTime(entry: TranscriptHistoryEntry) {
  const time = Date.parse(entry.createdAt);
  return Number.isFinite(time) ? time : 0;
}

function historyDayLabel(date: Date) {
  const today = new Date();
  const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
  const key = localDayKey(date);
  if (key === localDayKey(today)) return "Today";
  if (key === localDayKey(yesterday)) return "Yesterday";
  return new Intl.DateTimeFormat(undefined, {
    weekday: "long",
    month: "short",
    day: "numeric",
  }).format(date);
}

function formatHistoryTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Saved";

  return new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
  }).format(date);
}

function groupHistoryByDay(entries: TranscriptHistoryEntry[]) {
  const sorted = [...entries].sort((a, b) => historyEntryTime(b) - historyEntryTime(a));
  const groups: { key: string; label: string; entries: TranscriptHistoryEntry[] }[] = [];
  const indexByKey = new Map<string, number>();

  for (const entry of sorted) {
    const date = new Date(entry.createdAt);
    const key = Number.isNaN(date.getTime()) ? "unknown" : localDayKey(date);
    const label = Number.isNaN(date.getTime()) ? "Earlier" : historyDayLabel(date);
    const index = indexByKey.get(key);

    if (index === undefined) {
      indexByKey.set(key, groups.length);
      groups.push({ key, label, entries: [entry] });
    } else {
      groups[index].entries.push(entry);
    }
  }

  return groups;
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
  const [searchFilter, setSearchFilter] = useState("");

  const visibleGroups = useMemo(() => {
    const query = searchFilter.trim().toLowerCase();
    const filtered = query
      ? entries.filter((entry) => `${entry.name} ${entry.sourcePath}`.toLowerCase().includes(query))
      : entries;

    return groupHistoryByDay(filtered);
  }, [entries, searchFilter]);

  return (
    <Card className="min-w-0 bg-card py-0">
      <CardHeader className="p-4 sm:p-5">
        <div className="min-w-0">
          <CardTitle className="flex items-center gap-2 text-xl">
            History
            <Badge className="tabular-nums" variant="secondary">
              {entries.length}
            </Badge>
          </CardTitle>
          <CardDescription>Saved transcripts stay on this computer.</CardDescription>
        </div>
      </CardHeader>
      <CardContent className="grid gap-4 p-4 sm:p-5">
        {entries.length ? (
          <>
            <InputGroup>
              <InputGroupInput
                aria-label="Search transcripts"
                onChange={(event) => setSearchFilter(event.target.value)}
                placeholder="Search transcripts"
                type="search"
                value={searchFilter}
              />
              <InputGroupAddon align="inline-end">
                <Search />
              </InputGroupAddon>
            </InputGroup>

            <ScrollArea className="h-[min(520px,calc(100vh-280px))] pr-3">
              {visibleGroups.length ? (
                <div className="flex flex-col gap-6">
                  {visibleGroups.map((group) => (
                    <section key={group.key}>
                      <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                        {group.label}
                      </h3>
                      <ul className="flex flex-col gap-1">
                        {group.entries.map((entry) => {
                          const selected = entry.outputPath === selectedOutputPath;

                          function selectFromKeyboard(event: KeyboardEvent<HTMLButtonElement>) {
                            if (event.key === "Enter" || event.key === " ") {
                              event.preventDefault();
                              onSelect(entry);
                            }
                          }

                          return (
                            <li key={entry.outputPath}>
                              <button
                                className={cn(
                                  "flex w-full items-center gap-3 rounded-lg border bg-background px-3 py-2.5 text-left outline-none transition-colors",
                                  "hover:bg-secondary/70 focus-visible:ring-2 focus-visible:ring-ring/50",
                                  selected && "border-primary bg-[var(--primary-soft)]/40",
                                )}
                                onClick={() => onSelect(entry)}
                                onKeyDown={selectFromKeyboard}
                                type="button"
                              >
                                <FileText className="size-4 shrink-0 text-muted-foreground" />
                                <span className="min-w-0 flex-1 truncate font-medium">{entry.name}</span>
                                <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
                                  {formatHistoryTime(entry.createdAt)}
                                </span>
                                <HistoryActionMenu
                                  entry={entry}
                                  onCopy={onCopy}
                                  onOpen={onOpen}
                                  onPreview={onPreview}
                                  onRemove={onRemove}
                                  onReveal={onReveal}
                                />
                              </button>
                            </li>
                          );
                        })}
                      </ul>
                    </section>
                  ))}
                </div>
              ) : (
                <p className="py-8 text-center text-sm text-muted-foreground">No transcripts match that search.</p>
              )}
            </ScrollArea>
          </>
        ) : (
          <Empty className="min-h-[260px]">
            <EmptyMedia>
              <FileText />
            </EmptyMedia>
            <div>
              <EmptyTitle>No saved transcripts yet</EmptyTitle>
              <EmptyDescription>Finished transcriptions will appear here, grouped by day.</EmptyDescription>
            </div>
          </Empty>
        )}
      </CardContent>
    </Card>
  );
}

function HistoryActionMenu({
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
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          aria-label={`Actions for ${entry.name}`}
          onClick={(event) => event.stopPropagation()}
          size="icon-xs"
          type="button"
          variant="ghost"
        >
          <MoreHorizontal />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" onClick={(event) => event.stopPropagation()}>
        <DropdownMenuLabel>Transcript</DropdownMenuLabel>
        <DropdownMenuGroup>
          <DropdownMenuItem onSelect={() => onPreview(entry)}>
            <FileText />
            Preview
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => onCopy(entry)}>
            <Copy />
            Copy transcript
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => onOpen(entry)}>
            <FileText />
            Open file
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => onReveal(entry)}>
            <FolderOpen />
            Reveal in Explorer
          </DropdownMenuItem>
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => onRemove(entry.outputPath)} variant="destructive">
          <Trash2 />
          Remove from history
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
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
    <Card className="min-w-0 bg-card py-0">
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
                <Spinner data-icon="inline-start" />
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
              {saving ? <Spinner data-icon="inline-start" /> : <Save data-icon="inline-start" />}
              Save
            </Button>
          </ButtonGroup>
        </CardAction>
      </CardHeader>
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
    <Card className="min-w-0 gap-0 border bg-[var(--surface-transcript)] py-0 shadow-none">
      <CardHeader className="border-b p-3">
        <CardTitle className="text-xs font-semibold uppercase text-muted-foreground">{title}</CardTitle>
      </CardHeader>
      <CardContent className="p-0">
        <ScrollArea className="h-[220px]">
          <div className="p-4">
            {value?.trim() ? (
              <pre className="whitespace-pre-wrap break-words text-[15px] leading-7 text-foreground">{value}</pre>
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
  const output = item?.output;

  return (
    <Card className="flex min-h-[420px] min-w-0 flex-col bg-card py-0 xl:sticky xl:top-5 xl:min-h-[calc(100vh-180px)]">
      <CardHeader className="gap-3 border-b p-4 sm:p-5">
        <div className="min-w-0">
          <CardTitle className="truncate text-lg">{item?.name ?? "Transcript"}</CardTitle>
          <CardDescription>
            {item?.status === "done"
              ? "Saved locally"
              : item?.status === "running"
                ? "Transcribing locally…"
                : item?.status === "error"
                  ? "Transcription failed"
                  : item
                    ? "Waiting in queue"
                    : "Select a file or finish a transcription to preview text here."}
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
                Copy
              </Button>
              <Button onClick={() => onOpen(output)} size="sm" type="button" variant="outline">
                <FileText data-icon="inline-start" />
                Open
              </Button>
              <Button onClick={() => onReveal(output)} size="sm" type="button" variant="outline">
                <FolderOpen data-icon="inline-start" />
                Reveal
              </Button>
            </ButtonGroup>
          </CardAction>
        ) : null}
      </CardHeader>
      <CardContent className="flex min-h-0 flex-1 flex-col p-0">
        <ScrollArea className="min-h-[280px] flex-1 bg-[var(--surface-transcript)]">
          <div className="min-h-[280px] p-5">
            {item?.status === "done" ? (
              text ? (
                <pre className="whitespace-pre-wrap break-words text-[15px] leading-7 text-foreground">{text}</pre>
              ) : (
                <div className="flex flex-col gap-3">
                  <Skeleton className="h-4 w-3/4" />
                  <Skeleton className="h-4 w-full" />
                  <Skeleton className="h-4 w-5/6" />
                  <p className="text-sm text-muted-foreground">Loading transcript…</p>
                </div>
              )
            ) : item?.status === "error" ? (
              <Alert variant="destructive">
                <HelpCircle />
                <AlertDescription>{item.error}</AlertDescription>
              </Alert>
            ) : item ? (
              <div className="flex flex-col gap-3">
                <Badge variant="secondary">{running ? "Transcribing" : "Queued"}</Badge>
                <p className="text-[15px] leading-7 text-muted-foreground">
                  The finished transcript will appear here as soon as the local run completes.
                </p>
              </div>
            ) : (
              <Empty className="border-0 bg-transparent">
                <EmptyMedia>
                  <FileText />
                </EmptyMedia>
                <div>
                  <EmptyTitle>No transcript selected</EmptyTitle>
                  <EmptyDescription>Drop a recording on Home or pick one from History.</EmptyDescription>
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
        <div className="flex flex-col gap-3 p-4 pt-0">
          <StatusRow icon={BadgeCheck} label="Status" value={status} />
          <StatusRow icon={Sparkles} label="Model" value={model} />
          <StatusRow icon={Cpu} label="Runner" value="RTX local runner" />
          <StatusRow icon={LockKeyhole} label="Auth" value={auth} />
          <StatusRow icon={FolderOutput} label="Output" value="Source folder, local fallback" />
        </div>
        <DrawerFooter>
          <DrawerClose asChild>
            <Button type="button" variant="outline">Close</Button>
          </DrawerClose>
        </DrawerFooter>
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
        <div className="flex flex-col gap-3 p-4 pt-0">
          <StatusRow icon={UploadCloud} label="Add files" value="Drag files in, or click Drop files here." wrap />
          <StatusRow icon={Sparkles} label="Transcribe" value="Saves beside the source when allowed, otherwise to local Yap transcripts." wrap />
          <StatusRow icon={Copy} label="Copy" value="Copies transcript text after a file finishes." wrap />
          <StatusRow icon={FolderOpen} label="Reveal" value="Shows the saved transcript in File Explorer." wrap />
        </div>
        <DrawerFooter>
          <DrawerClose asChild>
            <Button type="button" variant="outline">Close</Button>
          </DrawerClose>
        </DrawerFooter>
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
    <Item size="sm" variant="outline">
      <ItemMedia variant="icon">
        <Icon />
      </ItemMedia>
      <ItemContent className="min-w-0">
        <ItemTitle className="text-xs text-muted-foreground">{label}</ItemTitle>
        <ItemDescription className={cn("font-semibold text-foreground", wrap ? "line-clamp-none break-words" : "truncate")}>
          {value}
        </ItemDescription>
      </ItemContent>
    </Item>
  );
}

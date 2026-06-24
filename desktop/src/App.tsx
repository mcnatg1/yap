import { invoke, isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  BadgeCheck,
  Copy,
  Cpu,
  FileAudio2,
  FileText,
  FolderOpen,
  FolderOutput,
  LockKeyhole,
  RotateCw,
  Settings2,
  Sparkles,
  Trash2,
  UploadCloud,
} from "lucide-react";
import { type ElementType, useEffect, useMemo, useState } from "react";

import { StackedUpload, type UploadItem } from "@/components/stacked-upload";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet";
import { cn } from "@/lib/utils";

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

const audioExts = new Set([".mp3", ".m4a", ".wav", ".mp4", ".flac", ".ogg", ".webm"]);
const acceptedFormats = "MP3, M4A, WAV, MP4, FLAC, OGG, WEBM";

function basename(path: string) {
  return path.split(/[\\/]/).pop() ?? path;
}

function extension(path: string) {
  const name = basename(path);
  const dot = name.lastIndexOf(".");
  return dot === -1 ? "" : name.slice(dot).toLowerCase();
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

  const hasRunnable = useMemo(
    () => queue.some((item) => item.status === "queued" || item.status === "error"),
    [queue],
  );
  const completed = queue.filter((item) => item.status === "done").length;
  const selectedItem =
    queue.find((item) => item.id === selectedId) ??
    [...queue].reverse().find((item) => item.status === "done") ??
    queue[0];

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
    if (!queue.length) {
      setSelectedId(undefined);
      return;
    }

    if (!selectedId || !queue.some((item) => item.id === selectedId)) {
      setSelectedId(queue[queue.length - 1].id);
    }
  }, [queue, selectedId]);

  async function loadStatus() {
    if (!isTauri()) return;

    try {
      const setup = await invoke<SetupStatus>("setup_status");
      setModel(setup.model.replace("CohereLabs/", ""));
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
      if (newItems.length) setSelectedId(newItems[newItems.length - 1].id);
      return [...current, ...newItems];
    });
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

      setQueue((items) =>
        items.map((item) =>
          outputs.has(item.path) ? { ...item, output: outputs.get(item.path), status: "done" } : item,
        ),
      );
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

  function clearQueue() {
    if (!running) setQueue([]);
  }

  async function copyPath(path: string) {
    try {
      await navigator.clipboard.writeText(path);
      setStatus("Copied");
    } catch {
      setStatus("Copy failed");
    }
  }

  return (
    <main className="min-h-screen overflow-x-hidden bg-background p-4 text-foreground sm:p-5">
      <div className="mx-auto flex max-w-7xl flex-col gap-4">
        <header className="grid grid-cols-1 gap-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
          <div className="flex min-w-0 flex-1 items-center gap-3">
            <div className="grid size-10 shrink-0 place-items-center rounded-lg bg-primary text-primary-foreground shadow-sm">
              <FileAudio2 className="size-5" />
            </div>
            <div className="min-w-0">
              <h1 className="truncate text-xl font-semibold tracking-tight">Yapx3</h1>
              <p className="truncate text-sm text-muted-foreground">Private local transcription</p>
            </div>
          </div>

          <div className="flex min-w-0 shrink-0 items-center gap-2">
            <Badge
              className="min-w-0 max-w-[132px] shrink"
              variant={status === "Ready" || status === "Copied" ? "default" : "outline"}
            >
              <BadgeCheck />
              <span className="truncate">{status}</span>
            </Badge>
            <DetailsSheet auth={auth} model={model} status={status} />
          </div>
        </header>

        <section className="grid gap-4 xl:grid-cols-[minmax(0,1.08fr)_minmax(360px,0.92fr)]">
          <div className="flex min-w-0 flex-col gap-4">
            <Card
              className={cn(
                "relative min-h-[282px] overflow-hidden border-dashed py-0 transition duration-200",
                dragging && "border-primary shadow-lg shadow-primary/10",
              )}
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
            >
              <div
                aria-hidden="true"
                className="absolute inset-0 bg-[radial-gradient(circle_at_top,var(--primary-soft),transparent_38%)]"
              />
              <CardContent className="relative flex min-h-[282px] flex-col items-center justify-center gap-5 p-6 text-center sm:p-10">
                <div className="grid size-16 place-items-center rounded-xl bg-primary/10 text-primary ring-1 ring-primary/20">
                  <UploadCloud className="size-8" />
                </div>
                <div className="flex flex-col gap-2">
                  <h2 className="text-3xl font-semibold tracking-tight sm:text-4xl">Drop recordings</h2>
                  <p className="text-sm text-muted-foreground">{acceptedFormats}</p>
                </div>
                <Badge variant="outline">
                  <LockKeyhole />
                  Files stay on this machine
                </Badge>
              </CardContent>
            </Card>

            <Card className="py-0">
              <CardHeader className="p-4 sm:p-5">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0">
                    <CardTitle className="flex items-center gap-2 text-base">
                      Queue
                      <Badge variant="secondary">{queue.length}</Badge>
                    </CardTitle>
                    <CardDescription>
                      {completed
                        ? `${completed} transcript${completed === 1 ? "" : "s"} ready`
                        : "Ready for audio files"}
                    </CardDescription>
                  </div>
                  <div className="flex w-full flex-wrap justify-start gap-2 sm:w-auto sm:justify-end">
                  <Button
                    disabled={running || !queue.length}
                    onClick={clearQueue}
                    size="sm"
                    type="button"
                    variant="outline"
                  >
                    <Trash2 data-icon="inline-start" />
                    Clear
                  </Button>
                  <Button disabled={running || !hasRunnable} onClick={runQueue} size="sm" type="button">
                    {running ? <RotateCw data-icon="inline-start" className="animate-spin" /> : <Sparkles data-icon="inline-start" />}
                    Transcribe
                  </Button>
                  </div>
                </div>
              </CardHeader>
              <Separator />
              <CardContent className="p-4 sm:p-5">
                <StackedUpload
                  items={queue}
                  onRemove={removeItem}
                  onReveal={(path) => void revealItemInDir(path)}
                  onSelect={setSelectedId}
                  selectedId={selectedId}
                />
              </CardContent>
            </Card>
          </div>

          <TranscriptPanel
            item={selectedItem}
            onCopy={copyPath}
            onReveal={(path) => void revealItemInDir(path)}
            running={running}
          />
        </section>
      </div>
    </main>
  );
}

function TranscriptPanel({
  item,
  onCopy,
  onReveal,
  running,
}: {
  item?: UploadItem;
  onCopy: (path: string) => void;
  onReveal: (path: string) => void;
  running: boolean;
}) {
  const title = item?.status === "done" ? "Transcript ready" : item ? "Transcript workspace" : "No transcript yet";
  const output = item?.output;

  return (
    <Card className="min-h-[420px] py-0 xl:sticky xl:top-5 xl:min-h-[calc(100vh-112px)]">
      <CardHeader className="p-4 sm:p-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <Badge className="w-fit" variant="outline">
              <FileText />
              Transcript
            </Badge>
            <CardTitle className="mt-3 text-2xl">{title}</CardTitle>
            <CardDescription className="truncate">
              {item ? item.name : "Drop audio to start"}
            </CardDescription>
          </div>
          {output ? (
            <div className="flex w-full flex-wrap justify-start gap-2 sm:w-auto sm:justify-end">
              <Button onClick={() => void onCopy(output)} size="sm" type="button" variant="outline">
                <Copy data-icon="inline-start" />
                Copy path
              </Button>
              <Button onClick={() => onReveal(output)} size="sm" type="button">
                <FolderOpen data-icon="inline-start" />
                Reveal
              </Button>
            </div>
          ) : null}
        </div>
      </CardHeader>
      <Separator />
      <CardContent className="flex min-h-0 flex-1 flex-col gap-4 p-4 sm:p-5">
        <div className="rounded-lg border bg-muted p-4">
          <div className="text-xs font-medium text-muted-foreground">Current file</div>
          <div className="mt-1 truncate text-sm font-semibold">{item?.name ?? "Nothing selected"}</div>
          <div className="mt-1 truncate text-xs text-muted-foreground">
            {output ?? item?.path ?? "Files stay local until you drop them here."}
          </div>
        </div>

        <ScrollArea className="min-h-[240px] flex-1 rounded-lg border bg-card">
          <div className="flex min-h-[240px] flex-col justify-center gap-4 p-5">
            {item?.status === "done" ? (
              <>
                <div className="flex items-center gap-2">
                  <Badge>
                    <BadgeCheck />
                    Saved
                  </Badge>
                  <span className="truncate text-sm text-muted-foreground">{basename(output ?? "")}</span>
                </div>
                <p className="max-w-prose text-sm leading-6 text-muted-foreground">
                  The transcript is ready beside the source file. Open it from here to review, edit, or move it into
                  your notes.
                </p>
              </>
            ) : item?.status === "error" ? (
              <>
                <Badge variant="destructive">Needs attention</Badge>
                <p className="text-sm leading-6 text-muted-foreground">{item.error}</p>
              </>
            ) : item ? (
              <>
                <Badge variant="secondary">{running ? "Transcribing locally" : "Queued"}</Badge>
                <p className="text-sm leading-6 text-muted-foreground">
                  This panel switches to the finished transcript as soon as the local run completes.
                </p>
              </>
            ) : (
              <div className="flex flex-col items-center gap-3 text-center">
                <div className="grid size-12 place-items-center rounded-lg bg-muted text-muted-foreground">
                  <FileText className="size-5" />
                </div>
                <div>
                  <div className="text-sm font-semibold">Drop audio to create a transcript</div>
                  <div className="mt-1 text-xs text-muted-foreground">Yapx3 writes the text file next to the source.</div>
                </div>
              </div>
            )}
          </div>
        </ScrollArea>
      </CardContent>
    </Card>
  );
}

function DetailsSheet({ auth, model, status }: { auth: string; model: string; status: string }) {
  return (
    <Sheet>
      <SheetTrigger asChild>
        <Button aria-label="Open setup details" size="icon-sm" type="button" variant="outline">
          <Settings2 />
        </Button>
      </SheetTrigger>
      <SheetContent className="w-[min(420px,calc(100vw-24px))]">
        <SheetHeader>
          <SheetTitle>Setup Details</SheetTitle>
          <SheetDescription>Local runner and output settings.</SheetDescription>
        </SheetHeader>
        <div className="mt-6 flex flex-col gap-3">
          <StatusRow icon={BadgeCheck} label="Status" value={status} />
          <StatusRow icon={Sparkles} label="Model" value={model} />
          <StatusRow icon={Cpu} label="Runner" value="RTX local runner" />
          <StatusRow icon={LockKeyhole} label="Auth" value={auth} />
          <StatusRow icon={FolderOutput} label="Output" value="Same folder as source" />
        </div>
      </SheetContent>
    </Sheet>
  );
}

function StatusRow({ icon: Icon, label, value }: { icon: ElementType; label: string; value: string }) {
  return (
    <div className="flex items-center gap-3 rounded-lg border bg-card p-3">
      <div className="grid size-9 shrink-0 place-items-center rounded-md bg-muted text-muted-foreground">
        <Icon className="size-4" />
      </div>
      <div className="min-w-0">
        <div className="text-xs font-medium text-muted-foreground">{label}</div>
        <div className="truncate text-sm font-semibold">{value}</div>
      </div>
    </div>
  );
}

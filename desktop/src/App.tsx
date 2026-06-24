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
  Grid2X2,
  HelpCircle,
  LockKeyhole,
  RotateCw,
  Settings2,
  Sparkles,
  Trash2,
  UploadCloud,
} from "lucide-react";
import { type DragEvent, type ElementType, useEffect, useMemo, useState } from "react";

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
  const today = new Intl.DateTimeFormat(undefined, {
    month: "long",
    day: "numeric",
    weekday: "long",
  }).format(new Date());

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
    <main className="min-h-screen overflow-x-hidden bg-background p-3 text-foreground sm:p-4">
      <div className="mx-auto flex w-full max-w-[1480px] min-w-0 gap-4">
        <ProductRail auth={auth} model={model} status={status} />

        <section className="w-full min-w-0 flex-1 overflow-hidden rounded-[28px] border bg-card/95 p-4 shadow-[0_20px_70px_rgba(32,28,20,0.08)] sm:p-6 lg:p-8">
          <header className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-start">
            <div className="min-w-0">
              <div className="mb-4 flex items-center gap-3 lg:hidden">
                <div className="grid size-10 shrink-0 place-items-center rounded-xl bg-primary text-primary-foreground shadow-sm">
                  <FileAudio2 className="size-5" />
                </div>
                <div>
                  <div className="text-lg font-semibold">Yapx3</div>
                  <div className="text-sm text-muted-foreground">Private local transcription</div>
                </div>
              </div>
              <p className="text-sm font-medium text-muted-foreground">{today}</p>
              <h1 className="mt-2 text-3xl font-semibold tracking-tight sm:text-4xl">Ready when you are.</h1>
            </div>

            <div className="grid w-full min-w-0 grid-cols-[repeat(3,minmax(0,1fr))_auto] items-center gap-1 rounded-2xl bg-secondary p-1 lg:flex lg:w-auto lg:gap-2 lg:rounded-full">
              <Metric icon={FileAudio2} label={`${queue.length} file${queue.length === 1 ? "" : "s"}`} />
              <Metric icon={FileText} label={`${completed} done`} />
              <Metric icon={LockKeyhole} label={auth === "Authorized" ? "Local" : status} />
              <DetailsSheet auth={auth} model={model} status={status} />
            </div>
          </header>

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
          />

          <section className="mt-7 grid w-full min-w-0 gap-5 xl:grid-cols-[minmax(0,1fr)_minmax(360px,0.78fr)]">
            <Card className="min-w-0 border-[#eee8de] bg-card py-0 shadow-none">
              <CardHeader className="p-4 sm:p-5">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0">
                    <p className="text-xs font-semibold uppercase text-muted-foreground">Today</p>
                    <CardTitle className="mt-2 flex items-center gap-2 text-xl">
                      Queue
                      <Badge variant="secondary">{queue.length}</Badge>
                    </CardTitle>
                    <CardDescription>
                      {completed
                        ? `${completed} transcript${completed === 1 ? "" : "s"} ready`
                        : "Drop recordings and transcribe them in place"}
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
                      {running ? (
                        <RotateCw data-icon="inline-start" className="animate-spin" />
                      ) : (
                        <Sparkles data-icon="inline-start" />
                      )}
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

            <TranscriptPanel
              item={selectedItem}
              onCopy={copyPath}
              onReveal={(path) => void revealItemInDir(path)}
              running={running}
            />
          </section>
        </section>
      </div>
    </main>
  );
}

function ProductRail({ auth, model, status }: { auth: string; model: string; status: string }) {
  return (
    <aside className="hidden min-h-[calc(100vh-32px)] w-[238px] shrink-0 flex-col rounded-[28px] bg-background p-3 lg:flex">
      <div className="flex items-center gap-3 px-3 py-4">
        <div className="grid size-10 place-items-center rounded-xl bg-primary text-primary-foreground">
          <FileAudio2 className="size-5" />
        </div>
        <div className="text-2xl font-semibold tracking-tight">Yapx3</div>
      </div>

      <nav className="mt-8 flex flex-col gap-1">
        <RailItem active icon={Grid2X2} label="Home" />
        <RailItem icon={FileAudio2} label="Recordings" />
        <RailItem icon={FileText} label="Transcripts" />
        <RailItem icon={Sparkles} label="Polish" />
      </nav>

      <div className="mt-auto flex flex-col gap-1">
        <RailItem icon={LockKeyhole} label={auth === "Authorized" ? "Local mode" : status} />
        <RailItem icon={Settings2} label={model} />
        <RailItem icon={HelpCircle} label="Help" />
      </div>
    </aside>
  );
}

function RailItem({ active, icon: Icon, label }: { active?: boolean; icon: ElementType; label: string }) {
  return (
    <div
      className={cn(
        "flex min-w-0 items-center gap-3 rounded-lg px-3 py-3 text-sm font-semibold text-muted-foreground",
        active && "bg-secondary text-foreground",
      )}
    >
      <Icon className="size-5 shrink-0" />
      <span className="truncate">{label}</span>
    </div>
  );
}

function Metric({ icon: Icon, label }: { icon: ElementType; label: string }) {
  return (
    <div className="inline-flex min-w-0 max-w-full items-center justify-center gap-2 rounded-full px-2 py-2 text-sm font-semibold text-muted-foreground sm:px-3">
      <Icon className="size-4 shrink-0" />
      <span className="whitespace-nowrap max-[359px]:sr-only">{label}</span>
    </div>
  );
}

function DropHero({
  dragging,
  onDragLeave,
  onDragOver,
  onDrop,
}: {
  dragging: boolean;
  onDragLeave: () => void;
  onDragOver: (event: DragEvent<HTMLElement>) => void;
  onDrop: (event: DragEvent<HTMLElement>) => void;
}) {
  return (
    <section
      className={cn(
        "mt-7 w-full max-w-full overflow-hidden rounded-[28px] border bg-[#17120e] text-white shadow-[inset_0_1px_0_rgba(255,255,255,0.18)] transition duration-200",
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
            <LockKeyhole />
            Private on this device
          </Badge>
          <div>
            <h2 className="max-w-full break-words font-serif text-3xl leading-tight tracking-normal sm:text-5xl">
              Drop recordings. Get polished text.
            </h2>
            <p className="mt-4 max-w-full text-base leading-7 text-white/82 sm:max-w-xl">
              Bring in audio or video files and Yapx3 will write transcripts beside the source, ready to review.
            </p>
          </div>
          <div className="flex min-w-0 flex-wrap items-center gap-3">
            <div className="inline-flex max-w-full items-center gap-2 rounded-lg bg-white px-4 py-3 text-sm font-semibold text-[#221d18]">
              <UploadCloud className="size-4" />
              Drop files here
            </div>
            <span className="max-w-full break-words text-sm font-medium text-white/72">{acceptedFormats}</span>
          </div>
        </div>
      </div>
    </section>
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
    <Card className="min-h-[420px] min-w-0 border-[#eee8de] bg-card py-0 shadow-none xl:sticky xl:top-5 xl:min-h-[calc(100vh-180px)]">
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

        <ScrollArea className="min-h-[240px] flex-1 rounded-lg border bg-[#fffdf8]">
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

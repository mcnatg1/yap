import { invoke, isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  BadgeCheck,
  Cpu,
  FileAudio2,
  FolderOutput,
  LockKeyhole,
  RotateCw,
  Sparkles,
  Trash2,
  UploadCloud,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { StackedUpload, type UploadItem } from "@/components/stacked-upload";
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

  const hasRunnable = useMemo(
    () => queue.some((item) => item.status === "queued" || item.status === "error"),
    [queue],
  );

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
        setStatus("Drop MP3, M4A, WAV, MP4, FLAC, OGG, or WEBM files.");
        return current;
      }

      const newItems = accepted.map((path, index) => ({
        id: nextId + index,
        path,
        name: basename(path),
        status: "queued" as const,
      }));
      setNextId((id) => id + newItems.length);
      return [...current, ...newItems];
    });
  }

  async function runQueue() {
    const pending = queue.filter((item) => item.status === "queued" || item.status === "error");
    if (!pending.length || running) return;

    setRunning(true);
    setStatus("Transcribing locally...");
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

  return (
    <main className="min-h-screen overflow-x-hidden bg-[radial-gradient(circle_at_top_left,#f8fafc_0,#eef3f2_46%,#e9eeee_100%)] p-4 text-slate-950 sm:p-5">
      <div className="mx-auto grid max-w-7xl gap-4">
        <header className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <div className="grid size-10 place-items-center rounded-lg bg-teal-600 text-white shadow-sm shadow-teal-900/10">
              <FileAudio2 className="size-5" />
            </div>
            <div>
              <h1 className="text-xl font-semibold tracking-tight">Yap^3</h1>
              <p className="text-sm text-slate-500">Files stay on this machine.</p>
            </div>
          </div>

          <div className="inline-flex max-w-full items-center gap-2 rounded-lg border border-teal-200 bg-white px-3 py-2 text-sm font-semibold text-teal-700 shadow-sm">
            <BadgeCheck className="size-4" />
            {status}
          </div>
        </header>

        <section className="grid gap-4 lg:grid-cols-[minmax(0,1.5fr)_360px]">
          <div className="grid gap-4">
            <section
              className={cn(
                "group relative min-h-[300px] overflow-hidden rounded-lg border bg-white shadow-sm transition duration-200",
                dragging ? "border-teal-400 shadow-lg shadow-teal-900/10" : "border-slate-200",
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
              <div className="absolute inset-0 bg-[linear-gradient(to_right,#0f766e10_1px,transparent_1px),linear-gradient(to_bottom,#0f766e10_1px,transparent_1px)] bg-[size:28px_28px]" />
              <div className="absolute inset-x-12 top-10 h-32 rounded-full bg-teal-200/20 blur-3xl" />

              <div className="relative grid h-full place-items-center p-8 text-center">
                <div>
                  <div className="mx-auto grid size-20 place-items-center rounded-lg border border-teal-200 bg-teal-50 text-teal-700 shadow-inner">
                    <UploadCloud className="size-9" />
                  </div>
                  <h2 className="mt-6 text-3xl font-semibold tracking-tight">Drop audio files</h2>
                  <p className="mt-2 text-sm text-slate-500">
                    MP3, M4A, WAV, MP4, FLAC, OGG, WEBM
                  </p>
                  <p className="mt-5 inline-flex items-center gap-2 rounded-md border border-slate-200 bg-white px-3 py-2 text-xs font-medium text-slate-600 shadow-sm">
                    <LockKeyhole className="size-3.5 text-teal-600" />
                    RTX local runner
                  </p>
                </div>
              </div>
            </section>

            <section className="min-h-[300px] rounded-lg border border-slate-200 bg-white p-4 shadow-sm">
              <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
                <div>
                  <div className="flex items-center gap-2">
                    <h2 className="text-sm font-semibold">Queue</h2>
                    <span className="rounded-md bg-slate-100 px-2 py-0.5 text-xs font-semibold text-slate-600">
                      {queue.length}
                    </span>
                  </div>
                  <p className="text-xs text-slate-500">Drop more files any time.</p>
                </div>
                <div className="flex flex-wrap justify-end gap-2">
                  <button
                    className="inline-flex items-center gap-2 rounded-md border border-slate-200 px-3 py-2 text-sm font-semibold text-slate-700 transition hover:border-slate-300 disabled:cursor-not-allowed disabled:opacity-50"
                    disabled={running || !queue.length}
                    onClick={clearQueue}
                    type="button"
                  >
                    <Trash2 className="size-4" />
                    Clear
                  </button>
                  <button
                    className="inline-flex items-center gap-2 rounded-md bg-teal-700 px-3 py-2 text-sm font-semibold text-white shadow-sm transition hover:bg-teal-800 disabled:cursor-not-allowed disabled:opacity-50"
                    disabled={running || !hasRunnable}
                    onClick={runQueue}
                    type="button"
                  >
                    {running ? <RotateCw className="size-4 animate-spin" /> : <Sparkles className="size-4" />}
                    Transcribe
                  </button>
                </div>
              </div>

              <StackedUpload
                items={queue}
                onRemove={removeItem}
                onReveal={(path) => void revealItemInDir(path)}
              />
            </section>
          </div>

          <aside className="grid content-start gap-3">
            <StatusRow icon={Sparkles} label="Model" value={model} />
            <StatusRow icon={Cpu} label="Runner" value="RTX local runner" />
            <StatusRow icon={LockKeyhole} label="Auth" value={auth} tone={auth === "Authorized" ? "good" : "warn"} />
            <StatusRow icon={FolderOutput} label="Output" value="Same folder as source" />
            <div className="mt-2 rounded-lg border border-slate-200 bg-white p-3 shadow-sm">
              <div className="text-xs font-semibold uppercase text-slate-500">Ready</div>
              <p className="mt-1 text-sm text-slate-700">
                Drop files, transcribe locally, reveal transcripts beside the original audio.
              </p>
            </div>
          </aside>
        </section>
      </div>
    </main>
  );
}

function StatusRow({
  icon: Icon,
  label,
  value,
  tone = "default",
}: {
  icon: React.ElementType;
  label: string;
  value: string;
  tone?: "default" | "good" | "warn";
}) {
  return (
    <div className="flex items-center gap-3 rounded-lg border border-slate-200 bg-white p-3 shadow-sm">
      <div
        className={cn(
          "grid size-9 place-items-center rounded-md bg-white text-slate-500 ring-1 ring-slate-200",
          tone === "good" && "text-teal-700 ring-teal-200",
          tone === "warn" && "text-amber-700 ring-amber-200",
        )}
      >
        <Icon className="size-4" />
      </div>
      <div className="min-w-0">
        <div className="text-xs font-semibold uppercase text-slate-500">{label}</div>
        <div className="truncate text-sm font-semibold text-slate-900">{value}</div>
      </div>
    </div>
  );
}

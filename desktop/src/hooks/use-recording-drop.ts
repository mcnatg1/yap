import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { type DragEvent, useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

export function useRecordingDrop() {
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    if (!isTauri()) return;

    const unlistenDrag = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "enter") setDragging(true);
      if (event.payload.type === "leave" || event.payload.type === "drop") setDragging(false);
    });
    const unlistenError = listen<string>("recording-jobs-import-error", (event) => {
      toast.error(`Could not add recordings: ${event.payload}`);
    });

    return () => {
      void unlistenDrag.then((fn) => fn());
      void unlistenError.then((fn) => fn());
    };
  }, []);

  const onDragLeave = useCallback(() => setDragging(false), []);

  const onDragOver = useCallback((event: DragEvent<HTMLElement>) => {
    event.preventDefault();
    setDragging(true);
  }, []);

  const onDrop = useCallback((event: DragEvent<HTMLElement>) => {
    event.preventDefault();
    setDragging(false);
    if (!isTauri()) toast.info("Preview only");
  }, []);

  return { dragging, onDragLeave, onDragOver, onDrop };
}

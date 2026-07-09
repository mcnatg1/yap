import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { type DragEvent, useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

type RecordingDropHandler = (paths: string[]) => Promise<void> | void;

export function useRecordingDrop(onDropPaths: RecordingDropHandler) {
  const [dragging, setDragging] = useState(false);
  const onDropPathsRef = useRef(onDropPaths);

  useEffect(() => {
    onDropPathsRef.current = onDropPaths;
  }, [onDropPaths]);

  useEffect(() => {
    if (!isTauri()) return;

    const unlistenDrag = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "enter") setDragging(true);
      if (event.payload.type === "leave" || event.payload.type === "drop") setDragging(false);
      if (event.payload.type === "drop") void onDropPathsRef.current(event.payload.paths);
    });

    return () => {
      void unlistenDrag.then((fn) => fn());
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

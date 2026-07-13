import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { type DragEvent, useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { fireAndReport } from "@/lib/fire-and-report";

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
      if (event.payload.type === "drop") {
        const { paths } = event.payload;
        fireAndReport(
          () => onDropPathsRef.current(paths),
          (error) => toast.error(`Could not add recordings: ${error.message}`),
        );
      }
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

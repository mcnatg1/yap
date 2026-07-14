import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { capturePhysicalShortcut } from "@/components/settings/shortcut-chord";

export function ShortcutRecorder({
  current,
  disabled,
  label,
  onChange,
  onReset,
}: {
  current: string;
  disabled?: boolean;
  label: string;
  onChange: (chord: string) => void;
  onReset: () => void;
}) {
  const [armed, setArmed] = useState(false);
  const [error, setError] = useState("");
  const containerRef = useRef<HTMLDivElement>(null);
  const captureRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!armed) return;
    captureRef.current?.focus();

    function cancelFromOutside(event: PointerEvent) {
      if (event.target instanceof Node && containerRef.current?.contains(event.target)) return;
      setArmed(false);
      setError("");
    }

    document.addEventListener("pointerdown", cancelFromOutside, true);
    return () => document.removeEventListener("pointerdown", cancelFromOutside, true);
  }, [armed]);

  useEffect(() => {
    if (!disabled) return;
    setArmed(false);
    setError("");
  }, [disabled]);

  function cancel() {
    setArmed(false);
    setError("");
  }

  return (
    <div className="flex min-w-0 flex-wrap items-center justify-end gap-2" ref={containerRef}>
      {armed ? (
        <>
          <Button
            aria-label={`Press shortcut for ${label}`}
            className="min-w-[154px] border-fuchsia-400/60 bg-fuchsia-50 text-fuchsia-950"
            data-testid={`shortcut-capture-${label.toLowerCase().replace(/\s+/g, "-")}`}
            disabled={disabled}
            onKeyDown={(event) => {
              event.preventDefault();
              event.stopPropagation();
              const result = capturePhysicalShortcut(event);
              if (result.kind === "ignore") return;
              if (result.kind === "cancel") {
                cancel();
                return;
              }
              if (result.kind === "invalid") {
                setError(result.message);
                return;
              }
              cancel();
              if (result.chord !== current) onChange(result.chord);
            }}
            ref={captureRef}
            type="button"
            variant="outline"
          >
            Press keys…
          </Button>
          <Button onClick={cancel} type="button" variant="ghost">
            Cancel
          </Button>
        </>
      ) : (
        <>
          <Button
            disabled={disabled}
            onClick={() => {
              setError("");
              setArmed(true);
            }}
            type="button"
            variant="secondary"
          >
            Change shortcut
          </Button>
          <Button disabled={disabled} onClick={onReset} type="button" variant="ghost">
            Reset
          </Button>
        </>
      )}
      {error ? (
        <span aria-live="polite" className="w-full text-right text-xs text-destructive">
          {error}
        </span>
      ) : null}
    </div>
  );
}

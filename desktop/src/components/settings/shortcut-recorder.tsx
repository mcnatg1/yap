import { Button } from "@/components/ui/button";

export function ShortcutRecorder({
  disabled,
  onRecord,
  onReset,
}: {
  disabled?: boolean;
  onRecord: () => void;
  onReset: () => void;
}) {
  return (
    <div className="flex min-w-0 flex-wrap items-center justify-end gap-2">
      <Button disabled={disabled} onClick={onRecord} type="button" variant="secondary">
        Record shortcut
      </Button>
      <Button disabled={disabled} onClick={onReset} type="button" variant="ghost">
        Reset
      </Button>
    </div>
  );
}

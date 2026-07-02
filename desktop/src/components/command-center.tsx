import {
  FileText,
  Grid2X2,
  HelpCircle,
  Mic,
  Settings2,
  Sparkles,
  UploadCloud,
} from "lucide-react";

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
import type { TranscriptHistoryEntry } from "@/history";
import { formatHistoryDate, type RailAction } from "@/lib/app-types";

export function CommandCenter({
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
            Add files
            <CommandShortcut>Files</CommandShortcut>
          </CommandItem>
          <CommandItem onSelect={() => run(() => onAction("home"))}>
            <Grid2X2 />
            Go home
          </CommandItem>
          <CommandItem onSelect={() => run(() => onAction("transcribe"))}>
            <Mic />
            Transcribe
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
            Settings
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

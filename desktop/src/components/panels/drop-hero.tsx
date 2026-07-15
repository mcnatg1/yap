import { type DragEvent } from "react";
import { CloudArrowUp as UploadCloud } from "@phosphor-icons/react/CloudArrowUp";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { acceptedFormats } from "@/lib/media-file";
import { cn } from "@/lib/utils";

export function DropHero({
  dragging,
  onDragLeave,
  onDragOver,
  onDrop,
  onOpenHelp,
  onPickFiles,
}: {
  dragging: boolean;
  onDragLeave: () => void;
  onDragOver: (event: DragEvent<HTMLElement>) => void;
  onDrop: (event: DragEvent<HTMLElement>) => void;
  onOpenHelp?: () => void;
  onPickFiles: () => void;
}) {
  return (
    <section
      className={cn(
        "surface-workspace-inset mt-5 w-full border-2 border-dashed bg-[var(--surface-transcript)] transition-[border-color,background-color,box-shadow] duration-200",
        dragging ? "border-primary bg-[var(--primary-soft)] shadow-sm" : "border-border",
      )}
      onDragLeave={onDragLeave}
      onDragOver={onDragOver}
      onDrop={onDrop}
    >
      <div className="flex min-h-[168px] flex-col items-center justify-center gap-4 px-6 py-8 text-center">
        <div className="flex size-12 items-center justify-center rounded-full bg-secondary">
          <UploadCloud className="text-primary" />
        </div>
        <div className="max-w-md">
          <h2 className="text-lg font-semibold tracking-tight">Drop recordings here</h2>
          <p className="mt-1.5 text-sm leading-6 text-muted-foreground">
            Choose files to add them to your organization's transcription server queue. {acceptedFormats}.
          </p>
        </div>
        <div className="flex flex-wrap items-center justify-center gap-3">
          <Button onClick={onPickFiles} type="button">
            <UploadCloud data-icon="inline-start" />
            Choose files
          </Button>
          <Badge className="border-primary/20 bg-[var(--primary-soft)] text-primary hover:bg-[var(--primary-soft)]" variant="outline">
            <UploadCloud data-icon="inline-start" />
            Organization server queue
          </Badge>
          {onOpenHelp ? (
            <Button
              className="h-auto px-0 text-muted-foreground"
              onClick={onOpenHelp}
              size="sm"
              type="button"
              variant="link"
            >
              How this works
            </Button>
          ) : null}
        </div>
      </div>
    </section>
  );
}

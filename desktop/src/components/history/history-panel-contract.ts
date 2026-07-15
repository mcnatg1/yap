import type { TranscriptHistoryEntry } from "@/history";

export type HistoryEntryActions = {
  onCopy: (entry: TranscriptHistoryEntry) => void;
  onDelete: (entry: TranscriptHistoryEntry) => void;
  onDeleteRecoverable: (entry: TranscriptHistoryEntry) => void;
  onHide: (outputPath: string) => void;
  onOpen: (entry: TranscriptHistoryEntry) => void;
  onPreview: (entry: TranscriptHistoryEntry) => void;
  onRecover: (entry: TranscriptHistoryEntry) => void;
  onReveal: (entry: TranscriptHistoryEntry) => void;
};

export type HistoryPanelProps = HistoryEntryActions & {
  entries: TranscriptHistoryEntry[];
  onLoadPreviewText?: (entry: TranscriptHistoryEntry) => Promise<string>;
  onOpenHelp?: () => void;
  onSelect: (entry: TranscriptHistoryEntry, origin?: DOMRect) => void;
  selectedOutputPath?: string;
};

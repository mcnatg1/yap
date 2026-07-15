import { ArrowClockwise as Recover } from "@phosphor-icons/react/ArrowClockwise";
import { Copy } from "@phosphor-icons/react/Copy";
import { EyeSlash } from "@phosphor-icons/react/EyeSlash";
import { FileText } from "@phosphor-icons/react/FileText";
import { FolderOpen } from "@phosphor-icons/react/FolderOpen";
import { DotsThree as MoreHorizontal } from "@phosphor-icons/react/DotsThree";
import { Trash as Trash2 } from "@phosphor-icons/react/Trash";
import { useState } from "react";

import type { HistoryEntryActions } from "@/components/history/history-panel-contract";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  canDeleteTranscriptHistoryEntry,
  isRecoverableTranscriptHistoryEntry,
  isUntrustedNativeLiveTranscriptHistoryEntry,
  recoverableLiveSessionActionIdentity,
} from "@/native-history";
import type { TranscriptHistoryEntry } from "@/history-model";

export function HistoryActionMenu({
  actions,
  entry,
}: {
  actions: HistoryEntryActions;
  entry: TranscriptHistoryEntry;
}) {
  const [confirmDelete, setConfirmDelete] = useState(false);
  const canDelete = canDeleteTranscriptHistoryEntry(entry);
  const recoverable = isRecoverableTranscriptHistoryEntry(entry);
  const canMutateRecoverable = recoverableLiveSessionActionIdentity(entry) !== undefined;
  const hideOnly = isUntrustedNativeLiveTranscriptHistoryEntry(entry);

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            aria-label={`Actions for ${entry.name}`}
            onClick={(event) => event.stopPropagation()}
            size="icon-xs"
            type="button"
            variant="ghost"
          >
            <MoreHorizontal />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" onClick={(event) => event.stopPropagation()}>
          <DropdownMenuLabel>{recoverable ? "Partial" : "Transcript"}</DropdownMenuLabel>
          {recoverable ? (
            <DropdownMenuGroup>
              {canMutateRecoverable ? (
                <DropdownMenuItem onSelect={() => actions.onRecover(entry)}>
                  <Recover />
                  Recover
                </DropdownMenuItem>
              ) : null}
              <DropdownMenuItem onSelect={() => actions.onHide(entry.outputPath)}>
                <EyeSlash />
                Hide
              </DropdownMenuItem>
              {canMutateRecoverable ? (
                <DropdownMenuItem
                  onSelect={() => actions.onDeleteRecoverable(entry)}
                  variant="destructive"
                >
                  <Trash2 />
                  Delete
                </DropdownMenuItem>
              ) : null}
            </DropdownMenuGroup>
          ) : hideOnly ? (
            <DropdownMenuItem onSelect={() => actions.onHide(entry.outputPath)}>
              <EyeSlash />
              Hide
            </DropdownMenuItem>
          ) : (
            <>
              <DropdownMenuGroup>
                <DropdownMenuItem onSelect={() => actions.onPreview(entry)}>
                  <FileText />
                  Preview
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => actions.onCopy(entry)}>
                  <Copy />
                  Copy transcript
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => actions.onOpen(entry)}>
                  <FileText />
                  Open file
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => actions.onReveal(entry)}>
                  <FolderOpen />
                  Reveal in Explorer
                </DropdownMenuItem>
              </DropdownMenuGroup>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => actions.onHide(entry.outputPath)}>
                <EyeSlash />
                Hide
              </DropdownMenuItem>
            </>
          )}
          {!recoverable && canDelete ? (
            <DropdownMenuItem
              onSelect={(event) => {
                event.preventDefault();
                setConfirmDelete(true);
              }}
              variant="destructive"
            >
              <Trash2 />
              Delete
            </DropdownMenuItem>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>

      <AlertDialog onOpenChange={setConfirmDelete} open={confirmDelete}>
        <AlertDialogContent onClick={(event) => event.stopPropagation()}>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete from device?</AlertDialogTitle>
            <AlertDialogDescription>
              This deletes the saved transcript. If the recording was captured by Yap, that audio file
              is deleted too.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-white hover:bg-destructive/90 focus-visible:ring-destructive/20"
              onClick={() => actions.onDelete(entry)}
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

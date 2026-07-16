import { CaretDown as ChevronDown } from "@phosphor-icons/react/CaretDown";

import type { PolishRunDetails } from "@/components/polish/use-polish-draft";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { ScrollArea } from "@/components/ui/scroll-area";

export function PolishPreviewColumn({
  empty,
  title,
  value,
}: {
  empty: string;
  title: string;
  value?: string;
}) {
  return (
    <div className="min-w-0">
      <div className="border-b p-3">
        <p className="text-xs font-semibold text-muted-foreground">{title}</p>
      </div>
      <ScrollArea className="h-[220px]">
        <div className="p-4">
          {value?.trim() ? (
            <pre className="whitespace-pre-wrap break-words text-[15px] leading-7 text-foreground">
              {value}
            </pre>
          ) : (
            <p className="text-sm leading-6 text-muted-foreground">{empty}</p>
          )}
        </div>
      </ScrollArea>
    </div>
  );
}

export function PolishDetails({
  ready,
  runDetails,
  statusLine,
}: {
  ready: boolean;
  runDetails: PolishRunDetails | null;
  statusLine: string;
}) {
  return (
    <div className="flex flex-col gap-2">
      <p className="text-sm text-muted-foreground">{statusLine}</p>
      {ready ? (
        <Collapsible>
          <CollapsibleTrigger asChild>
            <Button
              className="h-auto w-fit gap-1 px-0 text-muted-foreground hover:text-foreground"
              size="sm"
              type="button"
              variant="link"
            >
              Details
              <ChevronDown className="size-4 transition-transform [[data-state=open]_&]:rotate-180" />
            </Button>
          </CollapsibleTrigger>
          <CollapsibleContent className="text-sm leading-6 text-muted-foreground">
            {runDetails ? (
              <p>
                {[
                  runDetails.totalSeconds ? `${runDetails.totalSeconds}s` : "",
                  runDetails.tokensPerSecond ? `${runDetails.tokensPerSecond} tok/s` : "",
                  runDetails.model,
                  "On this device",
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </p>
            ) : (
              <p>Polish runs locally on this device.</p>
            )}
          </CollapsibleContent>
        </Collapsible>
      ) : null}
    </div>
  );
}

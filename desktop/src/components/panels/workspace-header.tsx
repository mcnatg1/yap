import { Search, Settings2 } from "lucide-react";

import { PrivacyStatus } from "@/components/app/privacy-status";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Kbd, KbdGroup } from "@/components/ui/kbd";

export function WorkspaceHeader({
  auth,
  description,
  historyCount,
  onOpenCommand,
  onOpenDetails,
  onOpenHelp,
  status,
  title,
}: {
  auth: string;
  description: string;
  historyCount: number;
  onOpenCommand: () => void;
  onOpenDetails: () => void;
  onOpenHelp: () => void;
  status: string;
  title: string;
}) {
  return (
    <header className="flex flex-wrap items-start justify-between gap-4">
      <div className="min-w-0">
        <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
        <p className="mt-1.5 max-w-xl text-sm leading-6 text-muted-foreground">{description}</p>
      </div>

      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <PrivacyStatus auth={auth} status={status} />
        {historyCount ? (
          <Badge className="rounded-full px-3 py-1.5 text-sm font-semibold tabular-nums" variant="secondary">
            {historyCount} saved
          </Badge>
        ) : null}
        <Button
          aria-label="Open command menu"
          className="px-2"
          onClick={onOpenCommand}
          size="sm"
          type="button"
          variant="outline"
        >
          <Search data-icon="inline-start" />
          <span className="hidden xl:inline">Search</span>
          <KbdGroup className="hidden xl:inline-flex">
            <Kbd>Ctrl</Kbd>
            <Kbd>K</Kbd>
          </KbdGroup>
        </Button>
        <Button
          className="h-auto px-1 text-muted-foreground"
          onClick={onOpenHelp}
          size="sm"
          type="button"
          variant="link"
        >
          Help
        </Button>
        <Button aria-label="Open settings" onClick={onOpenDetails} size="icon-sm" type="button" variant="outline">
          <Settings2 />
        </Button>
      </div>
    </header>
  );
}

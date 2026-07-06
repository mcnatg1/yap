import { GearSix as Settings2 } from "@phosphor-icons/react/GearSix";

import { PrivacyStatus } from "@/components/app/privacy-status";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

export function WorkspaceHeader({
  auth,
  description,
  historyCount,
  onOpenDetails,
  onOpenHelp,
  status,
  title,
}: {
  auth: string;
  description: string;
  historyCount: number;
  onOpenDetails: () => void;
  onOpenHelp: () => void;
  status: string;
  title: string;
}) {
  return (
    <header className="flex flex-wrap items-start justify-between gap-4">
      <div className="min-w-0">
        <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
        {description ? (
          <p className="mt-1.5 max-w-xl text-sm leading-6 text-muted-foreground">{description}</p>
        ) : null}
      </div>

      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <PrivacyStatus auth={auth} status={status} />
        {historyCount ? (
          <Badge className="rounded-full px-3 py-1.5 text-sm font-semibold tabular-nums" variant="secondary">
            {historyCount} saved
          </Badge>
        ) : null}
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

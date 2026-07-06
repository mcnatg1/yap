import { LockKey as LockKeyhole } from "@phosphor-icons/react/LockKey";

import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Skeleton } from "@/components/ui/skeleton";

export function PrivacyStatus({ auth, status }: { auth: string; status: string }) {
  const label = auth === "Ready" || auth === "Authorized" ? "Ready" : status;
  const checking = auth === "Checking" || status === "Starting";

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button className="rounded-full px-3 font-semibold" size="sm" type="button" variant="secondary">
          <LockKeyhole data-icon="inline-start" />
          {checking ? <Skeleton className="h-4 w-14 rounded-full" /> : label}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end">
        <PopoverHeader>
          <PopoverTitle>{checking ? "Checking setup" : label}</PopoverTitle>
          <PopoverDescription>
            {label === "Ready" ? "Fallback ready." : status}
          </PopoverDescription>
        </PopoverHeader>
      </PopoverContent>
    </Popover>
  );
}

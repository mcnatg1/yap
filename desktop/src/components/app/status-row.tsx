import { type ElementType } from "react";

import {
  Item,
  ItemContent,
  ItemDescription,
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item";
import { cn } from "@/lib/utils";

export function StatusRow({
  icon: Icon,
  label,
  value,
  wrap,
}: {
  icon: ElementType;
  label: string;
  value: string;
  wrap?: boolean;
}) {
  return (
    <Item size="sm" variant="outline">
      <ItemMedia variant="icon">
        <Icon />
      </ItemMedia>
      <ItemContent className="min-w-0">
        <ItemTitle className="text-xs text-muted-foreground">{label}</ItemTitle>
        <ItemDescription className={cn("font-semibold tabular-nums text-foreground", wrap ? "line-clamp-none break-words" : "truncate")}>
          {value}
        </ItemDescription>
      </ItemContent>
    </Item>
  );
}

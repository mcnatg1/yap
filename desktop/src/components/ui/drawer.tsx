import * as React from "react"

import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { cn } from "@/lib/utils"

function Drawer({ ...props }: React.ComponentProps<typeof Sheet>) {
  return <Sheet data-slot="drawer" {...props} />
}

function DrawerContent({
  className,
  children,
  ...props
}: Omit<React.ComponentProps<typeof SheetContent>, "side">) {
  return (
    <SheetContent
      data-slot="drawer-content"
      side="bottom"
      className={cn(
        "inset-x-3 bottom-3 max-h-[86vh] overflow-y-auto rounded-2xl border bg-background p-0 sm:left-auto sm:w-[420px]",
        className,
      )}
      {...props}
    >
      <div className="mx-auto mt-3 h-1.5 w-12 rounded-full bg-muted" />
      {children}
    </SheetContent>
  )
}

function DrawerHeader({ className, ...props }: React.ComponentProps<typeof SheetHeader>) {
  return <SheetHeader data-slot="drawer-header" className={cn("pt-5", className)} {...props} />
}

function DrawerTitle({ ...props }: React.ComponentProps<typeof SheetTitle>) {
  return <SheetTitle data-slot="drawer-title" {...props} />
}

function DrawerDescription({ ...props }: React.ComponentProps<typeof SheetDescription>) {
  return <SheetDescription data-slot="drawer-description" {...props} />
}

export { Drawer, DrawerContent, DrawerDescription, DrawerHeader, DrawerTitle }

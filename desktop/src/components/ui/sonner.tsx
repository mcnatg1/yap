"use client"

import { CheckCircle as CircleCheckIcon } from "@phosphor-icons/react/CheckCircle";
import { Info as InfoIcon } from "@phosphor-icons/react/Info";
import { SpinnerGap as Loader2Icon } from "@phosphor-icons/react/SpinnerGap";
import { XCircle as OctagonXIcon } from "@phosphor-icons/react/XCircle";
import { Warning as TriangleAlertIcon } from "@phosphor-icons/react/Warning";
import { Toaster as Sonner, type ToasterProps } from "sonner"

const Toaster = ({ ...props }: ToasterProps) => {
  return (
    <Sonner
      theme="light"
      className="toaster group"
      icons={{
        success: <CircleCheckIcon data-icon="inline-start" />,
        info: <InfoIcon data-icon="inline-start" />,
        warning: <TriangleAlertIcon data-icon="inline-start" />,
        error: <OctagonXIcon data-icon="inline-start" />,
        loading: <Loader2Icon className="animate-spin" data-icon="inline-start" />,
      }}
      style={
        {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border)",
          "--border-radius": "var(--radius)",
        } as React.CSSProperties
      }
      {...props}
    />
  )
}

export { Toaster }

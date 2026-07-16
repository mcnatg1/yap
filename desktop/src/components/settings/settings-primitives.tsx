import type { ReactNode } from "react";

export function SettingsGroup({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-2xl bg-muted/35 p-6 shadow-[0_0_0_1px_rgba(0,0,0,0.04)]">
      {children}
    </div>
  );
}

export function SettingsRow({
  action,
  children,
  detail,
  error,
  label,
  value,
}: {
  action?: ReactNode;
  children?: ReactNode;
  detail?: string;
  error?: string;
  label: string;
  value: string;
}) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_minmax(260px,360px)] gap-4 border-b py-5 first:pt-0 last:border-b-0 last:pb-0">
      <div className="min-w-0 text-pretty">
        <div className="font-medium">{label}</div>
        <div className="mt-1 break-words text-sm text-foreground/80">{value}</div>
        {detail ? <div className="mt-1 break-words text-xs text-muted-foreground">{detail}</div> : null}
        {error ? <div className="mt-1 break-words text-xs text-destructive">{error}</div> : null}
      </div>
      <div className="flex min-w-0 flex-wrap items-center justify-end gap-2">
        {children}
        {action}
      </div>
    </div>
  );
}

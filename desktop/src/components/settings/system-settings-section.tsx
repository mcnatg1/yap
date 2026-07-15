import { useId } from "react";

import { SettingsGroup, SettingsRow } from "@/components/settings/settings-primitives";
import type { FallbackLifecycleActionId, FallbackLifecycleProjection } from "@/components/settings/settings-lifecycle";
import type { ServerSettingsDraftController } from "@/components/settings/use-server-settings-draft";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { LocalComputeTargetView } from "@/lib/app-types";

export function SystemSettingsSection({
  busy,
  fallbackLifecycle,
  fallbackLocked,
  liveActive,
  localComputeTargets,
  onFallbackAction,
  onSetLocalComputeTarget,
  server,
}: {
  busy: boolean;
  fallbackLifecycle: FallbackLifecycleProjection;
  fallbackLocked: boolean;
  liveActive: boolean;
  localComputeTargets: LocalComputeTargetView[];
  onFallbackAction: (actionId: FallbackLifecycleActionId) => void;
  onSetLocalComputeTarget: (targetId: string) => void;
  server: ServerSettingsDraftController;
}) {
  const computeLabelId = useId();
  const selectedComputeTarget = localComputeTargets.find((target) => target.selected);
  const primaryFallbackAction = fallbackLifecycle.primaryAction;

  return (
    <SettingsGroup>
      <SettingsRow
        detail={server.notice || "HTTPS required outside approved private development."}
        error={server.error}
        label="Server"
        value={server.pending ? "Checking" : server.enabled ? "Enabled" : "Disabled"}
      >
        <div className="flex w-full max-w-[520px] flex-wrap justify-end gap-2">
          <Input
            aria-label="Server URL"
            className="min-w-[240px] flex-1"
            disabled={server.pending}
            onChange={(event) => server.setUrl(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void server.save();
            }}
            placeholder="https://server.example"
            value={server.url}
          />
          <Button
            aria-checked={server.enabled}
            disabled={server.pending}
            onClick={server.toggleEnabled}
            role="switch"
            type="button"
            variant={server.enabled ? "default" : "secondary"}
          >
            {server.enabled ? "Enabled" : "Disabled"}
          </Button>
          <Button
            disabled={server.pending}
            onClick={() => void server.save()}
            type="button"
            variant="secondary"
          >
            Save
          </Button>
          <Button
            disabled={server.pending || !server.enabled || !server.url.trim()}
            onClick={() => void server.testConnection()}
            type="button"
          >
            Test Connection
          </Button>
        </div>
      </SettingsRow>
      <SettingsRow
        detail={liveActive ? "Stop live before changing compute." : "Local live uses the CPU runtime. Server owns GPU routing."}
        label="Compute"
        value={selectedComputeTarget?.label ?? "Auto"}
      >
        <Label className="sr-only" id={computeLabelId}>
          Compute
        </Label>
        <Select
          disabled={busy || fallbackLocked}
          onValueChange={onSetLocalComputeTarget}
          value={selectedComputeTarget?.id ?? "auto"}
        >
          <SelectTrigger aria-labelledby={computeLabelId} className="w-full max-w-[360px]">
            <SelectValue placeholder="Auto" />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              {localComputeTargets.map((target) => (
                <SelectItem key={target.id} value={target.id}>
                  {target.label}
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>
      </SettingsRow>
      <SettingsRow
        detail={fallbackLifecycle.detail}
        label="Local fallback"
        value={fallbackLifecycle.value}
      >
        <div className="flex flex-wrap justify-end gap-2">
          {primaryFallbackAction ? (
            <Button
              disabled={primaryFallbackAction.disabled}
              onClick={() => onFallbackAction(primaryFallbackAction.id)}
              type="button"
            >
              {primaryFallbackAction.label}
            </Button>
          ) : null}
          {fallbackLifecycle.secondaryActions.map((action) => (
            <Button
              disabled={action.disabled}
              key={action.id}
              onClick={() => onFallbackAction(action.id)}
              type="button"
              variant={action.id === "open-folder" ? "ghost" : "secondary"}
            >
              {action.label}
            </Button>
          ))}
        </div>
      </SettingsRow>
    </SettingsGroup>
  );
}

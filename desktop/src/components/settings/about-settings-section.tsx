import { SealCheck as BadgeCheck } from "@phosphor-icons/react/SealCheck";
import { FolderSimple as FolderOutput } from "@phosphor-icons/react/FolderSimple";
import { LockKey as LockKeyhole } from "@phosphor-icons/react/LockKey";
import { HardDrives as Server } from "@phosphor-icons/react/HardDrives";

import { StatusRow } from "@/components/app/status-row";
import { SettingsGroup } from "@/components/settings/settings-primitives";
import { Button } from "@/components/ui/button";

export function AboutSettingsSection({
  auth,
  canSkipSetup,
  onSkipSetup,
  serverLabel,
  skipSetupDisabled,
  status,
}: {
  auth: string;
  canSkipSetup: boolean;
  onSkipSetup: () => void;
  serverLabel: string;
  skipSetupDisabled: boolean;
  status: string;
}) {
  return (
    <SettingsGroup>
      <StatusRow icon={BadgeCheck} label="Status" value={status} />
      <StatusRow icon={Server} label="Server" value={serverLabel} />
      <StatusRow icon={LockKeyhole} label="Auth" value={auth} />
      <StatusRow icon={FolderOutput} label="Output" value="Source folder" />
      {canSkipSetup ? (
        <Button
          disabled={skipSetupDisabled}
          onClick={onSkipSetup}
          type="button"
          variant="secondary"
        >
          Skip setup prompt
        </Button>
      ) : null}
    </SettingsGroup>
  );
}

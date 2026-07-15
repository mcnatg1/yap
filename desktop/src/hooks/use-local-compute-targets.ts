import { isTauri } from "@tauri-apps/api/core";
import { useCallback, useState } from "react";
import { toast } from "sonner";

import type { LocalComputeTargetView } from "@/lib/setup-model";
import { listLocalComputeTargets, setLocalComputeTarget } from "@/settings";

const initialLocalComputeTargets: LocalComputeTargetView[] = [
  { id: "auto", label: "Auto", selected: true },
  { id: "cpu", label: "CPU", selected: false },
];

export function useLocalComputeTargets(blocked: boolean) {
  const [computeTargetPending, setComputeTargetPending] = useState(false);
  const [localComputeTargets, setLocalComputeTargets] = useState<LocalComputeTargetView[]>(initialLocalComputeTargets);

  const loadComputeTargets = useCallback(async () => {
    setLocalComputeTargets(await listLocalComputeTargets());
  }, []);

  const updateLocalComputeTarget = useCallback(
    async (targetId: string) => {
      if (!isTauri() || blocked || computeTargetPending) return;

      setComputeTargetPending(true);
      try {
        setLocalComputeTargets(await setLocalComputeTarget(targetId));
        toast.success("Local compute updated");
      } catch (error) {
        toast.error(String(error));
        await loadComputeTargets();
      } finally {
        setComputeTargetPending(false);
      }
    },
    [blocked, computeTargetPending, loadComputeTargets],
  );

  return {
    computeTargetPending,
    loadComputeTargets,
    localComputeTargets,
    updateLocalComputeTarget,
  };
}

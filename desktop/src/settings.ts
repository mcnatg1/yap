import { invoke } from "@tauri-apps/api/core";

import type { LocalComputeTargetView } from "@/lib/app-types";

export async function polishNumGpuLayers(): Promise<number> {
  return invoke<number>("polish_num_gpu");
}

export async function listLocalComputeTargets(): Promise<LocalComputeTargetView[]> {
  return invoke<LocalComputeTargetView[]>("list_local_compute_targets");
}

export async function setLocalComputeTarget(targetId: string): Promise<LocalComputeTargetView[]> {
  return invoke<LocalComputeTargetView[]>("set_local_compute_target", { targetId });
}

import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { FallbackModelView, LocalComputeTargetView } from "@/lib/app-types";

export function fallbackModelStatus(): Promise<FallbackModelView> {
  return invoke<FallbackModelView>("fallback_model_status");
}

export function installFallbackModel(options: { force?: boolean } = {}): Promise<FallbackModelView> {
  return options.force
    ? invoke<FallbackModelView>("fallback_model_install", { force: true })
    : invoke<FallbackModelView>("fallback_model_install");
}

export function cancelFallbackModelInstall(): Promise<FallbackModelView> {
  return invoke<FallbackModelView>("fallback_model_cancel_install");
}

export function verifyFallbackModel(): Promise<FallbackModelView> {
  return invoke<FallbackModelView>("fallback_model_verify");
}

export function removeFallbackModel(): Promise<FallbackModelView> {
  return invoke<FallbackModelView>("fallback_model_remove");
}

export function setFallbackModelEnabled(enabled: boolean): Promise<FallbackModelView> {
  return invoke<FallbackModelView>("fallback_model_set_enabled", { enabled });
}

export function openFallbackModelFolder(): Promise<void> {
  return invoke<void>("fallback_model_open_folder");
}

export async function polishNumGpuLayers(): Promise<number> {
  return invoke<number>("polish_num_gpu");
}

export async function listLocalComputeTargets(): Promise<LocalComputeTargetView[]> {
  return invoke<LocalComputeTargetView[]>("list_local_compute_targets");
}

export async function setLocalComputeTarget(targetId: string): Promise<LocalComputeTargetView[]> {
  return invoke<LocalComputeTargetView[]>("set_local_compute_target", { targetId });
}

export async function listenFallbackModelProgress(
  onProgress: (view: FallbackModelView) => void,
): Promise<UnlistenFn> {
  if (!isTauri()) return () => undefined;
  return listen<FallbackModelView>("fallback-model-progress", (event) => onProgress(event.payload));
}

export async function listenFallbackModelStatus(
  onStatus: (view: FallbackModelView) => void,
): Promise<UnlistenFn> {
  if (!isTauri()) return () => undefined;
  return listen<FallbackModelView>("fallback-model-status", (event) => onStatus(event.payload));
}

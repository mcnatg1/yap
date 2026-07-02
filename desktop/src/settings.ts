import { invoke } from "@tauri-apps/api/core";

export async function polishNumGpuLayers(): Promise<number> {
  return invoke<number>("polish_num_gpu");
}

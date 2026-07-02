import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type GpuSetting = "cpu" | "auto";

export type AppSettings = {
  useGpu: GpuSetting;
};

export type EngineBootstrapProgressEvent = {
  message: string;
};

export type EngineBootstrapErrorEvent = {
  message: string;
};

export async function getAppSettings(): Promise<AppSettings> {
  return invoke<AppSettings>("get_app_settings");
}

export async function saveAppSettings(settings: AppSettings): Promise<void> {
  await invoke("save_app_settings", { settings });
}

export async function installEngine(): Promise<string> {
  return invoke<string>("install_engine");
}

export async function listenEngineBootstrap(listeners: {
  onProgress: (event: EngineBootstrapProgressEvent) => void;
  onComplete: () => void;
  onError: (event: EngineBootstrapErrorEvent) => void;
}): Promise<UnlistenFn> {
  const unsubs = await Promise.all([
    listen<EngineBootstrapProgressEvent>("engine-bootstrap-progress", (event) => {
      listeners.onProgress(event.payload);
    }),
    listen<EngineBootstrapErrorEvent>("engine-bootstrap-error", (event) => {
      listeners.onError(event.payload);
    }),
    listen("engine-bootstrap-complete", () => {
      listeners.onComplete();
    }),
  ]);

  return () => {
    for (const unsub of unsubs) {
      unsub();
    }
  };
}

export async function polishNumGpuLayers(): Promise<number> {
  return invoke<number>("polish_num_gpu");
}

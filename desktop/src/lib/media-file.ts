export const acceptedFormats = "WAV only for now (mono PCM16, 16 kHz)";

export const audioExtensions = ["mp3", "m4a", "wav", "mp4", "flac", "ogg", "webm"];
export const audioExts = new Set(audioExtensions.map((format) => `.${format}`));

export function basename(path: string) {
  return path.split(/[\\/]/).pop() ?? path;
}

export function extension(path: string) {
  const name = basename(path);
  const dot = name.lastIndexOf(".");
  return dot === -1 ? "" : name.slice(dot).toLowerCase();
}

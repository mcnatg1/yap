export type PreviewTextEntry = {
  outputPath: string;
};

export type PreviewTextCache = Record<string, string>;

export function createPreviewTextLoader() {
  const inFlight = new Map<string, Promise<string>>();

  return {
    load<Entry extends PreviewTextEntry>(
      entry: Entry,
      cache: PreviewTextCache,
      readText: ((entry: Entry) => Promise<string>) | undefined,
      onLoaded: (outputPath: string, text: string) => void,
    ) {
      const cached = cache[entry.outputPath];
      if (cached !== undefined) return Promise.resolve(cached);
      if (!readText) return Promise.resolve("");

      const active = inFlight.get(entry.outputPath);
      if (active) return active;

      const pending = readText(entry)
        .then((text) => {
          onLoaded(entry.outputPath, text);
          return text;
        })
        .finally(() => {
          inFlight.delete(entry.outputPath);
        });
      inFlight.set(entry.outputPath, pending);
      return pending;
    },
  };
}

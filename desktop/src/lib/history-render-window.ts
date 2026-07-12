export const historyRenderWindowSize = 80;

export function renderHistoryWindow<T>(entries: T[], limit = historyRenderWindowSize) {
  const normalizedLimit = Math.max(0, Math.floor(limit));
  const visibleEntries = entries.slice(0, normalizedLimit);

  return {
    hiddenCount: Math.max(0, entries.length - visibleEntries.length),
    nextLimit: Math.min(entries.length, normalizedLimit + historyRenderWindowSize),
    visibleEntries,
  };
}

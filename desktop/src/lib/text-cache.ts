const defaultMaxEntries = 8;
const defaultMaxChars = 200_000;
const defaultMaxTotalChars = 400_000;
const oversizedTranscriptMessage =
  "Transcript is too large to preview in the app. Open it from disk instead.";

export function boundTextForCache(text: string, maxChars = defaultMaxChars) {
  const limit = Math.max(0, maxChars);
  return text.length <= limit
    ? text
    : oversizedTranscriptMessage.slice(0, limit);
}

export function pruneTextCache(
  cache: Record<string, string>,
  retainedPaths: ReadonlySet<string>,
) {
  const entries = Object.entries(cache);
  const retainedEntries = entries.filter(([path]) => retainedPaths.has(path));
  return retainedEntries.length === entries.length
    ? cache
    : Object.fromEntries(retainedEntries);
}

export function rememberText(
  cache: Record<string, string>,
  path: string,
  text: string,
  maxEntries = defaultMaxEntries,
  maxChars = defaultMaxChars,
  maxTotalChars = defaultMaxTotalChars,
) {
  return rememberTexts(cache, [[path, text]], maxEntries, maxChars, maxTotalChars);
}

export function rememberTexts(
  cache: Record<string, string>,
  texts: readonly (readonly [path: string, text: string])[],
  maxEntries = defaultMaxEntries,
  maxChars = defaultMaxChars,
  maxTotalChars = defaultMaxTotalChars,
) {
  const entries = new Map(
    Object.entries(cache).map(([path, text]) => [path, boundTextForCache(text, maxChars)]),
  );
  for (const [path, text] of texts) {
    entries.delete(path);
    entries.set(path, boundTextForCache(text, maxChars));
  }

  let totalChars = [...entries.values()].reduce((total, text) => total + text.length, 0);
  while (entries.size > maxEntries || totalChars > maxTotalChars) {
    const oldest = entries.entries().next().value as [string, string] | undefined;
    if (!oldest) break;
    entries.delete(oldest[0]);
    totalChars -= oldest[1].length;
  }
  return Object.fromEntries(entries);
}

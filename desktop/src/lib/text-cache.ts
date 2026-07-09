const defaultMaxEntries = 8;
const defaultMaxChars = 200_000;
const oversizedTranscriptMessage =
  "Transcript is too large to preview in the app. Open it from disk instead.";

export function rememberText(
  cache: Record<string, string>,
  path: string,
  text: string,
  maxEntries = defaultMaxEntries,
  maxChars = defaultMaxChars,
) {
  const entries = Object.entries(cache).filter(([key]) => key !== path);
  entries.push([path, text.length <= maxChars ? text : oversizedTranscriptMessage]);
  return Object.fromEntries(entries.slice(-maxEntries));
}

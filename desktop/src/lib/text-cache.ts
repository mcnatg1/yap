const defaultMaxEntries = 8;
const defaultMaxChars = 200_000;

export function rememberText(
  cache: Record<string, string>,
  path: string,
  text: string,
  maxEntries = defaultMaxEntries,
  maxChars = defaultMaxChars,
) {
  const entries = Object.entries(cache).filter(([key]) => key !== path);
  if (text.length <= maxChars) entries.push([path, text]);
  return Object.fromEntries(entries.slice(-maxEntries));
}

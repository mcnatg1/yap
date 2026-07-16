import path from "node:path";


function stripExtendedWindowsPrefix(candidate) {
  if (/^\\\\\?\\UNC\\/i.test(candidate)) return `\\\\${candidate.slice(8)}`;
  if (/^\\\\\?\\/i.test(candidate)) return candidate.slice(4);
  return candidate;
}

function normalizeWindowsPath(candidate) {
  return path.win32.normalize(stripExtendedWindowsPrefix(candidate));
}

export function sameWindowsPath(left, right) {
  return normalizeWindowsPath(left).toLocaleLowerCase("en-US")
    === normalizeWindowsPath(right).toLocaleLowerCase("en-US");
}

export function requireAbsoluteWindowsPath(candidate, label) {
  if (typeof candidate !== "string" || !path.win32.isAbsolute(candidate)) {
    throw new Error(`${label} must be an absolute Windows path.`);
  }
  return normalizeWindowsPath(candidate);
}

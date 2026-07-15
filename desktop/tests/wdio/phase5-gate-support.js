import path from "node:path";

const defaultPhase5GateTimeoutMs = 2_700_000;

function stripExtendedWindowsPrefix(candidate) {
  if (/^\\\\\?\\UNC\\/i.test(candidate)) return `\\\\${candidate.slice(8)}`;
  if (/^\\\\\?\\/i.test(candidate)) return candidate.slice(4);
  return candidate;
}

function normalizeWindowsPath(candidate) {
  const normalized = path.win32.normalize(stripExtendedWindowsPrefix(candidate));
  const root = path.win32.parse(normalized).root;
  return normalized.length > root.length
    ? normalized.replace(/[\\/]+$/, "")
    : normalized;
}

export function sameWindowsPath(left, right) {
  return normalizeWindowsPath(left).toLocaleLowerCase("en-US")
    === normalizeWindowsPath(right).toLocaleLowerCase("en-US");
}

export function matchCompletedRemoteTranscript(job, catalog) {
  if (
    job?.route !== "serverBatch"
    || !/^job-[0-9a-f]{24}$/.test(job.id)
    || typeof job.sourcePath !== "string"
    || !Array.isArray(catalog?.sessions)
  ) {
    return undefined;
  }
  const sessionId = `s-${job.id.slice("job-".length)}`;
  return catalog.sessions.find(
    (session) => session?.sessionId === sessionId
      && typeof session.sourcePath === "string"
      && sameWindowsPath(session.sourcePath, job.sourcePath),
  );
}

export function resolvePhase5GateTimeout(value) {
  const timeoutMs = Number(value ?? defaultPhase5GateTimeoutMs);
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 60_000 || timeoutMs > 7_200_000) {
    throw new Error("YAP_PHASE5_GATE_TIMEOUT_MS must be between one minute and two hours.");
  }
  return timeoutMs;
}

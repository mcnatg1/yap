import { invoke, isTauri } from "@tauri-apps/api/core";

import type { PlaybackAdmission } from "@/lib/recording-job";
import {
  maxPlaybackRestoreConcurrency,
  type ReleasePlayback,
} from "@/lib/playback-admission-queue";

export const maxWaveformAdmissionBytes = 32 * 1024 * 1024;
const maxWaveformAdmissionBytesExact = BigInt(maxWaveformAdmissionBytes);
const unclaimedAdmissionGraceMs = 5_000;
const runtimePlaybackPathPattern = /^\/media\/[0-9a-f]{64}$/;

type PlaybackAdmissionTracker = ReturnType<typeof createPlaybackAdmissionTracker>;

export function validatePlaybackAdmission(value: unknown): PlaybackAdmission {
  if (!value || typeof value !== "object") throw new Error("Invalid playback admission.");
  const admission = value as Record<string, unknown>;
  if (
    typeof admission.playbackPath !== "string" ||
    !isRuntimePlaybackPath(admission.playbackPath) ||
    typeof admission.byteLength !== "string" ||
    !/^(0|[1-9]\d*)$/.test(admission.byteLength) ||
    typeof admission.waveformEligible !== "boolean"
  ) {
    throw new Error("Invalid playback admission.");
  }

  let exactLength: bigint;
  try {
    exactLength = BigInt(admission.byteLength);
  } catch {
    throw new Error("Invalid playback admission.");
  }
  const waveformEligible = admission.waveformEligible &&
    exactLength <= maxWaveformAdmissionBytesExact;

  // RecordingJobView predates native admission metadata. Preserve its safe
  // numeric slot as an eligibility classification, never as a converted u64.
  return {
    byteLength: waveformEligible ? 0 : maxWaveformAdmissionBytes + 1,
    playbackPath: admission.playbackPath,
  };
}

export function createPlaybackAdmissionTracker(
  revoke: (playbackPath: string) => void | Promise<unknown>,
  graceMs = unclaimedAdmissionGraceMs,
) {
  const entries = new Map<string, {
    claimed: boolean;
    provisional: boolean;
    revoking: boolean;
    timer?: ReturnType<typeof setTimeout>;
  }>();

  function forget(playbackPath: string) {
    const entry = entries.get(playbackPath);
    if (!entry) return;
    if (entry.timer !== undefined) clearTimeout(entry.timer);
    entries.delete(playbackPath);
  }

  function revokeTracked(playbackPath: string) {
    const entry = entries.get(playbackPath);
    if (!entry || entry.revoking) return;
    if (entry.timer !== undefined) {
      clearTimeout(entry.timer);
      entry.timer = undefined;
    }
    entry.revoking = true;
    let revoked: void | Promise<unknown>;
    try {
      revoked = revoke(playbackPath);
    } catch {
      revoked = Promise.reject(new Error("Playback revocation failed."));
    }
    void Promise.resolve(revoked).then(
      () => {
        if (entries.get(playbackPath) === entry) forget(playbackPath);
      },
      () => {
        if (entries.get(playbackPath) !== entry) return;
        entry.revoking = false;
        if (!entry.claimed) {
          entry.timer = setTimeout(() => revokeTracked(playbackPath), graceMs);
        }
      },
    );
  }

  function claim(playbackPath: string) {
    if (!isRuntimePlaybackPath(playbackPath)) return;
    const entry = entries.get(playbackPath);
    if (entry) {
      entry.claimed = true;
      entry.provisional = false;
      if (entry.timer !== undefined) {
        clearTimeout(entry.timer);
        entry.timer = undefined;
      }
      return;
    }
    entries.set(playbackPath, {
      claimed: true,
      provisional: false,
      revoking: false,
    });
  }

  function hold(playbackPath: string, expiresAt: number) {
    if (!isRuntimePlaybackPath(playbackPath)) return;
    let entry = entries.get(playbackPath);
    if (!entry) {
      entry = { claimed: true, provisional: true, revoking: false };
      entries.set(playbackPath, entry);
    } else if (entry.claimed && !entry.provisional) {
      return;
    } else {
      entry.claimed = true;
      entry.provisional = true;
      if (entry.timer !== undefined) clearTimeout(entry.timer);
    }
    const heldEntry = entry;
    heldEntry.timer = setTimeout(() => {
      if (entries.get(playbackPath) !== heldEntry || !heldEntry.provisional) return;
      heldEntry.timer = undefined;
      heldEntry.claimed = false;
      heldEntry.provisional = false;
      revokeTracked(playbackPath);
    }, Math.max(0, expiresAt - Date.now()));
  }

  return {
    claim,
    dispose() {
      for (const playbackPath of [...entries.keys()]) forget(playbackPath);
    },
    forget,
    reconcile(activePlaybackPaths: Iterable<string>) {
      const active = new Set(
        [...activePlaybackPaths].filter(isRuntimePlaybackPath),
      );
      for (const playbackPath of active) {
        claim(playbackPath);
      }
      for (const [playbackPath, entry] of [...entries]) {
        if (entry.claimed && !entry.provisional && !active.has(playbackPath)) {
          entry.claimed = false;
          revokeTracked(playbackPath);
        }
      }
    },
    hold,
    track(playbackPath: string) {
      if (!isRuntimePlaybackPath(playbackPath) || entries.has(playbackPath)) return;
      const entry: {
        claimed: boolean;
        provisional: boolean;
        revoking: boolean;
        timer?: ReturnType<typeof setTimeout>;
      } = {
        claimed: false,
        provisional: false,
        revoking: false,
      };
      entry.timer = setTimeout(() => {
        if (entries.get(playbackPath) === entry && !entry.claimed) {
          revokeTracked(playbackPath);
        }
      }, graceMs);
      entries.set(playbackPath, entry);
    },
  };
}

const runtimeAdmissionTracker: PlaybackAdmissionTracker = createPlaybackAdmissionTracker(
  (playbackPath) => invokeRelease(playbackPath),
);

function isRuntimePlaybackPath(playbackPath: string) {
  try {
    const url = new URL(playbackPath);
    return (
      url.protocol === "http:" &&
      url.hostname === "127.0.0.1" &&
      Boolean(url.port) &&
      !url.username &&
      !url.password &&
      !url.search &&
      !url.hash &&
      runtimePlaybackPathPattern.test(url.pathname)
    );
  } catch {
    return false;
  }
}

async function invokeRelease(playbackPath: string) {
  if (!isTauri() || !isRuntimePlaybackPath(playbackPath)) return;
  await invoke("release_recording_playback", { playbackPath });
}

export async function restoreRecordingPlaybackPath(path: string) {
  const admission = validatePlaybackAdmission(
    await invoke<unknown>("restore_recording_playback_path", { path }),
  );
  runtimeAdmissionTracker.track(admission.playbackPath);
  return admission;
}

export async function releaseRecordingPlaybackPath(playbackPath: string) {
  await invokeRelease(playbackPath);
  runtimeAdmissionTracker.forget(playbackPath);
}

export function reconcilePlaybackAdmissionLifecycle(activePlaybackPaths: Iterable<string>) {
  runtimeAdmissionTracker.reconcile(activePlaybackPaths);
}

export function holdPlaybackAdmissionUntil(playbackPath: string, deadlineAt: number) {
  runtimeAdmissionTracker.hold(playbackPath, deadlineAt + unclaimedAdmissionGraceMs);
}

export function hasNativePlaybackRuntime() {
  return isTauri();
}

export async function releaseRecordingPlaybackPaths(
  playbackPaths: Iterable<string>,
  release: ReleasePlayback = releaseRecordingPlaybackPath,
) {
  const paths = [...new Set(playbackPaths)];
  let cursor = 0;

  async function worker() {
    while (true) {
      const index = cursor;
      cursor += 1;
      if (index >= paths.length) return;
      await release(paths[index]).catch(() => undefined);
    }
  }

  await Promise.all(
    Array.from(
      { length: Math.min(maxPlaybackRestoreConcurrency, paths.length) },
      () => worker(),
    ),
  );
}

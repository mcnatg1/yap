import type { PlaybackAdmission } from "@/lib/recording-job";

export const maxPlaybackRestoreConcurrency = 4;
export const playbackAdmissionDeadlineMs = 10_000;

export type RestorePlayback = (path: string) => Promise<PlaybackAdmission>;
export type ReleasePlayback = (playbackPath: string) => Promise<unknown>;

type NativeAdmissionTask = {
  admit: () => Promise<PlaybackAdmission>;
  deadlineAt: number;
  releaseLate: ReleasePlayback;
  resolve: (admission: PlaybackAdmission | undefined) => void;
  settled: boolean;
  started: boolean;
  timer?: ReturnType<typeof setTimeout>;
};

const pendingNativeAdmissions: NativeAdmissionTask[] = [];
let activeNativeAdmissions = 0;

function settleNativeAdmission(
  task: NativeAdmissionTask,
  admission: PlaybackAdmission | undefined,
) {
  if (task.settled) return;
  task.settled = true;
  if (task.timer !== undefined) clearTimeout(task.timer);
  task.resolve(admission);
}

function finishNativeAdmission() {
  activeNativeAdmissions -= 1;
  pumpNativeAdmissions();
}

function pumpNativeAdmissions() {
  while (
    activeNativeAdmissions < maxPlaybackRestoreConcurrency
    && pendingNativeAdmissions.length
  ) {
    const task = pendingNativeAdmissions.shift()!;
    if (task.settled || task.deadlineAt <= Date.now()) {
      settleNativeAdmission(task, undefined);
      continue;
    }

    task.started = true;
    activeNativeAdmissions += 1;
    let pending: Promise<PlaybackAdmission>;
    try {
      pending = task.admit();
    } catch {
      finishNativeAdmission();
      settleNativeAdmission(task, undefined);
      continue;
    }

    void Promise.resolve(pending).then(
      (admission) => {
        const late = task.settled;
        if (late) {
          finishNativeAdmission();
          void Promise.resolve()
            .then(() => task.releaseLate(admission.playbackPath))
            .catch(() => undefined);
          return;
        }
        settleNativeAdmission(task, admission);
        finishNativeAdmission();
      },
      () => {
        finishNativeAdmission();
        settleNativeAdmission(task, undefined);
      },
    );
  }
}

export function settlePlaybackAdmissionBeforeDeadline(
  admit: () => Promise<PlaybackAdmission>,
  releaseLate: ReleasePlayback,
  deadlineAt = Date.now() + playbackAdmissionDeadlineMs,
) {
  return new Promise<PlaybackAdmission | undefined>((resolve) => {
    if (deadlineAt <= Date.now()) {
      resolve(undefined);
      return;
    }

    const task: NativeAdmissionTask = {
      admit,
      deadlineAt,
      releaseLate,
      resolve,
      settled: false,
      started: false,
    };
    task.timer = setTimeout(() => {
      settleNativeAdmission(task, undefined);
      if (task.started) return;
      const index = pendingNativeAdmissions.indexOf(task);
      if (index >= 0) pendingNativeAdmissions.splice(index, 1);
      pumpNativeAdmissions();
    }, Math.max(0, deadlineAt - Date.now()));
    pendingNativeAdmissions.push(task);
    pumpNativeAdmissions();
  });
}

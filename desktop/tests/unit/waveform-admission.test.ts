import { describe, expect, it, vi } from "vitest";

import {
  maxDecodedWaveformBytes,
  maxDecodedWaveformDurationSeconds,
  mountDecodedWaveform,
} from "@/components/panels/transcript-panel";

function waveformFactory() {
  const destroy = vi.fn();
  const create = vi.fn(() => ({ destroy }));
  return { create, destroy };
}

describe("decoded waveform admission", () => {
  it("never constructs WaveSurfer for media above the byte budget", () => {
    const factory = waveformFactory();

    const mounted = mountDecodedWaveform({
      byteLength: maxDecodedWaveformBytes + 1,
      create: factory.create,
      durationSeconds: 30,
      subscribe: () => [],
    });

    expect(mounted).toBeUndefined();
    expect(factory.create).not.toHaveBeenCalled();
  });

  it("never constructs WaveSurfer for media above the duration budget", () => {
    const factory = waveformFactory();

    const mounted = mountDecodedWaveform({
      byteLength: 4_096,
      create: factory.create,
      durationSeconds: maxDecodedWaveformDurationSeconds + 1,
      subscribe: () => [],
    });

    expect(mounted).toBeUndefined();
    expect(factory.create).not.toHaveBeenCalled();
  });

  it("constructs WaveSurfer for bounded media after native metadata arrives", () => {
    const factory = waveformFactory();

    const mounted = mountDecodedWaveform({
      byteLength: 4_096,
      create: factory.create,
      durationSeconds: 30,
      subscribe: () => [],
    });

    expect(factory.create).toHaveBeenCalledOnce();
    expect(mounted).toBeDefined();
  });

  it("unsubscribes and destroys the decoded waveform on cleanup", () => {
    const factory = waveformFactory();
    const firstUnsubscribe = vi.fn();
    const secondUnsubscribe = vi.fn();
    const mounted = mountDecodedWaveform({
      byteLength: 4_096,
      create: factory.create,
      durationSeconds: 30,
      subscribe: () => [firstUnsubscribe, secondUnsubscribe],
    });

    mounted?.dispose();
    mounted?.dispose();

    expect(firstUnsubscribe).toHaveBeenCalledOnce();
    expect(secondUnsubscribe).toHaveBeenCalledOnce();
    expect(factory.destroy).toHaveBeenCalledOnce();
  });
});

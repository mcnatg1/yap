import { describe, expect, it, vi } from "vitest";

import {
  decodedWaveformSampleRate,
  maxDecodedWaveformBytes,
  mountDecodedWaveform,
  maxDecodedWaveformChannels,
  maxWaveformSourceBytes,
  roundedMediaSecond,
  seekRatioFromBounds,
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
      byteLength: maxWaveformSourceBytes + 1,
      create: factory.create,
      durationSeconds: 30,
      requested: true,
      subscribe: () => [],
    });

    expect(mounted).toBeUndefined();
    expect(factory.create).not.toHaveBeenCalled();
  });

  it("never constructs WaveSurfer when a compressed recording expands past the PCM budget", () => {
    const factory = waveformFactory();
    const decodedBytesPerSecond = decodedWaveformSampleRate *
      maxDecodedWaveformChannels * Float32Array.BYTES_PER_ELEMENT;

    const mounted = mountDecodedWaveform({
      byteLength: 4_096,
      create: factory.create,
      durationSeconds: (maxDecodedWaveformBytes + 1) / decodedBytesPerSecond,
      requested: true,
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
      requested: true,
      subscribe: () => [],
    });

    expect(factory.create).toHaveBeenCalledOnce();
    expect(mounted).toBeDefined();
  });

  it("does not construct WaveSurfer until playback is requested", () => {
    const factory = waveformFactory();

    const mounted = mountDecodedWaveform({
      byteLength: 4_096,
      create: factory.create,
      durationSeconds: 30,
      requested: false,
      subscribe: () => [],
    });

    expect(mounted).toBeUndefined();
    expect(factory.create).not.toHaveBeenCalled();
  });

  it("does not construct WaveSurfer before media metadata arrives", () => {
    const factory = waveformFactory();

    const mounted = mountDecodedWaveform({
      byteLength: 4_096,
      create: factory.create,
      durationSeconds: undefined,
      requested: true,
      subscribe: () => [],
    });

    expect(mounted).toBeUndefined();
    expect(factory.create).not.toHaveBeenCalled();
  });

  it("unsubscribes and destroys the decoded waveform on cleanup", () => {
    const factory = waveformFactory();
    const firstUnsubscribe = vi.fn();
    const secondUnsubscribe = vi.fn();
    const mounted = mountDecodedWaveform({
      byteLength: 4_096,
      create: factory.create,
      durationSeconds: 30,
      requested: true,
      subscribe: () => [firstUnsubscribe, secondUnsubscribe],
    });

    mounted?.dispose();
    mounted?.dispose();

    expect(firstUnsubscribe).toHaveBeenCalledOnce();
    expect(secondUnsubscribe).toHaveBeenCalledOnce();
    expect(factory.destroy).toHaveBeenCalledOnce();
  });

  it("lets an emitted waveform error dispose the mounted owner exactly once", () => {
    const factory = waveformFactory();
    const unsubscribe = vi.fn();
    let disposeFromError: (() => void) | undefined;
    const mounted = mountDecodedWaveform({
      byteLength: 4_096,
      create: factory.create,
      durationSeconds: 30,
      requested: true,
      subscribe: (_waveform, lifecycle) => {
        disposeFromError = lifecycle.dispose;
        return [unsubscribe];
      },
    });

    expect(disposeFromError).toBeTypeOf("function");
    disposeFromError?.();
    mounted?.dispose();

    expect(unsubscribe).toHaveBeenCalledOnce();
    expect(factory.destroy).toHaveBeenCalledOnce();
  });

  it("disposes decoding after a bounded ready timeout", async () => {
    vi.useFakeTimers();
    const factory = waveformFactory();
    const onReadyTimeout = vi.fn();
    const mounted = mountDecodedWaveform({
      byteLength: 4_096,
      create: factory.create,
      durationSeconds: 30,
      onReadyTimeout,
      readyTimeoutMs: 1_000,
      requested: true,
      subscribe: () => [],
    });

    await vi.advanceTimersByTimeAsync(1_000);

    expect(onReadyTimeout).toHaveBeenCalledOnce();
    expect(factory.destroy).toHaveBeenCalledOnce();
    mounted?.dispose();
    vi.useRealTimers();
  });

  it("keeps a ready waveform mounted past the decoding timeout", async () => {
    vi.useFakeTimers();
    const factory = waveformFactory();
    let markReady: (() => void) | undefined;
    const mounted = mountDecodedWaveform({
      byteLength: 4_096,
      create: factory.create,
      durationSeconds: 30,
      readyTimeoutMs: 1_000,
      requested: true,
      subscribe: (_waveform, lifecycle) => {
        markReady = lifecycle.markReady;
        return [];
      },
    });

    markReady?.();
    await vi.advanceTimersByTimeAsync(1_000);

    expect(factory.destroy).not.toHaveBeenCalled();
    mounted?.dispose();
    vi.useRealTimers();
  });

  it("maps pointer endpoints to the visible track instead of its outer container", () => {
    const bounds = { left: 112, width: 176 };

    expect(seekRatioFromBounds(100, bounds)).toBe(0);
    expect(seekRatioFromBounds(200, bounds)).toBe(0.5);
    expect(seekRatioFromBounds(300, bounds)).toBe(1);
  });

  it("uses the same floor rounding for ARIA current and endpoint values", () => {
    expect(roundedMediaSecond(100.9)).toBe(100);
    expect(roundedMediaSecond(0)).toBe(0);
    expect(roundedMediaSecond(undefined)).toBe(0);
  });
});

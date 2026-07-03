import { describe, expect, it } from "vitest";

import { waveformPeaks } from "@/components/panels/transcript-panel";

function audioBuffer(samples: number[] | number[][]) {
  const channels = (Array.isArray(samples[0]) ? (samples as number[][]) : [samples as number[]]).map((channel) =>
    Float32Array.from(channel),
  );

  return {
    getChannelData: (index: number) => channels[index] ?? channels[0],
    length: channels[0]?.length ?? 0,
    numberOfChannels: channels.length,
  } as unknown as AudioBuffer;
}

describe("waveformPeaks", () => {
  it("makes louder windows taller", () => {
    const peaks = waveformPeaks(audioBuffer([0, 0, 0, 0, 1, 1, 1, 1]), 2);

    expect(peaks[0]).toBeLessThan(peaks[1]);
  });

  it("keeps silent audio visible", () => {
    expect(waveformPeaks(audioBuffer([0, 0, 0, 0]), 3)).toEqual([16, 16, 16]);
  });

  it("returns the requested count within the render range", () => {
    const peaks = waveformPeaks(audioBuffer([0, 0.25, 0.5, 1]), 4);

    expect(peaks).toHaveLength(4);
    expect(peaks.every((peak) => peak >= 16 && peak <= 100)).toBe(true);
  });

  it("averages stereo channels", () => {
    const peaks = waveformPeaks(audioBuffer([[0, 0, 1, 1], [0, 0, 0, 0]]), 2);

    expect(peaks[0]).toBeLessThan(peaks[1]);
  });
});

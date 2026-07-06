export type TestToneOptions = {
  durationMs?: number;
  frequencyHz?: number;
  sampleRate?: number;
  amplitude?: number;
};

const DEFAULT_OPTIONS = {
  amplitude: 0.25,
  durationMs: 250,
  frequencyHz: 440,
  sampleRate: 16_000,
} satisfies Required<TestToneOptions>;

export function makeTestToneWav(options: TestToneOptions = {}) {
  const resolved = { ...DEFAULT_OPTIONS, ...options };
  const sampleCount = Math.max(1, Math.floor((resolved.sampleRate * resolved.durationMs) / 1_000));
  const bytesPerSample = 2;
  const dataSize = sampleCount * bytesPerSample;
  const buffer = new ArrayBuffer(44 + dataSize);
  const view = new DataView(buffer);

  writeAscii(view, 0, "RIFF");
  view.setUint32(4, 36 + dataSize, true);
  writeAscii(view, 8, "WAVE");
  writeAscii(view, 12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, resolved.sampleRate, true);
  view.setUint32(28, resolved.sampleRate * bytesPerSample, true);
  view.setUint16(32, bytesPerSample, true);
  view.setUint16(34, 16, true);
  writeAscii(view, 36, "data");
  view.setUint32(40, dataSize, true);

  for (let index = 0; index < sampleCount; index += 1) {
    const seconds = index / resolved.sampleRate;
    const envelope = Math.min(index / 120, (sampleCount - index) / 120, 1);
    const value =
      Math.sin(2 * Math.PI * resolved.frequencyHz * seconds) *
      resolved.amplitude *
      Math.max(0, envelope);
    view.setInt16(44 + index * bytesPerSample, Math.round(value * 0x7fff), true);
  }

  return new Uint8Array(buffer);
}

function writeAscii(view: DataView, offset: number, text: string) {
  for (let index = 0; index < text.length; index += 1) {
    view.setUint8(offset + index, text.charCodeAt(index));
  }
}

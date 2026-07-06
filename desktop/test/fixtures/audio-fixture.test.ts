import { describe, expect, it } from "vitest";

import { makeTestToneWav } from "./audio-fixture";

describe("generated audio fixtures", () => {
  it("creates a deterministic mono 16-bit wav tone", () => {
    const wav = makeTestToneWav();
    const view = new DataView(wav.buffer);

    expect(new TextDecoder("ascii").decode(wav.slice(0, 4))).toBe("RIFF");
    expect(new TextDecoder("ascii").decode(wav.slice(8, 12))).toBe("WAVE");
    expect(new TextDecoder("ascii").decode(wav.slice(12, 16))).toBe("fmt ");
    expect(new TextDecoder("ascii").decode(wav.slice(36, 40))).toBe("data");
    expect(view.getUint16(20, true)).toBe(1);
    expect(view.getUint16(22, true)).toBe(1);
    expect(view.getUint32(24, true)).toBe(16_000);
    expect(view.getUint16(34, true)).toBe(16);
  });

  it("is stable for identical options", () => {
    const first = makeTestToneWav({ durationMs: 120, frequencyHz: 330 });
    const second = makeTestToneWav({ durationMs: 120, frequencyHz: 330 });

    expect(first).toEqual(second);
  });
});

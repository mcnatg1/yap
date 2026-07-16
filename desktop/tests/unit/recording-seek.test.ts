import { describe, expect, it } from "vitest";

import {
  roundedMediaSecond,
  seekRatioFromBounds,
} from "@/components/playback/recording-seek";

describe("recording seek controls", () => {
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

import { describe, expect, it, vi } from "vitest";

import { readReducedMotionPreference } from "../../src/components/live/use-prefers-reduced-motion";

describe("reduced motion preference", () => {
  it("reads the browser preference synchronously for the first render", () => {
    const matchMedia = vi.fn(() => ({ matches: true }));

    expect(readReducedMotionPreference(matchMedia)).toBe(true);
    expect(matchMedia).toHaveBeenCalledWith("(prefers-reduced-motion: reduce)");
  });

  it("defaults safely when no browser media query is available", () => {
    expect(readReducedMotionPreference()).toBe(false);
  });
});

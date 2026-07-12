import { describe, expect, it } from "vitest";

import { projectTranscriptText } from "@/lib/transcript-text";

describe("transcript text projection", () => {
  it("keeps loading, empty, and ready transcript text distinct", () => {
    expect(projectTranscriptText(undefined)).toEqual({
      state: "loading",
      text: "Loading transcript...",
    });
    expect(projectTranscriptText("")).toEqual({
      state: "empty",
      text: "Empty transcript.",
    });
    expect(projectTranscriptText("hello")).toEqual({
      state: "ready",
      text: "hello",
    });
  });
});

import { describe, expect, it } from "vitest";

import { historyEntryToRecordingJob } from "@/lib/history-utils";

describe("history job projection", () => {
  it("projects partial live saves with their warning", () => {
    const job = historyEntryToRecordingJob({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-123",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt",
      warning: "Live audio could not be saved. Transcript was saved.",
    });

    expect(job.status).toBe("partial");
    expect(job.error).toBe("Live audio could not be saved. Transcript was saved.");
    expect(job.pipeline.postprocessing).toBe("error");
  });
});

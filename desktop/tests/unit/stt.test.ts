import { describe, expect, it } from "vitest";

import { isSttErrorCode, SttInvokeError, sttErrorMessage } from "@/stt";

describe("stt error mapping", () => {
  it("maps every known code to a non-empty message", () => {
    const codes = [
      "MODEL_MISSING",
      "MODEL_CORRUPT",
      "MODEL_INSTALL_CANCELLED",
      "BAD_LANG",
      "OOM",
      "AUDIO_DECODE",
      "SIDECAR_CRASH",
      "SIDECAR_UNREACHABLE",
      "SERVER_UNAVAILABLE",
      "FALLBACK_DISABLED",
      "BUSY",
      "TIMEOUT",
    ] as const;
    for (const code of codes) {
      expect(sttErrorMessage(code).length).toBeGreaterThan(0);
    }
    expect(sttErrorMessage("SIDECAR_UNREACHABLE")).toBe("Transcription engine didn't start.");
    expect(sttErrorMessage("MODEL_INSTALL_CANCELLED")).toBe("Local fallback install was cancelled.");
  });

  it("recognizes known codes and rejects unknown ones", () => {
    expect(isSttErrorCode("BUSY")).toBe(true);
    expect(isSttErrorCode("MODEL_INSTALL_CANCELLED")).toBe(true);
    expect(isSttErrorCode("NOPE")).toBe(false);
  });

  it("uses the mapped message for known codes and the detail otherwise", () => {
    expect(new SttInvokeError("MODEL_CORRUPT", "raw").message).toBe("Model file failed verification.");
    expect(new SttInvokeError("PYTHON_WEIRD", "raw detail").message).toBe("raw detail");
  });
});

import { describe, expect, it, vi } from "vitest";

import { fireAndReport } from "@/lib/fire-and-report";

describe("fireAndReport", () => {
  it("runs a successful user action without reporting an error", async () => {
    const action = vi.fn(async () => "done");
    const report = vi.fn();

    fireAndReport(action, report);
    await Promise.resolve();

    expect(action).toHaveBeenCalledOnce();
    expect(report).not.toHaveBeenCalled();
  });

  it("normalizes and reports a synchronous throw exactly once", () => {
    const report = vi.fn();

    expect(() => {
      fireAndReport(() => {
        throw { message: "Native drop failed" };
      }, report);
    }).not.toThrow();

    expect(report).toHaveBeenCalledOnce();
    expect(report.mock.calls[0]?.[0]).toBeInstanceOf(Error);
    expect(report.mock.calls[0]?.[0].message).toBe("Native drop failed");
  });

  it("absorbs an asynchronous rejection and reports it exactly once", async () => {
    const report = vi.fn();

    fireAndReport(() => Promise.reject("Import command rejected"), report);

    await vi.waitFor(() => expect(report).toHaveBeenCalledOnce());
    expect(report.mock.calls[0]?.[0]).toBeInstanceOf(Error);
    expect(report.mock.calls[0]?.[0].message).toBe("Import command rejected");
  });
});

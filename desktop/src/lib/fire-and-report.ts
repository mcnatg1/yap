export type UserActionErrorReporter = (error: Error) => void;

function asError(error: unknown) {
  if (error instanceof Error) return error;
  if (
    typeof error === "object"
    && error !== null
    && "message" in error
    && typeof error.message === "string"
    && error.message.trim()
  ) {
    return new Error(error.message);
  }
  if (typeof error === "string" && error.trim()) return new Error(error);
  return new Error("Unknown error");
}

export function fireAndReport(
  action: () => unknown,
  report: UserActionErrorReporter,
): void {
  let reported = false;
  const reportOnce = (error: unknown) => {
    if (reported) return;
    reported = true;
    try {
      report(asError(error));
    } catch {
      // A best-effort reporter must not turn a handled action failure into a rejection.
    }
  };

  try {
    void Promise.resolve(action()).then(undefined, reportOnce);
  } catch (error) {
    reportOnce(error);
  }
}

const forbiddenRequiredTestSyntax = /\b(?:describe|context|it|specify|test)\s*\.\s*(?:only|skip)\s*\(|\b(?:fdescribe|fit|xdescribe|xit)\s*\(|\bthis\s*\.\s*skip\s*\(/;

export function assertRequiredSpecPolicy(source, label) {
  if (forbiddenRequiredTestSyntax.test(source)) {
    throw new Error(`Required WDIO spec contains a focused or skipped test: ${label}`);
  }
}

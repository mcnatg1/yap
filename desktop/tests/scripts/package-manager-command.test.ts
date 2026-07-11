import { describe, expect, it } from "vitest";

import { resolvePackageManagerCommand } from "./package-manager-command.mjs";

const args = ["tauri", "build", "--debug"];

describe("package manager command resolution", () => {
  it("runs JavaScript package-manager entrypoints through Node", () => {
    expect(resolvePackageManagerCommand({
      args,
      nodeExecPath: "C:\\Program Files\\nodejs\\node.exe",
      npmExecPath: "C:\\pnpm\\pnpm.cjs",
    })).toEqual({
      args: ["C:\\pnpm\\pnpm.cjs", ...args],
      command: "C:\\Program Files\\nodejs\\node.exe",
    });
  });

  it("runs native package-manager executables directly", () => {
    expect(resolvePackageManagerCommand({
      args,
      nodeExecPath: "C:\\Program Files\\nodejs\\node.exe",
      npmExecPath: "C:\\pnpm\\pnpm.exe",
    })).toEqual({
      args,
      command: "C:\\pnpm\\pnpm.exe",
    });
  });

  it("falls back to pnpm when npm_execpath is absent", () => {
    expect(resolvePackageManagerCommand({
      args,
      nodeExecPath: "C:\\Program Files\\nodejs\\node.exe",
    })).toEqual({
      args,
      command: "pnpm",
    });
  });
});

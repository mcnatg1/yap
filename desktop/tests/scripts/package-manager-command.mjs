const javaScriptEntrypoint = /\.(?:c|m)?js$/i;

export function resolvePackageManagerCommand({ args, nodeExecPath, npmExecPath }) {
  if (!npmExecPath) {
    return { args: [...args], command: "pnpm" };
  }

  if (javaScriptEntrypoint.test(npmExecPath)) {
    return { args: [npmExecPath, ...args], command: nodeExecPath };
  }

  return { args: [...args], command: npmExecPath };
}

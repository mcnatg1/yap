const major = Number.parseInt(process.versions.node.split(".")[0] ?? "", 10);

if (major !== 24) {
  console.error(
    `Yap desktop requires Node 24.x LTS. Current runtime is ${process.version}. ` +
      "Switch to the version in ../.node-version before running desktop scripts.",
  );
  process.exit(1);
}

export const reviewedActions = Object.freeze({
  cacheRestore: "actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9",
  cacheSave: "actions/cache/save@55cc8345863c7cc4c66a329aec7e433d2d1c52a9",
  checkout: "actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0",
  downloadArtifact: "actions/download-artifact@37930b1c2abaa49bbe596cd826c3c89aef350131",
  setupNode: "actions/setup-node@48b55a011bda9f5d6aeb4c2d9c7362e8dae4041e",
  setupPnpm: "pnpm/action-setup@0ebf47130e4866e96fce0953f49152a61190b271",
  setupPython: "actions/setup-python@ece7cb06caefa5fff74198d8649806c4678c61a1",
  setupRust: "dtolnay/rust-toolchain@4be7066ada62dd38de10e7b70166bc74ed198c30",
  uploadArtifact: "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
});

export const reviewedActionUses = new Set(Object.values(reviewedActions));

export const workflowPaths = Object.freeze([
  ".github/workflows/ci.yml",
  ".github/workflows/nsis-smoke.yml",
  ".github/workflows/release.yml",
]);

export const exactCacheKeys = Object.freeze({
  cargo: "cargo-deps-v1-${{ runner.os }}-${{ runner.arch }}-${{ hashFiles('desktop/src-tauri/Cargo.lock') }}",
  playwright: "playwright-v1-${{ runner.os }}-${{ runner.arch }}-${{ hashFiles('desktop/pnpm-lock.yaml') }}",
  pnpm: "pnpm-store-v11-${{ runner.os }}-${{ runner.arch }}-${{ hashFiles('desktop/pnpm-lock.yaml') }}",
});

export const expectedCacheFamilies = Object.freeze({
  ".github/workflows/ci.yml": Object.freeze({
    frontend: Object.freeze(["playwright", "pnpm"]),
    "native-wdio": Object.freeze(["cargo", "pnpm"]),
    rust: Object.freeze(["cargo"]),
  }),
  ".github/workflows/nsis-smoke.yml": Object.freeze({
    "nsis-bundle-smoke": Object.freeze(["cargo", "pnpm"]),
  }),
  ".github/workflows/release.yml": Object.freeze({
    "build-nsis": Object.freeze(["cargo", "pnpm"]),
  }),
});

export const pnpmStoreBindingScriptPath = "desktop/tests/scripts/bind-pnpm-cache-store.ps1";

export const reviewedPnpmStoreBindingInvocation = String.raw`
& "$env:GITHUB_WORKSPACE\desktop\tests\scripts\bind-pnpm-cache-store.ps1"
`.trim();

export const reviewedPnpmStoreBindingScript = String.raw`
#requires -Version 7.4
#requires -PSEdition Core

$ErrorActionPreference = "Stop"
$localAppData = [Environment]::GetFolderPath(
  [Environment+SpecialFolder]::LocalApplicationData
)
$expectedStore = [IO.Path]::GetFullPath(
  (Join-Path $localAppData "pnpm\store\v11")
)
$cacheStore = [IO.Path]::GetFullPath(
  (Join-Path $HOME "AppData\Local\pnpm\store\v11")
)
if ($expectedStore -ine $cacheStore) {
  throw "The reviewed pnpm cache path does not match Windows LocalApplicationData."
}
$env:PNPM_CONFIG_STORE_DIR = $expectedStore
$actualStoreOutput = @(pnpm store path)
if ($LASTEXITCODE -ne 0 -or $actualStoreOutput.Count -ne 1) {
  throw "Failed to resolve the configured pnpm dependency store."
}
$actualStore = [IO.Path]::GetFullPath(([string]$actualStoreOutput[0]).Trim())
if ($actualStore -ine $expectedStore) {
  throw "pnpm did not accept the reviewed dependency store."
}
"PNPM_CONFIG_STORE_DIR=$expectedStore" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
`.trim();

export const releaseActionUses = new Set([
  reviewedActions.cacheRestore,
  reviewedActions.checkout,
  reviewedActions.downloadArtifact,
  reviewedActions.setupNode,
  reviewedActions.setupPnpm,
  reviewedActions.setupRust,
  reviewedActions.uploadArtifact,
]);

export const reviewedWindowsGraphBoundaryRun = String.raw`
$ErrorActionPreference = "Stop"
$windowsPackages = @(cargo tree --locked --offline --target x86_64-pc-windows-msvc --prefix none --format "{p}")
if ($LASTEXITCODE -ne 0) {
  throw "Unable to inspect the locked Windows dependency graph."
}
$windowsGlibPackages = @($windowsPackages | Where-Object { $_ -match '^glib v' })
if ($windowsGlibPackages.Count -ne 0) {
  throw "glib became reachable on Windows; reevaluate GHSA-wrw7-89jp-8q8g: $($windowsGlibPackages -join ', ')"
}
`.trim();

export const reviewedCargoAuditRun = String.raw`
$ErrorActionPreference = "Stop"
$archive = Join-Path $env:RUNNER_TEMP "cargo-audit-x86_64-pc-windows-msvc-v0.22.2.zip"
$url = "https://github.com/RustSec/rustsec/releases/download/cargo-audit/v0.22.2/cargo-audit-x86_64-pc-windows-msvc-v0.22.2.zip"
$extractRoot = Join-Path $env:RUNNER_TEMP "cargo-audit-0.22.2"
Invoke-WebRequest -Uri $url -OutFile $archive
$actualSha256 = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha256 -cne "0a7316540862c13d954f648917ceacca593747baed6eec180fafa590be2710ab") {
  throw "Pinned cargo-audit archive hash mismatch."
}
Expand-Archive -LiteralPath $archive -DestinationPath $extractRoot -Force
$cargoAudit = Join-Path $extractRoot "cargo-audit-x86_64-pc-windows-msvc-v0.22.2\cargo-audit.exe"
if (-not (Test-Path -LiteralPath $cargoAudit -PathType Leaf)) {
  throw "Pinned cargo-audit executable was not extracted."
}
$cargoAuditVersion = & $cargoAudit --version
if ($LASTEXITCODE -ne 0 -or $cargoAuditVersion -cne "cargo-audit 0.22.2") {
  throw "Pinned cargo-audit executable has an unexpected version."
}
# Policy: cargo-audit warnings from Tauri's target-all desktop
# transitive crates are allowed for now. Vulnerabilities fail CI.
& $cargoAudit audit --target-os windows --target-arch x86_64
if ($LASTEXITCODE -ne 0) { throw "cargo-audit failed." }
`.trim();

# Third-Party Provenance

This page explains Yap's source-reuse and runtime provenance boundaries. The
machine-readable authority is `THIRD_PARTY_PROVENANCE.json`; complete shipped
license text is in `THIRD_PARTY_NOTICES.md` and the server runtime license tree.

## Direct source adaptation

Yap currently records one directly adapted source identity:

| Source ID | Upstream | Revision | License | Local scope |
| --- | --- | --- | --- | --- |
| `freeflow-zachlatta` | `zachlatta/freeflow` | `7427ca982c19746770f5357ced16e993f2eb27fd` | MIT | Live overlay/presentation/waveform/reduced-motion files and audio preprocessing listed in the machine manifest. |

The manifest records upstream source hashes, the upstream license hash, every
attributed local derivative path, and each local SHA-256. The release contract
can verify the pinned upstream and rejects an unrecorded local change.

`mrinalwadhwa/freeflow` is a separate Apache-2.0 repository used only as a
reviewed behavior donor. It must never be conflated with the MIT
`zachlatta/freeflow` source identity. Meetily is also a reviewed workflow donor;
the 2026-07-12 audit did not authorize or incorporate donor code. See the
[Freeflow/Meetily reuse audit](../research/2026-07-12-freeflow-meetily-reuse-audit.md).

## Dependency and runtime provenance

- Frontend packages are declared in `desktop/package.json` and frozen by
  `desktop/pnpm-lock.yaml`.
- Rust crates are declared in `desktop/src-tauri/Cargo.toml` and frozen by
  `desktop/src-tauri/Cargo.lock`; bundled SQLite notice text is shipped.
- The portable server requires Python 3.12 (`>=3.12,<3.13`).
- The GPU worker base is an immutable digest of NVIDIA PyTorch 26.06. The image
  build asserts Python 3.12, the expected NVIDIA Torch build, and CUDA version.
- The worker's resolver-minimal Python overlay uses exact versions and hashes in
  `server/runtime/asr/requirements.lock`.
- Model, runtime, public byte-distribution, licensed fixture, and evidence
  identities are pinned in `server/model-pools.lock.json`.
- Server runtime notices and full licenses ship from
  `server/runtime/asr/THIRD_PARTY_NOTICES.md` and
  `server/runtime/asr/licenses/`.

## Reuse policy

Before incorporating external source:

1. record the exact repository and immutable revision;
2. verify the license and preserve required notice/license text;
3. identify the smallest source slice and whether behavior reimplementation is
   safer than direct adaptation;
4. record provenance for every resulting local derivative;
5. add a Yap-owned behavior/security test;
6. run the provenance contract with upstream verification; and
7. keep branding, binaries, models, data, and unrelated code out unless each has
   separate authority and license evidence.

Package-manager dependency metadata is not a substitute for direct-source
provenance. Likewise, visual inspiration or behavior comparison must not be
misrepresented as copied source.

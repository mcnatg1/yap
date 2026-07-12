# Research audits

Research audits pin external references and compare them with the current Yap
implementation. They are non-normative: ADRs still own architectural decisions,
and executable code/tests still own implementation truth.

An audit must distinguish three states:

- **Studied:** behavior or architecture was inspected; no donor source ships.
- **Adapted:** donor source influenced a Yap implementation and requires exact
  file-level provenance plus the applicable notice.
- **Copied:** donor source is substantially retained and requires exact
  file-level provenance, license compliance, modification notices, and tests.

Do not add a studied donor to `THIRD_PARTY_PROVENANCE.json`. Add it only when
adapted or copied source enters a shipped artifact, then update
`THIRD_PARTY_NOTICES.md` and the release-contract evidence in the same change.

## Audits

| Audit | Decision |
|-------|----------|
| [Freeflow and Meetily donor audit](2026-07-12-freeflow-meetily-reuse-audit.md) | Preserve Yap's runtime/safety core; selectively adapt donor behaviors after parity, security, license, and native tests |

---
target: desktop/src/components/panels/
total_score: 35
p0_count: 0
p1_count: 0
p2_count: 3
timestamp: 2026-06-30T16-43-04Z
slug: desktop-src-components-panels
---
Method: ⚠️ DEGRADED: single-context (subagent under parent; spawn prohibited)

**Target:** `desktop/src/components/panels/` (9 files). App shell note: `workspace-header.tsx` Help link and `App.tsx` wiring evaluated as part of the de84bb3d fix pass; primary scope remains panels.

## Anti-Patterns Verdict

**LLM assessment**: The panel layer now reads as a trustworthy local workbench, not an AI dashboard. Run 2's biggest tells are largely addressed: Polish finished state leads with a filled Copy action; History exposes Copy per row without opening a menu; workspace header surfaces Help without burying it in the rail alone. Remaining product-slop tells are minor and localized: uppercase column labels in Polish previews, three equal outline actions on TranscriptPanel after completion (inconsistent with Polish hierarchy), and infrastructure phrasing ("CPU only") quarantined in Details but still present. A Linear/Figma-fluent user would trust this surface; pauses would be about efficiency gaps, not strangeness.

**Deterministic scan**: `detect.mjs` on all 9 panel files returned `[]` (0 findings). No gradient text, side-stripe borders, or absolute-ban hits in panel markup.

**Visual overlays**: Not attempted — Tauri desktop target with no browser automation in this harness. No reliable user-visible overlay available.

## Design Health Score

| # | Heuristic | Score | Key Issue |
|---|-----------|-------|-----------|
| 1 | Visibility of System Status | 4 | Transcribing banner, queue progress, inline glance, transcript states |
| 2 | Match System / Real World | 4 | Tone hints; runner details tucked in Details disclosure |
| 3 | User Control and Freedom | 4 | History remove confirms; queue clear guarded; row selection + preview |
| 4 | Consistency and Standards | 3 | Post-done action hierarchy differs: Polish Copy primary vs Transcript three outlines |
| 5 | Error Prevention | 4 | Destructive actions confirmed; runnable gates on transcribe/polish |
| 6 | Recognition Rather Than Recall | 4 | History row Copy visible; Help in workspace header; popover preview on focus/hover |
| 7 | Flexibility and Efficiency | 2 | Ctrl+K only; no panel shortcuts, bulk history, or polish accelerators |
| 8 | Aesthetic and Minimalist Design | 4 | Dashboard metrics gone; polish hierarchy clear; minor uppercase preview labels |
| 9 | Error Recovery | 3 | Transcript retry + inline errors; polish errors via toast |
| 10 | Help and Documentation | 3 | Help link in workspace header; no panel-scoped guidance at first-run decision points |
| **Total** | | **35/40** | **Good — near Excellent; efficiency and cross-panel consistency remain** |

**Cognitive load**: 1 checklist failure (TranscriptPanel post-done presents three equal-weight actions). Low load overall, improved from Run 2.

## Overall Impression

Run 3 confirms the panel refactor is paying off. The de84bb3d fixes directly closed Run 2's P1 (Polish action hierarchy) and both P2s (History copy excavation, help discoverability via workspace header). What remains is refinement: align TranscriptPanel export actions with Polish's primary-led pattern, and add power-user paths without reintroducing dashboard density.

## What's Working

1. **`polish-panel.tsx`**: Post-polish ButtonGroup now correctly elevates Copy (default), demotes Save (secondary), and tucks Polish again into ghost — matches DESIGN.md principle 4. Tone hints + Details disclosure keep infrastructure secondary.
2. **`history-panel.tsx`**: Per-row Copy icon beside the overflow menu gives Alex a one-click path; AlertDialog on remove; sentence-case day headers; updated copy ("Select a row or focus a name to preview") matches Popover behavior.
3. **`workspace-header.tsx` + `home-panel.tsx`**: Help link in header improves discoverability across all workspace views; Home glance is a single muted sentence, not hero metrics; transcribing banner with Continue preserves task continuity.

## Priority Issues

### [P2] TranscriptPanel post-done actions lack a clear primary
- **Why it matters**: After transcription, Copy / Open / Reveal share equal outline weight in `transcript-panel.tsx`, while Polish now signals Copy as canonical. Jordan won't know whether to copy or reveal first; cross-panel inconsistency breaks learned patterns.
- **Fix**: Make Copy the filled primary in the done-state ButtonGroup; demote Open/Reveal to secondary or ghost, mirroring `polish-panel.tsx`.
- **Suggested command**: `/impeccable layout desktop/src/components/panels/transcript-panel.tsx`

### [P2] Help is global, not contextual at panel decision points
- **Why it matters**: Workspace Help link fixes discoverability (Run 2 gap), but Polish first run, History empty state, and Transcript empty state still don't offer scoped "How this works" entry. Jordan at an empty History or first Polish visit must infer from generic HelpSheet.
- **Fix**: Add a subtle panel-scoped link in empty states or panel footers that opens HelpSheet scrolled/filtered to that flow (or duplicate the two most relevant StatusRows inline).
- **Suggested command**: `/impeccable onboard desktop/src/components/panels/`

### [P2] No power-user efficiency paths in panels
- **Why it matters**: History copy is faster now, but no bulk select/transcribe, no keyboard run/save in Polish, no panel-level shortcuts beyond Ctrl+K in header. Alex still hits friction on repeated exports.
- **Fix**: Add keyboard accelerators for Copy in focused transcript/history rows; consider multi-select in History for batch copy/export.
- **Suggested command**: `/impeccable harden desktop/src/components/panels/`

## Persona Red Flags

**Alex (Power User)**: Row Copy is fixed; bulk history actions and polish keyboard shortcuts still missing. Transcript three-outline row slows repeated copy-after-transcribe.

**Sam (Accessibility)**: HistoryEntryPreview opens on focus (good). Popover still schedules close on pointer leave — keyboard users tabbing into PopoverContent may lose preview unless they interact quickly. Focus ring on row Copy buttons is present.

**Jordan (First-Timer)**: Polish post-success hierarchy is clear (Copy first). Transcript done state still ambiguous. "Reveal in Explorer" assumes Windows literacy. Empty states teach next action but don't link to help.

**Morgan (Privacy-conscious journalist)**: "CPU only" in Polish Details reads as infrastructure; prefer "On this device" phrasing. Privacy badge on DropHero and header PrivacyStatus reinforce trust.

## Minor Observations

- `polish-panel.tsx` PreviewColumn headers use `uppercase` on muted labels — minor product eyebrow tell; sentence case would match History day headers.
- `drop-hero.tsx` + `queue-panel.tsx` remain strong: privacy badge, operational copy, confirmed clear, elapsed timer.
- `app-sheets.tsx` runner jargon appropriately quarantined in Setup Details.
- Detector clean on panels; muted `#756F66` on warm canvas should still be verified via `/impeccable audit`.
- Signals also flag `desktop/src/components/app/` + `App.tsx` for shell consistency; no blocking issues found beyond Help wiring already scored above.

## Questions to Consider

- Should TranscriptPanel mirror Polish's Copy-primary pattern everywhere transcripts can be exported?
- Would History preview work better as row selection opening the full TranscriptPanel reading surface instead of a small popover?
- What if Help appeared once per panel on first visit, then collapsed to the header link?

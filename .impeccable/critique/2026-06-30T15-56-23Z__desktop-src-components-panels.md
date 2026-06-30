---
target: desktop/src/components/panels/
total_score: 32
p0_count: 0
p1_count: 1
p2_count: 2
timestamp: 2026-06-30T15-56-23Z
slug: desktop-src-components-panels
---
Method: ⚠️ DEGRADED: single-context (sub-agent harness; no Task tool exposed for isolated Assessment A/B)

## Anti-Patterns Verdict

**LLM assessment**: The panels now read as a cohesive local workbench rather than a dashboard-plus-dev-tools hybrid. Home's inline glance line and transcribing banner reinforce task state without hero metrics; History preview is a proper Popover with keyboard focus; Polish leads with tone + CTA and tucks runner stats behind Details. Remaining slop tells are minor: uppercase column labels in Polish previews, equal-weight outline buttons after a successful polish run, and setup jargon safely quarantined in `DetailsSheet`. A category-fluent user would trust this more than the first pass.

**Deterministic scan**: `detect.mjs` on all 9 panel files returned `[]` (0 findings). No gradient text, side-stripe borders, or absolute-ban hits in markup.

**Visual overlays**: Not attempted — Tauri desktop target with no browser automation exposed in this harness. No reliable user-visible overlay available.

## Design Health Score

| # | Heuristic | Score | Key Issue |
|---|-----------|-------|-----------|
| 1 | Visibility of System Status | 4 | Transcribing banner + inline glance; queue/transcript states strong |
| 2 | Match System / Real World | 4 | Tone hints clarify polish; model/tok/s moved to Details disclosure |
| 3 | User Control and Freedom | 4 | History remove now confirms; queue clear pattern consistent |
| 4 | Consistency and Standards | 3 | List vs table vs split-preview patterns still diverge by panel |
| 5 | Error Prevention | 4 | Destructive history remove mirrors queue clear guardrail |
| 6 | Recognition Rather Than Recall | 3 | Popover preview works on focus/hover; copy still buried in History menu |
| 7 | Flexibility and Efficiency | 2 | Ctrl+K in header only; no panel shortcuts or bulk history actions |
| 8 | Aesthetic and Minimalist Design | 3 | Dashboard metrics gone; post-polish three-button row still noisy |
| 9 | Error Recovery | 3 | Transcript retry + inline errors; polish errors via toast |
| 10 | Help and Documentation | 2 | HelpSheet exists but panels don't surface contextual help entry points |
| **Total** | | **32/40** | **Good — solid foundation, targeted polish needed** |

**Cognitive load**: 1–2 checklist failures (Polish finished state presents three equal actions; Transcribe three-pane still moderate). Low–moderate load, improved from prior run.

## Overall Impression

The panel layer now consistently reinforces "quiet, capable, local." The first critique's dashboard chrome and developer status grid are largely gone; what remains is refinement around action hierarchy after success states and power-user paths in History. The single biggest opportunity is to give Polish and History clearer post-success primaries without reintroducing density.

## What's Working

1. **`home-panel.tsx`**: Inline glance summary (`3 saved · 2 in queue · Ready`), transcribing banner with Continue, and a single filled CTA with "choose files" demoted to a link — matches DESIGN.md one-primary-action guidance.
2. **`history-panel.tsx` + `history-entry-preview.tsx`**: Popover preview on hover/focus with skeleton loading; sentence-case day headers; AlertDialog on remove; actions isolated in their own table cell (no nested-button focus trap).
3. **`polish-panel.tsx`**: Tone hints under toggle, flattened side-by-side previews, single status line, and Details disclosure for model/speed — infrastructure stays secondary until the user asks.

## Priority Issues

### [P1] Polish finished state has three equal outline actions
- **Why it matters**: After polish completes, Copy / Save / Polish again share the same outline weight. DESIGN.md principle 4 calls for one primary action per state; Jordan won't know whether to save or copy first.
- **Fix**: Make Save (or Copy) the single filled primary; demote Polish again to link or ghost; keep the other as secondary outline.
- **Suggested command**: `/impeccable layout desktop/src/components/panels/polish-panel.tsx`

### [P2] History transcript actions require dropdown excavation
- **Why it matters**: Copy and Preview live behind MoreHorizontal per row — four interactions for a common power-user path vs one-click Copy in TranscriptPanel header (Alex persona).
- **Fix**: Expose Copy on row hover/focus as a visible secondary icon, or mirror TranscriptPanel's header ButtonGroup when a row is selected.
- **Suggested command**: `/impeccable layout desktop/src/components/panels/history-panel.tsx`

### [P2] Panels don't link to contextual help
- **Why it matters**: HelpSheet documents drop/transcribe/copy flows but no panel surfaces a help entry at decision points (first History visit, first Polish run). Jordan still has no in-context escape hatch.
- **Fix**: Add a subtle "How this works" link in empty states or panel headers that opens HelpSheet scoped to that panel.
- **Suggested command**: `/impeccable onboard desktop/src/components/panels/`

## Persona Red Flags

**Alex (Power User)**: History copy still requires opening a per-row dropdown. No bulk select/transcribe from History. Polish has no keyboard accelerator for run/save.

**Sam (Accessibility)**: HistoryEntryPreview opens on focus (good), but preview dismisses on pointer leave — keyboard users moving into popover content may lose it unless they click. Reduced-motion handling in StackedUpload and AnimatedActionIcon addresses prior queue animation gap.

**Jordan (First-Timer)**: Tone hints (`polishToneHints`) now explain Light/Medium/Heavy. "Reveal in Explorer" still assumes Windows literacy. Post-polish three-button row doesn't signal the recommended next step.

**Morgan (Privacy-conscious journalist)**: Runner details (`tok/s`, model name, "CPU only") appear only after polish in Details disclosure — acceptable, but the word "CPU" still reads as infrastructure; consider "On this device" phrasing.

## Minor Observations

- `polish-panel.tsx` PreviewColumn headers use `uppercase` on muted labels — minor product eyebrow tell; sentence case would match History day headers.
- `drop-hero.tsx` + `transcript-panel.tsx` remain strong: privacy badge, transcript-as-hero typography, skeleton loading.
- `queue-panel.tsx` operational copy with elapsed timer and confirmed clear is unchanged and still solid.
- `app-sheets.tsx` "RTX local runner" remains appropriate inside Setup Details, not on primary surfaces.
- Detector clean; muted text contrast on warm canvas should still be verified via `/impeccable audit`.

## Questions to Consider

- After polish succeeds, is Save or Copy the canonical next step — and does the UI say so explicitly?
- Should History selection open the same TranscriptPanel reading surface instead of a small popover?
- What if Help appeared only on first visit to Polish or History, then tucked away?

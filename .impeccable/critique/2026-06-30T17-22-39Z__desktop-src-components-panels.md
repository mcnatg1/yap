---
target: desktop/src/components/panels/
total_score: 37
p0_count: 0
p1_count: 0
p2_count: 2
timestamp: 2026-06-30T17-22-39Z
slug: desktop-src-components-panels
---
Method: ⚠️ DEGRADED: single-context (subagent under parent; spawn prohibited)

**Target:** `desktop/src/components/panels/` (9 files). Run 4 evaluates the P2 polish pass: Transcript Copy-primary hierarchy, contextual Help links, Ctrl+Shift+C, Enter for Polish, sentence-case preview labels, and "On this device" phrasing. PRODUCT.md/DESIGN.md now cite Tauri 1122×740 minimum (not 360px mobile).

## Anti-Patterns Verdict

**LLM assessment**: The panel layer now reads as a mature local workbench a Linear/Figma-fluent user would trust without pausing. Run 3's three P2 gaps are largely closed: TranscriptPanel mirrors Polish's Copy-led ButtonGroup; empty and first-run surfaces expose contextual "How this works" / "Learn more" links; accelerators (Ctrl+Shift+C, Enter to Polish) appear inline without dashboard density. Remaining tells are edge-case: History popover still schedules close on pointer leave (keyboard preview fragility), contextual help opens the same generic HelpSheet (no deep-link), and Alex still lacks bulk history or post-polish Copy shortcut. None read as AI slop — earned familiarity throughout.

**Deterministic scan**: `detect.mjs` on all 9 panel files returned `[]` (0 findings). No gradient text, side-stripe borders, or absolute-ban hits in panel markup.

**Visual overlays**: Not attempted — Tauri desktop target with no browser automation in this harness. No reliable user-visible overlay available.

## Design Health Score

| # | Heuristic | Score | Key Issue |
|---|-----------|-------|-----------|
| 1 | Visibility of System Status | 4 | Transcribing banner, queue progress, inline glance, transcript states |
| 2 | Match System / Real World | 4 | "On this device" phrasing; operational copy; runner jargon in Details only |
| 3 | User Control and Freedom | 4 | History remove confirms; queue clear guarded; row selection + preview |
| 4 | Consistency and Standards | 4 | Transcript + Polish share Copy-primary / secondary / ghost hierarchy |
| 5 | Error Prevention | 4 | Destructive actions confirmed; runnable gates on transcribe/polish |
| 6 | Recognition Rather Than Recall | 4 | Per-row Copy; shortcut hints in transcript header; contextual help links |
| 7 | Flexibility and Efficiency | 3 | Ctrl+Shift+C + Enter for Polish added; no bulk history or polish Copy shortcut |
| 8 | Aesthetic and Minimalist Design | 4 | Sentence-case preview labels; clear post-success hierarchy |
| 9 | Error Recovery | 3 | Transcript retry + inline errors; polish errors via toast |
| 10 | Help and Documentation | 3 | Contextual links at decision points; HelpSheet still undifferentiated |
| **Total** | | **37/40** | **Excellent — minor power-user and help-scoping polish remain** |

**Cognitive load**: 0 checklist failures. Post-done states have one clear primary; empty states teach next action with optional help link.

## Overall Impression

Run 4 crosses into Excellent territory. The P2 polish pass directly addressed Run 3's cross-panel inconsistency, help discoverability, and efficiency gaps without reintroducing density. What remains is refinement at the margins: deep-linked help, keyboard-stable History preview, and batch export paths for repeat users.

## What's Working

1. **`transcript-panel.tsx`**: Done-state ButtonGroup elevates Copy (default), demotes Open (secondary) and Reveal (ghost) — mirrors Polish. Ctrl+Shift+C listener with visible Kbd hint in CardDescription closes the power-user loop after transcribe.
2. **`polish-panel.tsx`**: PreviewColumn titles are sentence-case; Details disclosure says "On this device" instead of infrastructure phrasing; Enter-to-run hint in tone description; contextual help on waiting and pre-run states.
3. **`drop-hero.tsx` + `history-panel.tsx` + `workspace-header.tsx`**: Privacy badge and header Help remain; DropHero and History empty states add panel-scoped help links; per-row Copy icon preserved from Run 3.

## Priority Issues

### [P2] History preview popover fragile for keyboard users
- **Why it matters**: `history-entry-preview.tsx` opens on focus but schedules close on `pointerLeave` with an 80ms delay. Keyboard users tabbing into PopoverContent can lose the preview when the pointer isn't over the popover — undermining the "focus a name to preview" promise in History copy.
- **Fix**: Keep popover open while focus is inside trigger or content; only auto-close on pointer leave when neither has focus.
- **Suggested command**: `/impeccable harden desktop/src/components/panels/history-entry-preview.tsx`

### [P2] Contextual help links open undifferentiated HelpSheet
- **Why it matters**: Drop, Transcript empty, History empty, and Polish waiting states now surface help — good recognition — but all call the same generic HelpSheet. Jordan clicking "How this works" on Polish still sees transcribe-oriented rows first; the link overpromises panel-specific guidance.
- **Fix**: Pass a `section` or `focus` prop to HelpSheet (e.g. `helpFocus="polish"`) and scroll/highlight the relevant StatusRows, or split help into short inline paragraphs per panel.
- **Suggested command**: `/impeccable onboard desktop/src/components/panels/app-sheets.tsx`

### [P3] Power-user efficiency still partial
- **Why it matters**: Ctrl+Shift+C and Enter for Polish help Alex on two hot paths, but History has no multi-select/bulk copy, Polish post-success Copy has no keyboard accelerator, and queue has no run shortcut. Repeat export workflows still click-heavy.
- **Fix**: Add Ctrl+Shift+C parity when Polish panel has focus and polished text; consider Shift+click multi-select in History for batch copy.
- **Suggested command**: `/impeccable harden desktop/src/components/panels/`

## Persona Red Flags

**Alex (Power User)**: Transcript copy accelerator is fixed; bulk history and polish Copy shortcut still missing. Contextual help is discoverable but not shortcut-friendly.

**Sam (Accessibility)**: HistoryEntryPreview focus opens immediately (good), but pointer-leave close can dismiss preview while exploring content via keyboard. Transcript Kbd hints are visible; contrast on muted operational copy should still be verified via audit.

**Jordan (First-Timer)**: Post-transcribe and post-polish hierarchies are unambiguous (Copy first). Contextual help links reduce guesswork on empty states. "Reveal in Explorer" in History menu still assumes Windows literacy.

**Morgan (Privacy-conscious journalist)**: "On this device" in Polish Details and "Private on this device" on DropHero reinforce trust without GPU jargon on primary surfaces.

## Minor Observations

- `home-panel.tsx` empty "No transcripts today" has no help link while other panels do — minor inconsistency.
- `queue-panel.tsx` and `drop-hero.tsx` remain strong operational surfaces; no changes needed this pass.
- `app-sheets.tsx` runner details appropriately quarantined; HelpSheet copy could mention Polish and History flows.
- Detector clean on all 9 panel files; muted `#756F66` on warm canvas still warrants `/impeccable audit` for contrast verification.
- PRODUCT.md/DESIGN.md Tauri 1122×740 alignment removes false mobile-responsive concern from prior critiques.

## Questions to Consider

- Should HelpSheet accept a focus key so contextual links feel honest?
- Would History work better with row selection opening the full TranscriptPanel reading surface instead of a small popover?
- Is Excellent (37) the ceiling without bulk actions, or does one more efficiency pass reach 38–39?

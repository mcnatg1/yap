---
target: desktop/src/components/panels/
total_score: 26
p0_count: 0
p1_count: 3
p2_count: 2
timestamp: 2026-06-30T15-09-30Z
slug: desktop-src-components-panels
---
Method: ⚠️ DEGRADED: single-context (sub-agent harness; no Task tool exposed for isolated Assessment A/B)

## Anti-Patterns Verdict

**LLM assessment**: The panels read as a coherent warm workbench, not a generic AI landing page. shadcn vocabulary is consistent, copy is mostly operational, and the transcript surface earns its place as the reward state. The main product-slop tells are localized: Home's StatBlock metrics echo SaaS dashboard hero-metric scaffolding; History uses uppercase tracked day eyebrows; Polish nests cards and surfaces CPU/model stats on the primary task surface. None of these scream "AI made this," but a Linear/Figma-fluent user would pause at the dashboard-ish Home sidebar and the developer-facing Polish status grid.

**Deterministic scan**: `detect.mjs` on all 9 panel files returned `[]` (0 findings). No gradient text, side-stripe borders, or other absolute-ban hits in markup.

**Visual overlays**: Not attempted — Tauri desktop target with no browser automation exposed in this harness. No reliable user-visible overlay available.

## Design Health Score

| # | Heuristic | Score | Key Issue |
|---|-----------|-------|-----------|
| 1 | Visibility of System Status | 3 | Queue/transcript states strong; Home "At a glance" underplays active transcribe |
| 2 | Match System / Real World | 3 | Plain copy overall; Polish exposes tok/s and model IDs on main surface |
| 3 | User Control and Freedom | 3 | Queue clear confirmed; history remove is one-click destructive |
| 4 | Consistency and Standards | 3 | Shared `surface-workspace-inset` cards; list vs table vs attachment patterns diverge |
| 5 | Error Prevention | 3 | Runnable/disabled gates good; missing confirm on history delete |
| 6 | Recognition Rather Than Recall | 2 | History preview is hover-only despite copy promising hover |
| 7 | Flexibility and Efficiency | 2 | Ctrl+K in header only; no panel-level shortcuts or bulk actions |
| 8 | Aesthetic and Minimalist Design | 2 | Polish four-up StatusRow grid + nested preview cards add noise |
| 9 | Error Recovery | 3 | Transcript retry + inline errors; polish errors via toast |
| 10 | Help and Documentation | 2 | HelpSheet exists but panels don't surface contextual help paths |
| **Total** | | **26/40** | **Acceptable — solid foundation, targeted polish needed** |

**Cognitive load**: 3 checklist failures (single focus on Polish, minimal choices on Polish tone+actions+status grid, one-thing-at-a-time on Transcribe three-pane). Moderate load.

## Overall Impression

The panel layer successfully implements Yap's drop → queue → transcript loop with warm, document-like surfaces and sensible empty states. The single biggest opportunity is to strip dashboard and developer chrome from Home and Polish so every panel reinforces "quiet, capable, local" instead of mixing utility and metrics UI.

## What's Working

1. **`drop-hero.tsx` + `transcript-panel.tsx`**: Drop hero states privacy clearly ("Private on this device"), responds to drag, and the transcript panel treats finished text as the hero (15px/`leading-7`, copy/open/reveal adjacent, skeleton while loading).
2. **`queue-panel.tsx`**: Operational status copy with elapsed timer, progress field, clear-with-confirmation, and primary Transcribe gated on runnable state — matches DESIGN.md running-state guidance.
3. **Cross-panel empty states**: Home, History, Queue (via StackedUpload), and Transcript all use purposeful Empty components that teach the next action instead of bare "nothing here."

## Priority Issues

### [P1] Home "At a glance" StatBlocks read as SaaS dashboard metrics
- **Why it matters**: PRODUCT.md anti-references "SaaS dashboard density." Big-number stat tiles (`text-2xl` values for Saved/In queue/Status) compete with the transcribe CTA for attention on the hub screen.
- **Fix**: Replace StatBlocks with compact inline status rows or a single sentence status line; keep counts in muted inline text, not hero metrics.
- **Suggested command**: `/impeccable distill desktop/src/components/panels/home-panel.tsx`

### [P1] History preview is hover-only but marketed as hover interaction
- **Why it matters**: `history-panel.tsx` CardDescription says "Hover a name for a quick preview," but `history-entry-preview.tsx` uses HoverCard — unusable for keyboard-only and touch users (Sam persona).
- **Fix**: Add keyboard-focus trigger (focus opens preview), click/tap preview affordance, or move preview to row selection side panel; update copy to match actual input modalities.
- **Suggested command**: `/impeccable harden desktop/src/components/panels/history-entry-preview.tsx`

### [P1] Polish panel overloads the primary surface with status grid and tech jargon
- **Why it matters**: Four StatusRows (model, CPU, speed, draft state) plus tone toggle and three action buttons violate "one primary action per state" and expose runner details PRODUCT.md relegates to secondary areas (`defaultPolishModel`, "19 tok/s CPU").
- **Fix**: Collapse status to one line; move model/speed behind disclosure or post-run footnote; lead with tone + Polish CTA + side-by-side previews only.
- **Suggested command**: `/impeccable distill desktop/src/components/panels/polish-panel.tsx`

### [P2] History delete lacks confirmation
- **Why it matters**: Queue clear uses AlertDialog; history "Remove from history" in `history-panel.tsx` fires immediately — inconsistent guardrail for destructive action (Riley persona).
- **Fix**: Mirror queue clear pattern with confirm dialog explaining history vs file deletion scope.
- **Suggested command**: `/impeccable harden desktop/src/components/panels/history-panel.tsx`

### [P2] Competing primary actions on Home hub
- **Why it matters**: DESIGN.md principle 4: one primary action per state. Home offers both "Open transcribe" (filled) and "Choose files" (outline) at equal visual weight — Jordan may not know which path is canonical.
- **Fix**: Single primary ("Open transcribe" or "Choose files" depending on intended flow); demote the other to text link or secondary.
- **Suggested command**: `/impeccable layout desktop/src/components/panels/home-panel.tsx`

## Persona Red Flags

**Alex (Power User)**: History actions live inside a `MoreHorizontal` dropdown per row — four clicks to copy vs one in TranscriptPanel header. No bulk select/transcribe from History. Polish requires sequential tone pick + Polish click with no keyboard accelerator.

**Sam (Accessibility)**: HoverCard preview in `history-entry-preview.tsx` is pointer-dependent. Queue list animations in StackedUpload (used by QueuePanel) have no `prefers-reduced-motion` handling anywhere in `desktop/src`. Nested `<button>` inside clickable `<TableRow>` in history creates confusing focus order.

**Jordan (First-Timer)**: Polish tone toggle labels (`polishToneLabels`) have no inline explanation of what Light/Medium/Heavy mean. "Reveal in Explorer" assumes Windows literacy. Home dual CTAs split the obvious first action.

**Morgan (Privacy-conscious journalist — project persona)**: Seeing `Cpu` icon, `defaultPolishModel`, and "tok/s CPU" on the Polish panel feels like infrastructure tooling, undermining "quiet, capable, local" trust even though processing is on-device.

## Minor Observations

- `history-panel.tsx` day headers use `uppercase tracking-wide` — product eyebrow tell; sentence-case labels would match DESIGN.md label guidance.
- `polish-panel.tsx` `TextPreview` nests Card inside Card — product register nested-card smell.
- `app-sheets.tsx` DetailsSheet "RTX local runner" is acceptable in setup drawer but inconsistent with anti-reference to GPU jargon on main screens.
- `workspace-header.tsx` is clean; history count badge and PrivacyStatus placement work well.
- Detector clean; contrast of `#756f66` muted text on warm canvas should be verified in `/impeccable audit` (not flagged by detect).

## Questions to Consider

- What if Home showed today's transcripts only, with zero metric tiles?
- Does Polish need live CPU stats before the user runs it, or only after?
- Should History preview become the same TranscriptPanel pattern instead of a hover tooltip?

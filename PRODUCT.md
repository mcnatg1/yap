# Product

## Register

product

## Users

Privacy-conscious people who need accurate text from audio or video files — journalists transcribing interviews, researchers working through field recordings, podcasters, students, and anyone who batch-processes media without sending it to a third-party cloud.

They usually arrive with files already on disk (MP3, M4A, WAV, MP4, and similar formats). Live capture is explicit and secondary, used for local/offline fallback or saved sessions, not always-listening dictation. Context is focused work: drop files, wait for trusted transcription, review and export text, occasionally polish wording before sharing. They care that the current route is clear: local fallback stays on this device; team/server mode uses org-owned GB-class hardware, not third-party cloud.

## Product Purpose

Yap is a desktop transcription app (Tauri + React in `desktop/`). The current desktop implementation records and transcribes explicit live sessions locally with Nemotron 3.5 ASR Streaming 0.6B INT8 through in-process `sherpa-onnx`. Users can also import, review, and queue audio or video files, but official imported-file transcription still waits for the private organization-server connector; disconnected imports remain queued or blocked instead of receiving official-looking fallback output.

The target product loop is files in, accurate transcripts out, with minimal friction between drop → durable queue → private server transcript → copy/export. Until that connected path lands, the implemented offline loop is explicit live capture → local transcript → history/playback/copy or reveal. The interface should make the current file and its transcript the center of attention; model names, auth paths, and runner details stay in secondary status unless something needs attention.

Current production navigation:

- **Home** — hub with recent transcripts and a quick path back into work
- **Transcribe** — the workbench: drop zone, queue, progress, and live transcript preview

Transcript history currently lives on Home; there is no separate Transcripts navigation item or dedicated export command yet. A development-only, opt-in Polish surface exists but is hidden from production builds. A later product slice may split history into its own destination and promote Polish only after a governed LLM route exists.

This is not live-only dictation or a Wispr Flow clone. Batch recordings remain the target core loop once the trusted server route exists; live capture is the implemented compact, explicit companion path.

## Brand Personality

Calm, direct, private, warm, fast to understand. Three words: **quiet**, **capable**, **local**.

Voice is operational and plain — short labels like "Drop recordings" or "Transcript ready," not product essays on every screen. Confidence comes from clarity and visible progress, not from hype or technical jargon. The app should feel like a polished consumer utility: sparse navigation, one obvious action at a time, document-like transcript surfaces.

Reference feel (specific traits, not category buckets):

- **Wispr Flow** — practical sparse nav and compact status, adapted for batch files rather than live mic input
- **Figma-style tools** — soft canvas, one generous work surface, secondary details tucked away until needed

Emotional goal: users trust the visible route — local fallback on this device, server work on org-owned hardware — and that the tool disappears into the task.

## Anti-references

- Always-listening mic UX or live-only dictation (Wispr-style realtime capture is not the whole product)
- SaaS dashboard density — this is a file-to-transcript tool, not an analytics hub
- Marketing landing page inside the app: giant hero copy, gradient blobs, decorative sections, nested card grids
- Developer-first chrome: leading with model IDs, Python paths, GPU jargon, or auth mechanism details on the main screen
- Generic beige AI utility aesthetic with no identity (anonymous cream canvas with no purposeful accent discipline)
- Modal-heavy flows where inline or progressive disclosure would suffice
- Cloud-upload patterns that hide where files are processed

## Design Principles

1. **Drop audio, get text.** Every screen should reinforce the core loop; secondary capabilities (polish, history, setup) support it, they don't compete with it.
2. **The transcript is the reward.** When transcription completes, the text surface becomes the hero; copy and reveal actions stay adjacent to the content, with dedicated export remaining a target capability.
3. **Trusted route, stated simply.** Say "Private on this device" for local fallback and "Org server" for team/server work — not implementation details — unless an error requires technical context.
4. **One primary action per state.** Empty → drop; queued → wait for the trusted route; running → progress + cancel; done → read, copy, or reveal. Avoid competing primary buttons.
5. **Technical setup is secondary.** Model, auth, runner, and output path belong in details/status areas until something needs attention.

## Accessibility & Inclusion

Target WCAG 2.1 AA for text contrast and interactive states. Body and label text must remain readable on warm canvas and surface backgrounds (no washed-out muted gray for operational copy).

- Respect `prefers-reduced-motion`: state feedback should crossfade or snap instead of choreographed entrance sequences.
- Keyboard paths for navigation rail, queue actions, transcript copy/export, and shipped shortcuts.
- Tooltips and accessible names on icon-only actions (remove, reveal, copy, settings).
- Window floor matches Tauri minimum size (`minWidth` 1122, `minHeight` 740; default 1122×760 in `desktop/src-tauri/tauri.conf.json`). Layout is designed for that footprint; responsive single-column at mobile widths (e.g. 360px) is not a current target unless `minWidth` is lowered later. No overlapping controls at minimum size; transcript reading comfortable at 15px+ with generous line height.
- No information conveyed by color alone for queue status — pair color with label or icon.

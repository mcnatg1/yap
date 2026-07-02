# Product

## Register

product

## Users

Privacy-conscious people who need accurate text from audio or video files — journalists transcribing interviews, researchers working through field recordings, podcasters, students, and anyone who batch-processes media without sending it to a third-party cloud.

They arrive with files already on disk (MP3, M4A, WAV, MP4, and similar formats), not with a microphone open for live dictation. Context is focused work: drop files, wait for local transcription, review and export text, occasionally polish wording before sharing. They care that processing stays on-device and that the app feels like a trustworthy utility, not a developer tool or a marketing site.

## Product Purpose

Yap is a desktop transcription app (Tauri + React in `desktop/`). Users drop audio or video files, transcribe through the best available trusted runtime, and get readable text saved beside the source or in a local transcripts folder. Local Moonshine is the offline/live fallback; higher-quality recording transcription belongs on the DGX/server Cohere path.

Success looks like: files in, accurate transcripts out, with minimal friction between drop → queue → transcript → copy/export. The interface should make the current file and its transcript the center of attention; model names, auth paths, and runner details stay in secondary status unless something needs attention.

Primary navigation:

- **Home** — hub with recent transcripts and a quick path back into work
- **Transcribe** — the workbench: drop zone, queue, progress, and live transcript preview
- **Transcripts** — history grouped by day for reopening past work
- **Polish** — rewrite/cleanup pass on a selected transcript before export

This is not live dictation (not a Wispr Flow clone). It is batch processing for people who already have recordings.

## Brand Personality

Calm, direct, private, warm, fast to understand. Three words: **quiet**, **capable**, **local**.

Voice is operational and plain — short labels like "Drop recordings" or "Transcript ready," not product essays on every screen. Confidence comes from clarity and visible progress, not from hype or technical jargon. The app should feel like a polished consumer utility: sparse navigation, one obvious action at a time, document-like transcript surfaces.

Reference feel (specific traits, not category buckets):

- **Wispr Flow** — practical sparse nav and compact status, adapted for batch files rather than live mic input
- **Figma-style tools** — soft canvas, one generous work surface, secondary details tucked away until needed

Emotional goal: users trust that their files never leave the machine and that the tool disappears into the task.

## Anti-references

- Live dictation / always-listening mic UX (Wispr-style realtime capture is the wrong mental model)
- SaaS dashboard density — this is a file-to-transcript tool, not an analytics hub
- Marketing landing page inside the app: giant hero copy, gradient blobs, decorative sections, nested card grids
- Developer-first chrome: leading with model IDs, Python paths, GPU jargon, or auth mechanism details on the main screen
- Generic beige AI utility aesthetic with no identity (anonymous cream canvas with no purposeful accent discipline)
- Modal-heavy flows where inline or progressive disclosure would suffice
- Cloud-upload patterns that imply files leave the device

## Design Principles

1. **Drop audio, get text.** Every screen should reinforce the core loop; secondary capabilities (polish, history, setup) support it, they don't compete with it.
2. **The transcript is the reward.** When transcription completes, the text surface becomes the hero; export and copy actions stay adjacent to the content.
3. **Local, stated simply.** Say "Private on this device" or "Files stay on this machine" — not implementation details — unless an error requires technical context.
4. **One primary action per state.** Empty → drop; queued → transcribe; running → progress + cancel; done → read and export. Avoid competing primary buttons.
5. **Technical setup is secondary.** Model, auth, runner, and output path belong in details/status areas until something needs attention.

## Accessibility & Inclusion

Target WCAG 2.1 AA for text contrast and interactive states. Body and label text must remain readable on warm canvas and surface backgrounds (no washed-out muted gray for operational copy).

- Respect `prefers-reduced-motion`: state feedback should crossfade or snap instead of choreographed entrance sequences.
- Keyboard paths for navigation rail, queue actions, transcript copy/export, and command palette.
- Tooltips and accessible names on icon-only actions (remove, reveal, copy, settings).
- Window floor matches Tauri minimum size (`minWidth` 1122, `minHeight` 740; default 1122×760 in `desktop/src-tauri/tauri.conf.json`). Layout is designed for that footprint; responsive single-column at mobile widths (e.g. 360px) is not a current target unless `minWidth` is lowered later. No overlapping controls at minimum size; transcript reading comfortable at 15px+ with generous line height.
- No information conveyed by color alone for queue status — pair color with label or icon.

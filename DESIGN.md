---
version: alpha
name: Yapx3 Drop & Done
description: A local-first consumer transcription design system for Yapx3.
colors:
  ink: "#0F172A"
  muted-ink: "#475569"
  quiet-ink: "#64748B"
  canvas: "#EEF3F2"
  surface: "#FFFFFF"
  surface-muted: "#F8FAFC"
  border: "#CBD5E1"
  border-soft: "#E2E8F0"
  primary: "#0F766E"
  primary-hover: "#115E59"
  primary-soft: "#CCFBF1"
  accent: "#4F46E5"
  success: "#0F766E"
  warning: "#B45309"
  danger: "#B91C1C"
typography:
  headline-lg:
    fontFamily: Inter
    fontSize: 32px
    fontWeight: 700
    lineHeight: 1.15
    letterSpacing: 0
  headline-md:
    fontFamily: Inter
    fontSize: 22px
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: 0
  body-md:
    fontFamily: Inter
    fontSize: 15px
    fontWeight: 400
    lineHeight: 1.55
    letterSpacing: 0
  body-sm:
    fontFamily: Inter
    fontSize: 13px
    fontWeight: 400
    lineHeight: 1.45
    letterSpacing: 0
  label-md:
    fontFamily: Inter
    fontSize: 13px
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: 0
  caption:
    fontFamily: Inter
    fontSize: 12px
    fontWeight: 500
    lineHeight: 1.35
    letterSpacing: 0
rounded:
  sm: 6px
  md: 8px
  lg: 12px
  full: 9999px
spacing:
  xs: 4px
  sm: 8px
  md: 16px
  lg: 24px
  xl: 32px
  xxl: 48px
  app-margin: 20px
components:
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.surface}"
    rounded: "{rounded.md}"
    padding: 10px 14px
    typography: "{typography.label-md}"
  button-primary-hover:
    backgroundColor: "{colors.primary-hover}"
  button-secondary:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    borderColor: "{colors.border-soft}"
    rounded: "{rounded.md}"
    padding: 10px 14px
    typography: "{typography.label-md}"
  card:
    backgroundColor: "{colors.surface}"
    borderColor: "{colors.border-soft}"
    rounded: "{rounded.md}"
    padding: "{spacing.md}"
  status-pill:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.primary}"
    borderColor: "{colors.primary-soft}"
    rounded: "{rounded.md}"
    typography: "{typography.label-md}"
---

# Yapx3 Design System

## Overview

Yapx3 is a local-first transcription app for people with recordings, meetings,
voice memos, interviews, and video files sitting on their machine. It should
feel like a polished consumer utility: calm, direct, private, and fast to
understand.

The core product promise is "drop audio, get text." The interface should make
the current file and transcript feel more important than the model, auth state,
or runner details. Technical setup belongs in a details drawer or secondary
status area unless something needs attention.

Use the phrase "local" sparingly but confidently. Users should understand that
files stay on this device without the app sounding like infrastructure tooling.

## Colors

The palette is quiet and work-focused, with enough contrast to keep the app from
feeling like a generic slate dashboard.

- **Ink (`#0F172A`):** Primary text, transcript text, and important labels.
- **Canvas (`#EEF3F2`):** App background. It should feel softer than pure gray.
- **Surface (`#FFFFFF`):** Primary panels, cards, transcript editor, and sheets.
- **Primary Teal (`#0F766E`):** Main action, local/privacy confidence, success.
- **Accent Indigo (`#4F46E5`):** Rare highlight for transcript intelligence,
  search matches, or selection; never compete with the primary action.
- **Warning and Danger:** Reserved for setup, auth, or failed transcription.

## Typography

Use Inter throughout. The product should read as a native desktop utility, not a
marketing page.

- **Headlines:** Short, concrete labels such as "Drop recordings" or
  "Transcript ready."
- **Body:** Plain operational copy. Avoid explaining the whole product on screen.
- **Labels:** Sentence case except compact status chips. Do not use decorative
  letter spacing.
- **Transcript text:** Comfortable long-form reading, at least 15px with a loose
  line height.

## Layout

The primary layout is a workbench with one obvious action at a time.

- Empty state: a large tactile drop zone, recent files below or beside it, and a
  small privacy badge.
- Running state: the active file card should show progress, elapsed time, and a
  clear cancel/remove path.
- Done state: the transcript preview becomes the hero; export actions sit close
  to the transcript, not in a distant toolbar.
- Settings/status: model, auth, runner, output path, and logs live in a secondary
  area. They should not dominate the first screen.

Use an 8px rhythm. Keep desktop widths readable, and collapse to one column
below tablet width. Nothing in the main flow should require horizontal scrolling
at the 360px minimum window width.

## Elevation & Depth

Use tonal depth instead of heavy shadows. Panels sit on the canvas with soft
borders and small shadows only where a surface is interactive or draggable.

The drop zone may feel slightly tactile when active: brighter border, subtle
surface tint, and a lifted shadow. Avoid decorative blobs, oversized gradients,
or effects that make the app feel like a landing page.

## Shapes

Use restrained rounded corners.

- Core cards and panels: 8px.
- Larger drop zones or transcript sheets: 12px when the surface is spacious.
- Icon buttons and compact controls: 6px to 8px.
- Pills: full radius only for tiny status or privacy badges.

Do not mix sharp and heavily rounded components in the same view.

## Components

**Drop Zone**

The drop zone is the first-run hero. It should include a clear icon, one direct
heading, supported formats in small text, and a privacy/local badge. During drag,
the whole surface should visibly respond.

**File Cards**

File cards show the filename first, then status or destination. Use status color
sparingly: queued is neutral, running is warning, done is teal, error is red.
Every completed card should have a reveal/open action.

**Transcript Preview**

The transcript is the reward state. It should use a readable text area or editor
surface with copy, export, and reveal actions nearby. Future speaker labels or
timestamps should be visually quiet and scannable.

**Buttons**

Use one primary button per screen state. Secondary actions are bordered buttons
or icon buttons with tooltips. Destructive actions stay icon-only when the label
is obvious, with a tooltip and accessible title.

**Status And Setup**

Technical status should be compact. Show "Ready", "Needs attention", or
"Transcribing locally" in the main UI. Put model names, auth paths, and runner
details behind disclosure unless an error requires them.

## Do's and Don'ts

- Do make the transcript or next user action the visual center.
- Do say "Private on this device" or "Files stay on this machine" instead of
  exposing implementation details.
- Do keep the app usable at 360px wide with no overlapping controls.
- Do use icons for actions like remove, reveal, copy, settings, and export.
- Don't lead with model IDs, Python paths, auth mechanisms, or RTX jargon.
- Don't make the app look like a dashboard when it is a file-to-transcript tool.
- Don't use giant hero copy, marketing sections, nested cards, or decorative
  gradient blobs.
- Don't use more than one primary accent in the same screen state.

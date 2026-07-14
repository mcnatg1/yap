import { describe, expect, it } from "vitest";

import { capturePhysicalShortcut } from "@/components/settings/shortcut-chord";

function key(
  code: string,
  modifiers: Partial<{
    altKey: boolean;
    ctrlKey: boolean;
    metaKey: boolean;
    repeat: boolean;
    shiftKey: boolean;
  }> = {},
) {
  return {
    altKey: false,
    code,
    ctrlKey: false,
    metaKey: false,
    repeat: false,
    shiftKey: false,
    ...modifiers,
  };
}

describe("physical shortcut capture", () => {
  it("normalizes physical codes in a stable modifier order", () => {
    expect(
      capturePhysicalShortcut(
        key("KeyV", { altKey: true, ctrlKey: true, shiftKey: true }),
      ),
    ).toEqual({ chord: "Ctrl+Shift+Alt+V", kind: "commit" });
    expect(
      capturePhysicalShortcut(key("Digit7", { altKey: true, ctrlKey: true })),
    ).toEqual({ chord: "Ctrl+Alt+7", kind: "commit" });
  });

  it("uses KeyboardEvent.code rather than a printable key value", () => {
    expect(
      capturePhysicalShortcut({
        ...key("KeyY", { ctrlKey: true }),
        key: "z",
      }),
    ).toEqual({ chord: "Ctrl+Y", kind: "commit" });
  });

  it("ignores repeats and modifier-only keydowns", () => {
    expect(
      capturePhysicalShortcut(key("KeyD", { ctrlKey: true, repeat: true })),
    ).toEqual({ kind: "ignore" });
    expect(capturePhysicalShortcut(key("ControlLeft", { ctrlKey: true }))).toEqual({
      kind: "ignore",
    });
  });

  it("treats Escape as an explicit cancel", () => {
    expect(capturePhysicalShortcut(key("Escape"))).toEqual({ kind: "cancel" });
  });

  it("rejects bare printable and unsupported physical keys", () => {
    expect(capturePhysicalShortcut(key("KeyD"))).toEqual({
      kind: "invalid",
      message: "Add Ctrl, Shift, or Alt.",
    });
    expect(capturePhysicalShortcut(key("Semicolon", { ctrlKey: true }))).toEqual({
      kind: "invalid",
      message: "That physical key is not supported.",
    });
  });

  it("rejects operating-system reserved chords before registration", () => {
    expect(capturePhysicalShortcut(key("F4", { altKey: true }))).toEqual({
      kind: "invalid",
      message: "That shortcut is reserved by Windows.",
    });
    expect(
      capturePhysicalShortcut(key("Escape", { ctrlKey: true, shiftKey: true })),
    ).toEqual({ kind: "cancel" });
    expect(capturePhysicalShortcut(key("KeyL", { metaKey: true }))).toEqual({
      kind: "invalid",
      message: "That shortcut is reserved by Windows.",
    });
    expect(
      capturePhysicalShortcut(key("Digit7", { ctrlKey: true, metaKey: true })),
    ).toEqual({
      kind: "invalid",
      message: "That shortcut is reserved by Windows.",
    });
    expect(capturePhysicalShortcut(key("F12", { ctrlKey: true }))).toEqual({
      kind: "invalid",
      message: "That shortcut is reserved by Windows.",
    });
  });
});

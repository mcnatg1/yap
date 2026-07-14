export type PhysicalShortcutInput = {
  altKey: boolean;
  code: string;
  ctrlKey: boolean;
  metaKey: boolean;
  repeat: boolean;
  shiftKey: boolean;
};

export type ShortcutCaptureResult =
  | { kind: "ignore" }
  | { kind: "cancel" }
  | { kind: "invalid"; message: string }
  | { kind: "commit"; chord: string };

const modifierCodes = new Set([
  "AltLeft",
  "AltRight",
  "ControlLeft",
  "ControlRight",
  "MetaLeft",
  "MetaRight",
  "ShiftLeft",
  "ShiftRight",
]);

export function capturePhysicalShortcut(
  event: PhysicalShortcutInput,
): ShortcutCaptureResult {
  if (event.repeat || modifierCodes.has(event.code)) return { kind: "ignore" };
  if (event.code === "Escape") return { kind: "cancel" };

  const key = physicalKeyName(event.code);
  if (!key) {
    return { kind: "invalid", message: "That physical key is not supported." };
  }

  const modifiers = [
    event.ctrlKey ? "Ctrl" : undefined,
    event.shiftKey ? "Shift" : undefined,
    event.altKey ? "Alt" : undefined,
    event.metaKey ? "Meta" : undefined,
  ].filter((modifier): modifier is string => Boolean(modifier));
  if (modifiers.length === 0) {
    return { kind: "invalid", message: "Add Ctrl, Shift, or Alt." };
  }

  const chord = [...modifiers, key].join("+");
  if (isWindowsReserved(event, key)) {
    return { kind: "invalid", message: "That shortcut is reserved by Windows." };
  }
  return { chord, kind: "commit" };
}

function isWindowsReserved(event: PhysicalShortcutInput, key: string) {
  return (
    event.metaKey ||
    key === "F12" ||
    (event.altKey && ["F4", "Space", "Tab"].includes(key))
  );
}

function physicalKeyName(code: string) {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F(?:[1-9]|1[0-2])$/.test(code)) return code;
  switch (code) {
    case "Backspace":
    case "Enter":
    case "Space":
    case "Tab":
      return code;
    default:
      return undefined;
  }
}

/**
 * A shortcut you press instead of spelling.
 *
 * Typing `Control+Alt+1` into a text box means knowing a spelling that is not
 * written down anywhere, getting the capitalisation right, and finding out it
 * was wrong only when the shortcut does not fire. This listens instead: press
 * the combination, and it is set. Escape backs out, Backspace clears it.
 *
 * Mouse buttons count, because a side button is the one key you can press
 * without letting go of the mouse. The press that starts the recording is not
 * the one that is recorded, because you press the button *after* the field, which is
 * what makes `Mouse1` assignable at all, since clicking the field is itself a
 * left click.
 *
 * A wheel notch counts too, as `WheelUp` or `WheelDown`. It is a direction
 * rather than a thing you hold, so it suits a macro, which runs on the press,
 * and the keyboard side is not consulted for it either.
 *
 * The names it produces are the ones the Rust side parses: `KeyA`/`Digit1`
 * become `A`/`1` because those are accepted too, everything else is passed
 * through from `KeyboardEvent.code`, whose names are the same vocabulary
 * (`ArrowUp`, `Space`, `PageUp`, `NumpadAdd`, …), and the mouse becomes
 * `Mouse1`–`Mouse5`, `WheelUp`, `WheelDown`. A combination it cannot parse comes
 * back from Rust as a message on the tab rather than silence.
 */

import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";

/** Keys that are not a shortcut on their own: they are part of one. */
const HOLD_KEYS = new Set([
  "Control",
  "Shift",
  "Alt",
  "AltGraph",
  "Meta",
  "CapsLock",
  "NumLock",
  "ScrollLock",
  "ContextMenu",
  "Fn",
  "OS",
]);

/** Written in this order, matching how the shortcut layer reports a key back. */
const MODIFIERS = [
  ["Control", "ctrlKey"],
  ["Alt", "altKey"],
  ["Shift", "shiftKey"],
  ["Meta", "metaKey"],
] as const;

/**
 * `MouseEvent.button` in the names Windows gives the buttons, so a mouse
 * shortcut means the same thing here as it does in a game: `Mouse1` is left,
 * `Mouse4` and `Mouse5` are the two on the side.
 */
const MOUSE_BUTTONS: Record<number, string> = {
  0: "Mouse1",
  1: "Mouse3",
  2: "Mouse2",
  3: "Mouse4",
  4: "Mouse5",
};

/** How each part of a stored shortcut is written out for a person. */
const PRETTY: Record<string, string> = {
  control: "Ctrl",
  ctrl: "Ctrl",
  shift: "Shift",
  alt: "Alt",
  meta: "Super",
  super: "Super",
  mouse1: "Mouse Left",
  mouse2: "Mouse Right",
  mouse3: "Mouse Middle",
  mouse4: "Mouse Back",
  mouse5: "Mouse Forward",
  wheelup: "Scroll Up",
  wheeldown: "Scroll Down",
};

type ModifierState = {
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
};

const modifierNames = (event: ModifierState): string[] =>
  MODIFIERS.filter(([, flag]) => Boolean(event[flag])).map(([name]) => name);

/** `Control+Mouse4` → `Ctrl + Mouse Back`. Empty stays empty. */
export function formatShortcut(value: string): string {
  if (!value.trim()) return "";
  return value
    .split("+")
    .map((part) => {
      const token = part.trim();
      return PRETTY[token.toLowerCase()] ?? token;
    })
    .join(" + ");
}

/**
 * The shortcut a key press stands for, or `null` if the press was a modifier
 * being held, or a key with no name to record.
 */
export function acceleratorFor(event: KeyboardEvent | ReactKeyboardEvent): string | null {
  if (!event.code || HOLD_KEYS.has(event.key) || MODIFIERS.some(([name]) => event.key === name)) {
    return null;
  }
  const code = event.code;
  const key = code.startsWith("Key")
    ? code.slice(3)
    : code.startsWith("Digit")
      ? code.slice(5)
      : code;
  // A layout that reports no code at all still has a printable key name.
  const named = key || (event.key.length === 1 ? event.key.toUpperCase() : "");
  if (!named) return null;
  return [...modifierNames(event), named].join("+");
}

/** The shortcut a mouse press stands for: the button, plus whatever is held. */
export function acceleratorForMouse(event: MouseEvent): string | null {
  const button = MOUSE_BUTTONS[event.button];
  if (!button) return null;
  return [...modifierNames(event), button].join("+");
}

/**
 * The shortcut a wheel notch stands for. A notch towards you scrolls down, and
 * `deltaY` is positive that way, which is the sense the stored names use. A
 * sideways wheel has no name here, so it is left alone.
 */
export function acceleratorForWheel(event: WheelEvent): string | null {
  if (event.deltaY === 0) return null;
  return [...modifierNames(event), event.deltaY < 0 ? "WheelUp" : "WheelDown"].join("+");
}

export function HotkeyField({
  label,
  note,
  value,
  onChange,
}: {
  label: string;
  note?: string;
  value: string;
  onChange: (value: string) => void;
}) {
  const [listening, setListening] = useState(false);
  const [held, setHeld] = useState<string[]>([]);
  const button = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!listening) return;

    const stop = () => {
      setListening(false);
      setHeld([]);
    };

    const onKeyDown = (event: KeyboardEvent) => {
      // Taking the press here, and on the way down, keeps the rest of the
      // window from acting on a shortcut that is only being recorded.
      event.preventDefault();
      event.stopPropagation();
      if (event.key === "Escape") {
        stop();
        button.current?.focus();
        return;
      }
      if (event.key === "Backspace" || event.key === "Delete") {
        onChange("");
        stop();
        return;
      }
      const accelerator = acceleratorFor(event);
      if (!accelerator) {
        // Only modifiers so far: show them so the press is visibly landing.
        setHeld(modifierNames(event));
        return;
      }
      onChange(accelerator);
      stop();
    };

    const onKeyUp = (event: KeyboardEvent) => {
      if (HOLD_KEYS.has(event.key)) setHeld(modifierNames(event));
    };

    // The recording starts on the press that opens it, so by the time this is
    // listening that press is already over: the next one is the answer.
    const onMouseDown = (event: MouseEvent) => {
      event.preventDefault();
      event.stopPropagation();
      const accelerator = acceleratorForMouse(event);
      if (!accelerator) return;
      onChange(accelerator);
      stop();
    };

    // A right button press would otherwise raise a menu over the recording.
    const onContextMenu = (event: MouseEvent) => {
      event.preventDefault();
      event.stopPropagation();
    };

    // Taken on the way up as well as captured, so the panel behind the recorder
    // does not scroll away while the wheel is being recorded.
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      event.stopPropagation();
      const accelerator = acceleratorForWheel(event);
      if (!accelerator) return;
      onChange(accelerator);
      stop();
    };

    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    window.addEventListener("mousedown", onMouseDown, true);
    window.addEventListener("contextmenu", onContextMenu, true);
    window.addEventListener("wheel", onWheel, { capture: true, passive: false });
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
      window.removeEventListener("mousedown", onMouseDown, true);
      window.removeEventListener("contextmenu", onContextMenu, true);
      window.removeEventListener("wheel", onWheel, { capture: true });
    };
  }, [listening, onChange]);

  const shown = listening
    ? held.length > 0
      ? `${held.join(" + ")} + …`
      : "press a key, a mouse button, or roll the wheel…"
    : formatShortcut(value) || "not set";

  return (
    <span className="hb__inline-field">
      <span className="hb__field-label">{label}</span>
      <button
        ref={button}
        type="button"
        className={`hb__field hb__hotkey${listening ? " hb__hotkey--live" : ""}${
          value || listening ? "" : " hb__hotkey--empty"
        }`}
        aria-label={label}
        aria-live="polite"
        title={note ?? "Press a key combination, a mouse button, or roll the wheel"}
        onMouseDown={() => {
          setHeld([]);
          setListening(true);
        }}
        onContextMenu={(event) => event.preventDefault()}
      >
        {shown}
      </button>
      {value && !listening && (
        <button
          type="button"
          className="hb__delete"
          aria-label={`Clear ${label}`}
          title="Clear"
          onClick={() => onChange("")}
        >
          ×
        </button>
      )}
    </span>
  );
}

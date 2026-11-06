import { useEffect, useState } from "react";
import { Trash2 } from "lucide-react";
import {
  MODE_LABELS,
  MODE_NOTES,
  mouseScanFromButton,
  scanFromCode,
  type MappingView,
  type Mode,
} from "@/bridge";
import { PressDepth } from "@/components/interior/press-depth";

const MODE_DOT: Record<Mode, string> = {
  OPPOSITE: "hb__mode-dot",
  OPPOSITE_NO_REPRESS: "hb__mode-dot hb__mode-dot--warn",
  NEUTRAL: "hb__mode-dot",
  REMAP: "hb__mode-dot hb__mode-dot--moss",
};

export function MappingTable({
  mappings,
  onDelete,
  onAdd,
  onPreset,
}: {
  mappings: MappingView[];
  onDelete: (index: number, mapping: MappingView) => void;
  onAdd: () => void;
  onPreset: (preset: "wasd" | "arrows") => void;
}) {
  if (mappings.length === 0) {
    return (
      <>
        <div className="hb__empty mat-well">
          <p className="hb__empty-title">No mappings in this profile yet</p>
          <p className="hb__empty-note">
            Add a mapping by hand, or start from a preset: both directions of W A S D
            (or the arrow keys) paired as<i> opposite</i>, which is what most games
            want for movement.
          </p>
          <div className="hb__actions" style={{ justifyContent: "center" }}>
            <PressDepth depth={3} tilt={5} onClick={() => onPreset("wasd")}>
              WASD preset
            </PressDepth>
            <PressDepth depth={3} tilt={5} onClick={() => onPreset("arrows")}>
              Arrow preset
            </PressDepth>
            <button type="button" className="appearance__reset" onClick={onAdd}>
              or add one
            </button>
          </div>
        </div>
        <ModeLegend />
      </>
    );
  }

  return (
    <>
      <div className="hb__table mat-panel">
        <div className="hb__thead">
          <span>Key</span>
          <span />
          <span>Opposite / output</span>
          <span>Mode</span>
          <span />
        </div>
        {mappings.map((mapping, index) => (
          <div className="hb__trow" key={`${mapping.mode}-${mapping.source}-${index}`}>
            <span className="kbd" title={mapping.source_label}>
              {mapping.source_label}
            </span>
            <span className="hb__arrow" aria-hidden="true">
              {mapping.mode === "REMAP" ? "→" : "⇄"}
            </span>
            <span className="kbd" title={mapping.dest_label}>
              {mapping.dest_label}
            </span>
            <span className="hb__mode" title={MODE_NOTES[mapping.mode]}>
              <span className={MODE_DOT[mapping.mode]} aria-hidden="true" />
              {MODE_LABELS[mapping.mode]}
            </span>
            <button
              type="button"
              className="hb__delete"
              aria-label={`Remove ${mapping.source_label} mapping`}
              onClick={() => onDelete(index, mapping)}
            >
              <Trash2 size={12} strokeWidth={1.7} aria-hidden="true" />
            </button>
          </div>
        ))}
      </div>
      <ModeLegend />
    </>
  );
}

function ModeLegend() {
  return (
    <div className="hb__legend">
      {(Object.keys(MODE_LABELS) as Mode[]).map((mode) => (
        <div className="hb__legend-item" key={mode}>
          <div className="hb__legend-name">{MODE_LABELS[mode]}</div>
          <div className="hb__legend-note">{MODE_NOTES[mode]}</div>
        </div>
      ))}
    </div>
  );
}

/** A key cap that binds the next key pressed while it is listening. */
export function KeyBinder({
  scan,
  onChange,
  label,
  mouse = false,
}: {
  scan: number | null;
  onChange: (scan: number) => void;
  label: string;
  /** Accept a mouse button as well as a key. */
  mouse?: boolean;
}) {
  const [listening, setListening] = useState(false);
  // A press that has no scan code behind it, said out loud rather than ignored.
  const [missed, setMissed] = useState(false);

  useEffect(() => {
    if (!listening) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setListening(false);
        return;
      }
      const next = scanFromCode(event.code);
      if (next === null) {
        // Dropping the press left the cap showing the key it already held, 
        // Left Ctrl on a new chord, which reads as the keyboard not being
        // heard at all. The press landed, it just has nothing to bind to.
        event.preventDefault();
        event.stopPropagation();
        setMissed(true);
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      setMissed(false);
      onChange(next);
      setListening(false);
    };
    // A button is caught before a key is, because the two never arrive from the
    // same press: the cap listens on the window, so a click anywhere in it is
    // the answer, and the webview's own right-click menu is suppressed while it
    // waits, otherwise a right button would open a menu instead of binding.
    const onDown = (event: MouseEvent) => {
      const next = mouseScanFromButton(event.button);
      if (next === null) return;
      event.preventDefault();
      event.stopPropagation();
      onChange(next);
      setListening(false);
    };
    const onMenu = (event: MouseEvent) => {
      if (listening) event.preventDefault();
    };
    window.addEventListener("keydown", onKey, true);
    if (mouse) {
      window.addEventListener("mousedown", onDown, true);
      window.addEventListener("contextmenu", onMenu, true);
    }
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("contextmenu", onMenu, true);
    };
  }, [listening, onChange, mouse]);

  return (
    <button
      type="button"
      aria-label={label}
      className={`kbd${listening ? " kbd--listening" : ""}`}
      style={{ cursor: "pointer", minWidth: 92 }}
      onClick={() => {
        setListening((current) => !current);
        setMissed(false);
      }}
    >
      {listening
        ? missed
          ? "that key is not one this can hold…"
          : mouse
            ? "press a key or click…"
            : "press a key…"
        : scan === null
          ? "pick a key"
          : label}
    </button>
  );
}

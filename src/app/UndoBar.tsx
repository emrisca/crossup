import { Undo2 } from "lucide-react";
import { clearUndo, useUndoEntry } from "./undo";

/**
 * The undo bar: what was deleted, and the one press that puts it back.
 *
 * It floats above the footer rather than sitting in the layout, so a delete
 * never pushes the page around while it is still on screen.
 */
export function UndoBar({ onUndo }: { onUndo: () => void }) {
  const entry = useUndoEntry();
  if (!entry) return null;

  return (
    <div className="hb__undo" role="status" aria-live="polite">
      <span className="hb__undo-text">{entry.message}</span>
      <button type="button" className="hb__undo-go" onClick={onUndo}>
        <Undo2 size={13} strokeWidth={1.7} aria-hidden="true" />
        Undo
      </button>
      <button
        type="button"
        className="hb__delete"
        aria-label="Dismiss the undo"
        onClick={clearUndo}
      >
        ×
      </button>
    </div>
  );
}

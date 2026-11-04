import { useSyncExternalStore } from "react";

/**
 * One undo, offered for a few seconds.
 *
 * A single slot is the whole design: the bar shows the last destructive edit,
 * so pressing Undo twice is not a thing a person can try to do. The state lives
 * outside React because the bar sits in the shell and the edits happen in the
 * tabs, and threading a callback through five tabs to reach one bar would be
 * more plumbing than the feature is worth.
 */
export type UndoEntry = {
  message: string;
  token: number;
  /** Puts the item back and restores whatever went with it. */
  undo: () => Promise<void> | void;
};

const listeners = new Set<() => void>();
let entry: UndoEntry | null = null;
let token = 0;
let timer: ReturnType<typeof setTimeout> | undefined;

function emit() {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Offer the edit that just happened as something to take back.
 *
 * The action belongs to the tab that did the deleting, not to this store: a
 * macro also has to become the selected macro again, a clicker row has to go
 * back into the draft, and only that tab knows how.
 */
export function pushUndo(message: string, undo: () => Promise<void> | void) {
  if (timer) clearTimeout(timer);
  entry = { message, token: ++token, undo };
  timer = setTimeout(clearUndo, 12000);
  emit();
}

export function clearUndo() {
  if (timer) {
    clearTimeout(timer);
    timer = undefined;
  }
  if (!entry) return;
  entry = null;
  emit();
}

export function useUndoEntry(): UndoEntry | null {
  return useSyncExternalStore(subscribe, () => entry, () => null);
}

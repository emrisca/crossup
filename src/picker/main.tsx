/**
 * The region picker.
 *
 * This page runs in its own window, stretched over the whole virtual desktop and
 * transparent, so a drag here is an ordinary mouse drag over whatever is behind
 * it. Nothing is hooked and nothing is captured from other windows: the press,
 * the move and the release all happen in this document, which is also why the
 * rectangle is exact rather than inferred from two timed cursor samples.
 *
 * It only reports the drag. What the rectangle means, a zone, a click point,
 * a replacement for an existing row, is decided by the window that opened it.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./picker.css";

type Rect = { x: number; y: number; width: number; height: number };
type Context = { purpose: string; index: number; zones: Rect[]; points: Rect[] };

/** A drag shorter than this in both directions reads as a click. */
const CLICK_SLOP = 4;
/** What a click becomes when a zone was asked for, since a zone needs an area. */
const DEFAULT_BOX = 140;

const clamp = (value: number, low: number, high: number) =>
  Math.min(Math.max(value, low), high);

/** The rectangle between two corners, in either direction. */
function between(anchor: { x: number; y: number }, at: { x: number; y: number }): Rect {
  return {
    x: Math.min(anchor.x, at.x),
    y: Math.min(anchor.y, at.y),
    width: Math.abs(at.x - anchor.x),
    height: Math.abs(at.y - anchor.y),
  };
}

/** A tiny drag, expanded around the point so it has an area to work with. */
function asBox(rect: Rect): Rect {
  const size = DEFAULT_BOX;
  const centreX = rect.x + rect.width / 2;
  const centreY = rect.y + rect.height / 2;
  return {
    x: Math.round(centreX - size / 2),
    y: Math.round(centreY - size / 2),
    width: size,
    height: size,
  };
}

function Picker() {
  /** `undefined` while loading, `null` when nothing asked for a rectangle. */
  const [request, setRequest] = useState<Context | null | undefined>(undefined);
  const [anchor, setAnchor] = useState<{ x: number; y: number } | null>(null);
  const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null);
  const [sending, setSending] = useState(false);
  const sent = useRef(false);

  // Fetched rather than pushed: the request is already stored when the window
  // is shown, and on the first open the page is still loading when that
  // happens, so there is nothing to lose by asking again.
  const load = useCallback(() => {
    sent.current = false;
    setSending(false);
    setAnchor(null);
    setCursor(null);
    void invoke<Context | null>("picker_context")
      .then(setRequest)
      .catch(() => setRequest(null));
  }, []);

  useEffect(() => {
    load();
    // Every later open reuses this window, so the page has to be told. Outside
    // the app there is no event to listen for, and no reason to complain.
    const pending = listen("picker-open", () => load()).catch(() => null);
    return () => {
      void pending.then((stop) => stop?.());
    };
  }, [load]);

  const cancel = useCallback(() => {
    if (sending) return;
    setSending(true);
    void invoke("picker_cancel").catch(() => undefined);
  }, [sending]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        cancel();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [cancel]);

  const finish = useCallback((rect: Rect) => {
    if (sent.current) return;
    sent.current = true;
    setSending(true);
    // A click is a legitimate answer for a click point: no drag, no area. A
    // zone needs one, so a click there arrives as a box instead.
    void invoke("picker_finish", { drag: rect }).catch(() => undefined);
  }, []);

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    // Right-click and middle-click mean "never mind", the same as Escape.
    if (event.button !== 0) {
      event.preventDefault();
      cancel();
      return;
    }
    if (!request) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    setAnchor({ x: event.clientX, y: event.clientY });
    setCursor({ x: event.clientX, y: event.clientY });
  };

  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    setCursor({ x: event.clientX, y: event.clientY });
  };

  const onPointerUp = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!request || !anchor) return;
    event.currentTarget.releasePointerCapture(event.pointerId);
    const drawn = between(anchor, { x: event.clientX, y: event.clientY });
    const purpose = request.purpose;
    const small = drawn.width <= CLICK_SLOP && drawn.height <= CLICK_SLOP;
    const box =
      purpose === "zone" && (small || drawn.width < 8 || drawn.height < 8)
        ? asBox(drawn)
        : drawn;
    setAnchor(null);
    finish(box);
  };

  if (request === undefined) return null;

  if (request === null) {
    return (
      <div className="picker picker--idle">
        <div className="picker__hint">
          <span>Nothing is being picked.</span>
          <button type="button" className="picker__button" onClick={cancel}>
            Close
          </button>
        </div>
      </div>
    );
  }

  const box = anchor && cursor ? between(anchor, cursor) : null;
  const dragging = Boolean(anchor && box && (box.width > CLICK_SLOP || box.height > CLICK_SLOP));
  const what =
    request.purpose === "zone"
      ? request.index < 0
        ? "Drag to draw a new zone"
        : `Drag over where zone ${request.index + 1} should go`
      : "Click where the click point should land";

  return (
    <div
      className="picker"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onContextMenu={(event) => {
        event.preventDefault();
        cancel();
      }}
    >
      <div className={`picker__dim${dragging ? " picker__dim--off" : ""}`} />

      {/* What is already placed, so a new rectangle is drawn in context. */}
      {request.zones.map((zone, index) => (
        <div
          className="picker__zone"
          key={`zone-${index}`}
          style={{
            left: zone.x,
            top: zone.y,
            width: Math.max(zone.width, 1),
            height: Math.max(zone.height, 1),
          }}
        >
          <span className="picker__badge">{index + 1}</span>
        </div>
      ))}
      {request.points.map((point, index) => (
        <div className="picker__point" key={`point-${index}`} style={{ left: point.x, top: point.y }} />
      ))}

      {cursor && !anchor && (
        <>
          <div className="picker__guide picker__guide--x" style={{ left: cursor.x }} />
          <div className="picker__guide picker__guide--y" style={{ top: cursor.y }} />
        </>
      )}

      {box && (
        <div
          className={`picker__box${dragging ? "" : " picker__box--tiny"}`}
          style={{
            left: box.x,
            top: box.y,
            width: Math.max(box.width, 1),
            height: Math.max(box.height, 1),
          }}
        >
          <span className="picker__size">
            {Math.round(box.width)} × {Math.round(box.height)}
          </span>
        </div>
      )}

      <div className="picker__hint">
        <span className="picker__what">{what}</span>
        <span className="picker__note">
          {request.purpose === "zone" ? "Click for a square · " : ""}Esc to cancel
        </span>
        <button type="button" className="picker__button" onClick={cancel}>
          Cancel
        </button>
      </div>
    </div>
  );
}

createRoot(document.getElementById("picker")!).render(<Picker />);

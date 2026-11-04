"use client";

import { useCallback, useEffect, useRef, useState } from "react";

export type UsePressDepthOptions = {
  disabled?: boolean;
  onPressStart?: () => void;
  onPressEnd?: () => void;
};

export type PressOrigin = { x: number; y: number };

export type UsePressDepthResult = {
  pressed: boolean;
  origin: PressOrigin | null;
  ref: (node: HTMLElement | null) => void;
  bind: {
    onPointerDown: (event: React.PointerEvent) => void;
    onKeyDown: (event: React.KeyboardEvent) => void;
    onKeyUp: (event: React.KeyboardEvent) => void;
    onBlur: () => void;
  };
};

export function usePressDepth(
  options: UsePressDepthOptions = {},
): UsePressDepthResult {
  const { disabled = false, onPressStart, onPressEnd } = options;

  const [pressed, setPressed] = useState(false);
  const [tracking, setTracking] = useState(false);
  const [origin, setOrigin] = useState<PressOrigin | null>(null);

  const node = useRef<HTMLElement | null>(null);
  const pointer = useRef<number | null>(null);
  const down = useRef(false);

  const began = useRef(onPressStart);
  began.current = onPressStart;
  const ended = useRef(onPressEnd);
  ended.current = onPressEnd;

  const setDown = useCallback((next: boolean) => {
    if (down.current === next) return;
    down.current = next;
    setPressed(next);
    if (next) began.current?.();
    else ended.current?.();
  }, []);

  const stop = useCallback(() => {
    pointer.current = null;
    setTracking(false);
    setOrigin(null);
    setDown(false);
  }, [setDown]);

  useEffect(() => {
    if (!tracking) return;

    const contains = (event: PointerEvent) => {
      const el = node.current;
      if (!el) return false;
      const r = el.getBoundingClientRect();
      return (
        event.clientX >= r.left &&
        event.clientX <= r.right &&
        event.clientY >= r.top &&
        event.clientY <= r.bottom
      );
    };

    const move = (event: PointerEvent) => {
      if (event.pointerId !== pointer.current) return;
      setDown(contains(event));
    };
    const lift = (event: PointerEvent) => {
      if (event.pointerId !== pointer.current) return;
      stop();
    };
    const bail = () => stop();
    const hidden = () => {
      if (document.hidden) stop();
    };

    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", lift);
    window.addEventListener("pointercancel", lift);
    window.addEventListener("blur", bail);
    document.addEventListener("visibilitychange", hidden);

    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", lift);
      window.removeEventListener("pointercancel", lift);
      window.removeEventListener("blur", bail);
      document.removeEventListener("visibilitychange", hidden);
    };
  }, [tracking, setDown, stop]);

  useEffect(() => {
    if (disabled) stop();
  }, [disabled, stop]);

  const ref = useCallback((next: HTMLElement | null) => {
    node.current = next;
  }, []);

  const bind = {
    onPointerDown: (event: React.PointerEvent) => {
      if (disabled) return;
      if (event.pointerType === "mouse" && event.button !== 0) return;
      const r = event.currentTarget.getBoundingClientRect();
      setOrigin({
        x: Math.max(-1, Math.min(1, ((event.clientX - r.left) / r.width) * 2 - 1)),
        y: Math.max(-1, Math.min(1, ((event.clientY - r.top) / r.height) * 2 - 1)),
      });
      pointer.current = event.pointerId;
      setTracking(true);
      setDown(true);
    },
    onKeyDown: (event: React.KeyboardEvent) => {
      if (disabled || event.repeat) return;
      if (event.key === " " || event.key === "Enter") setDown(true);
    },
    onKeyUp: (event: React.KeyboardEvent) => {
      if (event.key === " " || event.key === "Enter" || event.key === "Escape") {
        setDown(false);
      }
    },
    onBlur: () => stop(),
  };

  return { pressed, origin, ref, bind };
}

export type PressDepthProps = {
  children: React.ReactNode;
  /** How far the face sinks while held. */
  depth?: number;
  /** Retired: the face no longer tilts toward the pointer. Kept so the call
   *  sites that still pass it keep compiling. */
  tilt?: number;
  disabled?: boolean;
  type?: "button" | "submit" | "reset";
  onClick?: React.MouseEventHandler<HTMLButtonElement>;
  className?: string;
  "aria-label"?: string;
};

export function PressDepth({
  children,
  depth = 2,
  disabled = false,
  type = "button",
  onClick,
  className = "",
  "aria-label": ariaLabel,
}: PressDepthProps) {
  const { pressed, ref, bind } = usePressDepth({ disabled });

  // One flat face: the fill, the radius and the press travel all live in
  // `.hb__press` in app.css, beside the rest of the app's chrome.
  return (
    <button
      ref={ref}
      type={type}
      disabled={disabled}
      aria-label={ariaLabel}
      data-pressed={pressed ? "" : undefined}
      onClick={onClick}
      style={{
        transform: pressed ? `translateY(${depth}px)` : undefined,
        touchAction: "manipulation",
        WebkitTapHighlightColor: "transparent",
      }}
      className={`hb__press ${className}`.trim()}
      {...bind}
    >
      {children}
    </button>
  );
}

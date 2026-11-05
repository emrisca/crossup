import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";

/** The fields of `replay_state` this window reads. */
type ReplayState = {
  playing: boolean;
  speed: number;
};

/**
 * The badge: one line saying the desktop is being driven, and how fast.
 *
 * The window itself is shown and hidden by the replay thread, so this only has
 * to know the pace. It asks the engine rather than being pushed a value, so a
 * window that was hidden and shown again reports the run that is going now.
 */
function Badge() {
  const [speed, setSpeed] = useState(1);

  useEffect(() => {
    let live = true;
    const read = async () => {
      try {
        const state = await invoke<ReplayState>("replay_state");
        if (live && state.playing) setSpeed(state.speed);
      } catch {
        // A window with no engine behind it simply has nothing to report.
      }
    };
    void read();
    const id = window.setInterval(() => void read(), 400);
    return () => {
      live = false;
      window.clearInterval(id);
    };
  }, []);

  return (
    <div
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 7,
        margin: 4,
        padding: "5px 10px",
        borderRadius: 999,
        background: "rgba(18, 18, 20, 0.86)",
        color: "#f4f4f5",
        letterSpacing: ".01em",
        whiteSpace: "nowrap",
      }}
    >
      <span
        style={{ width: 6, height: 6, borderRadius: 999, background: "#4ade80" }}
      />
      Replaying{speed === 1 ? "" : ` ${speed}×`}
    </div>
  );
}

const host = document.getElementById("badge");
if (host) createRoot(host).render(<Badge />);

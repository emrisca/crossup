import { useEffect, useState } from "react";
import { PressDepth } from "@/components/interior/press-depth";
import { SegmentedControl } from "@/components/interior/segmented-control";
import {
  call,
  discardRecording,
  replayState,
  startRecording,
  startReplay,
  stopRecording,
  stopReplay,
  type ReplayStateView,
  type View,
} from "@/bridge";
import { Card, Line } from "./AutoClickerTab";
import { Switch } from "./Dialogs";
import { HotkeyField } from "./HotkeyField";
import { NumberField } from "./NumberField";
import { pushUndo } from "./undo";

const STATUS: Record<ReplayStateView["status"], string> = {
  idle: "Idle",
  recording: "Recording",
  replaying: "Replaying",
  done: "Finished",
  stopped: "Stopped",
};

/** A length of time, written the way a recording is described. */
const seconds = (ms: number): string =>
  ms >= 60_000 ? `${(ms / 60_000).toFixed(1)} min` : `${(ms / 1_000).toFixed(1)} s`;

/**
 * The replayer: record what the person does, then do it again.
 *
 * Two things make this tab different from the others. A recording takes over
 * the desktop and hides this window while it runs, so the shortcut is the way
 * back rather than a convenience; and what the run is doing arrives from
 * `replay_state` on a timer, because the window is not there to be told.
 */
export function ReplayerTab({
  view,
  run,
  onError,
}: {
  view: View;
  run: (command: string, args?: Record<string, unknown>) => Promise<void>;
  onError: (message: string | null) => void;
}) {
  const [state, setState] = useState<ReplayStateView | null>(null);
  const [name, setName] = useState("");
  const options = view.replay;
  const chosen =
    options.recordings.find((entry) => entry.id === options.selected) ?? null;

  useEffect(() => {
    let live = true;
    const tick = async () => {
      try {
        const next = await replayState();
        if (live) setState(next);
      } catch {
        // A preview without the engine simply has nothing to report.
      }
    };
    void tick();
    const id = window.setInterval(() => void tick(), 250);
    return () => {
      live = false;
      window.clearInterval(id);
    };
  }, []);

  const recording = state?.recording ?? false;
  const playing = state?.playing ?? false;
  const busy = recording || playing;
  const noRecordKey = !options.recordHotkey.trim();

  const toggleRecord = () => {
    onError(null);
    void (recording ? stopRecording() : startRecording())
      .then(setState)
      .catch((reason) => onError(String(reason)));
  };

  const toggleReplay = () => {
    onError(null);
    void (playing ? stopReplay() : startReplay(options.selected || undefined))
      .then(setState)
      .catch((reason) => onError(String(reason)));
  };

  const saveOptions = (patch: {
    recordHotkey?: string;
    replayHotkey?: string;
    repeat?: number | null;
  }) => {
    void run("save_replay_options", {
      record_hotkey: patch.recordHotkey ?? options.recordHotkey,
      replay_hotkey: patch.replayHotkey ?? options.replayHotkey,
      repeat: patch.repeat === undefined ? options.repeat : patch.repeat,
    });
  };

  const save = () => {
    onError(null);
    void run("save_recording", { name }).then(() => setName(""));
  };

  return (
    <div className="hb__split">
      <section className="hb__page">
        <div className="hb__head">
          <div style={{ minWidth: 0 }}>
            <h1 className="hb__title">Replayer</h1>
            <p className="hb__sub">
              {chosen
                ? `${chosen.name} · ${chosen.events} actions · ${seconds(chosen.durationMs)}`
                : "no recordings yet"}
            </p>
          </div>
          <span className="hb__spacer" />
          <span className={`hb__pill${busy ? " hb__pill--on" : ""}`}>
            <span
              className={`hb__gear-dot${busy ? "" : " hb__gear-dot--off"}`}
              aria-hidden="true"
            />
            {state ? STATUS[state.status] : "Idle"}
          </span>
        </div>

        <div className="hb__actions">
          {recording ? (
            <PressDepth depth={3} tilt={5} className="hb__press--flag" onClick={toggleRecord}>
              Stop recording
            </PressDepth>
          ) : (
            <PressDepth
              depth={3}
              tilt={5}
              disabled={!view.replay_enabled || busy || noRecordKey}
              onClick={() => {
                if (view.replay_enabled && !busy && !noRecordKey) toggleRecord();
              }}
            >
              Record
            </PressDepth>
          )}
          {playing ? (
            <PressDepth depth={3} tilt={5} className="hb__press--flag" onClick={toggleReplay}>
              Stop replay
            </PressDepth>
          ) : (
            <PressDepth
              depth={3}
              tilt={5}
              disabled={!view.replay_enabled || !chosen || busy}
              onClick={() => {
                if (view.replay_enabled && chosen && !busy) toggleReplay();
              }}
            >
              Replay
            </PressDepth>
          )}
          <span className="hb__field-label">Replayer</span>
          <Switch
            checked={view.replay_enabled}
            label="Replayer enabled"
            onChange={(enabled) => run("set_tool_enabled", { tool: "replay", enabled })}
          />
        </div>

        {noRecordKey && (
          <p className="hb__warn">
            Set a record shortcut first. Recording hides this window, so the shortcut is
            the only way to stop it.
          </p>
        )}
        {state?.note && <p className="hb__warn">{state.note}</p>}

        <Card
          title="Shortcuts"
          note="Press a shortcut to start, and press it again to stop. Neither shortcut is written into a recording, so a run that ends with the chord replays as recorded."
        >
          <div className="hb__rows">
            <Line
              label="Record"
              note="Starts a recording and hides the window. Press it again to stop and bring Crossup back."
            >
              <HotkeyField
                label="Record shortcut"
                value={options.recordHotkey}
                onChange={(recordHotkey) => saveOptions({ recordHotkey })}
              />
            </Line>
            <Line label="Replay" note="Plays the selected recording, and stops it again.">
              <HotkeyField
                label="Replay shortcut"
                value={options.replayHotkey}
                onChange={(replayHotkey) => saveOptions({ replayHotkey })}
              />
            </Line>
            <Line
              label="Repeat"
              note="Forever replays until the replay shortcut stops it. Times replays the number you set and stops."
            >
              <SegmentedControl
                label="Replays"
                options={[
                  { value: "forever", label: "Forever" },
                  { value: "times", label: "Times" },
                ]}
                value={options.repeat === null ? "forever" : "times"}
                onValueChange={(value) =>
                  saveOptions({ repeat: value === "forever" ? null : (options.repeat ?? 3) })
                }
              />
              {options.repeat !== null && (
                <>
                  <NumberField
                    label="How many times to replay"
                    value={options.repeat}
                    onValueChange={(repeat) => saveOptions({ repeat: Math.max(1, repeat) })}
                  />
                  <span className="hb__field-label">times</span>
                </>
              )}
            </Line>
          </div>
        </Card>

        {state?.pending && (
          <Card
            title="Save this recording"
            note={`${state.pending.events} actions over ${seconds(state.pending.durationMs)}. Nothing is kept until it has a name.`}
          >
            <div className="hb__row-actions" style={{ marginTop: 0 }}>
              <input
                className="hb__field"
                aria-label="Recording name"
                placeholder="Name this recording"
                value={name}
                onChange={(event) => setName(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") save();
                }}
              />
              <button type="button" className="hb__mini" disabled={!name.trim()} onClick={save}>
                Save
              </button>
              <button
                type="button"
                className="hb__mini"
                onClick={() => {
                  onError(null);
                  void discardRecording()
                    .then(setState)
                    .catch(() => undefined);
                }}
              >
                Discard
              </button>
            </div>
          </Card>
        )}

        <Card
          lead
          title="Recordings"
          note="Recordings live in settings.json with the rest of the settings, so copying that file carries them to another machine."
        >
          <div className="hb__recordings">
            {options.recordings.length === 0 && (
              <p className="hb__card-note hb__dim" style={{ margin: 0 }}>
                Nothing recorded yet.
              </p>
            )}
            {options.recordings.map((entry) => (
              <div
                className="hb__recording"
                key={entry.id}
                aria-current={entry.id === options.selected}
              >
                <button
                  type="button"
                  className="hb__recording-pick"
                  onClick={() => run("select_recording", { id: entry.id })}
                >
                  <span className="hb__recording-name">{entry.name}</span>
                  <span className="hb__recording-meta">
                    {entry.events} actions · {seconds(entry.durationMs)}
                  </span>
                </button>
                <button
                  type="button"
                  className="hb__delete"
                  aria-label={`Delete ${entry.name}`}
                  disabled={busy}
                  onClick={() => {
                    void run("delete_recording", { id: entry.id });
                    pushUndo(`Deleted the recording "${entry.name}"`, () =>
                      run("undo_delete"),
                    );
                  }}
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        </Card>
      </section>

      <aside className="hb__side">
        <Card title="This session" tight>
          <div className="hb__status-row">
            <span className={`hb__pill${busy ? " hb__pill--on" : ""}`}>
              {state ? STATUS[state.status] : "Idle"}
            </span>
            <span className="hb__field-label">
              {recording
                ? "the window is hidden"
                : playing
                  ? `pass ${state?.pass ?? 1} of ${state?.passes ?? "∞"}`
                  : " "}
            </span>
          </div>

          <Stat value={String(state?.events ?? 0)} label="Actions captured" />
          <Stat value={seconds(state?.durationMs ?? 0)} label="Length" />
          <Stat
            value={
              state && state.total > 0 ? `${state.played} / ${state.total}` : "0"
            }
            label="Actions replayed"
          />

          <p className="hb__card-note" style={{ marginTop: 8 }}>
            {recording
              ? "Recording. Press the record shortcut to stop. The window comes back with the recording waiting for a name."
              : playing
                ? "Replaying. The replay shortcut stops it at any point."
                : state?.status === "done"
                  ? "The last replay finished every pass it was asked for."
                  : state?.status === "stopped"
                    ? "The last run was stopped before it finished."
                    : chosen
                      ? `Ready to replay "${chosen.name}".`
                      : "Record something first."}
          </p>
        </Card>

        <Card title="While it runs" tight>
          <p className="hb__card-note">
            Recording hides this window so you can reach the program underneath, and so
            the window does not appear in the recording.
          </p>
          <p className="hb__card-note">
            The shortcut that ends a recording is never written into it, so a recording
            that ends on the chord does not replay its
            own stop.
          </p>
          <p className="hb__card-note">
            Only one run happens at a time. A replay shares the run slot with the clicker,
            the typer and macros, so starting it while one of them runs is refused instead
            of fighting over the cursor.
          </p>
          <p className="hb__card-note">
            A replay presses what was recorded at the recorded pace, so a
            recording made against one window clicks on whatever is in that spot now.
          </p>
        </Card>

      </aside>
    </div>
  );
}

function Stat({ value, label }: { value: string; label: string }) {
  return (
    <div className="hb__stat">
      <span className="hb__stat-value">{value}</span>
      <span className="hb__stat-label">{label}</span>
    </div>
  );
}

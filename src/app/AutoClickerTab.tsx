import { useCallback, useEffect, useRef, useState } from "react";
import { PressDepth } from "@/components/interior/press-depth";
import { SegmentedControl } from "@/components/interior/segmented-control";
import {
  call,
  DEFAULT_CLICKER,
  on,
  pickRegion,
  PICK_CANCELLED,
  REGION_PICKED,
  type Activity,
  type ClickerSettings,
  type ClickPoint,
  type Guard,
  type PickResult,
  type StopZone,
  type View,
} from "@/bridge";
import { Switch } from "./Dialogs";
import { HotkeyField } from "./HotkeyField";
import { KeyBinder } from "./Mappings";
import { NumberField } from "./NumberField";
import { pushUndo } from "./undo";

const UNIT_MS: Record<ClickerSettings["rateUnit"], number> = {
  second: 1_000,
  minute: 60_000,
  hour: 3_600_000,
  day: 86_400_000,
};

const UNIT_NAME: Record<ClickerSettings["rateUnit"], string> = {
  second: "second",
  minute: "minute",
  hour: "hour",
  day: "day",
};

/** Clicks per second, counted after the double-click doubling. */
export function clicksPerSecond(settings: ClickerSettings): number {
  return (
    ((settings.rate || 0) * (settings.doubleClick ? 2 : 1) * 1_000) /
    UNIT_MS[settings.rateUnit]
  );
}

/**
 * The engine's own interval between cycles, in milliseconds.
 *
 * This is the number the duty cycle is a share of, so it is the one the two
 * sliders have to report against. Note it is not the same as the gap between
 * clicks when a double click is on: that halves the interval by putting two
 * clicks in each cycle, which is why `clicksPerSecond` doubles.
 */
export function cycleMs(settings: ClickerSettings): number {
  if (!(settings.rate > 0)) return 0;
  return UNIT_MS[settings.rateUnit] / settings.rate;
}

/** A length of time, written the way its largest part reads. */
const duration = (ms: number): string => {
  if (!Number.isFinite(ms) || ms <= 0) return "0 ms";
  if (ms >= 60_000) return `${(ms / 60_000).toFixed(1)} min`;
  if (ms >= 1_000) return `${(ms / 1_000).toFixed(1)} s`;
  return ms >= 10 ? `${Math.round(ms)} ms` : `${ms.toFixed(1)} ms`;
};

/** The gap between clicks at a rate, for the readout above the run. */
const interval = (cps: number): string => (cps > 0 ? duration(1_000 / cps) : "-");

export function AutoClickerTab({
  view,
  activity,
  run,
  onError,
}: {
  view: View;
  activity: Activity | null;
  run: (command: string, args?: Record<string, unknown>) => Promise<void>;
  onError: (message: string | null) => void;
}) {
  const [draft, setDraft] = useState<ClickerSettings>(view.clicker);
  /**
   * What the picker is drawing, while it is open. The overlay owns the mouse
   * until it answers, so this only decides what the buttons say.
   */
  const [drawing, setDrawing] = useState<{
    purpose: "zone" | "point";
    index: number;
  } | null>(null);
  const [processes, setProcesses] = useState<string[]>([]);
  const [program, setProgram] = useState("");
  const sent = useRef(JSON.stringify(view.clicker));
  const timer = useRef<number | null>(null);

  // The settings live on the Rust side; this draft is the editing copy. If they
  // change somewhere else, adopt the new ones rather than fight over them.
  useEffect(() => {
    const incoming = JSON.stringify(view.clicker);
    if (incoming === sent.current) return;
    sent.current = incoming;
    setDraft(view.clicker);
  }, [view.clicker]);

  const update = useCallback(
    (patch: Partial<ClickerSettings>) => {
      const next = { ...draft, ...patch };
      setDraft(next);
      if (timer.current) window.clearTimeout(timer.current);
      // Dragging a slider produces a stream of changes; one write after it
      // settles is enough, and it keeps the shortcut re-registration rare.
      timer.current = window.setTimeout(() => {
        sent.current = JSON.stringify(next);
        void run("save_clicker", { clicker: next });
      }, 300);
    },
    [draft, run],
  );

  const patchZone = useCallback(
    (index: number, patch: Partial<StopZone>) => {
      update({
        stopZones: draft.stopZones.map((zone, at) =>
          at === index ? { ...zone, ...patch } : zone,
        ),
      });
    },
    // update already closes over the draft, so this is rebuilt with it.
    [draft.stopZones, update],
  );

  /**
   * Open the picker over the desktop and let the rectangle be dragged out.
   *
   * The zones and points already placed travel with the request, so the new one
   * is drawn against them rather than blind, and `index` says which row the
   * answer belongs to, with `-1` for a new one.
   */
  const draw = useCallback(
    (purpose: "zone" | "point", index: number) => {
      onError(null);
      setDrawing({ purpose, index });
      void pickRegion({
        purpose,
        index,
        zones: draft.stopZones.map(({ x, y, width, height }) => ({ x, y, width, height })),
        points: draft.points.map(({ x, y }) => ({ x, y, width: 0, height: 0 })),
      }).catch((reason) => {
        setDrawing(null);
        onError(String(reason));
      });
    },
    [draft.points, draft.stopZones, onError],
  );

  const running = activity?.running === "clicker";
  const cps = clicksPerSecond(draft);
  const tooFast = cps > 1_000;
  const cycle = cycleMs(draft);

  /**
   * Starting writes first. The run takes one copy of the settings when it
   * begins, so a slider moved and started inside the write delay below would
   * otherwise run the previous value for the whole run.
   */
  const start = () => {
    onError(null);
    if (timer.current) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    sent.current = JSON.stringify(draft);
    void run("save_clicker", { clicker: draft }).then(() => run("start_clicker"));
  };
  const stop = () => {
    void call("stop_activity").catch(() => undefined);
  };

  /**
   * What to do with a rectangle once one is drawn. Held in a ref so the
   * subscription below is made once, rather than on every keystroke.
   */
  const applyPick = useCallback(
    (result: PickResult) => {
      setDrawing(null);
      // The macro builder picks spots through the same overlay, and its answers
      // are none of this tab's business.
      if (result.purpose !== "point" && result.purpose !== "zone") return;
      if (result.purpose === "point") {
        // A dragged area places its point at the centre; a plain click already
        // is a point, with the centre and the corner being the same pixel.
        const x = Math.round(result.x + result.width / 2);
        const y = Math.round(result.y + result.height / 2);
        update({
          points: [
            ...draft.points,
            { x, y, clicks: 1, radius: draft.humanize ? 2 : 0 },
          ],
        });
        return;
      }
      const zone = {
        x: result.x,
        y: result.y,
        width: Math.max(1, result.width),
        height: Math.max(1, result.height),
        action: "stop" as const,
      };
      if (result.index < 0) {
        update({ stopZones: [...draft.stopZones, zone], zonesEnabled: true });
      } else {
        patchZone(result.index, zone);
      }
    },
    [draft.points, draft.stopZones, draft.humanize, patchZone, update],
  );

  const applyPickRef = useRef(applyPick);
  useEffect(() => {
    applyPickRef.current = applyPick;
  }, [applyPick]);

  // Subscribed once, and cleaned up in full: the picker answers long after the
  // click that opened it, and the tab may be gone by then.
  useEffect(() => {
    let live = true;
    const stops: (() => void)[] = [];
    const keep = (stop: () => void) => {
      if (live) stops.push(stop);
      else stop();
    };
    void on<PickResult>(REGION_PICKED, (result) => applyPickRef.current(result)).then(keep);
    void on(PICK_CANCELLED, () => setDrawing(null)).then(keep);
    return () => {
      live = false;
      for (const stop of stops) stop();
    };
  }, []);

  useEffect(() => {
    if (!draft.targets.enabled || processes.length > 0) return;
    call<string[]>("running_processes")
      .then(setProcesses)
      .catch(() => undefined);
  }, [draft.targets.enabled, processes.length]);

  const addProgram = () => {
    const name = program.trim();
    if (!name) return;
    const entry = name.includes(".") ? name : `${name}.exe`;
    if (!draft.targets.programs.some((existing) => existing.toLowerCase() === entry.toLowerCase())) {
      update({ targets: { ...draft.targets, programs: [...draft.targets.programs, entry] } });
    }
    setProgram("");
  };

  /** Put the clicker back the way it was a moment ago. The write is immediate:
   *  the bar is the only copy of the state a removed row lived in. */
  const restore = (previous: ClickerSettings) => {
    if (timer.current) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    sent.current = JSON.stringify(previous);
    setDraft(previous);
    void run("save_clicker", { clicker: previous });
  };

  const patchPoint = (index: number, patch: Partial<ClickPoint>) =>
    update({
      points: draft.points.map((point, at) => (at === index ? { ...point, ...patch } : point)),
    });

  return (
    <div className="hb__split">
      <section className="hb__page">
        <div className="hb__head">
          <div style={{ minWidth: 0 }}>
            <h1 className="hb__title">Auto clicker</h1>
            <p className="hb__sub">
              {cps > 0
                ? `${cps.toFixed(cps < 10 ? 1 : 0)} clicks a second · one every ${interval(cps)}`
                : "pick a rate"}
              {draft.doubleClick ? " · two clicks a cycle" : ""}
            </p>
          </div>
          <span className="hb__spacer" />
          <span className={`hb__pill${running ? " hb__pill--on" : ""}`}>
            <span
              className={`hb__gear-dot${running ? "" : " hb__gear-dot--off"}`}
              aria-hidden="true"
            />
            {running ? "Clicking" : "Idle"}
          </span>
        </div>

        <div className="hb__actions">
          {running ? (
            <PressDepth depth={3} tilt={5} className="hb__press--flag" onClick={stop}>
              Stop
            </PressDepth>
          ) : (
            <PressDepth
              depth={3}
              tilt={5}
              disabled={!view.clicker_enabled}
              onClick={() => {
                if (view.clicker_enabled) start();
              }}
            >
              Start clicking
            </PressDepth>
          )}
          <HotkeyField
            label="Start shortcut"
            note="Press a key combination, a mouse button or a wheel notch. Side buttons are Mouse4 and Mouse5; wheel notches are WheelUp and WheelDown."
            value={draft.hotkey}
            onChange={(hotkey) => update({ hotkey })}
          />
          <SegmentedControl
            label="Activation"
            options={[
              { value: "toggle", label: "Toggle" },
              { value: "hold", label: "Hold" },
            ]}
            value={draft.holdMode ? "hold" : "toggle"}
            onValueChange={(value) => update({ holdMode: value === "hold" })}
          />
        </div>

        {tooFast && (
          <p className="hb__warn">
            {cps.toFixed(0)} clicks a second is past the point Windows can pace;
            the run will be refused.
          </p>
        )}

        <Card
          title="What it presses"
          note="Sends hardware scan codes, so a game that reads input directly still sees the key."
        >
          <SegmentedControl
            label="Input"
            options={[
              { value: "mouse", label: "Mouse" },
              { value: "keyboard", label: "Key" },
            ]}
            value={draft.input}
            onValueChange={(value) =>
              update({ input: value as ClickerSettings["input"] })
            }
          />
          <div className="hb__rows">
            {draft.input === "mouse" ? (
              <Line label="Button">
                <SegmentedControl
                  label="Mouse button"
                  options={[
                    { value: "left", label: "Left" },
                    { value: "right", label: "Right" },
                    { value: "middle", label: "Middle" },
                  ]}
                  value={draft.button}
                  onValueChange={(value) =>
                    update({ button: value as ClickerSettings["button"] })
                  }
                />
              </Line>
            ) : (
              <>
                <Line label="Key">
                  <KeyBinder
                    scan={draft.keyScan}
                    label={
                      view.keys.find((key) => key.scan === draft.keyScan)?.label ??
                      `Scan ${draft.keyScan}`
                    }
                    onChange={(keyScan) => update({ keyScan })}
                  />
                </Line>
                <Line
                  label="Hold Shift"
                  note="Turn on for programs that read the shifted character, such as a text box."
                >
                  <Switch
                    checked={draft.uppercase}
                    label="Hold shift"
                    onChange={(checked) => update({ uppercase: checked })}
                  />
                </Line>
              </>
            )}
          </div>
        </Card>

        <Card
          lead
          title="How often"
          note="The rate is timed against a clock, so a slow moment does not slow the rest of the run."
        >
          <div className="hb__rows">
            <Line label="Rate">
              <input
                className="hb__field hb__num"
                type="number"
                min={0}
                step="any"
                aria-label="Rate"
                value={draft.rate}
                onChange={(event) => update({ rate: Number(event.target.value) })}
              />
              <span className="hb__field-label">clicks a</span>
              <select
                className="hb__field hb__field--unit"
                aria-label="Rate unit"
                value={draft.rateUnit}
                onChange={(event) =>
                  update({ rateUnit: event.target.value as ClickerSettings["rateUnit"] })
                }
              >
                {(["second", "minute", "hour", "day"] as const).map((unit) => (
                  <option key={unit} value={unit}>
                    {UNIT_NAME[unit]}
                  </option>
                ))}
              </select>
            </Line>

            <Line label="Duty cycle" note="How long the button stays down each interval.">
              <input
                className="hb__range"
                type="range"
                min={0}
                max={99}
                value={draft.duty}
                aria-label="Duty cycle"
                onChange={(event) => update({ duty: Number(event.target.value) })}
              />
              <span className="hb__readout">
                {draft.duty}%
                {cycle > 0
                  ? ` · holds ${duration((cycle * draft.duty) / 100)} of every ${duration(cycle)}`
                  : ""}
              </span>
            </Line>

            <Line label="Random rate" note="Varies each interval by this much, so the rhythm is not exact.">
              <input
                className="hb__range"
                type="range"
                min={0}
                max={90}
                value={draft.variation}
                aria-label="Random rate spread"
                onChange={(event) => update({ variation: Number(event.target.value) })}
              />
              <span className="hb__readout">
                ±{draft.variation}%
                {cycle > 0 && draft.variation > 0
                  ? ` · ${duration(Math.max(1, cycle * (1 - draft.variation / 100)))} to ${duration(cycle * (1 + draft.variation / 100))}`
                  : ""}
              </span>
            </Line>

            <Line label="Double click" note="Two clicks a cycle, the Windows double-click interval apart.">
              <Switch
                checked={draft.doubleClick}
                label="Double click"
                onChange={(checked) => update({ doubleClick: checked })}
              />
              {draft.doubleClick && (
                <NumberField
                  label="Double click gap"
                  value={draft.doubleClickGapMs}
                  onValueChange={(doubleClickGapMs) => update({ doubleClickGapMs })}
                />
              )}
            </Line>
          </div>
        </Card>

        <Card title="Limits" note="A limit of zero means it runs until you stop it.">
          <div className="hb__rows">
            <Line label="Clicks">
              <NumberField
                label="Click limit"
                value={draft.maxClicks}
                onValueChange={(maxClicks) => update({ maxClicks })}
              />
              <span className="hb__field-label">then stop</span>
            </Line>
            <Line label="Seconds">
              <NumberField
                label="Time limit in seconds"
                title="Whole seconds"
                value={draft.timeLimitSecs}
                onValueChange={(timeLimitSecs) => update({ timeLimitSecs })}
              />
              <span className="hb__field-label">then stop</span>
            </Line>
          </div>
        </Card>

        <p className="hb__card-note hb__tail-note">
          Windows wakes a timer no faster than 1 ms. A run asks for a 1 ms timer and
          releases it when it stops. If Windows refuses, the panel on the right says so
          and a high rate comes out lower.
        </p>
      </section>

      <aside className="hb__side">
        <Card title="This run" tight>
          <Stat value={activity ? activity.clicks.toLocaleString() : "0"} label="Clicks" />
          <Stat
            value={activity ? `${activity.elapsedSecs.toFixed(1)} s` : "0.0 s"}
            label="Elapsed"
          />
          <Stat
            value={
              activity && activity.running === "clicker"
                ? `${activity.achievedCps.toFixed(1)} / ${activity.requestedCps.toFixed(1)}`
                : activity?.last
                  ? `${activity.last.achievedCps.toFixed(1)} / ${activity.last.requestedCps.toFixed(1)}`
                  : "-"
            }
            label="Achieved / asked"
          />
          <p className="hb__card-note" style={{ marginTop: 8 }}>
            {activity?.running === "clicker"
              ? `Pressing the ${activity.source}.`
              : activity?.last
                ? `Last run: ${activity.last.stopReason.toLowerCase()}.`
                : "Nothing has run yet."}
          </p>
          {activity?.running === "clicker" && !activity.timerRaised && (
            <p className="hb__warn" style={{ marginTop: 8 }}>
              Windows refused the 1 ms timer, so the rate will come out low.
            </p>
          )}
        </Card>

        <Card title="Where it clicks" tight>
          <p className="hb__card-note">
            With no points the clicker clicks wherever the cursor is. With points it moves
            to each in turn and clicks it as many times as you set.
          </p>
          <div className="hb__points">
            {draft.points.length === 0 && (
              <p className="hb__card-note hb__dim">Clicking in place.</p>
            )}
            {draft.points.map((point, index) => (
              <div className="hb__point" key={`${point.x}-${point.y}-${index}`}>
                <span className="hb__point-at">
                  {point.x}, {point.y}
                </span>
                <input
                  className="hb__field hb__num hb__field--tiny"
                  type="number"
                  min={1}
                  aria-label="Clicks at this point"
                  value={point.clicks}
                  onChange={(event) =>
                    patchPoint(index, { clicks: Number(event.target.value) })
                  }
                />
                <span className="hb__field-label">×</span>
                <input
                  className="hb__field hb__num hb__field--tiny"
                  type="number"
                  min={0}
                  aria-label="Scatter radius"
                  value={point.radius}
                  onChange={(event) =>
                    patchPoint(index, { radius: Number(event.target.value) })
                  }
                />
                <span className="hb__field-label">px</span>
                <button
                  type="button"
                  className="hb__delete"
                  aria-label={`Remove the point at ${point.x}, ${point.y}`}
                  onClick={() => {
                    const previous = draft;
                    update({ points: draft.points.filter((_, at) => at !== index) });
                    pushUndo(`Removed the point at ${point.x}, ${point.y}`, () =>
                      restore(previous),
                    );
                  }}
                >
                  ×
                </button>
              </div>
            ))}
          </div>
          <div className="hb__row-actions">
            <button
              type="button"
              className="hb__mini"
              disabled={drawing !== null}
              onClick={() => draw("point", -1)}
            >
              {drawing?.purpose === "point" ? "Waiting for a spot…" : "Pick a spot…"}
            </button>
            {draft.points.length > 0 && (
              <button
                type="button"
                className="hb__mini"
                onClick={() => update({ points: [] })}
              >
                Clear
              </button>
            )}
          </div>
          <Line label="Humanise" note="Spreads each click inside the radius and moves the cursor instead of jumping.">
            <Switch
              checked={draft.humanize}
              label="Humanise"
              onChange={(checked) => update({ humanize: checked })}
            />
          </Line>
          {draft.humanize && (
            <Line label="Travel time">
              <NumberField
                label="Travel time"
                value={draft.moveDurationMs}
                onValueChange={(moveDurationMs) => update({ moveDurationMs })}
              />
              <span className="hb__field-label">ms</span>
            </Line>
          )}
        </Card>

        <Card title="When to stop" tight>
          <Line
            label="Escape ×3"
            note="Three quick taps of Escape stop the run, wherever the cursor is. Stays on because it works even while the button is held."
          >
            <span className="hb__field-label">always on</span>
          </Line>
          <Line
            label="Screen edges"
            note="Stops when the cursor reaches an edge."
          >
            <Switch
              checked={draft.edgeStop.enabled}
              label="Edge stop"
              onChange={(checked) =>
                update({ edgeStop: { ...draft.edgeStop, enabled: checked } })
              }
            />
          </Line>
          {draft.edgeStop.enabled && (
            <>
              <GuardGrid
                entries={[
                  {
                    key: "top",
                    label: "Top",
                    guard: draft.edgeStop.top,
                    patch: (next) =>
                      update({ edgeStop: { ...draft.edgeStop, top: next } }),
                  },
                  {
                    key: "right",
                    label: "Right",
                    guard: draft.edgeStop.right,
                    patch: (next) =>
                      update({ edgeStop: { ...draft.edgeStop, right: next } }),
                  },
                  {
                    key: "bottom",
                    label: "Bottom",
                    guard: draft.edgeStop.bottom,
                    patch: (next) =>
                      update({ edgeStop: { ...draft.edgeStop, bottom: next } }),
                  },
                  {
                    key: "left",
                    label: "Left",
                    guard: draft.edgeStop.left,
                    patch: (next) =>
                      update({ edgeStop: { ...draft.edgeStop, left: next } }),
                  },
                ]}
              />
              <p className="hb__card-note hb__dim" style={{ margin: "0 0 4px" }}>
                Each side keeps its own distance, because the bottom of a screen often sits
                under a taskbar. Switch a side off to leave it unwatched.
              </p>
            </>
          )}

          <Line
            label="Screen corners"
            note="Stops when the cursor reaches a corner."
          >
            <Switch
              checked={draft.cornerStop.enabled}
              label="Corner stop"
              onChange={(checked) =>
                update({ cornerStop: { ...draft.cornerStop, enabled: checked } })
              }
            />
          </Line>
          {draft.cornerStop.enabled && (
            <GuardGrid
              entries={[
                {
                  key: "topLeft",
                  label: "Top left",
                  guard: draft.cornerStop.topLeft,
                  patch: (next) =>
                    update({ cornerStop: { ...draft.cornerStop, topLeft: next } }),
                },
                {
                  key: "topRight",
                  label: "Top right",
                  guard: draft.cornerStop.topRight,
                  patch: (next) =>
                    update({ cornerStop: { ...draft.cornerStop, topRight: next } }),
                },
                {
                  key: "bottomLeft",
                  label: "Bottom left",
                  guard: draft.cornerStop.bottomLeft,
                  patch: (next) =>
                    update({ cornerStop: { ...draft.cornerStop, bottomLeft: next } }),
                },
                {
                  key: "bottomRight",
                  label: "Bottom right",
                  guard: draft.cornerStop.bottomRight,
                  patch: (next) =>
                    update({ cornerStop: { ...draft.cornerStop, bottomRight: next } }),
                },
              ]}
            />
          )}

          <Line
            label="Only in these programs"
            note="Pauses while another window is in front, and carries on when this one comes back."
          >
            <Switch
              checked={draft.targets.enabled}
              label="Program list"
              onChange={(checked) =>
                update({ targets: { ...draft.targets, enabled: checked } })
              }
            />
          </Line>
          {draft.targets.enabled && (
            <>
              <SegmentedControl
                label="List mode"
                options={[
                  { value: "whitelist", label: "Only these" },
                  { value: "blacklist", label: "Not these" },
                ]}
                value={draft.targets.mode}
                onValueChange={(value) =>
                  update({
                    targets: { ...draft.targets, mode: value as "whitelist" | "blacklist" },
                  })
                }
              />
              <div className="hb__chips">
                {draft.targets.programs.map((entry) => (
                  <span className="hb__tag" key={entry}>
                    {entry}
                    <button
                      type="button"
                      aria-label={`Remove ${entry}`}
                      onClick={() => {
                        const previous = draft;
                        update({
                          targets: {
                            ...draft.targets,
                            programs: draft.targets.programs.filter((name) => name !== entry),
                          },
                        });
                        pushUndo(`Removed ${entry} from the program list`, () =>
                          restore(previous),
                        );
                      }}
                    >
                      ×
                    </button>
                  </span>
                ))}
                {draft.targets.programs.length === 0 && (
                  <span className="hb__card-note hb__dim">Nothing listed yet.</span>
                )}
              </div>
              <div className="hb__row-actions">
                <input
                  className="hb__field"
                  list="hb-programs"
                  aria-label="Program to add"
                  placeholder="notepad.exe"
                  value={program}
                  onChange={(event) => setProgram(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") addProgram();
                  }}
                />
                <button type="button" className="hb__mini" onClick={addProgram}>
                  Add
                </button>
              </div>
              <datalist id="hb-programs">
                {processes.map((name) => (
                  <option key={name} value={name} />
                ))}
              </datalist>
            </>
          )}
        </Card>

        <Card
          title="Zones"
          note="Drag a rectangle on the desktop. The run stops, pauses or resumes when the cursor enters it."
          tight
        >
          <Line
            label="Watch zones"
            note="The cursor is read every cycle, so zones only work while this is on."
          >
            <Switch
              checked={draft.zonesEnabled}
              label="Watch zones"
              onChange={(checked) => update({ zonesEnabled: checked })}
            />
          </Line>
          {draft.zonesEnabled && (
            <>
              <div className="hb__zones">
                {draft.stopZones.map((zone, index) => {
                  const drawingHere =
                    drawing?.purpose === "zone" && drawing.index === index;
                  return (
                    <div className="hb__zone" key={index}>
                      <div className="hb__zone-head">
                        <span className="hb__zone-index">{index + 1}</span>
                        <select
                          className="hb__field hb__field--short"
                          aria-label={`Zone ${index + 1} action`}
                          value={zone.action}
                          onChange={(event) =>
                            patchZone(index, {
                              action: event.target.value as StopZone["action"],
                            })
                          }
                        >
                          <option value="stop">Stop</option>
                          <option value="pause">Pause</option>
                          <option value="start">Carry on</option>
                        </select>
                        <span className="hb__spacer" />
                        <button
                          type="button"
                          className="hb__delete"
                          aria-label={`Remove zone ${index + 1}`}
                          onClick={() => {
                            const previous = draft;
                            update({
                              stopZones: draft.stopZones.filter(
                                (_, at) => at !== index,
                              ),
                            });
                            pushUndo(`Removed zone ${index + 1}`, () => restore(previous));
                          }}
                        >
                          ×
                        </button>
                      </div>
                      <div className="hb__zone-fields">
                        {(
                          [
                            ["x", "X"],
                            ["y", "Y"],
                            ["width", "W"],
                            ["height", "H"],
                          ] as const
                        ).map(([field, label]) => (
                          <label key={field}>
                            <span className="hb__field-label">{label}</span>
                            <input
                              className="hb__field hb__num"
                              type="number"
                              aria-label={`Zone ${index + 1} ${label}`}
                              min={field === "width" || field === "height" ? 1 : undefined}
                              value={zone[field]}
                              onChange={(event) =>
                                patchZone(index, {
                                  [field]: Number(event.target.value),
                                })
                              }
                            />
                          </label>
                        ))}
                      </div>
                      <div className="hb__zone-capture">
                        <button
                          type="button"
                          className="hb__mini"
                          disabled={drawing !== null}
                          onClick={() => draw("zone", index)}
                        >
                          {drawingHere ? "Drag on the screen…" : "Draw on screen"}
                        </button>
                      </div>
                    </div>
                  );
                })}
                {draft.stopZones.length === 0 && (
                  <p className="hb__card-note hb__dim" style={{ margin: 0 }}>
                    No zones yet.
                  </p>
                )}
              </div>
              <div className="hb__row-actions">
                <button
                  type="button"
                  className="hb__mini"
                  disabled={drawing !== null}
                  onClick={() => draw("zone", -1)}
                >
                  {drawing?.purpose === "zone" && drawing.index < 0
                    ? "Drag on the screen…"
                    : "Draw a zone"}
                </button>
                {draft.stopZones.length > 0 && (
                  <button
                    type="button"
                    className="hb__mini"
                    onClick={() => update({ stopZones: [] })}
                  >
                    Clear
                  </button>
                )}
              </div>
              <p className="hb__card-note hb__dim" style={{ margin: "8px 0 0" }}>
                A stop zone wins wherever it sits in the list. A carry-on zone overrides a
                pause zone inside it.
              </p>
            </>
          )}
        </Card>

        <button
          type="button"
          className="hb__mini hb__mini--wide"
          onClick={() => {
            setDraft(DEFAULT_CLICKER);
            void run("save_clicker", { clicker: DEFAULT_CLICKER });
          }}
        >
          Reset the clicker to its defaults
        </button>
      </aside>
    </div>
  );
}

// ── small pieces the tab is built from ─────────────────────────────

export function Card({
  title,
  note,
  tight,
  lead,
  children,
}: {
  title: string;
  note?: string;
  tight?: boolean;
  /** The one lifted surface on the page. Everything else is a quiet group. */
  lead?: boolean;
  children: React.ReactNode;
}) {
  return (
    <section
      className={`hb__card${tight ? " hb__card--tight" : ""}${lead ? " hb__card--lead" : ""}`}
    >
      <h2 className="hb__card-title">{title}</h2>
      {note && <p className="hb__card-note">{note}</p>}
      {children}
    </section>
  );
}

export function Line({
  label,
  note,
  children,
}: {
  label: string;
  note?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="hb__line">
      <span className="hb__line-text">
        <span className="hb__row-name">{label}</span>
        {note && <span className="hb__row-note">{note}</span>}
      </span>
      <span className="hb__line-control">{children}</span>
    </div>
  );
}

/** Four boundaries, each with its own switch and its own distance. */
function GuardGrid({
  entries,
}: {
  entries: {
    key: string;
    label: string;
    guard: Guard;
    patch: (guard: Guard) => void;
  }[];
}) {
  return (
    <div className="hb__guards">
      {entries.map((entry) => (
        <div className="hb__guard" key={entry.key}>
          <button
            type="button"
            className="hb__chip"
            aria-pressed={entry.guard.on}
            title={entry.guard.on ? "Watched" : "Not watched"}
            onClick={() => entry.patch({ ...entry.guard, on: !entry.guard.on })}
          >
            {entry.label}
          </button>
          <input
            className="hb__field hb__num"
            type="number"
            min={0}
            max={4_000}
            disabled={!entry.guard.on}
            aria-label={`${entry.label} distance`}
            value={entry.guard.margin}
            onChange={(event) =>
              entry.patch({
                ...entry.guard,
                margin: Math.max(0, Number(event.target.value)),
              })
            }
          />
          <span className="hb__field-label">px</span>
        </div>
      ))}
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

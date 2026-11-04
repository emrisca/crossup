import { useCallback, useEffect, useRef, useState } from "react";
import { PressDepth } from "@/components/interior/press-depth";
import {
  call,
  type TyperPlan,
  type TyperSettings,
  type TyperStatusView,
  type View,
  typerPause,
  typerResume,
  typerStop,
} from "@/bridge";
import { Card, Line } from "./AutoClickerTab";
import { Switch } from "./Dialogs";
import { HotkeyField } from "./HotkeyField";
import { NumberField } from "./NumberField";

const SAMPLE =
  "We shipped the update on a Friday, which everyone says not to do. It broke the " +
  "one thing nobody tested, and the release notes still claimed it worked. Next " +
  "time the checklist runs before the tag, not after it.";

const LABELS: Record<TyperStatusView["status"], string> = {
  idle: "Ready",
  typing: "Typing",
  paused: "Paused",
  done: "Finished",
  stopped: "Stopped",
  unfocused: "Stopped: the wrong window was in front",
};

/** A pace written the way the clicker writes one: per second, and the gap. */
const pace = (cps: number): string =>
  `${cps.toFixed(cps < 10 ? 1 : 0)} keys a second · one every ${Math.round(1_000 / cps)} ms`;

/** What the run is doing, in words: thinking is the run's own pause between
 *  words, told apart from a pause somebody pressed. */
const describe = (status: TyperStatusView | null): string =>
  !status ? "Ready" : status.thinking ? "Thinking" : LABELS[status.status];

/**
 * The typer: a draft, a pace, and a run that types it into whichever window is
 * in front.
 *
 * The settings live on the Rust side; this is the editing copy, saved as it is
 * changed. What the run is doing is polled rather than pushed, the same way the
 * clicker's counters are, so the tab is one loop reading one answer.
 */
export function TyperTab({
  view,
  run,
  onError,
}: {
  view: View;
  run: (command: string, args?: Record<string, unknown>) => Promise<void>;
  onError: (message: string | null) => void;
}) {
  const [draft, setDraft] = useState<TyperSettings>(view.typer);
  const [plan, setPlan] = useState<TyperPlan | null>(null);
  const [status, setStatus] = useState<TyperStatusView | null>(null);
  const sent = useRef(JSON.stringify(view.typer));
  const timer = useRef<number | null>(null);

  // The settings live on the Rust side; this draft is the editing copy. If they
  // change somewhere else, adopt the new ones rather than fight over them.
  useEffect(() => {
    const incoming = JSON.stringify(view.typer);
    if (incoming === sent.current) return;
    sent.current = incoming;
    setDraft(view.typer);
  }, [view.typer]);

  const update = useCallback(
    (patch: Partial<TyperSettings>) => {
      const next = { ...draft, ...patch };
      setDraft(next);
      if (timer.current) window.clearTimeout(timer.current);
      // A paragraph is a lot of text to write per keystroke, and a slider is a
      // stream of changes; one write after it settles is enough.
      timer.current = window.setTimeout(() => {
        sent.current = JSON.stringify(next);
        void run("save_typer", { typer: next });
      }, 350);
    },
    [draft, run],
  );

  useEffect(() => {
    let live = true;
    const tick = async () => {
      try {
        const next = await call<TyperStatusView>("typer_status");
        if (live) setStatus(next);
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

  // The estimate comes from Rust, so the number on screen is the pace the
  // driver will actually keep rather than a second opinion.
  useEffect(() => {
    if (!draft.text.trim()) {
      setPlan(null);
      return;
    }
    const id = window.setTimeout(() => {
      void call<TyperPlan>("typer_plan", { text: draft.text, settings: draft })
        .then(setPlan)
        .catch(() => setPlan(null));
    }, 250);
    return () => window.clearTimeout(id);
  }, [draft]);

  const running = status?.status === "typing" || status?.status === "paused";
  const paused = status?.status === "paused";
  const percent =
    status && status.total > 0
      ? Math.min(100, Math.round((status.typed / status.total) * 100))
      : 0;
  // Words a minute as it goes, counted the way a typing test counts: five
  // characters to a word, from the wall clock.
  const wpm =
    status && status.typed > 0 && status.elapsedSecs > 0.4
      ? Math.round(status.typed / 5 / (status.elapsedSecs / 60))
      : 0;

  /** Starting writes first: the run takes one copy of the settings as it begins. */
  const start = () => {
    onError(null);
    if (timer.current) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    sent.current = JSON.stringify(draft);
    void run("save_typer", { typer: draft }).then(() => run("start_typer"));
  };
  const stop = () => {
    void typerStop()
      .then(setStatus)
      .catch(() => undefined);
  };

  return (
    <div className="hb__split">
      <section className="hb__page">
        <div className="hb__head">
          <div style={{ minWidth: 0 }}>
            <h1 className="hb__title">Typer</h1>
            <p className="hb__sub">
              {plan
                ? `${pace(plan.cps)} · ${Math.round(plan.totalMs / 1_000)}s a run${
                    plan.thinkingMs >= 500
                      ? ` · ${(plan.thinkingMs / 1_000).toFixed(1)}s of it thinking`
                      : ""
                  }`
                : "paste what you want typed"}
            </p>
          </div>
          <span className="hb__spacer" />
          <span className={`hb__pill${running ? " hb__pill--on" : ""}`}>
            <span
              className={`hb__gear-dot${running ? "" : " hb__gear-dot--off"}`}
              aria-hidden="true"
            />
            {describe(status)}
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
              disabled={!view.typer_enabled || !draft.text.trim()}
              onClick={() => {
                if (view.typer_enabled && draft.text.trim()) start();
              }}
            >
              Start typing
            </PressDepth>
          )}
          <HotkeyField
            keyboardOnly
            label="Start shortcut"
            note="Starts the draft, or stops a run that is going. This one is keyboard only, because a mouse button cannot type."
            value={draft.hotkey}
            onChange={(hotkey) => update({ hotkey })}
          />
        </div>

        {status?.status === "unfocused" && (
          <p className="hb__warn">
            The window in front was not the one the run started in — or Crossup itself
            was still in front when the countdown ended — so nothing was typed. Press
            Start, then click into the window that should receive the text.
          </p>
        )}

        <Card
          lead
          title="Text to type"
          note="The typer sends one keystroke at a time to whichever window is in front. Enter and Tab go as themselves, so paragraphs and indentation arrive as written."
        >
          <textarea
            className="hb__code hb__typer-text"
            value={draft.text}
            onChange={(event) => update({ text: event.target.value })}
            placeholder="Paste or write what should be typed…"
            aria-label="Text to type"
            spellCheck
            readOnly={running}
          />
          <div className="hb__row-actions">
            <span className="hb__field-label">
              {plan
                ? `${plan.words} ${plan.words === 1 ? "word" : "words"} · ${plan.chars} keystrokes`
                : "nothing to type yet"}
            </span>
            <span className="hb__spacer" />
            {!draft.text && (
              <button type="button" className="hb__mini" onClick={() => update({ text: SAMPLE })}>
                Use a sample
              </button>
            )}
            <button
              type="button"
              className="hb__mini"
              disabled={running || !draft.text}
              onClick={() => update({ text: "" })}
            >
              Clear
            </button>
          </div>
        </Card>

        <Card
          title="How it types"
          note="The typer keeps a human pace. Each word gets its own speed, a little either side of the one asked for, and the run stops to think at a full stop and now and then inside a long sentence. A mistyped character becomes a wrong key, a pause and a backspace instead of a silent correction."
        >
          <div className="hb__rows">
            <Line label="Pace" note="Words a minute, at five characters to the word.">
              <input
                className="hb__range"
                type="range"
                min={20}
                max={200}
                step={5}
                value={draft.wpm}
                disabled={running || draft.deadlineSecs !== null}
                aria-label="Pace in words a minute"
                onChange={(event) => update({ wpm: Number(event.target.value) })}
              />
              <span className="hb__readout">{draft.wpm} wpm</span>
            </Line>

            <Line
              label="Pace fluctuation"
              note="How far each word's pace may wander from the slider, either side, and how far the gaps between keys do. Zero types like a metronome; a fifth loses the rhythm and finds it again. A deadline keeps the exact pace instead, because the last keystroke has to land on time."
            >
              <input
                className="hb__range"
                type="range"
                min={0}
                max={50}
                step={1}
                value={Math.round(draft.paceVariation * 100)}
                disabled={running || draft.deadlineSecs !== null}
                aria-label="Pace fluctuation"
                onChange={(event) =>
                  update({ paceVariation: Number(event.target.value) / 100 })
                }
              />
              <span className="hb__readout">{Math.round(draft.paceVariation * 100)}%</span>
            </Line>

            <Line
              label="Typo rate"
              note="The share of characters mistyped, then corrected with a backspace."
            >
              <input
                className="hb__range"
                type="range"
                min={0}
                max={15}
                step={1}
                value={Math.round(draft.typoRate * 100)}
                disabled={running}
                aria-label="Typo rate"
                onChange={(event) => update({ typoRate: Number(event.target.value) / 100 })}
              />
              <span className="hb__readout">{Math.round(draft.typoRate * 100)}%</span>
            </Line>

            <Line
              label="Fix it later"
              note="Leave a mistake standing and come back for it after one to three sentences, the way a person notices what they wrote a line ago. The run erases back to the mistake and types that stretch again, so the plan above gets longer. Off, a mistake is corrected where it is made."
            >
              <Switch
                checked={draft.typoDefer}
                label="Fix mistakes a few sentences later"
                disabled={running || draft.typoRate === 0}
                onChange={(typoDefer) => update({ typoDefer })}
              />
            </Line>

            <Line
              label="Thinking pauses"
              note="Whether the run stops between words at all. Each pause is picked at random up to this length, and it counts towards the run's time."
            >
              <Switch
                checked={draft.thinkPauses}
                label="Thinking pauses"
                disabled={running}
                onChange={(thinkPauses) => update({ thinkPauses })}
              />
              <input
                className="hb__range"
                type="range"
                min={0.5}
                max={8}
                step={0.5}
                value={draft.thinkMaxSecs}
                disabled={running || !draft.thinkPauses}
                aria-label="Longest thinking pause"
                onChange={(event) => update({ thinkMaxSecs: Number(event.target.value) })}
              />
              <span className="hb__readout">{draft.thinkMaxSecs.toFixed(1)} s</span>
            </Line>

            <Line
              label="After each sentence"
              note="A stop after every full stop, the beat before the next thought."
            >
              <Switch
                checked={draft.thinkAfterSentence}
                label="Pause after each sentence"
                disabled={running || !draft.thinkPauses}
                onChange={(thinkAfterSentence) => update({ thinkAfterSentence })}
              />
            </Line>

            <Line
              label="Inside long sentences"
              note="A stop after a random fifteen to thirty words, for a sentence that runs on without a full stop. It counts from the last pause, so the two rules do not stack up."
            >
              <Switch
                checked={draft.thinkInLongSentence}
                label="Pause inside long sentences"
                disabled={running || !draft.thinkPauses}
                onChange={(thinkInLongSentence) => update({ thinkInLongSentence })}
              />
            </Line>

            <Line
              label="Finish by"
              note="Spreads the run so the last keystroke lands on time. This overrides the pace above."
            >
              <Switch
                checked={draft.deadlineSecs !== null}
                label="Finish by a deadline"
                onChange={(on) => update({ deadlineSecs: on ? 45 : null })}
              />
              {draft.deadlineSecs !== null && (
                <>
                  <NumberField
                    label="Seconds to finish in"
                    value={draft.deadlineSecs}
                    onValueChange={(deadlineSecs) => update({ deadlineSecs })}
                  />
                  <span className="hb__field-label">seconds</span>
                </>
              )}
            </Line>

            <Line
              label="Countdown"
              note="The run waits this long before the first keystroke, so you can click into the window that should receive the text."
            >
              <NumberField
                label="Seconds before the first keystroke"
                value={draft.countdownSecs}
                onValueChange={(countdownSecs) => update({ countdownSecs })}
              />
              <span className="hb__field-label">seconds</span>
            </Line>
          </div>
          {plan?.clamped && (
            <p className="hb__warn">
              That deadline needs more than {pace(plan.cps)}. The typer uses its fastest
              pace, so the run finishes late instead of sending keystrokes in a burst.
            </p>
          )}
        </Card>

        <p className="hb__card-note hb__tail-note">
          The typer sends Unicode keystrokes, so a keyboard layout the machine is not set
          to still receives the characters it can display. Nothing goes through the
          clipboard; the typer types what is in the editor.
        </p>
      </section>

      <aside className="hb__side">
        <Card title="This run" tight>
          <div className="hb__status-row">
            <span className={`hb__pill${running ? " hb__pill--on" : ""}`}>
              {describe(status)}
            </span>
            <span className="hb__field-label">
              {wpm > 0 ? `${wpm} wpm` : plan ? `${plan.cps.toFixed(1)} keys/s` : " "}
            </span>
          </div>
          <div className="hb__meter" aria-hidden="true">
            <i style={{ width: `${percent}%` }} />
          </div>

          <Stat
            value={`${status?.typed ?? 0} / ${status?.total || plan?.chars || 0}`}
            label="Keystrokes"
          />
          <Stat
            value={status ? `${status.elapsedSecs.toFixed(1)} s` : "0.0 s"}
            label="Elapsed"
          />
          <Stat value={plan ? `${plan.cps.toFixed(1)} /s` : "-"} label="Pace asked" />

          <p className="hb__card-note" style={{ marginTop: 8 }}>
            {status?.status === "typing" && status.typed === 0
              ? "Click into the window that should receive the text. The run starts when the countdown ends."
              : status?.thinking
                ? "Stopped between words, thinking about the next one."
                : running
                ? "Typing into the window that was in front when the run started."
                : status?.status === "done"
                  ? "The whole draft is out."
                  : status?.status === "unfocused"
                    ? "Stopped because the window in front was not the one the text was meant for."
                    : status?.status === "paused"
                      ? "Paused. Nothing is typed until you resume."
                      : "Nothing has run yet."}
          </p>

          {running && (
            <div className="hb__row-actions">
              <button
                type="button"
                className="hb__mini"
                onClick={() =>
                  void (paused ? typerResume() : typerPause())
                    .then(setStatus)
                    .catch(() => undefined)
                }
              >
                {paused ? "Resume" : "Pause"}
              </button>
              <button type="button" className="hb__mini hb__mini--danger" onClick={stop}>
                Stop
              </button>
            </div>
          )}
        </Card>

        <Card title="While it types" tight>
          <p className="hb__card-note">
            The window in front receives the text. If another window comes
            forward, this one included, the run stops instead of
            typing into the wrong window.
          </p>
          <p className="hb__card-note">
            Only one run happens at a time. Starting the typer while the clicker or a macro
            runs is refused with a message, and turning the typer off in the Tools menu stops it.
          </p>
          <p className="hb__card-note">
            Escape ×3 stops the clicker, and the shortcut above stops the typer. Both are
            read between keystrokes, so a stop lands within about a frame.
          </p>
        </Card>

        <button
          type="button"
          className="hb__mini hb__mini--wide"
          onClick={() =>
            update({
              wpm: 65,
              typoRate: 0.04,
              typoDefer: false,
              deadlineSecs: null,
              countdownSecs: 3,
              thinkPauses: true,
              thinkMaxSecs: 2.5,
              thinkAfterSentence: true,
              thinkInLongSentence: true,
              paceVariation: 0.2,
            })
          }
        >
          Reset the typer to its defaults
        </button>
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

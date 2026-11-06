import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Modal } from "@/components/interior/modal";
import { PressDepth } from "@/components/interior/press-depth";
import { SegmentedControl } from "@/components/interior/segmented-control";
import {
  BLOCK_LABELS,
  BLOCK_TEMPLATES,
  DRAG_TEMPLATE,
  blockTitle,
  call,
  isMouseScan,
  keyLabel,
  on,
  pickRegion,
  forgetPicture,
  pictureHistory,
  picturePreview,
  pictureUses,
  savePicture,
  PICK_CANCELLED,
  REGION_PICKED,
  stepSummary,
  type Activity,
  type Macro,
  type MouseButton,
  type PickResult,
  type Picture,
  type PictureUser,
  type Step,
  type View,
  MACRO_PROGRAM,
  TOOL_LABELS,
  ToolName,
  blockChoices,
  chooseProgram,
  runningWindows,
} from "@/bridge";
import { Card, Line } from "./AutoClickerTab";
import { Switch } from "./Dialogs";
import { HotkeyField } from "./HotkeyField";
import { KeyBinder } from "./Mappings";
import { NumberField } from "./NumberField";
import { pushUndo } from "./undo";

/** Deepest a Repeat may be nested, matching the engine's own limit. */
const MAX_DEPTH = 6;

/**
 * The palette, grouped.
 *
 * Eleven blocks in one wrapping row were readable; twenty-one are not, and the
 * one thing a person needs from a palette is to find the block they already
 * have in mind. The groups follow the question being asked: what goes in, how
 * long it waits, what it looks at, and what it decides.
 */
const BLOCK_GROUPS: { title: string; kinds: Step["kind"][] }[] = [
  { title: "Input", kinds: ["key", "text", "chord", "click", "move", "scroll", "scrollAt"] },
  { title: "Time", kinds: ["delay", "randomDelay", "until"] },
  { title: "Screen", kinds: ["waitImage", "clickImage", "assertImage"] },
  {
    title: "Tools",
    kinds: ["focus", "launch", "clipboard", "screenshot", "tool"],
  },
  {
    title: "Flow",
    kinds: [
      "if",
      "repeat",
      "repeatFor",
      "callMacro",
      "replay",
      "while",
      "note",
      "log",
      "stop",
      "fail",
    ],
  },
];

// ── tree surgery, so a drop is one immutable rewrite ───────────────

/**
 * A position in the block tree.
 *
 * Step indexes, plus which arm of an If to step into: `[2, "else", 0]` is the
 * first block under the third block's Otherwise. A Repeat has one list of
 * children, so its paths carry indexes straight through.
 */
type Path = (number | "then" | "else")[];

const samePath = (a: Path, b: Path) =>
  a.length === b.length && a.every((value, index) => value === b[index]);

const isAncestor = (path: Path, other: Path) =>
  path.length < other.length && path.every((value, index) => value === other[index]);

const keyAt = (path: Path) => path.join(".");

/** Rewrite the list of blocks the path at `parent` points into. */
function withChildren(
  steps: Step[],
  parent: Path,
  change: (list: Step[]) => Step[],
): Step[] {
  if (parent.length === 0) return change(steps);
  const [head, ...rest] = parent;
  if (typeof head !== "number") return steps;
  return steps.map((entry, at) => {
    if (at !== head) return entry;
    const [arm, ...tail] = rest;
    if (entry.kind === "repeat" || entry.kind === "while") {
      return { ...entry, steps: withChildren(entry.steps, rest, change) };
    }
    if (entry.kind === "if") {
      if (arm === "then") {
        return { ...entry, steps: withChildren(entry.steps, tail, change) };
      }
      if (arm === "else") {
        return { ...entry, otherwise: withChildren(entry.otherwise, tail, change) };
      }
    }
    return entry;
  });
}

function pickStep(steps: Step[], path: Path): Step | undefined {
  if (path.length === 0) return undefined;
  const [head, ...rest] = path;
  if (typeof head !== "number") return undefined;
  const step = steps[head];
  if (!step) return undefined;
  if (rest.length === 0) return step;
  if (step.kind === "repeat" || step.kind === "while") return pickStep(step.steps, rest);
  if (step.kind === "if") {
    const [arm, ...tail] = rest;
    if (arm === "then") return pickStep(step.steps, tail);
    if (arm === "else") return pickStep(step.otherwise, tail);
  }
  return undefined;
}

/**
 * Put a block into the container at `parent`, at `index` within it.
 *
 * `parent` names a container, not a block, so the empty path is the list at the
 * top of the macro, and anything longer walks into each Repeat, or each arm of
 * an If, on the way down.
 */
function insertStep(steps: Step[], parent: Path, index: number, step: Step): Step[] {
  return withChildren(steps, parent, (list) => {
    const copy = [...list];
    copy.splice(Math.max(0, Math.min(index, copy.length)), 0, step);
    return copy;
  });
}

function removeStep(steps: Step[], path: Path): Step[] {
  const index = path[path.length - 1];
  if (typeof index !== "number") return steps;
  return withChildren(steps, path.slice(0, -1), (list) =>
    list.filter((_, at) => at !== index),
  );
}

function replaceStep(steps: Step[], path: Path, step: Step): Step[] {
  const index = path[path.length - 1];
  if (typeof index !== "number") return steps;
  return withChildren(steps, path.slice(0, -1), (list) =>
    list.map((entry, at) => (at === index ? step : entry)),
  );
}

/** Move a block, keeping it where the drop said and out of its own children. */
function moveStep(
  steps: Step[],
  from: Path,
  parent: Path,
  index: number,
): Step[] {
  const step = pickStep(steps, from);
  if (!step) return steps;
  if (isAncestor(from, parent) || samePath(from, parent)) return steps;

  const without = removeStep(steps, from);
  const from_index = from[from.length - 1];
  const sameContainer = samePath(from.slice(0, -1), parent);
  const target =
    sameContainer && typeof from_index === "number" && from_index < index
      ? index - 1
      : index;
  return insertStep(without, parent, target, step);
}

const swap = (steps: Step[], path: Path, by: number): Step[] => {
  const index = path[path.length - 1];
  if (typeof index !== "number") return steps;
  return withChildren(steps, path.slice(0, -1), (list) => {
    const to = index + by;
    if (to < 0 || to >= list.length) return list;
    const copy = [...list];
    [copy[index], copy[to]] = [copy[to], copy[index]];
    return copy;
  });
};

/** Every block in a container, in order, for the outline summary. */
const countBlocks = (steps: Step[]): number =>
  steps.reduce(
    (total, step) =>
      total +
      1 +
      (step.kind === "repeat" || step.kind === "while"
        ? countBlocks(step.steps)
        : step.kind === "if"
          ? countBlocks(step.steps) + countBlocks(step.otherwise)
          : 0),
    0,
  );

/** The child lists a block shows: a Repeat's or While's one, or an If's two arms. */
function branchAreas(step: Step, path: Path) {
  if (step.kind === "repeat" || step.kind === "while") {
    return [
      {
        key: step.kind,
        parent: path,
        steps: step.steps,
        label: null as string | null,
        emptyLabel: "Drag blocks in here",
      },
    ];
  }
  if (step.kind === "if") {
    return [
      {
        key: "then",
        parent: [...path, "then"] as Path,
        steps: step.steps,
        label: "Then",
        emptyLabel: "Drop blocks to do when it is true",
      },
      {
        key: "else",
        parent: [...path, "else"] as Path,
        steps: step.otherwise,
        label: "Otherwise",
        emptyLabel: "Drop blocks to do when it is not",
      },
    ];
  }
  return [];
}

// ── the tab ────────────────────────────────────────────────────────

export function MacrosTab({
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
  const [selectedId, setSelectedId] = useState<string | null>(view.macros[0]?.id ?? null);
  const [draft, setDraft] = useState<Macro | null>(null);

  const pageRef = useRef<HTMLElement | null>(null);
  const headRef = useRef<HTMLDivElement | null>(null);

  /* The palette pins under the title row rather than under the top of the page.
     Without this it pinned at the scrollport's top edge and the header, which
     paints above it, covered its first rows: the block buttons were there and
     impossible to press. The offset is measured, because the title field grows
     when the window is narrow and a hard-coded height would drift. */
  useEffect(() => {
    const page = pageRef.current;
    const head = headRef.current;
    if (!page || !head) return;
    const sync = () => page.style.setProperty("--head-h", `${head.offsetHeight}px`);
    sync();
    const observer = new ResizeObserver(sync);
    observer.observe(head);
    return () => observer.disconnect();
  }, [draft]);

  const [processes, setProcesses] = useState<string[]>([]);
  const [newOpen, setNewOpen] = useState(false);
  // Which block is waiting for a spot on the desktop, as a path into the tree,
  // so the answer lands on the block that asked rather than on the first one.
  const [picking, setPicking] = useState<Path | null>(null);
  const pickPath = useRef<Path | null>(null);
  // Which macro is being dragged in the rail, and the row it would land on.
  const [moving, setMoving] = useState<string | null>(null);
  // Which row a drag would land on, and on which side of it.
  const [overMacro, setOverMacro] = useState<{ id: string; after: boolean } | null>(null);
  const draftRef = useRef<Macro | null>(null);
  const sent = useRef<string>("");
  const timer = useRef<number | null>(null);

  const selected = useMemo(
    () => view.macros.find((entry) => entry.id === selectedId) ?? view.macros[0] ?? null,
    [view.macros, selectedId],
  );

  // Adopt the stored macro when it is the one being edited, unless the change is
  // the one this window just sent.
  useEffect(() => {
    if (!selected) {
      // Forget what was last sent. A macro that comes back through undo has the
      // same definition as the one that left, so the comparison below would
      // skip it and leave the page on the empty state it was deleted into.
      sent.current = "";
      setDraft(null);
      return;
    }
    const { problem, ...definition } = selected;
    void problem;
    const incoming = JSON.stringify(definition);
    if (incoming === sent.current) return;
    sent.current = incoming;
    setDraft(definition);
  }, [selected]);

  useEffect(() => {
    call<string[]>("running_processes")
      .then(setProcesses)
      .catch(() => undefined);
  }, []);

  const push = useCallback(
    (next: Macro) => {
      setDraft(next);
      if (timer.current) window.clearTimeout(timer.current);
      // Every keystroke in the name field would otherwise re-register every
      // shortcut on the machine.
      timer.current = window.setTimeout(() => {
        sent.current = JSON.stringify(next);
        void run("save_macro", { definition: next });
      }, 400);
    },
    [run],
  );

  // The picker answers long after the click that opened it, so the draft it is
  // answering into is read from a ref rather than from the closure of the click.
  useEffect(() => {
    draftRef.current = draft;
  }, [draft]);

  /** Ask the desktop for a rectangle; the block it belongs to is remembered. */
  const pickSpot = useCallback((path: Path) => {
    const step = pickStep(draftRef.current?.steps ?? [], path);
    if (!step) return;
    pickPath.current = path;
    setPicking(path);
    // A picker that never opened would leave the block waiting for an answer
    // that is not coming, so a failed open puts the button back as it was.
    const giveUp = (reason: unknown) => {
      pickPath.current = null;
      setPicking(null);
      onError(String(reason));
    };
    if (
      step.kind === "waitImage" ||
      step.kind === "clickImage" ||
      (step.kind === "if" && step.condition.kind === "picture") ||
      (step.kind === "while" && step.condition.kind === "picture")
    ) {
      // A picture is dragged out of the screen the same way a zone is, and kept
      // on disk rather than in the settings file.
      void pickRegion({ purpose: "macro-picture", index: 0, zones: [], points: [] }).catch(
        giveUp,
      );
      return;
    }
    if (step.kind === "screenshot") {
      // An area rather than a point: the rectangle dragged is the shot, and
      // the block already has one to draw under the cursor as a guide.
      void pickRegion({
        purpose: "macro-shot",
        index: 0,
        zones: [{ x: step.x, y: step.y, width: step.width, height: step.height }],
        points: [],
      }).catch(giveUp);
      return;
    }
    if (step.kind !== "move" && step.kind !== "click" && step.kind !== "scrollAt") {
      pickPath.current = null;
      setPicking(null);
      return;
    }
    void pickRegion({
      purpose: "macro-point",
      index: 0,
      zones: [],
      // A relative block's x and y are an offset, not a place: drawing them as a
      // marker on screen would point at the wrong spot entirely.
      points:
        step.kind === "scrollAt" || !step.relative
          ? [{ x: step.x, y: step.y, width: 0, height: 0 }]
          : [],
    }).catch(giveUp);
  }, [onError]);

  useEffect(() => {
    let live = true;
    const stops: (() => void)[] = [];
    const keep = (stop: () => void) => {
      if (live) stops.push(stop);
      else stop();
    };
    void on<PickResult>(REGION_PICKED, (result) => {
      if (result.purpose === "macro-picture") {
        const path = pickPath.current;
        const macro = draftRef.current;
        pickPath.current = null;
        setPicking(null);
        if (!path || !macro) return;
        void savePicture(result)
          .then((picture) => {
            const current = draftRef.current;
            const step = current ? pickStep(current.steps, path) : undefined;
            if (!current || !step) return;
            if (step.kind === "waitImage" || step.kind === "clickImage") {
              push({
                ...current,
                steps: replaceStep(current.steps, path, { ...step, image: picture.id }),
              });
              return;
            }
            if (
              (step.kind === "if" || step.kind === "while") &&
              step.condition.kind === "picture"
            ) {
              push({
                ...current,
                steps: replaceStep(current.steps, path, {
                  ...step,
                  condition: { ...step.condition, image: picture.id },
                }),
              });
            }
          })
          .catch((reason) => onError(String(reason)));
        return;
      }
      if (result.purpose === "macro-shot") {
        const path = pickPath.current;
        const macro = draftRef.current;
        pickPath.current = null;
        setPicking(null);
        if (!path || !macro) return;
        const step = pickStep(macro.steps, path);
        if (!step || step.kind !== "screenshot") return;
        push({
          ...macro,
          steps: replaceStep(macro.steps, path, {
            ...step,
            x: result.x,
            y: result.y,
            width: Math.max(1, result.width),
            height: Math.max(1, result.height),
          }),
        });
        return;
      }
      const path = pickPath.current;
      const macro = draftRef.current;
      pickPath.current = null;
      setPicking(null);
      if (!path || !macro) return;
      const step = pickStep(macro.steps, path);
      if (!step) return;
      // A dragged area places its point at the centre; a plain click already is
      // a point, with the centre and the corner being the same pixel.
      const x = Math.round(result.x + result.width / 2);
      const y = Math.round(result.y + result.height / 2);
      if (step.kind === "scrollAt") {
        // A scroll point is a place and nothing else, so it takes the spot and
        // no other field: handing it the `relative` a move block carries would
        // put a key in the settings this block does not have.
        push({ ...macro, steps: replaceStep(macro.steps, path, { ...step, x, y }) });
        return;
      }
      if (step.kind !== "move" && step.kind !== "click") return;
      push({
        ...macro,
        steps: replaceStep(macro.steps, path, { ...step, x, y, relative: false }),
      });
    }).then(keep);
    void on(PICK_CANCELLED, () => {
      pickPath.current = null;
      setPicking(null);
    }).then(keep);
    return () => {
      live = false;
      for (const stop of stops) stop();
    };
  }, [push, onError]);

  const runningMacro = activity?.running && activity.running !== "clicker";

  /**
   * Open the macro a picture is used in, and put that block in view.
   *
   * Asked for by an event rather than a callback passed down: the shelf sits in
   * a picture field, inside a block, inside a branch, and threading one action
   * through five components to reach it would be most of the tree rewritten for
   * a scroll. The row is then found by its path in the page instead of through
   * state, one row is being pointed at, and the whole tree re-rendering to
   * carry a spotlight to it would be a lot of renders for a moment's glow.
   */
  useEffect(() => {
    const onShow = (event: Event) => {
      const { macroId, path } = (
        event as CustomEvent<{ macroId: string; path: string[] }>
      ).detail;
      setSelectedId(macroId);
      const key = path.join(".");
      // The macro may not be the one that was on screen: give the tree a frame
      // to render before looking for the row inside it.
      window.setTimeout(() => {
        const row = document.querySelector<HTMLElement>(`[data-blk="${key}"]`);
        if (!row) return;
        row.scrollIntoView({ block: "center", behavior: "smooth" });
        row.classList.add("blk--found");
        window.setTimeout(() => row.classList.remove("blk--found"), 1600);
      }, 80);
    };
    window.addEventListener(SHOW_BLOCK, onShow);
    return () => window.removeEventListener(SHOW_BLOCK, onShow);
  }, []);

  /**
   * Put the dragged macro before or after another one, or at the end, when
   * there is no row to aim at, then save the order as one list.
   */
  const moveMacro = (from: string, target: string | null, after: boolean) => {
    if (target === from) return;
    const ids = view.macros.map((macro) => macro.id).filter((id) => id !== from);
    if (!target) {
      ids.push(from);
    } else {
      const at = ids.indexOf(target);
      ids.splice(at < 0 ? ids.length : at + (after ? 1 : 0), 0, from);
    }
    void run("reorder_macros", { ids });
  };

  const newMacro = () => {
    const id = `macro-${Date.now().toString(36)}`;
    const definition: Macro = {
      id,
      name: `Macro ${view.macros.length + 1}`,
      hotkey: "",
      onlyWhen: "",
      enabled: true,
      steps: [
        BLOCK_TEMPLATES.delay({ ms: 250 }) as Step,
      ],
    };
    setSelectedId(id);
    sent.current = JSON.stringify(definition);
    setDraft(definition);
    void run("save_macro", { definition });
  };

  return (
    <div className="hb__split">
      <aside className="hb__rail">
        <div className="hb__rail-head">
          <span className="hb__rail-label">Macros</span>
          <span className="hb__spacer" />
          <button type="button" className="hb__mini" onClick={() => setNewOpen(true)}>
            New
          </button>
        </div>
        <div className="hb__nav">
          {view.macros.length === 0 && (
            <p className="hb__card-note hb__dim" style={{ padding: "6px 10px" }}>
              No macros yet.
            </p>
          )}
          {view.macros.map((entry) => {
            const live = runningMacro && activity?.running === entry.name;
            // The whole row is the handle: there is no grip to aim at, a click
            // still opens the macro, and a drag is what moves it.
            return (
              <button
                key={entry.id}
                type="button"
                className={`hb__profile${moving === entry.id ? " hb__profile--moving" : ""}${
                  overMacro?.id === entry.id && moving !== entry.id
                    ? overMacro.after
                      ? " hb__profile--over-end"
                      : " hb__profile--over"
                    : ""
                }`}
                aria-current={entry.id === selected?.id}
                draggable
                onDragStart={(event) => {
                  event.dataTransfer.effectAllowed = "move";
                  event.dataTransfer.setData("text/plain", entry.id);
                  setMoving(entry.id);
                }}
                onDragOver={(event) => {
                  if (!moving || moving === entry.id) return;
                  event.preventDefault();
                  event.dataTransfer.dropEffect = "move";
                  // Which half of the row the pointer is in decides the side:
                  // the bottom half is the only way to say "after this one",
                  // and so the only way to reach the end of the list at all.
                  const box = event.currentTarget.getBoundingClientRect();
                  setOverMacro({
                    id: entry.id,
                    after: event.clientY > box.top + box.height / 2,
                  });
                }}
                onDragLeave={() =>
                  setOverMacro((current) => (current?.id === entry.id ? null : current))
                }
                onDrop={(event) => {
                  event.preventDefault();
                  const from = moving ?? event.dataTransfer.getData("text/plain");
                  const box = event.currentTarget.getBoundingClientRect();
                  const after = event.clientY > box.top + box.height / 2;
                  setMoving(null);
                  setOverMacro(null);
                  if (from) moveMacro(from, entry.id, after);
                }}
                onDragEnd={() => {
                  setMoving(null);
                  setOverMacro(null);
                }}
                onClick={() => setSelectedId(entry.id)}
              >
                <span
                  className={`hb__mode-dot${entry.problem ? " hb__mode-dot--warn" : entry.enabled ? " hb__mode-dot--moss" : ""}`}
                  aria-hidden="true"
                />
                <span className="hb__profile-name">{entry.name}</span>
                <span className="hb__profile-meta">
                  {live ? "running" : entry.hotkey || "-"}
                </span>
              </button>
            );
          })}
          {/* The foot of the rail is a drop surface of its own, so a macro can
              be moved past the last row without a row below it to aim at. */}
          {moving && (
            <div
              className={`hb__nav-tail${
                overMacro?.id === END_OF_LIST ? " hb__nav-tail--active" : ""
              }`}
              onDragOver={(event) => {
                event.preventDefault();
                event.dataTransfer.dropEffect = "move";
                setOverMacro({ id: END_OF_LIST, after: true });
              }}
              onDrop={(event) => {
                event.preventDefault();
                const from = moving ?? event.dataTransfer.getData("text/plain");
                setMoving(null);
                setOverMacro(null);
                if (from) moveMacro(from, null, true);
              }}
            >
              Move to the end
            </div>
          )}
        </div>
        <div className="hb__rail-foot">
          
        </div>
      </aside>

      <section className="hb__page" ref={pageRef}>
        {draft ? (
          <>
            <div className="hb__head" ref={headRef}>
              <div style={{ minWidth: 0, flex: 1 }}>
                <input
                  className="hb__field hb__field--title"
                  aria-label="Macro name"
                  value={draft.name}
                  onChange={(event) => push({ ...draft, name: event.target.value })}
                />
                <p className="hb__sub">
                  {countBlocks(draft.steps)} block{draft.steps.length === 1 ? "" : "s"}
                  {selected?.problem ? ` · ${selected.problem}` : ""}
                </p>
              </div>
              <span className="hb__spacer" />
              {runningMacro && activity?.running === draft.name ? (
                <PressDepth
                  depth={3}
                  tilt={5}
                  className="hb__press--flag"
                  onClick={() => void call("stop_activity").catch(() => undefined)}
                >
                  Stop
                </PressDepth>
              ) : (
                <PressDepth
                  depth={3}
                  tilt={5}
                  disabled={!view.macros_enabled}
                  onClick={() => {
                    if (!view.macros_enabled) return;
                    void run("save_macro", { definition: draft }).then(() =>
                      run("run_macro", { id: draft.id }),
                    );
                  }}
                >
                  Run macro
                </PressDepth>
              )}
            </div>

            <Card title="When it runs" tight>
              <div className="hb__rows">
                <Line
                  label="Shortcut"

                >
                  <HotkeyField
                    label="Macro shortcut"
                    value={draft.hotkey}
                    onChange={(hotkey) => push({ ...draft, hotkey })}
                  />
                </Line>
                <Line label="Only in" note="Leave empty to run anywhere.">
                  <input
                    className="hb__field hb__field--short"
                    list="hb-macro-programs"
                    aria-label="Only run in this program"
                    placeholder="anywhere"
                    value={draft.onlyWhen}
                    onChange={(event) => push({ ...draft, onlyWhen: event.target.value })}
                  />
                </Line>
                <Line label="Enabled" note="A disabled macro keeps its blocks and ignores its shortcut.">
                  <Switch
                    checked={draft.enabled}
                    label="Macro enabled"
                    onChange={(checked) => push({ ...draft, enabled: checked })}
                  />
                </Line>
              </div>
              <datalist id="hb-macro-programs">
                {processes.map((name) => (
                  <option key={name} value={name} />
                ))}
              </datalist>
            </Card>

            <BlockBuilder
              keys={view.keys}
              steps={draft.steps}
              picking={picking}
              onPick={pickSpot}
              onChange={(steps) => push({ ...draft, steps })}
            />

            <div className="hb__row-actions" style={{ marginTop: 14 }}>
              <button
                type="button"
                className="hb__mini hb__mini--danger"
                onClick={() => {
                  const next = view.macros.find((entry) => entry.id !== draft.id);
                  void run("delete_macro", { id: draft.id });
                  pushUndo(`Deleted the macro "${draft.name}"`, async () => {
                    await run("undo_delete");
                    // The engine puts the macro back; the tab makes it the open
                    // one again, which is the state the delete walked out of.
                    setSelectedId(draft.id);
                  });
                  setSelectedId(next?.id ?? null);
                }}
              >
                Delete macro
              </button>
            </div>
          </>
        ) : (
          <div className="hb__empty mat-well">
            <p className="hb__empty-title">No macro selected</p>
            <p className="hb__empty-note">
              A macro is a list of blocks: press a key, type something, click, move,
              wait, or repeat a group of them. Give it a shortcut and it runs from
              anywhere.
            </p>
            <div className="hb__row-actions" style={{ justifyContent: "center", marginTop: 14 }}>
              <PressDepth depth={3} tilt={5} onClick={newMacro}>
                New macro
              </PressDepth>
            </div>
          </div>
        )}
      </section>

      <NewMacroDialog
        open={newOpen}
        onClose={() => setNewOpen(false)}
        onCreate={() => {
          newMacro();
          setNewOpen(false);
        }}
      />
    </div>
  );
}

function NewMacroDialog({
  open,
  onClose,
  onCreate,
}: {
  open: boolean;
  onClose: () => void;
  onCreate: () => void;
}) {
  return (
    <Modal
      open={open}
      onClose={onClose}
      title="New macro"
      description="Drag blocks in. Nothing runs until you press Run."
      maxWidth={420}
      footer={
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <button type="button" className="appearance__reset" onClick={onClose}>
            Cancel
          </button>
          <span className="hb__spacer" />
          <PressDepth depth={3} tilt={5} onClick={onCreate}>
            Create
          </PressDepth>
        </div>
      }
    >
      
    </Modal>
  );
}

// ── the builder ────────────────────────────────────────────────────

/** The drop surface at the foot of the list, which no row owns. */
const END_OF_LIST = "__end";

/** Asked by a picture's shelf, answered by the tab that owns the macro tree. */
const SHOW_BLOCK = "macro-show-block";

type Drag =
  | { source: "palette"; make: () => Step }
  | { source: "tree"; path: Path };

type Drop = { parent: Path; index: number };

function BlockBuilder({
  keys,
  steps,
  picking,
  onPick,
  onChange,
}: {
  keys: View["keys"];
  steps: Step[];
  picking: Path | null;
  onPick: (path: Path) => void;
  onChange: (steps: Step[]) => void;
}) {
  const [drag, setDrag] = useState<Drag | null>(null);
  const [drop, setDrop] = useState<Drop | null>(null);
  // One group of blocks at a time. Twenty-nine buttons in five stacked rows was
  // the whole of the space a macro had to be written in; a tab row costs one
  // line above it instead.
  const [paletteGroup, setPaletteGroup] = useState(0);
  const shownGroup = BLOCK_GROUPS[paletteGroup] ?? BLOCK_GROUPS[0];

  /**
   * Stop believing a drag is in flight when one ends somewhere this cannot feel
   * it.
   *
   * A drag let go over the native window frame, cancelled by the system, or
   * finished on a part of the page that is not a drop surface can end without a
   * `dragend` ever reaching the builder. While a drag is believed to be alive
   * every row answers the pointer with a drop, so the stale state sat over the
   * grips and made the blocks impossible to pick up again, the symptom was
   * "the blocks stopped dragging", some of them, at random. Whatever can end a
   * drag clears it here.
   */
  useEffect(() => {
    const forget = () => {
      setDrag(null);
      setDrop(null);
    };
    window.addEventListener("dragend", forget, true);
    window.addEventListener("drop", forget);
    window.addEventListener("mouseup", forget);
    window.addEventListener("blur", forget);
    return () => {
      window.removeEventListener("dragend", forget, true);
      window.removeEventListener("drop", forget);
      window.removeEventListener("mouseup", forget);
      window.removeEventListener("blur", forget);
    };
  }, []);

  /**
   * Scroll the panel when a drag reaches its edge.
   *
   * A macro is usually taller than the window, and a drag can only be dropped
   * on rows that are on screen: without this, moving a block past the fold is
   * impossible and dragging looks like it only works near the top of the list.
   * The scroller is whatever ancestor actually scrolls, found by asking rather
   * than by class name, since the panel around the builder is not ours alone.
   */
  const autoScroll = (event: React.DragEvent) => {
    let scroller: HTMLElement | null = event.currentTarget as HTMLElement;
    while (scroller && scroller.scrollHeight <= scroller.clientHeight + 4) {
      scroller = scroller.parentElement;
    }
    if (!scroller) return;
    const box = scroller.getBoundingClientRect();
    const edge = 56;
    if (event.clientY - box.top < edge) {
      scroller.scrollTop -= Math.ceil((edge - (event.clientY - box.top)) / 3);
    } else if (box.bottom - event.clientY < edge) {
      scroller.scrollTop += Math.ceil((edge - (box.bottom - event.clientY)) / 3);
    }
  };

  const append = (make: () => Step) => onChange([...steps, make()]);

  /** A palette button adds the block it makes, whether it is clicked or
   * dropped: most make their kind's template, and drag makes a click block in
   * its drag mode. */
  const fresh = (kind: Step["kind"]) => () => BLOCK_TEMPLATES[kind]();

  const applyDrop = (target: Drop) => {
    if (!drag) return;
    if (drag.source === "palette") {
      onChange(insertStep(steps, target.parent, target.index, drag.make()));
    } else {
      onChange(moveStep(steps, drag.path, target.parent, target.index));
    }
    setDrag(null);
    setDrop(null);
  };

  return (
    <section
      className="hb__builder hb__builder--lead"
      data-dragging={drag ? "true" : undefined}
      onDragOver={autoScroll}
      onDragEnd={() => {
        setDrag(null);
        setDrop(null);
      }}
    >
      {/* The palette is pinned at the top of the builder: the macro scrolls
          under it, so the blocks are always within reach however long the
          macro gets. Tabs keep it to two rows instead of five stacked groups. */}
      <div className="pal">
        <div className="pal__tabs" role="tablist" aria-label="Block groups">
          {BLOCK_GROUPS.map((group, index) => (
            <button
              key={group.title}
              type="button"
              role="tab"
              aria-selected={index === paletteGroup}
              className={`pal__tab${index === paletteGroup ? " pal__tab--on" : ""}`}
              onClick={() => setPaletteGroup(index)}
            >
              {group.title}
            </button>
          ))}
        </div>
        <div className="pal__row">
          {shownGroup.kinds.map((kind) => (
            <button
              key={kind}
              type="button"
              className="pal__item"
              draggable
              title={`Drag “${BLOCK_LABELS[kind]}” into the macro, or click to add it at the end`}
              onDragStart={() => setDrag({ source: "palette", make: fresh(kind) })}
              onClick={() => append(fresh(kind))}
            >
              {BLOCK_LABELS[kind]}
            </button>
          ))}
          {/* A drag is the click block's mode, but it sits here as its own
              button: it is the block people reach for by name, and one drop
              puts it anywhere in the tree, including inside a Repeat. */}
          {shownGroup.title === "Input" && (
            <button
              type="button"
              className="pal__item"
              draggable
              title="Drag “Click and drag” into the macro, or click to add it at the end"
              onDragStart={() => setDrag({ source: "palette", make: DRAG_TEMPLATE })}
              onClick={() => append(DRAG_TEMPLATE)}
            >
              Click and drag
            </button>
          )}
        </div>
        <span className="pal__hint">Drag a block into the macro, or click to add it at the end.</span>
      </div>

      <div className="blk__list" style={{ marginTop: 12 }}>
        {steps.map((step, index) => (
          <BlockNode
            key={keyAt([index])}
            step={step}
            path={[index]}
            depth={0}
            keys={keys}
            picking={picking}
            onPick={onPick}
            drag={drag}
            drop={drop}
            setDrag={setDrag}
            setDrop={setDrop}
            onDropTarget={applyDrop}
            onReplace={(path, next) => onChange(replaceStep(steps, path, next))}
            onRemove={(path) => {
              const previous = steps;
              onChange(removeStep(steps, path));
              pushUndo("Removed a block", () => onChange(previous));
            }}
            onSwap={(path, by) => onChange(swap(steps, path, by))}
          />
        ))}
        <Tail
          parent={[]}
          active={Boolean(drop && samePath(drop.parent, []) && drop.index === steps.length)}
          onHover={() => setDrop({ parent: [], index: steps.length })}
          onDrop={() => applyDrop({ parent: [], index: steps.length })}
          empty={steps.length === 0}
        />
      </div>
    </section>
  );
}

function BlockNode({
  step,
  path,
  depth,
  keys,
  picking,
  onPick,
  drag,
  drop,
  setDrag,
  setDrop,
  onDropTarget,
  onReplace,
  onRemove,
  onSwap,
}: {
  step: Step;
  path: Path;
  depth: number;
  keys: View["keys"];
  picking: Path | null;
  onPick: (path: Path) => void;
  drag: Drag | null;
  drop: Drop | null;
  setDrag: (drag: Drag | null) => void;
  setDrop: (drop: Drop | null) => void;
  onDropTarget: (target: Drop) => void;
  onReplace: (path: Path, step: Step) => void;
  onRemove: (path: Path) => void;
  onSwap: (path: Path, by: number) => void;
}) {
  const tail = path[path.length - 1];
  const index = typeof tail === "number" ? tail : 0;
  const parent = path.slice(0, -1);
  const isRepeat = step.kind === "repeat" || step.kind === "while";
  const dragging = drag?.source === "tree" && samePath(drag.path, path);

  /** Which third of the row the pointer is over decides where it lands. */
  const place = (event: React.DragEvent<HTMLDivElement>): Drop => {
    const rect = event.currentTarget.getBoundingClientRect();
    const ratio = (event.clientY - rect.top) / Math.max(1, rect.height);
    if (isRepeat && ratio > 0.34 && ratio < 0.66 && depth < MAX_DEPTH) {
      const children =
        step.kind === "repeat" || step.kind === "while" ? step.steps.length : 0;
      return { parent: path, index: children };
    }
    return ratio < 0.5 ? { parent, index } : { parent, index: index + 1 };
  };

  const marked =
    drop &&
    (samePath(drop.parent, parent) && (drop.index === index || drop.index === index + 1));

  return (
    <div className={`blk__wrap${marked ? " blk__wrap--marked" : ""}`}>
      {/* The row is its own drop surface: the third of it the pointer is over
          decides whether the block lands before, after or inside. It is never a
          surface for the drag that started on it, a node added under the drag
          source while `dragstart` runs makes the webview cancel the drag on the
          spot, and it is not a layer of its own either: a child element laid
          over the row would sit over the grip as well, so a drag the builder
          failed to notice would leave nothing to pick the block up by. */}
      <div
        className={`blk${dragging ? " blk--dragging" : ""}`}
        data-blk={path.join(".")}
        onDragOver={
          drag && !dragging
            ? (event: React.DragEvent<HTMLDivElement>) => {
                event.preventDefault();
                setDrop(place(event));
              }
            : undefined
        }
        onDrop={
          drag && !dragging
            ? (event: React.DragEvent<HTMLDivElement>) => {
                event.preventDefault();
                onDropTarget(place(event));
              }
            : undefined
        }
      >
        <div className="blk__body">
          <div className="blk__title-row">
            <span
              className="blk__grip"
              draggable
              title="Drag to move this block"
              onDragStart={() => setDrag({ source: "tree", path })}
              aria-hidden="true"
            >
              ⠿
            </span>
            <span className="blk__title">{blockTitle(step)}</span>
            <span className="hb__spacer" />
            <div className="blk__buttons">
              <button
                type="button"
                className="hb__delete"
                aria-label="Move this block up"
                onClick={() => onSwap(path, -1)}
              >
                ↑
              </button>
              <button
                type="button"
                className="hb__delete"
                aria-label="Move this block down"
                onClick={() => onSwap(path, 1)}
              >
                ↓
              </button>
              <button
                type="button"
                className="hb__delete"
                aria-label={`Remove the ${blockTitle(step)} block`}
                onClick={() => onRemove(path)}
              >
                ×
              </button>
            </div>
          </div>
          <p className="blk__summary">{stepSummary(step, keys)}</p>
          <BlockFields
            step={step}
            keys={keys}
            picking={Boolean(picking && samePath(picking, path))}
            onPick={() => onPick(path)}
            onReplace={(next) => onReplace(path, next)}
          />
        </div>

      </div>

      {branchAreas(step, path).map((area) => (
        <div
          key={area.key}
          className={`blk__children${drop && samePath(drop.parent, area.parent) ? " blk__children--active" : ""}`}
        >
          {area.label && <span className="blk__branch">{area.label}</span>}
          {area.steps.map((child, at) => (
            <BlockNode
              key={keyAt([...area.parent, at])}
              step={child}
              path={[...area.parent, at]}
              depth={depth + 1}
              keys={keys}
              picking={picking}
              onPick={onPick}
              drag={drag}
              drop={drop}
              setDrag={setDrag}
              setDrop={setDrop}
              onDropTarget={onDropTarget}
              onReplace={onReplace}
              onRemove={onRemove}
              onSwap={onSwap}
            />
          ))}
          <Tail
            parent={area.parent}
            active={Boolean(drop && samePath(drop.parent, area.parent))}
            onHover={() => setDrop({ parent: area.parent, index: area.steps.length })}
            onDrop={() => onDropTarget({ parent: area.parent, index: area.steps.length })}
            empty={area.steps.length === 0}
            label={area.emptyLabel}
          />
        </div>
      ))}
    </div>
  );
}

function Tail({
  parent,
  active,
  onHover,
  onDrop,
  empty,
  label = "Drop a block here",
}: {
  parent: Path;
  active: boolean;
  onHover: () => void;
  onDrop: () => void;
  empty: boolean;
  label?: string;
}) {
  void parent;
  return (
    <div
      className={`blk__tail${active ? " blk__tail--active" : ""}${empty ? " blk__tail--empty" : ""}`}
      onDragOver={(event) => {
        event.preventDefault();
        onHover();
      }}
      onDrop={(event) => {
        event.preventDefault();
        onDrop();
      }}
    >
      {label}
    </div>
  );
}

/**
 * A relative move written as a direction and a distance, or as two numbers.
 *
 * Both are the same x and y: left is a negative x, down a positive y. Working
 * in a direction is how a nudge is actually thought about: a jiggle is twenty
 * pixels right and back, while a diagonal needs the two numbers, so the block
 * keeps both and the mode only decides which one is on screen.
 */
function OffsetFields({
  x,
  y,
  onChange,
}: {
  x: number;
  y: number;
  onChange: (x: number, y: number) => void;
}) {
  // A block on one axis is already a nudge, and a diagonal one is already two
  // numbers: which mode it opens in is read from the value rather than stored.
  const [way, setWay] = useState<"direction" | "numbers">(
    x === 0 || y === 0 ? "direction" : "numbers",
  );
  const direction =
    x < 0 ? "left" : x > 0 ? "right" : y < 0 ? "up" : y > 0 ? "down" : "right";
  const distance = Math.abs(x) || Math.abs(y);

  const nudge = (heading: string, px: number) => {
    const gap = Math.max(0, Math.min(100_000, px));
    if (heading === "left") onChange(-gap, 0);
    else if (heading === "right") onChange(gap, 0);
    else if (heading === "up") onChange(0, -gap);
    else onChange(0, gap);
  };

  return (
    <>
      <SegmentedControl
        label="Offset"
        options={[
          { value: "direction", label: "Direction" },
          { value: "numbers", label: "X and Y" },
        ]}
        value={way}
        onValueChange={(value) => setWay(value as typeof way)}
      />
      {way === "direction" ? (
        <>
          <SegmentedControl
            label="Direction"
            options={[
              { value: "left", label: "Left" },
              { value: "right", label: "Right" },
              { value: "up", label: "Up" },
              { value: "down", label: "Down" },
            ]}
            value={direction}
            onValueChange={(value) => nudge(value, distance)}
          />
          <NumberField
            label="Pixels"
            value={distance}
            onValueChange={(px) => nudge(direction, px)}
          />
          <span className="hb__field-label">px</span>
        </>
      ) : (
        <PointFields x={x} y={y} onChange={onChange} />
      )}
    </>
  );
}

/**
 * The two coordinate boxes of a point block.
 *
 * Labelled X and Y rather than two anonymous numbers, and signed, because a
 * screen left of the main one has negative x and one above it has negative y.
 * Typed entries, not spinners: a stepper moves a coordinate by one pixel, which
 * is never the correction anyone wants.
 */
function PointFields({
  x,
  y,
  onChange,
}: {
  x: number;
  y: number;
  onChange: (x: number, y: number) => void;
}) {
  return (
    <>
      <span className="blk__coord">
        <span className="blk__axis" aria-hidden="true">
          X
        </span>
        <NumberField
          signed
          label="X coordinate"
          value={x}
          onValueChange={(next) => onChange(next, y)}
        />
      </span>
      <span className="blk__coord">
        <span className="blk__axis" aria-hidden="true">
          Y
        </span>
        <NumberField
          signed
          label="Y coordinate"
          value={y}
          onValueChange={(next) => onChange(x, next)}
        />
      </span>
    </>
  );
}

/**
 * The picture a block looks for, with a thumbnail once one is picked.
 *
 * A macro that looks for the same thing in several blocks, wait for it, click
 * it, loop while it is on screen, would otherwise mean dragging the same
 * rectangle out again for each one. Every picture ever picked is still on disk,
 * so the block offers them back instead: the same picture is one click rather
 * than a second trip across the screen, and the blocks that share it keep
 * matching each other exactly.
 */
function PictureField({
  image,
  picking,
  onPick,
  onChoose,
}: {
  image: string;
  picking: boolean;
  onPick: () => void;
  /** Take a picture that is already saved rather than drawing a new one. */
  onChoose: (id: string) => void;
}) {
  const [src, setSrc] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [saved, setSaved] = useState<Picture[] | null>(null);
  // Why a picture could not be removed, in the shelf rather than off-screen,
  // and where it is still used, so the refusal says what to change.
  const [problem, setProblem] = useState<string | null>(null);
  const [users, setUsers] = useState<PictureUser[]>([]);

  useEffect(() => {
    let live = true;
    if (!image) {
      setSrc(null);
      return;
    }
    picturePreview(image)
      .then((url) => {
        if (live) setSrc(url);
      })
      .catch(() => {
        if (live) setSrc(null);
      });
    return () => {
      live = false;
    };
  }, [image]);

  /** Take a picture off the shelf, unless a block still looks for it. */
  const forget = (id: string) => {
    setProblem(null);
    setUsers([]);
    forgetPicture(id)
      .then(() => setSaved((list) => (list ?? []).filter((entry) => entry.id !== id)))
      .catch((reason) => {
        setProblem(String(reason));
        // Which macros and blocks hold it: the answer to "so what do I change?".
        pictureUses(id)
          .then(setUsers)
          .catch(() => undefined);
      });
  };

  // Read the folder each time the shelf is opened: a picture picked in another
  // block since the last look is exactly the one worth offering.
  useEffect(() => {
    if (!open) return;
    let live = true;
    setSaved(null);
    pictureHistory()
      .then((list) => {
        if (live) setSaved(list);
      })
      .catch(() => {
        if (live) setSaved([]);
      });
    return () => {
      live = false;
    };
  }, [open]);

  return (
    <>
      <span
        className="blk__picture"
        style={{ display: "inline-flex", alignItems: "center", gap: 8 }}
      >
        {src ? (
          <img
            src={src}
            alt=""
            style={{ height: 30, maxWidth: 120, objectFit: "contain", borderRadius: 4 }}
          />
        ) : (
          <span className="blk__hint">
            {image ? "the picture is missing, pick it again" : "No picture yet"}
          </span>
        )}
        <button type="button" className="hb__mini" disabled={picking} onClick={onPick}>
          {picking ? "Waiting for the screen…" : src ? "Pick another…" : "Pick picture…"}
        </button>
        <button type="button" className="hb__mini" onClick={() => setOpen((was) => !was)}>
          {open ? "Hide saved" : "Use a saved one…"}
        </button>
      </span>

      {open && (
        <div className="blk__gallery">
          {saved === null ? (
            <span className="blk__hint">Reading the pictures folder…</span>
          ) : saved.length === 0 ? (
            <span className="blk__hint">
              Nothing saved yet. A picture is offered here once it has been picked.
            </span>
          ) : (
            saved.map((entry) => (
              <div className="blk__shot-wrap" key={entry.id}>
                <button
                  type="button"
                  className={`blk__shot${entry.id === image ? " blk__shot--on" : ""}`}
                  title={`${entry.width} × ${entry.height}, use this picture`}
                  onClick={() => {
                    onChoose(entry.id);
                    setOpen(false);
                  }}
                >
                  <Shot id={entry.id} />
                </button>
                <button
                  type="button"
                  className="blk__shot-drop"
                  aria-label="Remove this picture from the shelf"
                  title="Remove this picture from the shelf"
                  onClick={() => forget(entry.id)}
                >
                  ×
                </button>
              </div>
            ))
          )}
          {problem && (
            <span className="blk__hint" style={{ flexBasis: "100%", color: "var(--accent)" }}>
              {problem}
            </span>
          )}
          {users.length > 0 && (
            <ul className="blk__uses">
              {users.map((user) => (
                <li key={user.id}>
                  <span className="blk__uses-macro">{user.macroName}</span>
                  {user.blocks.map((hit) => (
                    <button
                      key={hit.path.join(".")}
                      type="button"
                      className="blk__uses-block"
                      title="Show this block"
                      onClick={() =>
                        window.dispatchEvent(
                          new CustomEvent(SHOW_BLOCK, {
                            detail: { macroId: user.id, path: hit.path },
                          }),
                        )
                      }
                    >
                      {hit.label}
                    </button>
                  ))}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </>
  );
}

/** One thumbnail in the shelf, loaded the same way the block's own is. */
function Shot({ id }: { id: string }) {
  const [src, setSrc] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    picturePreview(id)
      .then((url) => {
        if (live) setSrc(url);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [id]);

  return src ? <img src={src} alt="" /> : <span className="blk__shot-blank" />;
}

/** How alike the screen has to be for a picture to count, as a percentage. */
function SimilarityField({
  value,
  onChange,
}: {
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="hb__check" style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
      <input
        type="range"
        min={50}
        max={100}
        step={1}
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
        style={{ width: 96 }}
      />
      <span className="hb__field-label">{value}% alike</span>
    </label>
  );
}

/**
 * Another macro, or a recording, chosen by name with its id kept beside it.
 *
 * Both lists come from the same command; which one is shown is the only thing
 * the two blocks differ by, so there is one field rather than two.
 */
function ChoiceField({
  kind,
  value,
  onChange,
}: {
  kind: "macros" | "recordings";
  value: string;
  onChange: (choice: { id: string; name: string }) => void;
}) {
  const [choices, setChoices] = useState<{ id: string; name: string }[]>([]);

  useEffect(() => {
    let live = true;
    blockChoices()
      .then((all) => {
        if (live) setChoices(all[kind]);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [kind]);

  return (
    <select
      className="hb__field hb__field--short"
      value={value}
      onChange={(event) => {
        const found = choices.find((choice) => choice.id === event.target.value);
        onChange(found ?? { id: "", name: "" });
      }}
    >
      <option value="">
        {kind === "macros" ? "choose a macro" : "choose a recording"}
      </option>
      {choices.map((choice) => (
        <option key={choice.id} value={choice.id}>
          {choice.name}
        </option>
      ))}
      {/* A block whose macro was renamed, or deleted, still says which one it
          was saved with instead of silently showing nothing. */}
      {value && !choices.some((choice) => choice.id === value) && (
        <option value={value}>the one this block was saved with</option>
      )}
    </select>
  );
}

/**
 * The title of a window to look for, with the windows that are open right now
 * offered as suggestions. Still a plain text field: the window a macro wants is
 * often not running while the macro is being written.
 */
function WindowTitleField({
  contains,
  onChange,
}: {
  contains: string;
  onChange: (title: string) => void;
}) {
  const [titles, setTitles] = useState<string[]>([]);
  // Each block gets its own list: the id is the only thing tying the field to
  // it, and two blocks sharing one would offer the same titles twice.
  const id = useRef(`hb-windows-${Math.random().toString(36).slice(2, 8)}`).current;

  useEffect(() => {
    let live = true;
    runningWindows()
      .then((open) => {
        if (live) setTitles(open.slice(0, 40));
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);

  return (
    <>
      <input
        className="hb__field hb__field--short"
        list={id}
        aria-label="Window title"
        placeholder="part of the title"
        value={contains}
        onChange={(event) => onChange(event.target.value)}
      />
      <datalist id={id}>
        {titles.map((title) => (
          <option key={title} value={title} />
        ))}
      </datalist>
    </>
  );
}

/**
 * A program's path, with the system's file dialog behind a button so nobody has
 * to type one. The dialog is modal and answers long after the click, so it
 * arrives as an event, and only the field that opened it takes the answer.
 */
function ProgramField({
  value,
  onChange,
}: {
  value: string;
  onChange: (path: string) => void;
}) {
  const [choosing, setChoosing] = useState(false);
  const waiting = useRef(false);
  const latest = useRef(onChange);

  useEffect(() => {
    latest.current = onChange;
  }, [onChange]);

  useEffect(() => {
    let live = true;
    void on<string>(MACRO_PROGRAM, (path) => {
      if (!waiting.current) return;
      waiting.current = false;
      setChoosing(false);
      latest.current(path);
    }).then((stop) => {
      if (!live) stop();
    });
    return () => {
      live = false;
    };
  }, []);

  return (
    <>
      <input
        className="hb__field hb__field--short"
        aria-label="Program"
        placeholder="its full path"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
      <button
        type="button"
        className="hb__mini"
        disabled={choosing}
        onClick={() => {
          waiting.current = true;
          setChoosing(true);
          void chooseProgram();
        }}
      >
        {choosing ? "Choosing…" : "Choose program…"}
      </button>
    </>
  );
}

/** The question an If or a While asks, borrowed from the block that asks it. */
type MacroCondition = Extract<Step, { kind: "if" }>["condition"];

/**
 * The question an If or a While asks.
 *
 * Shared, because the two blocks differ in what they do with the answer, not
 * in how they ask it.
 */
function ConditionFields({
  condition,
  picking,
  onPick,
  onChange,
}: {
  condition: MacroCondition;
  picking: boolean;
  onPick: () => void;
  onChange: (condition: MacroCondition) => void;
}) {
  return (
    <>
      <SegmentedControl
        label="Check"
        options={[
          { value: "picture", label: "A picture" },
          { value: "front", label: "A program" },
        ]}
        value={condition.kind}
        onValueChange={(value) =>
          onChange(
            value === "picture"
              ? { kind: "picture", image: "", similarity: 90 }
              : { kind: "front", program: "" },
          )
        }
      />
      {condition.kind === "picture" ? (
        <>
          <PictureField
            image={condition.image}
            picking={picking}
            onPick={onPick}
            onChoose={(id) => onChange({ ...condition, image: id })}
          />
          <SimilarityField
            value={condition.similarity}
            onChange={(similarity) => onChange({ ...condition, similarity })}
          />
        </>
      ) : (
        <input
          className="hb__field hb__field--short"
          list="hb-macro-programs"
          aria-label="Program to check for"
          placeholder="any program"
          value={condition.program}
          onChange={(event) => onChange({ kind: "front", program: event.target.value })}
        />
      )}
    </>
  );
}

/** The parameter controls for one block, inline so nothing needs a second click. */
function BlockFields({
  step,
  keys,
  picking,
  onPick,
  onReplace,
}: {
  step: Step;
  keys: View["keys"];
  /** This block is the one waiting for a spot on screen. */
  picking: boolean;
  onPick: () => void;
  onReplace: (step: Step) => void;
}) {
  switch (step.kind) {
    case "key":
      return (
        <div className="blk__fields">
          {/* Pressing the key is what the shortcut field does, so it is what this
              does too: the list below is for finding one by name. */}
          <KeyBinder
            mouse
            scan={step.scan}
            label={keys.find((key) => key.scan === step.scan)?.label ?? keyLabel(step.scan)}
            onChange={(scan) =>
              onReplace({
                ...step,
                scan,
                // Shift belongs to a key: a button has no shifted case.
                shift: isMouseScan(scan) ? false : step.shift,
              })
            }
          />
          <SegmentedControl
            label="Key mode"
            options={[
              { value: "tap", label: "Tap" },
              { value: "hold", label: "Hold" },
              { value: "down", label: "Down" },
              { value: "up", label: "Up" },
            ]}
            value={step.mode}
            onValueChange={(value) =>
              onReplace({ ...step, mode: value as typeof step.mode })
            }
          />
          {step.mode === "hold" && (
            <>
              <NumberField
                label="Hold time"
                value={step.holdMs}
                onValueChange={(holdMs) => onReplace({ ...step, holdMs })}
              />
              <span className="hb__field-label">ms</span>
            </>
          )}
          {(step.mode === "tap" || step.mode === "hold") &&
            !isMouseScan(step.scan) && (
            <label className="hb__check">
              <input
                type="checkbox"
                checked={step.shift}
                onChange={(event) =>
                  onReplace({ ...step, shift: event.target.checked })
                }
              />
              with Shift
            </label>
          )}
        </div>
      );

    case "text":
      return (
        <div className="blk__fields">
          <input
            className="hb__field"
            aria-label="Text to type"
            value={step.text}
            onChange={(event) => onReplace({ ...step, text: event.target.value })}
          />
          <NumberField
            label="Delay between characters"
            value={step.perCharMs}
            onValueChange={(perCharMs) => onReplace({ ...step, perCharMs })}
          />
          <span className="hb__field-label">ms a character</span>
        </div>
      );

    case "move":
      return (
        <div className="blk__fields">
          <SegmentedControl
            label="Move"
            options={[
              { value: "to", label: "To a point" },
              { value: "by", label: "By an offset" },
            ]}
            value={step.relative ? "by" : "to"}
            onValueChange={(value) => onReplace({ ...step, relative: value === "by" })}
          />
          {step.relative ? (
            <OffsetFields
              x={step.x}
              y={step.y}
              onChange={(x, y) => onReplace({ ...step, x, y })}
            />
          ) : (
            <PointFields
              x={step.x}
              y={step.y}
              onChange={(x, y) => onReplace({ ...step, x, y })}
            />
          )}
          {/* Absolute moves can be pointed at instead of measured. */}
          {!step.relative && (
            <button
              type="button"
              className="hb__mini"
              disabled={picking}
              onClick={onPick}
            >
              {picking ? "Waiting for a spot…" : "Pick spot…"}
            </button>
          )}
          {!step.relative && (
            <>
              <NumberField
                label="Travel time"
                value={step.durationMs}
                onValueChange={(durationMs) => onReplace({ ...step, durationMs })}
              />
              <span className="hb__field-label">ms to get there</span>
            </>
          )}
          <span className="blk__hint">
            {step.relative
              ? "The cursor moves from wherever it is: negative x goes left, negative y goes up."
              : "A negative coordinate points to a screen left of or above the main one."}
          </span>
        </div>
      );

    case "click":
      return (
        <div className="blk__fields">
          <SegmentedControl
            label="Mouse button"
            options={[
              { value: "left", label: "Left" },
              { value: "right", label: "Right" },
              { value: "middle", label: "Middle" },
            ]}
            value={step.button}
            onValueChange={(value) =>
              onReplace({ ...step, button: value as MouseButton })
            }
          />
          <SegmentedControl
            label="Click mode"
            options={[
              { value: "tap", label: "Tap" },
              { value: "hold", label: "Hold" },
              { value: "down", label: "Down" },
              { value: "up", label: "Up" },
              { value: "drag", label: "Drag" },
            ]}
            value={step.mode}
            onValueChange={(value) =>
              onReplace({ ...step, mode: value as typeof step.mode })
            }
          />
          {/* The whole click-and-drag in one block: press where the cursor is,
              travel, rest, let go. */}
          {step.mode === "drag" && (
            <>
              <SegmentedControl
                label="Drag"
                options={[
                  { value: "by", label: "By an offset" },
                  { value: "to", label: "To a point" },
                ]}
                value={step.relative ? "by" : "to"}
                onValueChange={(value) =>
                  onReplace({ ...step, relative: value === "by" })
                }
              />
              {step.relative ? (
                <OffsetFields
                  x={step.x}
                  y={step.y}
                  onChange={(x, y) => onReplace({ ...step, x, y })}
                />
              ) : (
                <PointFields
                  x={step.x}
                  y={step.y}
                  onChange={(x, y) => onReplace({ ...step, x, y })}
                />
              )}
              {!step.relative && (
                <button
                  type="button"
                  className="hb__mini"
                  disabled={picking}
                  onClick={onPick}
                >
                  {picking ? "Waiting for a spot…" : "Pick spot…"}
                </button>
              )}
              {!step.relative && (
                <>
                  <NumberField
                    label="Travel time"
                    value={step.durationMs}
                    onValueChange={(durationMs) => onReplace({ ...step, durationMs })}
                  />
                  <span className="hb__field-label">ms to get there</span>
                </>
              )}
              <span className="blk__hint">
                {step.relative
                  ? "Presses where the cursor is, moves by that offset, and releases."
                  : "Presses where the cursor is, moves to that point, and releases."}
              </span>
            </>
          )}
          {/* Only a tap repeats, so only a tap carries a count. */}
          {step.mode === "tap" && (
            <>
              <span className="hb__field-label">×</span>
              <NumberField
                label="Clicks"
                value={step.count}
                onValueChange={(count) =>
                  onReplace({ ...step, count: Math.max(1, Math.min(100_000, count)) })
                }
              />
            </>
          )}
          {(step.mode === "tap" || step.mode === "hold" || step.mode === "drag") && (
            <>
              <NumberField
                label="Hold time"
                value={step.holdMs}
                onValueChange={(holdMs) => onReplace({ ...step, holdMs })}
                title={
                  step.mode === "drag"
                    ? "How long the button stays down at the end of the drag before it releases."
                    : "How long the button stays down. A tap holds for at least 15 ms, one frame, because a game can miss a press and a release that arrive together."
                }
              />
              <span className="hb__field-label">
                {step.mode === "hold"
                  ? "ms held"
                  : step.mode === "drag"
                    ? "ms before letting go"
                    : "ms down"}
              </span>
            </>
          )}
        </div>
      );

    case "if": {
      const condition = step.condition;
      return (
        <div className="blk__fields">
          <ConditionFields
            condition={condition}
            picking={picking}
            onPick={onPick}
            onChange={(next) => onReplace({ ...step, condition: next })}
          />
          
        </div>
      );
    }

    case "waitImage":
      return (
        <div className="blk__fields">
          <PictureField
            image={step.image}
            picking={picking}
            onPick={onPick}
            onChoose={(id) => onReplace({ ...step, image: id })}
          />
          <NumberField
            label="Wait up to"
            value={step.timeoutMs}
            onValueChange={(timeoutMs) => onReplace({ ...step, timeoutMs })}
          />
          <span className="hb__field-label">ms, or 0 to look once</span>
          <SimilarityField
            value={step.similarity}
            onChange={(similarity) => onReplace({ ...step, similarity })}
          />
          
        </div>
      );

    case "clickImage":
      return (
        <div className="blk__fields">
          <PictureField
            image={step.image}
            picking={picking}
            onPick={onPick}
            onChoose={(id) => onReplace({ ...step, image: id })}
          />
          <SegmentedControl
            label="Mouse button"
            options={[
              { value: "left", label: "Left" },
              { value: "right", label: "Right" },
              { value: "middle", label: "Middle" },
            ]}
            value={step.button}
            onValueChange={(value) =>
              onReplace({ ...step, button: value as MouseButton })
            }
          />
          <NumberField
            label="Wait up to"
            value={step.timeoutMs}
            onValueChange={(timeoutMs) => onReplace({ ...step, timeoutMs })}
          />
          <span className="hb__field-label">ms before giving up</span>
          <SimilarityField
            value={step.similarity}
            onChange={(similarity) => onReplace({ ...step, similarity })}
          />
          
        </div>
      );

    case "scroll":
      return (
        <div className="blk__fields">
          <SegmentedControl
            label="Scroll direction"
            options={[
              { value: "up", label: "Up" },
              { value: "down", label: "Down" },
            ]}
            value={step.amount < 0 ? "down" : "up"}
            onValueChange={(value) =>
              onReplace({ ...step, amount: Math.abs(step.amount) * (value === "up" ? 1 : -1) })
            }
          />
          <NumberField
            label="Notches"
            value={Math.abs(step.amount)}
            onValueChange={(notches) =>
              onReplace({
                ...step,
                amount:
                  Math.max(1, Math.min(100, notches)) * (step.amount < 0 ? -1 : 1),
              })
            }
          />
          <span className="hb__field-label">notches</span>
        </div>
      );

    case "delay":
      return (
        <div className="blk__fields">
          <NumberField
            label="Wait time"
            value={step.ms}
            onValueChange={(ms) => onReplace({ ...step, ms })}
          />
          <span className="hb__field-label">ms</span>
        </div>
      );

    case "repeat":
      return (
        <div className="blk__fields">
          <span className="hb__field-label">Run these</span>
          <NumberField
            label="Repeat count"
            value={step.times}
            onValueChange={(times) =>
              onReplace({ ...step, times: Math.max(1, Math.min(9_999, times)) })
            }
          />
          <span className="hb__field-label">times over</span>
        </div>
      );

    case "note":
      return (
        <div className="blk__fields">
          <input
            className="hb__field"
            aria-label="Note"
            placeholder="What this part of the macro is for"
            value={step.text}
            onChange={(event) => onReplace({ ...step, text: event.target.value })}
          />
          
        </div>
      );

    case "log":
      return (
        <div className="blk__fields">
          <input
            className="hb__field"
            aria-label="Log line"
            placeholder="where the run has got to"
            value={step.text}
            onChange={(event) => onReplace({ ...step, text: event.target.value })}
          />
          
        </div>
      );

    case "randomDelay":
      return (
        <div className="blk__fields">
          <NumberField
            label="At least"
            value={step.minMs}
            onValueChange={(minMs) => onReplace({ ...step, minMs })}
          />
          <span className="hb__field-label">to</span>
          <NumberField
            label="At most"
            value={step.maxMs}
            onValueChange={(maxMs) => onReplace({ ...step, maxMs })}
          />
          <span className="hb__field-label">ms, picked at random</span>
          
        </div>
      );

    case "chord":
      return (
        <div className="blk__fields">
          {step.scans.map((scan, index) => (
            <div key={index} className="blk__fields">
              <KeyBinder
                scan={scan}
                label={keys.find((key) => key.scan === scan)?.label ?? keyLabel(scan)}
                onChange={(next) => {
                  const scans = [...step.scans];
                  scans[index] = next;
                  onReplace({ ...step, scans });
                }}
              />
              <button
                type="button"
                className="hb__mini"
                onClick={() =>
                  onReplace({ ...step, scans: step.scans.filter((_, at) => at !== index) })
                }
              >
                Remove
              </button>
            </div>
          ))}
          {step.scans.length < 4 && (
            <button
              type="button"
              className="hb__mini"
              onClick={() => onReplace({ ...step, scans: [...step.scans, 0x1d] })}
            >
              Add a key
            </button>
          )}
          <NumberField
            label="Hold together"
            value={step.holdMs}
            onValueChange={(holdMs) => onReplace({ ...step, holdMs })}
          />
          <span className="hb__field-label">ms</span>
          
        </div>
      );

    case "scrollAt":
      return (
        <div className="blk__fields">
          <PointFields
            x={step.x}
            y={step.y}
            onChange={(x, y) => onReplace({ ...step, x, y })}
          />
          <button type="button" className="hb__mini" disabled={picking} onClick={onPick}>
            {picking ? "Waiting for a spot…" : "Pick spot…"}
          </button>
          <SegmentedControl
            label="Scroll direction"
            options={[
              { value: "up", label: "Up" },
              { value: "down", label: "Down" },
            ]}
            value={step.amount < 0 ? "down" : "up"}
            onValueChange={(value) =>
              onReplace({ ...step, amount: Math.abs(step.amount) * (value === "up" ? 1 : -1) })
            }
          />
          <NumberField
            label="Notches"
            value={Math.abs(step.amount)}
            onValueChange={(notches) =>
              onReplace({
                ...step,
                amount: Math.max(1, Math.min(100, notches)) * (step.amount < 0 ? -1 : 1),
              })
            }
          />
          
        </div>
      );

    case "until":
      return (
        <div className="blk__fields">
          <input
            className="hb__field hb__field--short"
            type="time"
            aria-label="Time of day"
            value={step.at}
            onChange={(event) => onReplace({ ...step, at: event.target.value })}
          />
          
        </div>
      );

    case "repeatFor":
      return (
        <div className="blk__fields">
          <span className="hb__field-label">Keep running these blocks for</span>
          <NumberField
            label="Seconds"
            value={step.seconds}
            onValueChange={(seconds) =>
              onReplace({ ...step, seconds: Math.max(1, Math.min(3_600, seconds)) })
            }
          />
          <span className="hb__field-label">seconds</span>
        </div>
      );

    case "stop":
      return (
        <div className="blk__fields">
          <input
            className="hb__field"
            aria-label="Stop reason"
            placeholder="why the run ends here"
            value={step.reason}
            onChange={(event) => onReplace({ ...step, reason: event.target.value })}
          />
          
        </div>
      );

    case "fail":
      return (
        <div className="blk__fields">
          <input
            className="hb__field"
            aria-label="Failure message"
            placeholder="what went wrong"
            value={step.message}
            onChange={(event) => onReplace({ ...step, message: event.target.value })}
          />
          
        </div>
      );

    case "assertImage":
      return (
        <div className="blk__fields">
          <PictureField
            image={step.image}
            picking={picking}
            onPick={onPick}
            onChoose={(id) => onReplace({ ...step, image: id })}
          />
          <SimilarityField
            value={step.similarity}
            onChange={(similarity) => onReplace({ ...step, similarity })}
          />
          
        </div>
      );

    case "focus": {
      const target = step.target;
      return (
        <div className="blk__fields">
          <SegmentedControl
            label="Find the window by"
            options={[
              { value: "title", label: "Its title" },
              { value: "program", label: "Its program" },
            ]}
            value={target.kind}
            onValueChange={(value) =>
              onReplace({
                ...step,
                target:
                  value === "title"
                    ? { kind: "title", contains: "", exact: false }
                    : { kind: "program", exe: "" },
              })
            }
          />
          {target.kind === "title" ? (
            <>
              <WindowTitleField
                contains={target.contains}
                onChange={(contains) =>
                  onReplace({ ...step, target: { ...target, contains } })
                }
              />
              <label className="hb__check">
                <input
                  type="checkbox"
                  checked={target.exact}
                  onChange={(event) =>
                    onReplace({
                      ...step,
                      target: { ...target, exact: event.target.checked },
                    })
                  }
                />
                the whole title, exactly
              </label>
            </>
          ) : (
            <input
              className="hb__field hb__field--short"
              list="hb-macro-programs"
              aria-label="Program to focus"
              placeholder="game.exe"
              value={target.exe}
              onChange={(event) =>
                onReplace({
                  ...step,
                  target: { kind: "program", exe: event.target.value },
                })
              }
            />
          )}
          <NumberField
            label="Wait up to"
            value={step.timeoutMs}
            onValueChange={(timeoutMs) => onReplace({ ...step, timeoutMs })}
          />
          <span className="hb__field-label">ms for a window to turn up</span>
          
        </div>
      );
    }

    case "launch":
      return (
        <div className="blk__fields">
          <ProgramField
            value={step.path}
            onChange={(path) => onReplace({ ...step, path })}
          />
          <input
            className="hb__field"
            aria-label="Arguments"
            placeholder="--windowed"
            value={step.args}
            onChange={(event) => onReplace({ ...step, args: event.target.value })}
          />
          <label className="hb__check">
            <input
              type="checkbox"
              checked={step.waitForWindow}
              onChange={(event) =>
                onReplace({ ...step, waitForWindow: event.target.checked })
              }
            />
            wait for its window
          </label>
          {step.waitForWindow && (
            <>
              <NumberField
                label="Wait up to"
                value={step.timeoutMs}
                onValueChange={(timeoutMs) => onReplace({ ...step, timeoutMs })}
              />
              <span className="hb__field-label">ms</span>
            </>
          )}
          
        </div>
      );

    case "clipboard":
      return (
        <div className="blk__fields">
          <input
            className="hb__field"
            aria-label="Text to copy"
            placeholder="text to put on the clipboard"
            value={step.text}
            onChange={(event) => onReplace({ ...step, text: event.target.value })}
          />
          
        </div>
      );

    case "screenshot":
      return (
        <div className="blk__fields">
          <input
            className="hb__field hb__field--short"
            aria-label="Screenshot name"
            placeholder="after the fight"
            value={step.name}
            onChange={(event) => onReplace({ ...step, name: event.target.value })}
          />
          <span className="hb__field-label">name</span>
          <button type="button" className="hb__mini" disabled={picking} onClick={onPick}>
            {picking ? "Waiting for an area…" : "Pick area…"}
          </button>
          <NumberField
            label="X"
            value={step.x}
            onValueChange={(x) => onReplace({ ...step, x })}
          />
          <NumberField
            label="Y"
            value={step.y}
            onValueChange={(y) => onReplace({ ...step, y })}
          />
          <NumberField
            label="Width"
            value={step.width}
            onValueChange={(width) => onReplace({ ...step, width })}
          />
          <NumberField
            label="Height"
            value={step.height}
            onValueChange={(height) => onReplace({ ...step, height })}
          />
          
        </div>
      );

    case "callMacro":
      return (
        <div className="blk__fields">
          <ChoiceField
            kind="macros"
            value={step.id}
            onChange={({ id, name }) => onReplace({ ...step, id, name })}
          />
          
        </div>
      );

    case "replay":
      return (
        <div className="blk__fields">
          <ChoiceField
            kind="recordings"
            value={step.id}
            onChange={({ id, name }) => onReplace({ ...step, id, name })}
          />
          <NumberField
            label="Play"
            value={step.passes}
            onValueChange={(passes) =>
              onReplace({ ...step, passes: Math.max(1, Math.min(10_000, passes)) })
            }
          />
          <span className="hb__field-label">times</span>
          <NumberField
            label="Speed"
            value={step.speed}
            onValueChange={(speed) =>
              onReplace({ ...step, speed: Math.max(0, Math.min(10, speed)) })
            }
          />
          <span className="hb__field-label">× as recorded, or 0 for the replayer's own</span>
          
        </div>
      );

    case "tool":
      return (
        <div className="blk__fields">
          <select
            className="hb__field hb__field--short"
            aria-label="Tool"
            value={step.tool}
            onChange={(event) =>
              onReplace({ ...step, tool: event.target.value as ToolName })
            }
          >
            {(Object.keys(TOOL_LABELS) as ToolName[]).map((tool) => (
              <option key={tool} value={tool}>
                {TOOL_LABELS[tool]}
              </option>
            ))}
          </select>
          <SegmentedControl
            label="Switch it"
            options={[
              { value: "on", label: "On" },
              { value: "off", label: "Off" },
            ]}
            value={step.enabled ? "on" : "off"}
            onValueChange={(value) => onReplace({ ...step, enabled: value === "on" })}
          />
          
        </div>
      );

    case "while":
      return (
        <div className="blk__fields">
          <ConditionFields
            condition={step.condition}
            picking={picking}
            onPick={onPick}
            onChange={(condition) => onReplace({ ...step, condition })}
          />
          <NumberField
            label="At most"
            value={step.maxIterations}
            onValueChange={(maxIterations) =>
              onReplace({ ...step, maxIterations: Math.max(1, Math.min(10_000, maxIterations)) })
            }
          />
          <span className="hb__field-label">passes</span>
          <NumberField
            label="For at most"
            value={Math.round(step.timeoutMs / 1000)}
            onValueChange={(seconds) =>
              onReplace({ ...step, timeoutMs: Math.max(1, Math.min(3_600, seconds)) * 1000 })
            }
          />
          <span className="hb__field-label">seconds</span>
          
        </div>
      );

    default: {
      // Every kind in the union is handled above, and this assignment is what
      // makes TypeScript say so: add a block to `Step` without a case here and
      // `step` is no longer `never`, so the build fails instead of the block
      // appearing in the tree with no controls.
      const unhandled: never = step;
      return unhandled;
    }
  }
}

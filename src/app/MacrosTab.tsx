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
  PICK_CANCELLED,
  REGION_PICKED,
  stepSummary,
  type Activity,
  type Macro,
  type MouseButton,
  type PickResult,
  type Step,
  type View,
} from "@/bridge";
import { Card, Line } from "./AutoClickerTab";
import { Switch } from "./Dialogs";
import { HotkeyField } from "./HotkeyField";
import { KeyBinder } from "./Mappings";
import { NumberField } from "./NumberField";

/** Deepest a Repeat may be nested, matching the engine's own limit. */
const MAX_DEPTH = 6;

const BLOCK_ORDER: Step["kind"][] = [
  "key",
  "text",
  "click",
  "move",
  "scroll",
  "delay",
  "repeat",
];

// ── tree surgery, so a drop is one immutable rewrite ───────────────

const samePath = (a: number[], b: number[]) =>
  a.length === b.length && a.every((value, index) => value === b[index]);

const isAncestor = (path: number[], other: number[]) =>
  path.length < other.length && path.every((value, index) => value === other[index]);

const keyAt = (path: number[]) => path.join(".");

function pickStep(steps: Step[], path: number[]): Step | undefined {
  const [head, ...rest] = path;
  const step = steps[head];
  if (!step) return undefined;
  if (rest.length === 0) return step;
  if (step.kind !== "repeat") return undefined;
  return pickStep(step.steps, rest);
}

/**
 * Put a block into the container at `parent`, at `index` within it.
 *
 * `parent` names a container, not a block, so the empty path is the list at the
 * top of the macro and anything longer walks into each Repeat on the way down.
 */
function insertStep(steps: Step[], parent: number[], index: number, step: Step): Step[] {
  if (parent.length === 0) {
    const copy = [...steps];
    copy.splice(Math.max(0, Math.min(index, copy.length)), 0, step);
    return copy;
  }
  const [head, ...rest] = parent;
  return steps.map((entry, at) => {
    if (at !== head || entry.kind !== "repeat") return entry;
    return { ...entry, steps: insertStep(entry.steps, rest, index, step) };
  });
}

function removeStep(steps: Step[], path: number[]): Step[] {
  const [head, ...rest] = path;
  if (rest.length === 0) return steps.filter((_, at) => at !== head);
  return steps.map((entry, at) => {
    if (at !== head || entry.kind !== "repeat") return entry;
    return { ...entry, steps: removeStep(entry.steps, rest) };
  });
}

function replaceStep(steps: Step[], path: number[], step: Step): Step[] {
  const [head, ...rest] = path;
  return steps.map((entry, at) => {
    if (at !== head) return entry;
    if (rest.length === 0) return step;
    if (entry.kind !== "repeat") return entry;
    return { ...entry, steps: replaceStep(entry.steps, rest, step) };
  });
}

/** Move a block, keeping it where the drop said and out of its own children. */
function moveStep(
  steps: Step[],
  from: number[],
  parent: number[],
  index: number,
): Step[] {
  const step = pickStep(steps, from);
  if (!step) return steps;
  if (isAncestor(from, parent) || samePath(from, parent)) return steps;

  const without = removeStep(steps, from);
  const sameContainer = samePath(from.slice(0, -1), parent);
  const target = sameContainer && from[from.length - 1] < index ? index - 1 : index;
  return insertStep(without, parent, target, step);
}

const swap = (steps: Step[], path: number[], by: number): Step[] => {
  const [head, ...rest] = path;
  const entry = steps[head];
  if (!entry) return steps;
  if (rest.length === 0) {
    const to = head + by;
    if (to < 0 || to >= steps.length) return steps;
    const copy = [...steps];
    [copy[head], copy[to]] = [copy[to], copy[head]];
    return copy;
  }
  if (entry.kind !== "repeat") return steps;
  return steps.map((item, at) =>
    at === head ? { ...entry, steps: swap(entry.steps, rest, by) } : item,
  );
};

/** Every block in a container, in order, for the outline summary. */
const countBlocks = (steps: Step[]): number =>
  steps.reduce(
    (total, step) => total + 1 + (step.kind === "repeat" ? countBlocks(step.steps) : 0),
    0,
  );

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
  const [processes, setProcesses] = useState<string[]>([]);
  const [newOpen, setNewOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  // Which block is waiting for a spot on the desktop, as a path into the tree,
  // so the answer lands on the block that asked rather than on the first one.
  const [picking, setPicking] = useState<number[] | null>(null);
  const pickPath = useRef<number[] | null>(null);
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
  const pickSpot = useCallback((path: number[]) => {
    const step = pickStep(draftRef.current?.steps ?? [], path);
    if (!step || (step.kind !== "move" && step.kind !== "click")) return;
    pickPath.current = path;
    setPicking(path);
    void pickRegion({
      purpose: "macro-point",
      index: 0,
      zones: [],
      // A relative block's x and y are an offset, not a place: drawing them as a
      // marker on screen would point at the wrong spot entirely.
      points: step.relative ? [] : [{ x: step.x, y: step.y, width: 0, height: 0 }],
    });
  }, []);

  useEffect(() => {
    let live = true;
    const stops: (() => void)[] = [];
    const keep = (stop: () => void) => {
      if (live) stops.push(stop);
      else stop();
    };
    void on<PickResult>(REGION_PICKED, (result) => {
      const path = pickPath.current;
      const macro = draftRef.current;
      pickPath.current = null;
      setPicking(null);
      if (!path || !macro) return;
      const step = pickStep(macro.steps, path);
      if (!step || (step.kind !== "move" && step.kind !== "click")) return;
      // A dragged area places its point at the centre; a plain click already is
      // a point, with the centre and the corner being the same pixel.
      const x = Math.round(result.x + result.width / 2);
      const y = Math.round(result.y + result.height / 2);
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
  }, [push]);

  const runningMacro = activity?.running && activity.running !== "clicker";

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
            return (
              <button
                key={entry.id}
                type="button"
                className="hb__profile"
                aria-current={entry.id === selected?.id}
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
        </div>
        <div className="hb__rail-foot">
          <p className="hb__card-note hb__dim" style={{ margin: 0, padding: "0 4px" }}>
            A macro runs from its shortcut, or from the Run button. Only one run goes
            at a time, clicker or macro.
          </p>
        </div>
      </aside>

      <section className="hb__page">
        {draft ? (
          <>
            <div className="hb__head">
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
              <span className="hb__field-label">Macros</span>
              <Switch
                checked={view.macros_enabled}
                label="Macros enabled"
                onChange={(enabled) =>
                  run("set_tool_enabled", { tool: "macros", enabled })
                }
              />
            </div>

            <Card title="When it runs" tight>
              <div className="hb__rows">
                <Line
                  label="Shortcut"
                  note="Works globally, whether or not this window is in front."
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

            {activity?.hotkeyErrors.map((message) => (
              <p className="hb__warn" key={message}>
                {message}
              </p>
            ))}

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
                onClick={() => setDeleteOpen(true)}
              >
                Delete macro
              </button>
            </div>

            <Modal
              open={deleteOpen}
              onClose={() => setDeleteOpen(false)}
              title={`Delete "${draft.name}"?`}
              description="This removes the macro and its blocks, and frees its shortcut for another macro."
              maxWidth={400}
              footer={
                <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                  <button
                    type="button"
                    className="appearance__reset"
                    onClick={() => setDeleteOpen(false)}
                  >
                    Keep it
                  </button>
                  <span className="hb__spacer" />
                  <button
                    type="button"
                    className="hb__mini hb__mini--danger"
                    onClick={() => {
                      const next = view.macros.find((entry) => entry.id !== draft.id);
                      setDeleteOpen(false);
                      void run("delete_macro", { id: draft.id });
                      setSelectedId(next?.id ?? null);
                    }}
                  >
                    Delete
                  </button>
                </div>
              }
            />
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
      description="Drag blocks in, reorder them, and drop them inside a Repeat. Nothing runs until the macro has a shortcut or you press Run."
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
      <p className="hb__card-note" style={{ margin: 0 }}>
        The new macro starts with one Wait block.
      </p>
    </Modal>
  );
}

// ── the builder ────────────────────────────────────────────────────

type Drag =
  | { source: "palette"; make: () => Step }
  | { source: "tree"; path: number[] };

type Drop = { parent: number[]; index: number };

function BlockBuilder({
  keys,
  steps,
  picking,
  onPick,
  onChange,
}: {
  keys: View["keys"];
  steps: Step[];
  picking: number[] | null;
  onPick: (path: number[]) => void;
  onChange: (steps: Step[]) => void;
}) {
  const [drag, setDrag] = useState<Drag | null>(null);
  const [drop, setDrop] = useState<Drop | null>(null);

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
      <div className="pal">
        <span className="hb__rail-label" style={{ padding: "0 2px 6px" }}>
          Blocks. Drag one in, or click to add it at the end.
        </span>
        <div className="pal__row">
          {BLOCK_ORDER.map((kind) => (
            <button
              key={kind}
              type="button"
              className="pal__item"
              draggable
              onDragStart={() => setDrag({ source: "palette", make: fresh(kind) })}
              onClick={() => append(fresh(kind))}
            >
              <span className="pal__grip" aria-hidden="true">
                ⠿
              </span>
              {BLOCK_LABELS[kind]}
            </button>
          ))}
          {/* A drag is the click block's mode, but it sits here as its own
              button: it is the block people reach for by name, and one drop
              puts it anywhere in the tree, including inside a Repeat. */}
          <button
            type="button"
            className="pal__item"
            draggable
            onDragStart={() => setDrag({ source: "palette", make: DRAG_TEMPLATE })}
            onClick={() => append(DRAG_TEMPLATE)}
          >
            <span className="pal__grip" aria-hidden="true">
              ⠿
            </span>
            Click and drag
          </button>
        </div>
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
            onRemove={(path) => onChange(removeStep(steps, path))}
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
  path: number[];
  depth: number;
  keys: View["keys"];
  picking: number[] | null;
  onPick: (path: number[]) => void;
  drag: Drag | null;
  drop: Drop | null;
  setDrag: (drag: Drag | null) => void;
  setDrop: (drop: Drop | null) => void;
  onDropTarget: (target: Drop) => void;
  onReplace: (path: number[], step: Step) => void;
  onRemove: (path: number[]) => void;
  onSwap: (path: number[], by: number) => void;
}) {
  const index = path[path.length - 1];
  const parent = path.slice(0, -1);
  const isRepeat = step.kind === "repeat";
  const dragging = drag?.source === "tree" && samePath(drag.path, path);

  /** Which third of the row the pointer is over decides where it lands. */
  const place = (event: React.DragEvent<HTMLDivElement>): Drop => {
    const rect = event.currentTarget.getBoundingClientRect();
    const ratio = (event.clientY - rect.top) / Math.max(1, rect.height);
    if (isRepeat && ratio > 0.34 && ratio < 0.66 && depth < MAX_DEPTH) {
      const children = step.kind === "repeat" ? step.steps.length : 0;
      return { parent: path, index: children };
    }
    return ratio < 0.5 ? { parent, index } : { parent, index: index + 1 };
  };

  const marked =
    drop &&
    (samePath(drop.parent, parent) && (drop.index === index || drop.index === index + 1));

  return (
    <div className={`blk__wrap${marked ? " blk__wrap--marked" : ""}`}>
      <div className={`blk${dragging ? " blk--dragging" : ""}`}>
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

        {/* The row's own drop surface: the third of it the pointer is over
            decides whether the block lands before, after or inside.

            A block is never its own drop surface. Besides doing nothing, that
            surface would be a new element inside the block the drag started
            from, and a node added under the drag source while `dragstart` runs
            makes the webview drop the drag on the spot. Leaving the dragged
            row alone is what keeps every other row reachable. */}
        {drag && !dragging && (
          <div
            className="blk__drop"
            onDragOver={(event) => {
              event.preventDefault();
              setDrop(place(event));
            }}
            onDrop={(event) => {
              event.preventDefault();
              onDropTarget(place(event));
            }}
          />
        )}
      </div>

      {step.kind === "repeat" && (
        <div
          className={`blk__children${drop && samePath(drop.parent, path) ? " blk__children--active" : ""}`}
        >
          {step.steps.map((child, at) => (
            <BlockNode
              key={keyAt([...path, at])}
              step={child}
              path={[...path, at]}
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
            parent={path}
            active={Boolean(drop && samePath(drop.parent, path))}
            onHover={() => setDrop({ parent: path, index: step.steps.length })}
            onDrop={() => onDropTarget({ parent: path, index: step.steps.length })}
            empty={step.steps.length === 0}
            label="Drag blocks in here"
          />
        </div>
      )}
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
  parent: number[];
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
              { value: "left", label: "← Left" },
              { value: "right", label: "→ Right" },
              { value: "up", label: "↑ Up" },
              { value: "down", label: "↓ Down" },
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
                uppercase: isMouseScan(scan) ? false : step.uppercase,
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
                checked={step.uppercase}
                onChange={(event) =>
                  onReplace({ ...step, uppercase: event.target.checked })
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
  }
}

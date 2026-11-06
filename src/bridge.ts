/**
 * The one place the UI talks to Rust.
 *
 * In the packaged app every call is a Tauri command. In a plain browser there is
 * no hook and no OS, so the same calls are answered by an in-memory copy of the
 * same state, enough to build, review and screenshot the UI, and the banner in
 * the header says which side is live.
 */

export type Mode = "OPPOSITE" | "OPPOSITE_NO_REPRESS" | "NEUTRAL" | "REMAP";

export type MappingView = {
  mode: Mode;
  source: number;
  dest: number;
  source_label: string;
  dest_label: string;
};

export type ProfileView = {
  id: string;
  program: string;
  active: boolean;
  global: boolean;
  mappings: MappingView[];
};

export type KeyView = { scan: number; label: string; group: string };

// ── auto clicker ───────────────────────────────────────────────────

export type ClickInput = "mouse" | "keyboard";
export type MouseButton = "left" | "right" | "middle";
export type RateUnit = "second" | "minute" | "hour" | "day";
export type TargetMode = "whitelist" | "blacklist";

export type ClickPoint = { x: number; y: number; clicks: number; radius: number };

/** One watched boundary: whether the run stops there, and how far in from it. */
export type Guard = { on: boolean; margin: number };

export type EdgeStop = {
  enabled: boolean;
  top: Guard;
  right: Guard;
  bottom: Guard;
  left: Guard;
};

export type CornerStop = {
  enabled: boolean;
  topLeft: Guard;
  topRight: Guard;
  bottomLeft: Guard;
  bottomRight: Guard;
};

export type ZoneAction = "stop" | "pause" | "start";

/** A rectangle on the desktop that does something to the run when entered. */
export type StopZone = {
  x: number;
  y: number;
  width: number;
  height: number;
  action: ZoneAction;
};

/** A boundary watching everything at the same distance. */
const guard = (margin: number): Guard => ({ on: true, margin });

export type Targets = {
  enabled: boolean;
  mode: TargetMode;
  programs: string[];
};

export type ClickerSettings = {
  input: ClickInput;
  button: MouseButton;
  keyScan: number;
  shift: boolean;
  rateUnit: RateUnit;
  rate: number;
  duty: number;
  variation: number;
  maxClicks: number;
  timeLimitSecs: number;
  doubleClick: boolean;
  doubleClickGapMs: number;
  points: ClickPoint[];
  moveDurationMs: number;
  humanize: boolean;
  edgeStop: EdgeStop;
  cornerStop: CornerStop;
  stopZones: StopZone[];
  zonesEnabled: boolean;
  targets: Targets;
  hotkey: string;
  holdMode: boolean;
};

export const DEFAULT_CLICKER: ClickerSettings = {
  input: "mouse",
  button: "left",
  keyScan: 0x39,
  shift: false,
  rateUnit: "second",
  rate: 10,
  duty: 0,
  variation: 0,
  maxClicks: 0,
  timeLimitSecs: 0,
  doubleClick: false,
  doubleClickGapMs: 120,
  points: [],
  moveDurationMs: 120,
  humanize: false,
  edgeStop: {
    enabled: false,
    top: guard(40),
    right: guard(40),
    bottom: guard(40),
    left: guard(40),
  },
  cornerStop: {
    enabled: false,
    topLeft: guard(50),
    topRight: guard(50),
    bottomLeft: guard(50),
    bottomRight: guard(50),
  },
  stopZones: [],
  zonesEnabled: false,
  targets: { enabled: false, mode: "whitelist", programs: [] },
  // No shortcut by default, matching the Rust side.
  hotkey: "",
  holdMode: false,
};

/** What is running right now, as the Rust side reports it. */
export type Outcome = {
  stopReason: string;
  clicks: number;
  elapsedSecs: number;
  achievedCps: number;
  requestedCps: number;
};

export type MacroReport = {
  stopReason: string;
  actions: number;
  /** Lines the run's log blocks wrote, in order. */
  notes: string[];
};

export type Activity = {
  running: string | null;
  clicks: number;
  elapsedSecs: number;
  achievedCps: number;
  requestedCps: number;
  source: string;
  timerRaised: boolean;
  last: Outcome | null;
  lastMacro: MacroReport | null;
  hotkeyErrors: string[];
  /** Something asked for that could not happen, shown in the same strip. */
  alerts: string[];
};

// ── macros ─────────────────────────────────────────────────────────

/** What a press block does with what it presses. The key block and the click
 * block both offer tap/hold/down/up, so they share the type; "drag" is the
 * click block's own fifth and the engine refuses it on a key. */
export type PressMode = "tap" | "hold" | "down" | "up" | "drag";

/** What an If block checks. */
export type Condition =
  | { kind: "picture"; image: string; similarity: number }
  | { kind: "front"; program: string };

export type Step =
  | { kind: "key"; scan: number; mode: PressMode; holdMs: number; shift: boolean }
  | { kind: "text"; text: string; perCharMs: number }
  | { kind: "move"; x: number; y: number; relative: boolean; durationMs: number }
  | {
      kind: "click";
      button: MouseButton;
      mode: PressMode;
      count: number;
      holdMs: number;
      /** Where a drag goes and how long it takes; ignored by the other modes. */
      x: number;
      y: number;
      relative: boolean;
      durationMs: number;
    }
  | { kind: "scroll"; amount: number }
  | {
      /** Look for a picture on screen and wait until it is there. */
      kind: "waitImage";
      /** The saved picture's name, or empty before one is picked. */
      image: string;
      /** How long to keep looking; 0 looks once. */
      timeoutMs: number;
      /** How alike the screen has to look, as a percentage. */
      similarity: number;
    }
  | {
      /** Find a picture on screen and click it, wherever it is now. */
      kind: "clickImage";
      image: string;
      button: MouseButton;
      timeoutMs: number;
      similarity: number;
    }
  | { kind: "delay"; ms: number }
  | {
      /** One of two sets of blocks, chosen by what is on screen. */
      kind: "if";
      condition: Condition;
      /** The blocks to run when the check is true. */
      steps: Step[];
      /** The blocks to run when it is not. */
      otherwise: Step[];
    }
  | { kind: "repeat"; times: number; steps: Step[] }
  /** A note for whoever reads the macro. Runs nothing. */
  | { kind: "note"; text: string }
  /** A line for the run's report, so a long macro says where it got to. */
  | { kind: "log"; text: string }
  /** Wait a random time between two values. */
  | { kind: "randomDelay"; minMs: number; maxMs: number }
  /** Several keys pressed and released together. */
  | { kind: "chord"; scans: number[]; holdMs: number }
  /** Move the cursor somewhere and scroll there, as one block. */
  | { kind: "scrollAt"; x: number; y: number; amount: number }
  /** Wait until a time of day. A not-before gate, not an alarm. */
  | { kind: "until"; at: string }
  /** Run the inner blocks until a number of seconds has passed. */
  | { kind: "repeatFor"; seconds: number; steps: Step[] }
  /** End the run here, saying why. */
  | { kind: "stop"; reason: string }
  /** End the run as a failure, saying what went wrong. */
  | { kind: "fail"; message: string }
  /** The picture has to be on screen right now, or the run stops. */
  | { kind: "assertImage"; image: string; similarity: number }
  /** Bring a window in front. */
  | { kind: "focus"; target: WindowTarget; timeoutMs: number }
  /** Start a program, and optionally wait until its window is up. */
  | {
      kind: "launch";
      path: string;
      args: string;
      waitForWindow: boolean;
      timeoutMs: number;
    }
  /** Put text on the clipboard. */
  | { kind: "clipboard"; text: string }
  /** Save an area of the screen as a PNG. */
  | { kind: "screenshot"; x: number; y: number; width: number; height: number; name: string }
  /** Run another macro's blocks here. */
  | { kind: "callMacro"; id: string; name: string }
  /** Play a saved recording here. */
  | { kind: "replay"; id: string; name: string; passes: number; speed: number }
  /** Flip one of the app's switches. */
  | { kind: "tool"; tool: ToolName; enabled: boolean }
  /** Repeat the inner blocks while the check holds, up to two caps. */
  | {
      kind: "while";
      condition: Condition;
      steps: Step[];
      maxIterations: number;
      timeoutMs: number;
    };

/** One of the app's tools, as the switches name them. */
export type ToolName = "clicker" | "remapper" | "macros" | "typer" | "replay";

/** The same tools, as a sentence reads them. */
export const TOOL_LABELS: Record<ToolName, string> = {
  clicker: "the auto clicker",
  remapper: "the remapper",
  macros: "macros",
  typer: "the typer",
  replay: "the replayer",
};

/**
 * What a Focus block is looking for.
 *
 * A title is what a person reads in the taskbar; a program is what survives the
 * title changing with the level or the document being edited. `exact` is only
 * worth having for a title, where a fragment would match three windows.
 */
export type WindowTarget =
  | { kind: "title"; contains: string; exact: boolean }
  | { kind: "program"; exe: string };

export type Macro = {
  id: string;
  name: string;
  hotkey: string;
  onlyWhen: string;
  enabled: boolean;
  steps: Step[];
};

/** A macro plus the reason it cannot run, if there is one. */
export type MacroView = Macro & { problem: string | null };

/** The sign-in entry, as Windows has it, plus the copy that is running. */
export type AutostartView = {
  enabled: boolean;
  entry: string | null;
  exe: string;
  running: string;
};

export type View = {
  profiles: ProfileView[];
  /** The Remapper's switch. */
  hook_enabled: boolean;
  clicker_enabled: boolean;
  macros_enabled: boolean;
  installed: boolean;
  close_to_tray: boolean;
  debug: boolean;
  program: string;
  log: string[];
  keys: KeyView[];
  clicker: ClickerSettings;
  macros: MacroView[];
  typer: TyperSettings;
  typer_enabled: boolean;
  replay: ReplaySettingsView;
  replay_enabled: boolean;
  autostart: AutostartView;
};

// ── the typer ─────────────────────────────────────────────────────

/** What to type, and how. Mirrors `TyperSettings` on the Rust side. */
export type TyperSettings = {
  text: string;
  /** Resting pace, in words a minute. */
  wpm: number;
  /** Share of characters mistyped and corrected, 0 to 0.25. */
  typoRate: number;
  /** Finish by this many seconds; it wins over the pace. Null means never. */
  deadlineSecs: number | null;
  /** Seconds to wait before the first keystroke. */
  countdownSecs: number;
  /** The start/stop shortcut, in the spelling the other tools use. */
  hotkey: string;
  /** Whether the run stops between words at all. */
  thinkPauses: boolean;
  /** The longest such stop, in seconds. */
  thinkMaxSecs: number;
  /** Whether a run stops after a full stop, before the next sentence. */
  thinkAfterSentence: boolean;
  /** Whether a run also stops inside a long sentence, after 15 to 30 words. */
  thinkInLongSentence: boolean;
  /** Whether a mistake is left standing and fixed a sentence or three later. */
  typoDefer: boolean;
  /** How far each word's pace may wander, as a share either side. */
  paceVariation: number;
};

export const DEFAULT_TYPER: TyperSettings = {
  text: "",
  wpm: 65,
  typoRate: 0.04,
  deadlineSecs: null,
  countdownSecs: 3,
  hotkey: "",
  thinkPauses: true,
  thinkMaxSecs: 2.5,
  thinkAfterSentence: true,
  thinkInLongSentence: true,
  typoDefer: false,
  paceVariation: 0.2,
};

/** What a run is doing, as the shell reports it. */
export type TyperStatusView = {
  /** `idle`, `typing`, `paused`, `done`, `stopped` or `unfocused`. */
  status: "idle" | "typing" | "paused" | "done" | "stopped" | "unfocused";
  /** The run is stopped between words, thinking about the next one. */
  thinking: boolean;
  typed: number;
  total: number;
  plannedMs: number;
  elapsedSecs: number;
};

/** What a run will do, worked out before it starts. */
export type TyperPlan = {
  chars: number;
  words: number;
  /** Keystrokes a second, after any deadline. */
  cps: number;
  totalMs: number;
  /** How much of that is the run stopping to think between words. */
  thinkingMs: number;
  /** A deadline asked for a faster pace than the engine will keep. */
  clamped: boolean;
};

/** The estimate for a draft, for the readout above the run. */
export function typerPlan(text: string, settings: TyperSettings): Promise<TyperPlan> {
  return call<TyperPlan>("typer_plan", { text, settings });
}

/** Live counters; polled while the tab is open. */
export function typerStatus(): Promise<TyperStatusView> {
  return call<TyperStatusView>("typer_status");
}

export function typerPause(): Promise<TyperStatusView> {
  return call<TyperStatusView>("typer_pause");
}

export function typerResume(): Promise<TyperStatusView> {
  return call<TyperStatusView>("typer_resume");
}

export function typerStop(): Promise<TyperStatusView> {
  return call<TyperStatusView>("typer_stop");
}

// ── the replayer ──────────────────────────────────────────────────

/** One saved recording, as the list shows it. */
export type ReplaySummary = {
  id: string;
  name: string;
  events: number;
  durationMs: number;
};

/** The replayer's saved state. What it is doing is polled separately. */
export type ReplaySettingsView = {
  recordings: ReplaySummary[];
  selected: string;
  /** The one key that starts a recording and ends it again. */
  recordHotkey: string;
  /** The one key that starts a replay and stops it again. */
  replayHotkey: string;
  /** How many passes a replay makes. Null is forever. */
  repeat: number | null;
  /** Playback speed as a multiple of the recorded pace. */
  speed: number;
};

/** What a recording or a replay is doing right now. */
export type ReplayStateView = {
  recording: boolean;
  /** Events captured so far, and how long the recording has been running. */
  events: number;
  durationMs: number;
  /** A finished recording waiting for a name. */
  pending: { events: number; durationMs: number } | null;
  playing: boolean;
  status: "idle" | "recording" | "replaying" | "done" | "stopped";
  played: number;
  total: number;
  pass: number;
  passes: number | null;
  /** Why something could not start, if it could not. */
  note: string | null;
};

export function replayState(): Promise<ReplayStateView> {
  return call<ReplayStateView>("replay_state");
}

/** Begin recording; the shell hides the window until the shortcut ends it. */
export function startRecording(): Promise<ReplayStateView> {
  return call<ReplayStateView>("start_recording");
}

export function stopRecording(): Promise<ReplayStateView> {
  return call<ReplayStateView>("stop_recording");
}

export function startReplay(id?: string): Promise<ReplayStateView> {
  return call<ReplayStateView>("start_replay", { id: id ?? null });
}

export function stopReplay(): Promise<ReplayStateView> {
  return call<ReplayStateView>("stop_replay");
}

export function discardRecording(): Promise<ReplayStateView> {
  return call<ReplayStateView>("discard_recording");
}

// ── updates ──────────────────────────────────────────────────────

/**
 * An update as the shell reports it: what is waiting, and how far a download
 * got. The status is one of `idle`, `checking`, `available`, `downloading`,
 * `installing`, `restarting`, `upToDate` or `error`.
 */
export type UpdateView = {
  currentVersion: string;
  status: string;
  version: string | null;
  notes: string | null;
  downloaded: number;
  total: number | null;
  error: string | null;
};

/** What the updater last found. Polled; the shell changes it on its own clock. */
export function updateState(): Promise<UpdateView> {
  return call<UpdateView>("update_state");
}

/** Ask again now, rather than waiting for the shell's next six-hourly check. */
export function updateCheck(): Promise<UpdateView> {
  return call<UpdateView>("update_check");
}

/** Download, verify and install; the shell restarts itself when it lands. */
export function updateInstall(): Promise<void> {
  return call<void>("update_install");
}

// ── drawing on screen ──────────────────────────────────────────────

/** A rectangle in screen pixels. */
export type Rect = { x: number; y: number; width: number; height: number };

/**
 * Who asked for the rectangle.
 *
 * `macro-point` is a block in the macro builder aiming a move or a drag. It is
 * its own value rather than a second `point` because the clicker listens to the
 * same event and would otherwise add the answer to its click points.
 */
export type PickPurpose =
  | "zone"
  | "point"
  | "macro-point"
  | "macro-picture"
  | "macro-shot";

/** What to draw, and where to put it when it is drawn. */
export type PickRequest = {
  purpose: PickPurpose;
  index: number;
  zones: Rect[];
  points: Rect[];
};

export type PickResult = Rect & { purpose: PickPurpose; index: number };

/** Emitted by the picker when a drag is finished. */
export const REGION_PICKED = "region-picked";
/** Emitted when the picker is dismissed without an answer. */
export const PICK_CANCELLED = "region-pick-cancelled";
/** Emitted to the settings window when a path comes back from the file dialog. */
export const AUTOSTART_EXE = "autostart-exe";
/** Emitted when the builder's program dialog answers, with the path. */
export const MACRO_PROGRAM = "macro-program";

/** The titles of the windows a macro could focus. */
export function runningWindows(): Promise<string[]> {
  return call<string[]>("running_windows");
}

/** Ask for a program to launch, rather than making the person type a path. */
export function chooseProgram(): Promise<void> {
  return call<void>("choose_program");
}

/** One macro or recording a block can point at. */
export type BlockChoice = { id: string; name: string };

/** Everything a composition block can name. */
export type BlockChoices = { macros: BlockChoice[]; recordings: BlockChoice[] };

/** The macros and recordings, for the Run another macro and Run a replay blocks. */
export function blockChoices(): Promise<BlockChoices> {
  return call<BlockChoices>("block_choices");
}

/**
 * Ask for a rectangle, drawn by dragging on the desktop.
 *
 * In a browser there is no second window to draw in, so the stand-in answers
 * with a rectangle near the middle of the window instead, enough to exercise
 * the flow while reviewing the UI.
 */
export function pickRegion(request: PickRequest): Promise<void> {
  return call<void>("pick_region", { request });
}

/** A picture saved for a macro block. */
export type Picture = { id: string; width: number; height: number };

/** Keep a dragged rectangle as a picture the blocks can look for. */
export function savePicture(rect: Rect): Promise<Picture> {
  return call<Picture>("save_picture", {
    x: rect.x,
    y: rect.y,
    width: rect.width,
    height: rect.height,
  });
}

/** The picture itself, as a data URL, for its thumbnail. */
export function picturePreview(id: string): Promise<string> {
  return call<string>("picture_preview", { id });
}

/**
 * The pictures already saved, newest first.
 *
 * A macro that looks for the same thing in four blocks should not need the same
 * rectangle dragged out four times, so the blocks offer what is already there.
 */
export function pictureHistory(): Promise<Picture[]> {
  return call<Picture[]>("picture_history");
}

/**
 * Take a picture off the saved shelf.
 *
 * Refused when a block still looks for it, the shelf shares pictures, so a
 * shared one cannot be taken away from the blocks using it.
 */
export function forgetPicture(id: string): Promise<void> {
  return call<void>("forget_picture", { id });
}

/**
 * One macro that still looks for a picture, and the blocks in it that do.
 *
 * A block's `path` is its place in the tree, indices, with `steps` or
 * `otherwise` where it sits inside a branch, which is what the builder needs to
 * scroll to it.
 */
export type PictureUser = {
  id: string;
  macroName: string;
  blocks: { label: string; path: string[] }[];
};

/** Where a picture is still used, asked when removing it is refused. */
export function pictureUses(id: string): Promise<PictureUser[]> {
  return call<PictureUser[]>("picture_uses", { id });
}

/** Forget a picture whose block has gone. */
export function deletePicture(id: string): Promise<void> {
  return call<void>("delete_picture", { id });
}

/**
 * Follow an event from the Rust side.
 *
 * Outside Tauri the same names arrive as DOM events, so a component can be
 * written once and still work in the browser stand-in.
 */
export async function on<TPayload>(
  event: string,
  handler: (payload: TPayload) => void,
): Promise<() => void> {
  if (isTauri()) {
    const { listen } = await import("@tauri-apps/api/event");
    const stop = await listen<TPayload>(event, (message) => handler(message.payload));
    return () => void stop();
  }
  const listener = (domEvent: Event) =>
    handler((domEvent as CustomEvent<TPayload>).detail);
  window.addEventListener(event, listener);
  return () => window.removeEventListener(event, listener);
}

/** A fresh block of each kind, for the palette. */
export const BLOCK_TEMPLATES: Record<Step["kind"], (partial?: object) => Step> = {
  key: (partial) => ({ kind: "key", scan: 0x11, mode: "tap", holdMs: 80, shift: false, ...partial }),
  text: (partial) => ({ kind: "text", text: "hello", perCharMs: 40, ...partial }),
  move: (partial) => ({ kind: "move", x: 0, y: 0, relative: false, durationMs: 150, ...partial }),
  click: (partial) => ({
    kind: "click",
    button: "left",
    mode: "tap",
    count: 1,
    holdMs: 40,
    // A fresh drag block presses and lets go where the cursor already is: those
    // fields are only read by a drag, and a relative 0,0 is the harmless
    // reading of them.
    x: 0,
    y: 0,
    relative: true,
    durationMs: 200,
    ...partial,
  }),
  scroll: (partial) => ({ kind: "scroll", amount: -3, ...partial }),
  waitImage: (partial) => ({
    kind: "waitImage",
    image: "",
    timeoutMs: 5000,
    similarity: 90,
    ...partial,
  }),
  clickImage: (partial) => ({
    kind: "clickImage",
    image: "",
    button: "left",
    timeoutMs: 3000,
    similarity: 90,
    ...partial,
  }),
  delay: (partial) => ({ kind: "delay", ms: 250, ...partial }),
  repeat: (partial) => ({ kind: "repeat", times: 3, steps: [], ...partial }),
  if: (partial) => ({
    kind: "if",
    condition: { kind: "picture", image: "", similarity: 90 },
    steps: [],
    otherwise: [],
    ...partial,
  }),
  note: (partial) => ({ kind: "note", text: "", ...partial }),
  log: (partial) => ({ kind: "log", text: "", ...partial }),
  randomDelay: (partial) => ({ kind: "randomDelay", minMs: 250, maxMs: 900, ...partial }),
  chord: (partial) => ({ kind: "chord", scans: [0x1d, 0x2e], holdMs: 80, ...partial }),
  scrollAt: (partial) => ({ kind: "scrollAt", x: 0, y: 0, amount: -3, ...partial }),
  until: (partial) => ({ kind: "until", at: "09:00", ...partial }),
  repeatFor: (partial) => ({ kind: "repeatFor", seconds: 10, steps: [], ...partial }),
  stop: (partial) => ({ kind: "stop", reason: "", ...partial }),
  fail: (partial) => ({ kind: "fail", message: "", ...partial }),
  assertImage: (partial) => ({ kind: "assertImage", image: "", similarity: 90, ...partial }),
  focus: (partial) => ({
    kind: "focus",
    target: { kind: "program", exe: "" },
    timeoutMs: 5000,
    ...partial,
  }),
  launch: (partial) => ({
    kind: "launch",
    path: "",
    args: "",
    waitForWindow: true,
    timeoutMs: 15000,
    ...partial,
  }),
  clipboard: (partial) => ({ kind: "clipboard", text: "", ...partial }),
  screenshot: (partial) => ({
    kind: "screenshot",
    x: 0,
    y: 0,
    width: 800,
    height: 450,
    name: "",
    ...partial,
  }),
  callMacro: (partial) => ({ kind: "callMacro", id: "", name: "", ...partial }),
  replay: (partial) => ({
    kind: "replay",
    id: "",
    name: "",
    passes: 1,
    speed: 0,
    ...partial,
  }),
  tool: (partial) => ({ kind: "tool", tool: "clicker", enabled: false, ...partial }),
  while: (partial) => ({
    kind: "while",
    condition: { kind: "picture", image: "", similarity: 90 },
    steps: [],
    maxIterations: 100,
    timeoutMs: 60_000,
    ...partial,
  }),
};

export const BLOCK_LABELS: Record<Step["kind"], string> = {
  key: "Key",
  text: "Type text",
  move: "Move mouse",
  click: "Click",
  scroll: "Scroll",
  waitImage: "Wait for picture",
  clickImage: "Click picture",
  delay: "Wait",
  repeat: "Repeat",
  if: "If",
  note: "Note",
  log: "Log a line",
  randomDelay: "Random wait",
  chord: "Key chord",
  scrollAt: "Scroll at a point",
  until: "Wait until a time",
  repeatFor: "Repeat for a while",
  stop: "Stop macro",
  fail: "Fail",
  assertImage: "Picture must be there",
  focus: "Focus a window",
  launch: "Start a program",
  clipboard: "Copy to clipboard",
  screenshot: "Save a screenshot",
  callMacro: "Run another macro",
  replay: "Run a replay",
  tool: "Turn a tool on or off",
  while: "While it is true",
};

/** A fresh drag: a click block whose mode is `drag`, which presses, travels and lets go.
 * It is the click block's fifth mode rather than a kind of its own, so the
 * palette entry below uses this instead of a template lookup by kind. */
export const DRAG_TEMPLATE = (): Step => BLOCK_TEMPLATES.click({ mode: "drag" });

/** What a block calls itself in the tree. A drag is a click block, and calling
 * its row "Click" would read as a mistake. */
export function blockTitle(step: Step): string {
  if (step.kind === "click" && step.mode === "drag") return "Click and drag";
  return BLOCK_LABELS[step.kind];
}

/** One line describing a block in the tree, mirroring the Rust summary. */
export function stepSummary(step: Step, keys: KeyView[]): string {
  const label = (scan: number) =>
    keys.find((key) => key.scan === scan)?.label ?? keyLabel(scan);
  switch (step.kind) {
    case "if": {
      const check =
        step.condition.kind === "picture"
          ? "the picture is on screen"
          : `${step.condition.program || "a program"} is in front`;
      const otherwise = step.otherwise.length
        ? `, ${step.otherwise.length} otherwise`
        : "";
      return `If ${check}: ${step.steps.length} blocks${otherwise}`;
    }
    case "waitImage":
      return step.timeoutMs > 0
        ? `Look for the picture, up to ${step.timeoutMs} ms`
        : "Look for the picture once";
    case "clickImage": {
      const wait = step.timeoutMs > 0 ? `, up to ${step.timeoutMs} ms` : "";
      return `Click the picture with the ${step.button} button${wait}`;
    }
    case "key": {
      const prefix = step.shift ? "Shift+" : "";
      if (step.mode === "tap") return `Tap ${prefix}${label(step.scan)}`;
      if (step.mode === "hold") return `Hold ${prefix}${label(step.scan)} for ${step.holdMs} ms`;
      if (step.mode === "down") return `Press ${label(step.scan)} down`;
      return `Release ${label(step.scan)}`;
    }
    case "text":
      return step.perCharMs > 0         ? `Type "${step.text}", ${step.perCharMs} ms a character`
        : `Type "${step.text}"`;
    case "move":
      return step.relative
        ? `Move the mouse by ${step.x}, ${step.y}`
        : `Move the mouse to ${step.x}, ${step.y}`;
    case "click": {
      // Mirrors the Rust summary, so the block reads the same wherever it is
      // described: a tap repeats, a hold is one press, and down and up are a
      // single press or release.
      if (step.mode === "hold") return `Hold ${step.button} for ${step.holdMs} ms`;
      if (step.mode === "down") return `Press ${step.button} down`;
      if (step.mode === "up") return `Release ${step.button}`;
      if (step.mode === "drag") {
        const travel = step.relative
          ? `by ${step.x}, ${step.y}`
          : `to ${step.x}, ${step.y} over ${step.durationMs} ms`;
        const rest = step.holdMs ? `, resting ${step.holdMs} ms` : "";
        return `Drag ${step.button} ${travel}${rest}`;
      }
      return `${step.button} click ×${step.count}${step.holdMs ? `, held ${step.holdMs} ms` : ""}`;
    }
    case "scroll":
      return `Scroll ${Math.abs(step.amount)} ${step.amount < 0 ? "down" : "up"}`;
    case "delay":
      return `Wait ${step.ms} ms`;
    case "repeat":
      return `Repeat ${step.times}× (${step.steps.length} block${step.steps.length === 1 ? "" : "s"})`;
    case "note":
      return step.text.trim() ? `Note “${step.text.slice(0, 40)}”` : "Note";
    case "log":
      return `Log “${step.text.slice(0, 40)}”`;
    case "randomDelay":
      return `Wait ${step.minMs}, ${step.maxMs} ms, picked at random`;
    case "chord":
      return `${step.scans.map((scan) => label(scan)).join("+")} together, held ${step.holdMs} ms`;
    case "scrollAt":
      return `Scroll ${Math.abs(step.amount)} ${step.amount < 0 ? "down" : "up"} at ${step.x}, ${step.y}`;
    case "until":
      return `Wait until ${step.at}`;
    case "repeatFor":
      return `Run ${step.steps.length} block${step.steps.length === 1 ? "" : "s"} for ${step.seconds} s`;
    case "stop":
      return step.reason.trim() ? `Stop: “${step.reason}”` : "Stop the macro";
    case "fail":
      return `Fail: “${step.message}”`;
    case "assertImage":
      return "The picture has to be on screen";
    case "focus": {
      const what =
        step.target.kind === "program"
          ? step.target.exe.trim()
          : step.target.exact
            ? `“${step.target.contains.trim()}”`
            : `a window titled “${step.target.contains.trim()}”`;
      return step.timeoutMs ? `Focus ${what}, up to ${step.timeoutMs} ms` : `Focus ${what}`;
    }
    case "launch": {
      const name = step.path.trim().split(/[\\/]/).pop() || step.path.trim();
      const args = step.args.trim() ? ` ${step.args.trim()}` : "";
      return step.waitForWindow
        ? `Start ${name}${args} and wait for its window`
        : `Start ${name}${args}`;
    }
    case "clipboard":
      return `Copy ${step.text.length} characters`;
    case "screenshot":
      return step.name.trim()
        ? `Save a ${step.width}×${step.height} screenshot as “${step.name.trim()}”`
        : `Save a ${step.width}×${step.height} screenshot`;
    case "callMacro":
      return step.name.trim() ? `Run “${step.name.trim()}”` : "Run another macro";
    case "replay": {
      const label = step.name.trim() ? `“${step.name.trim()}”` : "a recording";
      const times = step.passes === 1 ? "once" : `${step.passes} times`;
      return `Play ${label} ${times}${step.speed > 0 ? ` at ${step.speed}×` : ""}`;
    }
    case "tool":
      return `${step.enabled ? "Turn on" : "Turn off"} ${TOOL_LABELS[step.tool]}`;
    case "while": {
      const check =
        step.condition.kind === "picture"
          ? "while the picture is on screen"
          : `while ${step.condition.program} is in front`;
      return `${check}: ${step.steps.length} blocks, at most ${step.maxIterations} passes`;
    }
  }
}

export const MODE_LABELS: Record<Mode, string> = {
  OPPOSITE: "Opposite",
  OPPOSITE_NO_REPRESS: "No repress",
  NEUTRAL: "Neutral",
  REMAP: "Remap",
};

export const MODE_NOTES: Record<Mode, string> = {
  OPPOSITE: "Last key wins, and the losing direction comes back when you let go.",
  OPPOSITE_NO_REPRESS: "Last key wins, and nothing comes back until you press again.",
  NEUTRAL: "Both directions cancel each other out while held.",
  REMAP: "The left key simply produces the right one.",
};

/** `KeyboardEvent.code` → hardware scan code. */
const SCAN_BY_CODE: Record<string, number> = {
  Escape: 0x01,
  Digit1: 0x02,
  Digit2: 0x03,
  Digit3: 0x04,
  Digit4: 0x05,
  Digit5: 0x06,
  Digit6: 0x07,
  Digit7: 0x08,
  Digit8: 0x09,
  Digit9: 0x0a,
  Digit0: 0x0b,
  Minus: 0x0c,
  Equal: 0x0d,
  Backspace: 0x0e,
  Tab: 0x0f,
  KeyQ: 0x10,
  KeyW: 0x11,
  KeyE: 0x12,
  KeyR: 0x13,
  KeyT: 0x14,
  KeyY: 0x15,
  KeyU: 0x16,
  KeyI: 0x17,
  KeyO: 0x18,
  KeyP: 0x19,
  BracketLeft: 0x1a,
  BracketRight: 0x1b,
  Enter: 0x1c,
  ControlLeft: 0x1d,
  KeyA: 0x1e,
  KeyS: 0x1f,
  KeyD: 0x20,
  KeyF: 0x21,
  KeyG: 0x22,
  KeyH: 0x23,
  KeyJ: 0x24,
  KeyK: 0x25,
  KeyL: 0x26,
  Semicolon: 0x27,
  Quote: 0x28,
  Backquote: 0x29,
  ShiftLeft: 0x2a,
  Backslash: 0x2b,
  KeyZ: 0x2c,
  KeyX: 0x2d,
  KeyC: 0x2e,
  KeyV: 0x2f,
  KeyB: 0x30,
  KeyN: 0x31,
  KeyM: 0x32,
  Comma: 0x33,
  Period: 0x34,
  Slash: 0x35,
  ShiftRight: 0x36,
  NumpadMultiply: 0x37,
  AltLeft: 0x38,
  Space: 0x39,
  CapsLock: 0x3a,
  F1: 0x3b,
  F2: 0x3c,
  F3: 0x3d,
  F4: 0x3e,
  F5: 0x3f,
  F6: 0x40,
  F7: 0x41,
  F8: 0x42,
  F9: 0x43,
  F10: 0x44,
  NumLock: 0x45,
  ScrollLock: 0x46,
  Numpad7: 0x47,
  Numpad8: 0x48,
  Numpad9: 0x49,
  NumpadSubtract: 0x4a,
  Numpad4: 0x4b,
  Numpad5: 0x4c,
  Numpad6: 0x4d,
  NumpadAdd: 0x4e,
  Numpad1: 0x4f,
  Numpad2: 0x50,
  Numpad3: 0x51,
  Numpad0: 0x52,
  NumpadDecimal: 0x53,
  F11: 0x57,
  F12: 0x58,
  ArrowUp: 0x48,
  ArrowDown: 0x50,
  ArrowLeft: 0x4b,
  ArrowRight: 0x4d,
  Insert: 0x52,
  Delete: 0x53,
  Home: 0x47,
  End: 0x4f,
  PageUp: 0x49,
  PageDown: 0x51,
  // The right-hand modifiers keep a value of their own; sharing 0x1d with the
  // left one made the option unselectable in a `<select>`.
  ControlRight: 0x11d,
  AltRight: 0x138,
  // Keys the table never named. A press of one of these found nothing and was
  // dropped, which left the cap showing the key it already held, Left Ctrl on
  // a fresh chord, as if the keyboard had not been heard at all.
  PrintScreen: 0x137,
  NumpadEnter: 0x11c,
  NumpadDivide: 0x135,
  MetaLeft: 0x15b,
  MetaRight: 0x15c,
  ContextMenu: 0x15d,
  IntlBackslash: 0x56,
  IntlRo: 0x73,
  IntlYen: 0x7d,
  NumpadComma: 0x7e,
  NumpadEqual: 0x59,
};

export function scanFromCode(code: string): number | null {
  return SCAN_BY_CODE[code] ?? null;
}

/** Readable names, so the browser stand-in labels keys like the app does. */
const LABELS: Record<number, string> = {
  0x01: "Esc", 0x0c: "-", 0x0d: "=", 0x0e: "Backspace", 0x0f: "Tab",
  0x1a: "[", 0x1b: "]", 0x1c: "Enter", 0x1d: "Left Ctrl", 0x27: ";",
  0x28: "'", 0x29: "`", 0x2a: "Left Shift", 0x2b: "\\", 0x36: "Right Shift",
  0x37: "Numpad *", 0x38: "Left Alt", 0x39: "Spacebar", 0x3a: "Caps Lock",
  0x45: "Num Lock", 0x46: "Scroll Lock", 0x4a: "Numpad -", 0x4e: "Numpad +",
  0x47: "Home", 0x48: "Up", 0x49: "Page up", 0x4b: "Left", 0x4d: "Right",
  0x4f: "End", 0x50: "Down", 0x51: "Page down", 0x52: "Insert", 0x53: "Delete",
  0x11d: "Right Ctrl", 0x138: "Right Alt",
  0x137: "Print Screen", 0x11c: "Numpad Enter", 0x135: "Numpad /",
  0x15b: "Left Windows", 0x15c: "Right Windows", 0x15d: "Menu",
  0x56: "ISO \\", 0x73: "Intl Ro", 0x7d: "Intl Yen",
  0x7e: "Numpad ,", 0x59: "Numpad =",
  // Mouse buttons, above the keyboard's scan codes so a key block can carry
  // either: the Rust side reads the same three numbers.
  0x201: "Left click", 0x202: "Right click", 0x203: "Middle click",
};

/** The scan a mouse button binds as, or null for one nothing uses. */
export function mouseScanFromButton(button: number): number | null {
  if (button === 0) return 0x201;
  if (button === 2) return 0x202;
  if (button === 1) return 0x203;
  return null;
}

/** Whether a bound input is a mouse button rather than a key. */
export const isMouseScan = (scan: number): boolean =>
  scan >= 0x201 && scan <= 0x203;

for (let i = 0; i < 10; i += 1) LABELS[0x02 + i] = String((i + 1) % 10);
for (const [index, letter] of "QWERTYUIOP".split("").entries()) LABELS[0x10 + index] = letter;
for (const [index, letter] of "ASDFGHJKL".split("").entries()) LABELS[0x1e + index] = letter;
for (const [index, letter] of "ZXCVBNM".split("").entries()) LABELS[0x2c + index] = letter;
for (let i = 0; i < 10; i += 1) LABELS[0x3b + i] = `F${i + 1}`;
LABELS[0x57] = "F11";
LABELS[0x58] = "F12";

export const keyLabel = (scan: number): string => LABELS[scan] ?? `Scan ${scan}`;

export const isTauri = (): boolean =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

// ── the browser stand-in ───────────────────────────────────────────

const labelOf = (scan: number): string => keyLabel(scan);

const toMapping = (mode: Mode, source: number, dest: number): MappingView => ({
  mode,
  source,
  dest,
  source_label: labelOf(source),
  dest_label: labelOf(dest),
});

/** A draft for the preview, so the typer tab has something to show. */
const SAMPLE_TYPER_TEXT =
  "We shipped the update on a Friday, which everyone says not to do. It broke the " +
  "one thing nobody tested, and the release notes still claimed it worked. Next " +
  "time the checklist runs before the tag, not after it.";

/** Preview-only: a copy of the state from before the last delete, so the undo
 *  bar has something to put back without a Rust engine behind it. */
let mockUndo: unknown = null;

let mock: View = {
  hook_enabled: true,
  clicker_enabled: true,
  macros_enabled: true,
  typer_enabled: true,
  replay_enabled: true,
  installed: false,
  close_to_tray: true,
  debug: false,
  program: "Crossup (preview)",
  log: [],
  clicker: { ...DEFAULT_CLICKER },
  typer: { ...DEFAULT_TYPER, text: SAMPLE_TYPER_TEXT },
  replay: {
    recordings: [
      { id: "replay-1", name: "Open the menu and type a line", events: 214, durationMs: 3_400 },
    ],
    selected: "replay-1",
    recordHotkey: "",
    replayHotkey: "",
    repeat: null,
    speed: 1,
  },
  autostart: {
    enabled: false,
    entry: null,
    exe: "C:\\Program Files\\Crossup\\Crossup.exe",
    running: "C:\\Program Files\\Crossup\\Crossup.exe",
  },
  macros: [
    {
      id: "macro-1",
      name: "Burst fire",
      hotkey: "Control+Alt+1",
      onlyWhen: "",
      enabled: true,
      problem: null,
      steps: [
        BLOCK_TEMPLATES.click({ button: "right", count: 1, holdMs: 60 }) as Step,
        BLOCK_TEMPLATES.delay({ ms: 120 }) as Step,
        BLOCK_TEMPLATES.repeat({
          times: 4,
          steps: [BLOCK_TEMPLATES.key({ scan: 0x11, mode: "tap" }) as Step],
        }) as Step,
      ],
    },
  ],
  keys: Object.entries(LABELS).map(([scan, label]) => ({
    scan: Number(scan),
    label,
    group: "Keys",
  })),
  profiles: [
    { id: "global", program: "Crossup Global", active: true, global: true, mappings: [] },
    {
      id: "profile-2",
      program: "Overwatch.exe",
      active: true,
      global: false,
      mappings: [
        toMapping("OPPOSITE", 0x11, 0x1f),
        toMapping("OPPOSITE", 0x1f, 0x11),
        toMapping("OPPOSITE", 0x1e, 0x20),
        toMapping("OPPOSITE", 0x20, 0x1e),
        toMapping("NEUTRAL", 0x4b, 0x4d),
        toMapping("NEUTRAL", 0x4d, 0x4b),
      ],
    },
  ],
};

const withMappings = (profileId: string, edit: (mappings: MappingView[]) => void): View => {
  const profile = mock.profiles.find((entry) => entry.id === profileId);
  if (profile) edit(profile.mappings);
  return mock;
};

let mockActivity: Activity = {
  running: null,
  clicks: 0,
  elapsedSecs: 0,
  achievedCps: 0,
  requestedCps: 0,
  source: "",
  timerRaised: true,
  last: null,
  lastMacro: null,
  hotkeyErrors: [],
  alerts: [],
};

let mockTicker: ReturnType<typeof setInterval> | null = null;
let mockStartedAt = 0;

const UNIT_MS: Record<RateUnit, number> = {
  second: 1_000,
  minute: 60_000,
  hour: 3_600_000,
  day: 86_400_000,
};

const clicksPerSecond = (settings: ClickerSettings): number =>
  ((settings.rate || 0) * (settings.doubleClick ? 2 : 1) * 1_000) / UNIT_MS[settings.rateUnit];

/** A stand-in that counts up the way the real engine would. */
function mockStart(running: string, requestedCps: number, source: string) {
  if (mockTicker) clearInterval(mockTicker);
  mockStartedAt = Date.now();
  mockActivity = {
    ...mockActivity,
    running,
    requestedCps,
    source,
    clicks: 0,
    elapsedSecs: 0,
    achievedCps: 0,
    last: null,
    lastMacro: null,
  };
  mockTicker = setInterval(() => {
    const elapsed = (Date.now() - mockStartedAt) / 1_000;
    mockActivity = {
      ...mockActivity,
      elapsedSecs: elapsed,
      clicks: Math.floor(mockActivity.requestedCps * elapsed),
      achievedCps: mockActivity.requestedCps,
    };
  }, 120);
}

function mockStop(reason: string) {
  if (mockTicker) clearInterval(mockTicker);
  mockTicker = null;
  const finished = mockActivity.running;
  mockActivity = {
    ...mockActivity,
    running: null,
    last:
      finished === "clicker"
        ? {
            stopReason: reason,
            clicks: mockActivity.clicks,
            elapsedSecs: mockActivity.elapsedSecs,
            achievedCps: mockActivity.requestedCps,
            requestedCps: mockActivity.requestedCps,
          }
        : mockActivity.last,
    lastMacro:
      finished && finished !== "clicker"
        ? { stopReason: `${finished}: ${reason}`, actions: 0, notes: [] }
        : mockActivity.lastMacro,
  };
}

// ── the typer's stand-in ───────────────────────────────────────────

/** The fastest pace the mock will keep, matching the Rust side. */
const MAX_TYPER_CPS = 25;

let mockTyper: TyperStatusView = {
  status: "idle",
  thinking: false,
  typed: 0,
  total: 0,
  plannedMs: 0,
  elapsedSecs: 0,
};

let typerTicker: ReturnType<typeof setInterval> | null = null;
let typerStartedAt = 0;
let typerCps = 6;
let typerChars = 0;

/** The same arithmetic the Rust side does, so the readout matches the run. */
function mockPlan(text: string, settings: TyperSettings): TyperPlan {
  const chars = Array.from(text).length;
  const words = text.trim() ? text.trim().split(/\s+/).length : 0;
  const resting = (Math.min(400, Math.max(5, settings.wpm)) * 5) / 60;
  const longest = Math.min(8, Math.max(0.35, settings.thinkMaxSecs));
  // One stop per sentence, plus one every nineteen words or so of the
  // sentences that run on, matching the Rust planner.
  const sentences = (text.match(/[.!?](?=\s|$)/g) ?? []).length;
  const pausing =
    settings.thinkPauses &&
    (settings.thinkAfterSentence || settings.thinkInLongSentence);
  const pauses = !pausing
    ? 0
    : (settings.thinkAfterSentence ? sentences : 0) +
      (settings.thinkInLongSentence ? Math.max(0, words - sentences) / 22.5 : 0);
  const thinking = pauses * ((0.35 + longest) / 2);
  // Thinking comes out of a deadline rather than pushing the last keystroke
  // past it, exactly as the Rust planner does it.
  const asked = settings.deadlineSecs
    ? chars / Math.max(0.5, Math.max(1, settings.deadlineSecs) - thinking)
    : resting;
  const cps = Math.max(0.5, Math.min(MAX_TYPER_CPS, asked));
  const charMs = 1_000 / cps;
  const corrections = Math.round(chars * Math.min(0.25, Math.max(0, settings.typoRate)));
  const thinkingMs = Math.round(thinking * 1_000);
  // A mistake left for later costs a rewind over the line, then the letter and
  // the jump home. Two sentences is the middle of the one-to-three window.
  const walked = settings.typoDefer
    ? Math.min(chars, (2 * chars) / Math.max(1, sentences))
    : 0;
  const correctionsMs = settings.typoDefer
    ? corrections * (walked * charMs * 0.13 + 780)
    : corrections * (charMs * 4.5 + 235);
  return {
    chars,
    words,
    cps,
    totalMs: Math.round(chars * charMs + correctionsMs + thinkingMs),
    thinkingMs,
    clamped: asked > MAX_TYPER_CPS,
  };
}

function mockStartTyper() {
  const draft = mock.typer;
  if (!draft.text.trim()) return;
  const estimate = mockPlan(draft.text, draft);
  if (typerTicker) clearInterval(typerTicker);
  typerCps = estimate.cps;
  typerChars = estimate.chars;
  typerStartedAt = Date.now();
  mockTyper = {
    status: "typing",
    thinking: false,
    typed: 0,
    total: estimate.chars,
    plannedMs: estimate.totalMs,
    elapsedSecs: 0,
  };
  mockActivity = {
    ...mockActivity,
    running: "typer",
    source: "typing the draft",
    clicks: 0,
    elapsedSecs: 0,
  };
  typerTicker = setInterval(() => {
    if (mockTyper.status !== "typing") return;
    const elapsed = (Date.now() - typerStartedAt) / 1_000;
    const typed = Math.min(typerChars, Math.floor(elapsed * typerCps));
    mockTyper = { ...mockTyper, typed, elapsedSecs: elapsed };
    if (typerChars > 0 && typed >= typerChars) mockStopTyper("done");
  }, 120);
}

function mockStopTyper(status: "done" | "stopped") {
  if (typerTicker) clearInterval(typerTicker);
  typerTicker = null;
  mockTyper = { ...mockTyper, status };
  mockActivity = { ...mockActivity, running: null };
}

// ── the replayer's stand-in ────────────────────────────────────────

let mockReplay: ReplayStateView = {
  recording: false,
  events: 0,
  durationMs: 0,
  pending: null,
  playing: false,
  status: "idle",
  played: 0,
  total: 0,
  pass: 0,
  passes: null,
  note: null,
};

let replayTicker: ReturnType<typeof setInterval> | null = null;

function mockStartRecording() {
  if (replayTicker) clearInterval(replayTicker);
  const startedAt = Date.now();
  mockReplay = {
    ...mockReplay,
    recording: true,
    events: 0,
    durationMs: 0,
    pending: null,
    status: "recording",
  };
  replayTicker = setInterval(() => {
    mockReplay = {
      ...mockReplay,
      events: mockReplay.events + 4,
      durationMs: Date.now() - startedAt,
    };
  }, 150);
}

function mockStopRecording() {
  if (replayTicker) clearInterval(replayTicker);
  replayTicker = null;
  mockReplay = {
    ...mockReplay,
    recording: false,
    status: "stopped",
    pending:
      mockReplay.events > 0
        ? { events: mockReplay.events, durationMs: mockReplay.durationMs }
        : null,
  };
}

function mockStopReplay() {
  if (replayTicker) clearInterval(replayTicker);
  replayTicker = null;
  mockReplay = { ...mockReplay, playing: false, status: "stopped" };
  mockActivity = { ...mockActivity, running: null };
}

function mockStartReplay() {
  const chosen =
    mock.replay.recordings.find((entry) => entry.id === mock.replay.selected) ??
    mock.replay.recordings[0];
  if (!chosen || replayTicker) return;
  const passes = mock.replay.repeat;
  mockReplay = {
    ...mockReplay,
    playing: true,
    status: "replaying",
    played: 0,
    total: chosen.events,
    pass: 1,
    passes,
  };
  mockActivity = { ...mockActivity, running: "replay", source: chosen.name, clicks: 0 };
  let played = 0;
  replayTicker = setInterval(() => {
    if (mockReplay.status !== "replaying") return;
    played += Math.max(1, Math.round(chosen.events / 16));
    if (played >= chosen.events) {
      const nextPass = mockReplay.pass + 1;
      if (passes !== null && nextPass > passes) {
        if (replayTicker) clearInterval(replayTicker);
        replayTicker = null;
        mockReplay = { ...mockReplay, playing: false, status: "done", played: chosen.events };
        mockActivity = { ...mockActivity, running: null };
        return;
      }
      played = 0;
      mockReplay = { ...mockReplay, pass: nextPass, played: 0 };
      return;
    }
    mockReplay = { ...mockReplay, played: (mockReplay.pass - 1) * chosen.events + played };
  }, 180);
}

async function mockCall<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const profile = String(args?.profile ?? "");

  switch (command) {
    case "activity":
      return structuredClone(mockActivity) as T;
    case "replay_state":
      return structuredClone(mockReplay) as T;
    case "start_recording":
      mockStartRecording();
      return structuredClone(mockReplay) as T;
    case "stop_recording":
      mockStopRecording();
      return structuredClone(mockReplay) as T;
    case "start_replay":
      mockStartReplay();
      return structuredClone(mockReplay) as T;
    case "stop_replay":
      mockStopReplay();
      return structuredClone(mockReplay) as T;
    case "discard_recording":
      mockReplay = { ...mockReplay, pending: null };
      return structuredClone(mockReplay) as T;
    case "save_recording": {
      const name = String(args?.name ?? "").trim();
      const pending = mockReplay.pending;
      if (name && pending) {
        const id = `replay-${mock.replay.recordings.length + 1}`;
        mock = {
          ...mock,
          replay: {
            ...mock.replay,
            selected: id,
            recordings: [
              ...mock.replay.recordings,
              { id, name, events: pending.events, durationMs: pending.durationMs },
            ],
          },
        };
        mockReplay = { ...mockReplay, pending: null, status: "idle" };
      }
      break;
    }
    case "delete_recording": {
      mockUndo = structuredClone(mock);
      const id = String(args?.id ?? "");
      const recordings = mock.replay.recordings.filter((entry) => entry.id !== id);
      mock = {
        ...mock,
        replay: {
          ...mock.replay,
          recordings,
          selected:
            mock.replay.selected === id
              ? (recordings[0]?.id ?? "")
              : mock.replay.selected,
        },
      };
      break;
    }
    case "undo_delete":
      if (mockUndo) {
        mock = mockUndo as View;
        mockUndo = null;
      }
      break;
    case "select_recording":
      mock = { ...mock, replay: { ...mock.replay, selected: String(args?.id ?? "") } };
      break;
    case "save_replay_options":
      mock = {
        ...mock,
        replay: {
          ...mock.replay,
          recordHotkey: String(args?.recordHotkey ?? ""),
          replayHotkey: String(args?.replayHotkey ?? ""),
          repeat: (args?.repeat ?? null) as number | null,
          speed: Number(args?.speed ?? 1),
        },
      };
      break;
    case "dismiss_notices":
      break;
    case "typer_status":
      return structuredClone(mockTyper) as T;
    case "typer_plan":
      return mockPlan(String(args?.text ?? ""), args?.settings as TyperSettings) as T;
    case "typer_pause":
      if (mockTyper.status === "typing") mockTyper = { ...mockTyper, status: "paused" };
      return structuredClone(mockTyper) as T;
    case "typer_resume":
      if (mockTyper.status === "paused") mockTyper = { ...mockTyper, status: "typing" };
      return structuredClone(mockTyper) as T;
    case "typer_stop":
      mockStopTyper("stopped");
      return structuredClone(mockTyper) as T;
    case "save_typer":
      mock = { ...mock, typer: args?.typer as TyperSettings };
      break;
    case "start_typer":
      mockStartTyper();
      break;
    case "save_clicker":
      mock = { ...mock, clicker: args?.clicker as ClickerSettings };
      mockActivity = { ...mockActivity, requestedCps: clicksPerSecond(args?.clicker as ClickerSettings) };
      break;
    case "start_clicker": {
      const settings = mock.clicker;
      // A rate that would be refused for real is refused here too.
      if (clicksPerSecond(settings) > 1_000) {
        return Promise.reject("that is more clicks per second than Windows can pace");
      }
      mockStart(
        "clicker",
        clicksPerSecond(settings),
        settings.input === "mouse"
          ? `${settings.button} mouse button`
          : "keyboard key",
      );
      break;
    }
    case "stop_activity":
      mockStop("Stopped");
      break;
    case "save_macro": {
      const definition = args?.definition as Macro;
      const exists = mock.macros.some((entry) => entry.id === definition.id);
      mock = {
        ...mock,
        macros: exists
          ? mock.macros.map((entry) =>
              entry.id === definition.id ? { ...definition, problem: null } : entry,
            )
          : [...mock.macros, { ...definition, problem: null }],
      };
      break;
    }
    case "reorder_macros": {
      const ids = (args?.ids as string[]) ?? [];
      const byId = new Map(mock.macros.map((entry) => [entry.id, entry]));
      const ordered = ids
        .map((id) => byId.get(id))
        .filter((entry): entry is MacroView => Boolean(entry));
      mock = {
        ...mock,
        macros: [...ordered, ...mock.macros.filter((entry) => !ids.includes(entry.id))],
      };
      break;
    }
    case "delete_macro":
      mockUndo = structuredClone(mock);
      mock = { ...mock, macros: mock.macros.filter((entry) => entry.id !== args?.id) };
      break;
    case "run_macro": {
      const definition = mock.macros.find((entry) => entry.id === args?.id);
      if (!definition) return Promise.reject("that macro is no longer in the list");
      mockStart(definition.name, 0, "macro blocks");
      break;
    }
    default:
      break;
  }

  switch (command) {
    case "snapshot":
      break;
    case "set_tool_enabled": {
      // All five tools, in the same shape the Rust command takes them: switching
      // a tool off also stops the run that belongs to it, so the preview cannot
      // show a typer still typing after its switch says off.
      const enabled = Boolean(args?.enabled);
      switch (args?.tool) {
        case "remapper":
          mock = { ...mock, hook_enabled: enabled };
          break;
        case "clicker":
          mock = { ...mock, clicker_enabled: enabled };
          break;
        case "macros":
          mock = { ...mock, macros_enabled: enabled };
          break;
        case "typer":
          if (!enabled && mockActivity.running === "typer") mockStopTyper("stopped");
          mock = { ...mock, typer_enabled: enabled };
          break;
        case "replay":
          if (!enabled && mockActivity.running === "replay") mockStopReplay();
          mock = { ...mock, replay_enabled: enabled };
          break;
        default:
          break;
      }
      break;
    }
    case "set_preferences":
      mock = {
        ...mock,
        close_to_tray: Boolean(args?.close_to_tray),
        debug: Boolean(args?.debug),
      };
      break;
    case "add_profile": {
      const program = String(args?.program ?? "").trim();
      if (program && !mock.profiles.some((entry) => entry.program === program)) {
        mock = {
          ...mock,
          profiles: [
            ...mock.profiles,
            {
              id: `profile-${mock.profiles.length + 1}`,
              program,
              active: true,
              global: false,
              mappings: [],
            },
          ],
        };
      }
      break;
    }
    case "remove_profile":
      mock = {
        ...mock,
        profiles: mock.profiles.filter((entry) => entry.id !== args?.id || entry.global),
      };
      break;
    case "set_profile_active":
      mock = {
        ...mock,
        profiles: mock.profiles.map((entry) =>
          entry.id === args?.id ? { ...entry, active: Boolean(args?.active) } : entry,
        ),
      };
      break;
    case "add_mapping":
      mock = withMappings(profile, (mappings) => {
        const mode = args?.mode as Mode;
        const source = Number(args?.source);
        if (!mappings.some((entry) => entry.mode === mode && entry.source === source)) {
          mappings.push(toMapping(mode, source, Number(args?.dest)));
        }
      });
      break;
    case "update_mapping":
      mock = withMappings(profile, (mappings) => {
        const index = Number(args?.index);
        if (mappings[index]) {
          mappings[index] = toMapping(
            args?.mode as Mode,
            Number(args?.source),
            Number(args?.dest),
          );
        }
      });
      break;
    case "remove_mapping":
      mockUndo = structuredClone(mock);
      mock = withMappings(profile, (mappings) => {
        mappings.splice(Number(args?.index), 1);
      });
      break;
    case "apply_preset": {
      const wasd: [number, number][] = [
        [0x11, 0x1f],
        [0x1f, 0x11],
        [0x1e, 0x20],
        [0x20, 0x1e],
      ];
      const arrows: [number, number][] = [
        [0x48, 0x50],
        [0x50, 0x48],
        [0x4b, 0x4d],
        [0x4d, 0x4b],
      ];
      const preset = String(args?.preset);
      const pairs =
        preset === "wasd" ? wasd : preset === "arrows" ? arrows : [...wasd, ...arrows];
      mock = withMappings(profile, (mappings) => {
        for (const [source, dest] of pairs) {
          if (!mappings.some((entry) => entry.mode === "OPPOSITE" && entry.source === source)) {
            mappings.push(toMapping("OPPOSITE", source, dest));
          }
        }
      });
      break;
    }
    case "set_autostart":
      mock = {
        ...mock,
        autostart: {
          ...mock.autostart,
          enabled: Boolean(args?.enabled),
          // Windows stores the value quoted; the Rust side strips the quotes
          // before reporting it, so this does too.
          entry: args?.enabled ? mock.autostart.exe : null,
          exe: String(args?.exe ?? "").trim() || mock.autostart.running,
        },
      };
      break;
    case "choose_exe":
      // No native file dialog in a browser; the running copy is the answer.
      window.setTimeout(() => {
        window.dispatchEvent(
          new CustomEvent("autostart-exe", { detail: mock.autostart.running }),
        );
      }, 0);
      break;
    case "save_picture": {
      const width = Number(args?.width ?? 0);
      const height = Number(args?.height ?? 0);
      return { id: `pic-preview-${width}x${height}`, width, height } as T;
    }
    case "picture_preview":
      // A one-pixel PNG, so a block shows a thumbnail in the browser too.
      return "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==" as T;
    case "picture_history":
      return [] as T;
    case "delete_picture":
      break;
    case "forget_picture":
      break;
    case "picture_uses":
      return [] as T;
    case "pick_region": {
      const request = args?.request as PickRequest;
      const width =
        request.purpose === "zone"
          ? 320
          : request.purpose === "macro-picture"
            ? 64
            : request.purpose === "macro-shot"
              ? 480
              : 0;
      const height =
        request.purpose === "zone"
          ? 200
          : request.purpose === "macro-picture"
            ? 40
            : request.purpose === "macro-shot"
              ? 270
              : 0;
      window.setTimeout(() => {
        window.dispatchEvent(
          new CustomEvent(REGION_PICKED, {
            detail: {
              purpose: request.purpose,
              index: request.index,
              x: 480,
              y: 320,
              width,
              height,
            } satisfies PickResult,
          }),
        );
      }, 0);
      break;
    }
    case "running_windows":
      // Enough titles to show the suggestions working in a browser.
      return ["Counter-Strike 2", "Notepad", "Visual Studio Code"] as T;
    case "choose_program":
      window.setTimeout(() => {
        window.dispatchEvent(
          new CustomEvent(MACRO_PROGRAM, {
            detail: "C:\\Program Files\\Game\\game.exe",
          }),
        );
      }, 0);
      break;
    case "block_choices":
      return {
        macros: [
          { id: "macro-1", name: "Open the menu" },
          { id: "macro-2", name: "Buy and equip" },
        ],
        recordings: mock.replay.recordings.map((entry) => ({
          id: entry.id,
          name: entry.name,
        })),
      } as T;
    case "picker_context":
    case "picker_finish":
    case "picker_cancel":
      break;
    case "running_processes":
      return [
        "Overwatch.exe",
        "Discord.exe",
        "steam.exe",
        "chrome.exe",
        "explorer.exe",
        "Code.exe",
      ] as T;
    case "export_settings":
      // There is no file behind the browser stand-in to hand back.
      return "{}" as T;
    case "import_settings":
      throw new Error("the browser preview keeps no settings file to import into");
    case "export_socd":
      return mock.profiles
        .map(
          (entry) =>
            `PROFILE_START\nprogram: ${entry.program}\nplatform: WINDOWS\n` +
            entry.mappings
              .map((mapping) => `mapping: ${mapping.mode} ${mapping.source} ${mapping.dest}`)
              .join("\n") +
            `\nPROFILE_END`,
        )
        .join("\n") as T;
    case "import_socd":
      break;
    case "update_state":
      // The browser stand-in has no installer behind it, so nothing is ever
      // offered here.
      return {
        currentVersion: "0.1.0",
        status: "upToDate",
        version: null,
        notes: null,
        downloaded: 0,
        total: null,
        error: null,
      } as T;
    case "update_install":
      break;
    case "activity":
    case "save_clicker":
    case "start_clicker":
    case "stop_activity":
    case "save_macro":
    case "delete_macro":
    case "run_macro":
      // Handled above, before the profile switch.
      break;
    default:
      break;
  }

  return structuredClone(mock) as T;
}

export async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri()) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<T>(command, args);
  }
  return mockCall<T>(command, args);
}

import { useCallback, useEffect, useRef, useState } from "react";
import { Settings, SlidersHorizontal } from "lucide-react";
import { PressDepth } from "@/components/interior/press-depth";
import { Popover } from "@/components/interior/popover";
import { Tabs } from "@/components/interior/tabs";
import { ThemeProvider, useTheme } from "@/theme/ThemeProvider";
import {
  AUTOSTART_EXE,
  call,
  isTauri,
  on,
  type Activity,
  type MappingView,
  type View,
} from "@/bridge";
import { AutoClickerTab } from "./AutoClickerTab";
import { AppearanceDialog } from "./AppearanceDialog";
import { AddProfileDialog, MappingDialog, SettingsDialog, SocdDialog } from "./Dialogs";
import { MacrosTab } from "./MacrosTab";
import { RemapperTab } from "./RemapperTab";
import { ReplayerTab } from "./ReplayerTab";
import { TyperTab } from "./TyperTab";
import { UndoBar } from "./UndoBar";
import { clearUndo, useUndoEntry } from "./undo";
import { UpdateBanner } from "./UpdateBanner";

/**
 * The tools, in the order they sit in the tab strip. Each carries the line the
 * Tools menu prints under its name, because the switch that used to explain
 * itself inside the tab now lives in the header.
 */
const TOOLS = [
  { value: "remapper", label: "Remapper", note: "Off lets every key through untouched." },
  { value: "clicker", label: "Auto clicker", note: "Off stops the clicker shortcut." },
  { value: "typer", label: "Typer", note: "Off stops the typing shortcut." },
  { value: "macros", label: "Macros", note: "Off stops every macro shortcut." },
  { value: "replay", label: "Replayer", note: "Off stops recording and replay." },
];

/** The tab the window was last left on, kept beside the palette. */
const LAST_TAB = "crossup.lastTab";

/**
 * The tab to open on: the one last used, if it is still a tool.
 *
 * A tab that was removed from the strip, or a value written by something else,
 * falls back to the first tool rather than opening on nothing.
 */
function readTab(): string {
  try {
    const saved = localStorage.getItem(LAST_TAB);
    if (saved && TOOLS.some((tool) => tool.value === saved)) return saved;
  } catch {
    // A webview with storage turned off starts on the first tool.
  }
  return TOOLS[0].value;
}

/**
 * The mark beside a tab's name: the tool's own switch, read without opening the
 * tab. A green dot is a tool that is on, a red one is a tool that is off, so the
 * strip answers "what is working?" at a glance. The dot carries a halo only when
 * on, so the state survives a reader who cannot tell the two hues apart.
 */
function ToolMark({ on }: { on: boolean }) {
  return <span className={`hb__tab-mark${on ? "" : " hb__tab-mark--off"}`} />;
}

/**
 * The five tool switches, in one place in the header instead of each tab growing
 * its own. The menu surface is the interior popover: it anchors itself to the
 * trigger, closes on an outside press, gives Escape back to the trigger and puts
 * focus in the panel. This used to do that by hand, and only partly: no outside
 * press, no focus, and a menu that could sit off the window edge.
 *
 * Flipping a switch leaves the menu open, because choosing two tools in a row
 * should not mean opening the menu twice.
 */
function ToolMenu({
  tools,
  onToggle,
}: {
  tools: { value: string; label: string; note: string; on: boolean }[];
  onToggle: (tool: string, enabled: boolean) => void;
}) {
  const [open, setOpen] = useState(false);

  return (
    <Popover
      label="Tools"
      open={open}
      onOpenChange={setOpen}
      align="end"
      triggerClassName="hb__pill hb__tools-trigger"
      trigger={
        <>
          <SlidersHorizontal size={12} strokeWidth={1.8} aria-hidden="true" />
          Tools
        </>
      }
    >
      <div className="hb__tools-menu" role="group" aria-label="Tools">
        {tools.map((tool) => (
          <button
            key={tool.value}
            type="button"
            role="switch"
            aria-checked={tool.on}
            className="hb__tool-row"
            onClick={() => onToggle(tool.value, !tool.on)}
          >
            <ToolMark on={tool.on} />
            <span className="hb__row-text">
              <span className="hb__row-name">{tool.label}</span>
              <span className="hb__row-note">{tool.note}</span>
            </span>
            <span className="hb__switch" aria-hidden="true" />
          </button>
        ))}
      </div>
    </Popover>
  );
}

export function App() {
  return (
    <ThemeProvider>
      <Shell />
    </ThemeProvider>
  );
}

function Shell() {
  const { palette } = useTheme();
  const [view, setView] = useState<View | null>(null);
  const [activity, setActivity] = useState<Activity | null>(null);
  const [tab, setTab] = useState(readTab);
  // Written on every change rather than on exit: the window is closed to the tray
  // far more often than the process ends, and a tab read at exit would be the one
  // from whenever that last happened.
  useEffect(() => {
    try {
      localStorage.setItem(LAST_TAB, tab);
    } catch {
      // Storage off: the tab works, it just is not remembered.
    }
  }, [tab]);
  const [selected, setSelected] = useState("global");
  const [error, setError] = useState<string | null>(null);

  const [settingsOpen, setSettingsOpen] = useState(false);
  const [appearanceOpen, setAppearanceOpen] = useState(false);
  const [addProfileOpen, setAddProfileOpen] = useState(false);
  const [socdOpen, setSocdOpen] = useState(false);
  const [editorOpen, setEditorOpen] = useState(false);
  const [editing, setEditing] = useState<MappingView | null>(null);

  const refresh = useCallback(async () => {
    try {
      const next = await call<View>("snapshot");
      setView(next);
      setSelected((current) =>
        next.profiles.some((profile) => profile.id === current)
          ? current
          : (next.profiles[0]?.id ?? "global"),
      );
    } catch (reason) {
      setError(String(reason));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // What is running is polled rather than pushed: it changes on its own clock,
  // and this is the only thing in the window that does.
  useEffect(() => {
    let live = true;
    const tick = async () => {
      try {
        const next = await call<Activity>("activity");
        if (live) setActivity(next);
      } catch {
        // A preview without the engine simply has nothing to report.
      }
    };
    void tick();
    const timer = setInterval(tick, 250);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, []);

  const run = useCallback(
    async (command: string, args?: Record<string, unknown>) => {
      try {
        const next = await call<View>(command, args);
        setView(next);
        setError(null);
      } catch (reason) {
        setError(String(reason));
      }
    },
    [],
  );

  // One delete is undoable at a time. The bar offers it, and the shortcut that
  // every other Windows app uses does the same thing without reaching the bar.
  const undoEntry = useUndoEntry();
  const undoNow = useCallback(async () => {
    const action = undoEntry?.undo;
    clearUndo();
    await action?.();
  }, [undoEntry]);

  useEffect(() => {
    if (!undoEntry) return;
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "z") return;
      const target = event.target as HTMLElement | null;
      if (target && /^(INPUT|TEXTAREA)$/.test(target.tagName)) return;
      event.preventDefault();
      void undoNow();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [undoEntry, undoNow]);

  /** Whether the sign-in entry is on, read from the latest view by the effect below. */
  const autostartOn = useRef(false);
  useEffect(() => {
    autostartOn.current = view?.autostart.enabled ?? false;
  }, [view]);

  // A chosen executable comes back as an event rather than as the answer to the
  // call that opened the dialog: the dialog is modal to the desktop, and the
  // command that starts it returns as soon as it is on screen.
  useEffect(() => {
    let live = true;
    const stops: (() => void)[] = [];
    void on<string>(AUTOSTART_EXE, (path) => {
      void run("set_autostart", { enabled: autostartOn.current, exe: path });
    }).then((stop) => {
      if (live) stops.push(stop);
      else stop();
    });
    return () => {
      live = false;
      for (const stop of stops) stop();
    };
  }, [run]);

  // The frame is drawn before the settings arrive, shaped like the window it is
  // about to become, so a cold start reads as the window opening rather than as
  // an empty rectangle holding a sentence.
  if (!view) {
    return (
      <div className="hb hb--shell" aria-busy="true">
        <div className="hb__sk-top" />
        <div className="hb__sk-tabs" />
        <div className="hb__sk-split" aria-hidden="true">
          <div className="hb__sk-rail" />
          <div className="hb__sk-page">
            <span className="hb__sk hb__sk-title" />
            <span className="hb__sk hb__sk-line" />
            <span className="hb__sk hb__sk-block" />
            <span className="hb__sk hb__sk-block hb__sk-block--short" />
          </div>
        </div>
        <span className="hb__sk-status">{error ?? "Loading settings"}</span>
      </div>
    );
  }

  const native = isTauri();
  // The header reports whether the hook is in place at all. What each tool does
  // with it is that tool's own switch, in the header's Tools menu.
  const live = native && view.installed;
  const status = !native ? "Preview" : live ? "Active" : "No hook";
  const offCount = [
    view.hook_enabled,
    view.clicker_enabled,
    view.typer_enabled,
    view.macros_enabled,
    view.replay_enabled,
  ].filter((on) => !on).length;
  // One strip, one answer: a shortcut the engine could not take, and anything
  // asked for that could not happen, printed together and dismissable as a set.
  const notices = [...(activity?.hotkeyErrors ?? []), ...(activity?.alerts ?? [])];
  const runningClicker = activity?.running === "clicker";
  const runningTyper = activity?.running === "typer";
  const runningReplay = activity?.running === "replay";
  const runningMacro =
    Boolean(activity?.running) && !runningClicker && !runningTyper && !runningReplay;
  // Which tools are off, by name: a footer of five switch states is a chain
  // nobody scans, but "typer and macros off" is one thing to read.
  const offNames = [
    !view.hook_enabled && "remapper",
    !view.clicker_enabled && "clicker",
    !view.typer_enabled && "typer",
    !view.macros_enabled && "macros",
    !view.replay_enabled && "replayer",
  ].filter((name): name is string => Boolean(name));
  // The same switches, in the order the strip draws them. A tool that is off is
  // still a tab you can open: the mark is a status, not a lock.
  const toolOn: Record<string, boolean> = {
    remapper: view.hook_enabled,
    clicker: view.clicker_enabled,
    typer: view.typer_enabled,
    macros: view.macros_enabled,
    replay: view.replay_enabled,
  };
  // The same five switches, drawn once in the header. The strip keeps the dot,
  // which is a status rather than a control, so there is one place to flip them.
  const tools = TOOLS.map((tool) => ({ ...tool, on: toolOn[tool.value] }));
  const tabs = TOOLS.map((tool) => ({
    value: tool.value,
    label: tool.label,
    indicator: <ToolMark on={toolOn[tool.value]} />,
    indicatorLabel: toolOn[tool.value] ? "on" : "off",
  }));

  return (
    <div className="hb hb--shell">
      <header className="hb__top mat-panel">
        <span className="hb__mark" aria-hidden="true">
          <svg width="13" height="13" viewBox="0 0 16 16" fill="none">
            <path
              d="M2.5 8h11M8 2.5v11"
              stroke="currentColor"
              strokeWidth="1.6"
              strokeLinecap="round"
            />
            <circle cx="8" cy="8" r="2.2" fill="currentColor" />
          </svg>
        </span>
        <span className="hb__brand-name">Crossup</span>

        <span className={`hb__pill${live ? " hb__pill--on" : ""}`}>
          <span
            className={`hb__gear-dot${live ? "" : " hb__gear-dot--off"}`}
            aria-hidden="true"
          />
          {status}
        </span>

        {offCount > 0 && (
          <span className="hb__pill hb__pill--off" title="A tool is switched off in the Tools menu">
            {offCount === 1 ? "1 tool off" : `${offCount} tools off`}
          </span>
        )}

        {activity?.running && (
          <span className="hb__pill hb__pill--on hb__pill--live">
            <span className="hb__pulse" aria-hidden="true" />
            {runningClicker
              ? `clicking · ${activity.clicks.toLocaleString()}`
              : runningTyper
                ? "typing"
                : runningReplay
                  ? "replaying"
                  : activity.running}
          </span>
        )}

        <span className="hb__spacer" />

        <span className="hb__brand-note" title="Program currently in front">
          {view.program || "no foreground program"}
        </span>
        <ToolMenu
          tools={tools}
          onToggle={(tool, enabled) => void run("set_tool_enabled", { tool, enabled })}
        />
        <button
          type="button"
          className="hb__icon"
          aria-label="Open settings"
          title="Settings"
          onClick={() => setSettingsOpen(true)}
        >
          <Settings size={16} strokeWidth={1.6} aria-hidden="true" />
        </button>
      </header>

      {/* Only while a newer release is waiting: one row, and Later hides it for
          the rest of the run. */}
      <UpdateBanner />

      {/* A shortcut the engine could not take, and anything that was asked for
          and could not happen, share one strip: a key that does nothing and says
          nothing is the same failure as a replay that does nothing and says
          nothing. Each tab used to print its own copy in a corner, so both were
          invisible on the tab that set them. */}
      {notices.length > 0 && (
        <div className="hb__notices" role="status">
          <div className="hb__notices-list">
            {notices.map((message) => (
              <p className="hb__warn" key={message}>
                {message}
              </p>
            ))}
          </div>
          <button
            type="button"
            className="hb__mini"
            title="Dismiss"
            onClick={() => void run("dismiss_notices")}
          >
            Dismiss
          </button>
        </div>
      )}

      <Tabs
        items={tabs}
        value={tab}
        onValueChange={setTab}
        label="Tools"
        className="hb__tabs"
        panelClassName="hb__tabpanel"
        renderPanel={(value) =>
          value === "remapper" ? (
            <RemapperTab
              view={view}
              selected={selected}
              onSelect={setSelected}
              onAddProfile={() => setAddProfileOpen(true)}
              onEdit={(mapping) => {
                setEditing(mapping);
                setEditorOpen(true);
              }}
              run={run}
            />
          ) : value === "clicker" ? (
            <AutoClickerTab
              view={view}
              activity={activity}
              run={run}
              onError={setError}
            />
          ) : value === "typer" ? (
            <TyperTab view={view} run={run} onError={setError} />
          ) : value === "replay" ? (
            <ReplayerTab view={view} run={run} onError={setError} />
          ) : (
            <MacrosTab view={view} activity={activity} run={run} onError={setError} />
          )
        }
      />

      {error && (
        <p className="hb__row-note hb__error" role="status">
          {error}
        </p>
      )}

      <UndoBar onUndo={undoNow} />

      <footer className="hb__foot">
        <span>{view.installed ? "Keyboard hook installed" : "Preview mode"}</span>
        <span>·</span>
        <span>
          {offNames.length === 0 ? "every tool on" : `${offNames.join(", ")} off`}
        </span>
        {activity?.running && (
          <>
            <span>·</span>
            <span>
              {runningClicker
                ? `${activity.achievedCps.toFixed(1)} of ${activity.requestedCps.toFixed(1)} CPS`
                : runningTyper
                  ? "typing the draft"
                  : runningReplay
                    ? `replaying "${activity.source}"`
                    : `running "${activity.running}"`}
            </span>
          </>
        )}
        {runningMacro && activity?.lastMacro && (
          <>
            <span>·</span>
            <span>{activity.lastMacro.stopReason}</span>
          </>
        )}
        <span className="hb__spacer" />
        <span>
          hue {Math.round(palette.hue)}° · {palette.mode}
        </span>
      </footer>

      <SettingsDialog
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        view={view}
        onPreferences={(next) =>
          run("set_preferences", {
            closeToTray: next.close_to_tray,
            debug: next.debug,
          })
        }
        onAutostart={(enabled, exe) => run("set_autostart", { enabled, exe })}
        onChooseExe={() => {
          void call("choose_exe").catch((reason) => setError(String(reason)));
        }}
        onAppearance={() => {
          setSettingsOpen(false);
          setAppearanceOpen(true);
        }}
        onSocd={() => {
          setSettingsOpen(false);
          setSocdOpen(true);
        }}
      />

      <AppearanceDialog open={appearanceOpen} onClose={() => setAppearanceOpen(false)} />

      <AddProfileDialog
        open={addProfileOpen}
        onClose={() => setAddProfileOpen(false)}
        profiles={view.profiles}
        onAdd={(program) => run("add_profile", { program })}
      />

      <MappingDialog
        open={editorOpen}
        onClose={() => setEditorOpen(false)}
        view={view}
        initial={editing}
        onSave={(mapping) => {
          const profile =
            view.profiles.find((entry) => entry.id === selected) ?? view.profiles[0];
          const index = editing
            ? profile.mappings.findIndex(
                (entry) =>
                  entry.mode === editing.mode && entry.source === editing.source,
              )
            : -1;
          if (index >= 0) {
            run("update_mapping", {
              profile: profile.id,
              index,
              mode: mapping.mode,
              source: mapping.source,
              dest: mapping.dest,
            });
          } else {
            run("add_mapping", {
              profile: profile.id,
              mode: mapping.mode,
              source: mapping.source,
              dest: mapping.dest,
            });
          }
        }}
      />

      <SocdDialog
        open={socdOpen}
        onClose={() => setSocdOpen(false)}
        text={
          view.profiles
            .map(
              (entry) =>
                `PROFILE_START\nprogram: ${entry.program}\nplatform: WINDOWS\n` +
                entry.mappings
                  .map(
                    (mapping) =>
                      `mapping: ${mapping.mode} ${mapping.source} ${mapping.dest}`,
                  )
                  .join("\n") +
                (entry.mappings.length ? "\n" : "") +
                "PROFILE_END",
            )
            .join("\n")
        }
        onImport={(text) => run("import_socd", { text })}
        onImportJson={(text) => run("import_settings", { text })}
      />
    </div>
  );
}

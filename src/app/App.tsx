import { useCallback, useEffect, useRef, useState } from "react";
import { PressDepth } from "@/components/interior/press-depth";
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
import { UpdateBanner } from "./UpdateBanner";

/** The three tools, in the order they sit in the tab strip. */
const TOOLS = [
  { value: "remapper", label: "Remapper" },
  { value: "clicker", label: "Auto clicker" },
  { value: "macros", label: "Macros" },
];

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
  const [tab, setTab] = useState("remapper");
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

  if (!view) {
    return (
      <div
        style={{
          display: "grid",
          placeItems: "center",
          height: "100%",
          color: "var(--ink-3)",
          fontSize: 12,
        }}
      >
        {error ?? "Loading the hook…"}
      </div>
    );
  }

  const native = isTauri();
  // The header reports whether the hook is in place at all. What each tool does
  // with it is that tool's own switch, in its own tab.
  const live = native && view.installed;
  const status = !native ? "Preview" : live ? "Active" : "No hook";
  const offCount = [view.hook_enabled, view.clicker_enabled, view.macros_enabled].filter(
    (on) => !on,
  ).length;
  const runningClicker = activity?.running === "clicker";
  const runningMacro = Boolean(activity?.running) && !runningClicker;

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
          <span className="hb__pill hb__pill--off" title="A tool is switched off in its own tab">
            {offCount === 1 ? "1 tool off" : `${offCount} tools off`}
          </span>
        )}

        {activity?.running && (
          <span className="hb__pill hb__pill--on hb__pill--live">
            <span className="hb__pulse" aria-hidden="true" />
            {runningClicker
              ? `clicking · ${activity.clicks.toLocaleString()}`
              : activity.running}
          </span>
        )}

        <span className="hb__spacer" />

        <span className="hb__brand-note" title="Program currently in front">
          {view.program || "no foreground program"}
        </span>
        <button
          type="button"
          className="hb__icon"
          aria-label="Open settings"
          title="Settings"
          onClick={() => setSettingsOpen(true)}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <circle cx="12" cy="12" r="3.2" stroke="currentColor" strokeWidth="1.5" />
            <path
              d="m19.4 13.5 1.3 1a.7.7 0 0 1 .1.9l-1.3 2.2a.7.7 0 0 1-.8.3l-1.6-.5a6.6 6.6 0 0 1-1.6.9l-.3 1.7a.7.7 0 0 1-.7.6h-2.6a.7.7 0 0 1-.7-.6l-.3-1.7a6.6 6.6 0 0 1-1.6-.9l-1.6.5a.7.7 0 0 1-.8-.3L4.2 15.4a.7.7 0 0 1 .1-.9l1.3-1a6.8 6.8 0 0 1 0-1.9l-1.3-1a.7.7 0 0 1-.1-.9l1.3-2.2a.7.7 0 0 1 .8-.3l1.6.5a6.6 6.6 0 0 1 1.6-.9l.3-1.7a.7.7 0 0 1 .7-.6h2.6a.7.7 0 0 1 .7.6l.3 1.7a6.6 6.6 0 0 1 1.6.9l1.6-.5a.7.7 0 0 1 .8.3l1.3 2.2a.7.7 0 0 1-.1.9l-1.3 1a6.8 6.8 0 0 1 0 1.9Z"
              stroke="currentColor"
              strokeWidth="1.3"
              strokeLinejoin="round"
            />
          </svg>
        </button>
      </header>

      {/* Only while a newer release is waiting: one row, and Later hides it for
          the rest of the run. */}
      <UpdateBanner />

      <Tabs
        items={TOOLS}
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

      <footer className="hb__foot">
        <span>{view.installed ? "Keyboard hook installed" : "Preview mode"}</span>
        <span>·</span>
        <span>
          remapper {view.hook_enabled ? "on" : "off"} · clicker{" "}
          {view.clicker_enabled ? "on" : "off"} · macros {view.macros_enabled ? "on" : "off"}
        </span>
        {activity?.running && (
          <>
            <span>·</span>
            <span>
              {runningClicker
                ? `${activity.achievedCps.toFixed(1)} of ${activity.requestedCps.toFixed(1)} CPS`
                : `running “${activity.running}”`}
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

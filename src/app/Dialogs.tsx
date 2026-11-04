import { useEffect, useMemo, useState } from "react";
import { ArrowDownUp, ChevronRight } from "lucide-react";
import { CopyButton } from "@/components/interior/copy-button";
import { Modal } from "@/components/interior/modal";
import { PressDepth } from "@/components/interior/press-depth";
import { SegmentedControl } from "@/components/interior/segmented-control";
import {
  MODE_LABELS,
  MODE_NOTES,
  call,
  isTauri,
  keyLabel,
  updateCheck,
  updateInstall,
  updateState,
  type MappingView,
  type Mode,
  type ProfileView,
  type UpdateView,
  type View,
} from "@/bridge";
import { KeyBinder } from "./Mappings";

const labelFor = (view: View, scan: number | null): string =>
  scan === null
    ? "pick a key"
    : (view.keys.find((key) => key.scan === scan)?.label ?? keyLabel(scan));

// ── settings ───────────────────────────────────────────────────────

export function SettingsDialog({
  open,
  onClose,
  view,
  onPreferences,
  onAutostart,
  onChooseExe,
  onAppearance,
  onSocd,
}: {
  open: boolean;
  onClose: () => void;
  view: View;
  onPreferences: (next: { close_to_tray: boolean; debug: boolean }) => void;
  /** `exe` empty means "the copy that is running". */
  onAutostart: (enabled: boolean, exe: string) => void;
  onChooseExe: () => void;
  onAppearance: () => void;
  onSocd: () => void;
}) {
  const autostart = view.autostart;
  // The entry can point somewhere else than the setting does: an older install,
  // a copy that has since moved. Saying which one Windows will actually run is
  // the difference between a switch that looks right and one that is.
  const wrongTarget =
    autostart.enabled && autostart.entry !== null && autostart.entry !== autostart.exe;
  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Settings"
      description="Everything here applies immediately."
      maxWidth={520}
      footer={
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <span className="hb__row-note" style={{ margin: 0 }}>
            {view.installed ? "Keyboard hook installed" : "Preview: no hook in a browser"}
          </span>
          <span className="hb__spacer" />
          <PressDepth depth={3} tilt={5} onClick={onClose}>
            Done
          </PressDepth>
        </div>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 9 }}>
        <button type="button" className="hb__row" onClick={onAppearance}>
          <span style={{ display: "flex", flexDirection: "column", flex: 1 }}>
            <span className="hb__row-name">Appearance</span>
            <span className="hb__row-note">Hue, saturation, mode and presets</span>
          </span>
          <ChevronRight size={13} strokeWidth={1.7} aria-hidden="true" />
        </button>

        <div className="hb__row">
          <span style={{ display: "flex", flexDirection: "column", flex: 1 }}>
            <span className="hb__row-name">Close to tray</span>
            <span className="hb__row-note">
              Closing the window keeps the hook running in the tray
            </span>
          </span>
          <Switch
            checked={view.close_to_tray}
            label="Close to tray"
            onChange={(checked) =>
              onPreferences({ close_to_tray: checked, debug: view.debug })
            }
          />
        </div>

        <div className="hb__row">
          <span style={{ display: "flex", flexDirection: "column", flex: 1 }}>
            <span className="hb__row-name">Debug logging</span>
            <span className="hb__row-note">
              Record key decisions and profile switches in the footer log
            </span>
          </span>
          <Switch
            checked={view.debug}
            label="Debug logging"
            onChange={(checked) =>
              onPreferences({ close_to_tray: view.close_to_tray, debug: checked })
            }
          />
        </div>

        <div className="hb__row">
          <span style={{ display: "flex", flexDirection: "column", flex: 1 }}>
            <span className="hb__row-name">Launch at sign in</span>
            <span className="hb__row-note">
              Starts with Windows, from the same entry Task Manager&rsquo;s Startup
              tab shows
            </span>
          </span>
          <Switch
            checked={autostart.enabled}
            label="Launch at sign in"
            onChange={(checked) => onAutostart(checked, "")}
          />
        </div>

        <div className="hb__row" style={{ alignItems: "flex-start" }}>
          <span
            style={{ display: "flex", flexDirection: "column", flex: 1, minWidth: 0, gap: 2 }}
          >
            <span className="hb__row-name">The copy it starts</span>
            <span className="hb__path" title={autostart.exe}>
              {autostart.exe || "unknown"}
            </span>
            {wrongTarget && (
              <span className="hb__row-note hb__warn" style={{ margin: 0 }}>
                Windows runs {autostart.entry} instead
              </span>
            )}
            {!wrongTarget && autostart.exe !== autostart.running && (
              <span className="hb__row-note" style={{ margin: 0 }}>
                That is not the copy running now
              </span>
            )}
          </span>
          <span style={{ display: "flex", gap: 5 }}>
            <button type="button" className="hb__mini" onClick={onChooseExe}>
              Choose…
            </button>
            {autostart.exe !== autostart.running && (
              <button
                type="button"
                className="hb__mini"
                onClick={() => onAutostart(autostart.enabled, "")}
              >
                Use this copy
              </button>
            )}
          </span>
        </div>

        <button type="button" className="hb__row" onClick={onSocd}>
          <span style={{ display: "flex", flexDirection: "column", flex: 1 }}>
            <span className="hb__row-name">Backup and restore</span>
            <span className="hb__row-note">
              Copy every setting out as settings.json, or put a copy back
            </span>
          </span>
          <ArrowDownUp size={13} strokeWidth={1.7} aria-hidden="true" />
        </button>

        <Updates />

        {view.debug && (
          <div
            className="mat-well"
            style={{
              maxHeight: 132,
              overflowY: "auto",
              padding: "9px 11px",
              borderRadius: 10,
              fontFamily: "var(--font-mono), ui-monospace, monospace",
              fontSize: 10.5,
              lineHeight: 1.6,
              color: "var(--ink-3)",
            }}
          >
            {view.log.length === 0 ? "Nothing logged yet." : view.log.slice(-40).join("\n")}
          </div>
        )}
      </div>
    </Modal>
  );
}

/**
 * The version, and the same check the shell runs on its own clock, on demand.
 *
 * The shell asks GitHub when it starts and every six hours after that; this row
 * is for the person who would rather not wait, and for the answer to be visible
 * when it arrives. An install restarts Crossup, so the button is one way only.
 */
function Updates() {
  const [state, setState] = useState<UpdateView | null>(null);
  const [checking, setChecking] = useState(false);
  const native = isTauri();

  useEffect(() => {
    if (!native) return;
    let live = true;
    updateState()
      .then((next) => {
        if (live) setState(next);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [native]);

  const check = async () => {
    setChecking(true);
    try {
      setState(await updateCheck());
    } catch (reason) {
      setState((current) =>
        current ? { ...current, status: "error", error: String(reason) } : current,
      );
    } finally {
      setChecking(false);
    }
  };

  const working =
    state?.status === "downloading" ||
    state?.status === "installing" ||
    state?.status === "restarting";
  const offered = state?.status === "available" && Boolean(state.version);
  const version = state?.currentVersion ? `Crossup ${state.currentVersion}` : "Crossup";

  const note = !native
    ? "Updates install from the desktop app, not a browser preview."
    : working
      ? state?.status === "downloading"
        ? `Downloading ${megabytes(state.downloaded)}${state.total ? ` of ${megabytes(state.total)}` : ""}. Crossup restarts when it lands.`
        : "Installing. Crossup restarts into the new build."
      : offered
        ? `Version ${state?.version} is available. Installing replaces this copy and restarts it.`
        : state?.status === "error"
          ? (state.error ?? "The check did not finish.")
          : state?.status === "upToDate"
            ? "This is the newest release."
            : "Checked when Crossup starts, and every six hours after that.";

  return (
    <div className="hb__row" style={{ alignItems: "flex-start" }}>
      <span
        style={{ display: "flex", flexDirection: "column", flex: 1, minWidth: 0, gap: 2 }}
      >
        <span className="hb__row-name">{version}</span>
        <span className="hb__row-note" style={{ margin: 0 }}>
          {note}
        </span>
        {offered && state?.notes && (
          <span
            className="hb__row-note hb__dim"
            style={{ margin: 0, whiteSpace: "pre-line" }}
          >
            {state.notes.split("\n").slice(0, 4).join("\n")}
          </span>
        )}
      </span>
      <span style={{ display: "flex", gap: 5 }}>
        {offered || working ? (
          <button
            type="button"
            className="hb__mini"
            disabled={working}
            onClick={() => void updateInstall().catch(() => undefined)}
          >
            {working ? "Installing…" : "Update now"}
          </button>
        ) : (
          <button
            type="button"
            className="hb__mini"
            disabled={!native || checking}
            onClick={() => void check()}
          >
            {checking ? "Checking…" : "Check for updates"}
          </button>
        )}
      </span>
    </div>
  );
}

/** A byte count as the update row writes one. */
const megabytes = (bytes: number): string => `${(bytes / (1024 * 1024)).toFixed(1)} MB`;

export function Switch({
  checked,
  label,
  onChange,
}: {
  checked: boolean;
  label: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className="hb__switch"
      onClick={() => onChange(!checked)}
    />
  );
}

// ── add program ────────────────────────────────────────────────────

export function AddProfileDialog({
  open,
  onClose,
  profiles,
  onAdd,
}: {
  open: boolean;
  onClose: () => void;
  profiles: ProfileView[];
  onAdd: (program: string) => void;
}) {
  const [processes, setProcesses] = useState<string[]>([]);
  const [query, setQuery] = useState("");
  const taken = useMemo(
    () => new Set(profiles.map((profile) => profile.program.toLowerCase())),
    [profiles],
  );

  useEffect(() => {
    if (!open) return;
    setQuery("");
    call<string[]>("running_processes")
      .then(setProcesses)
      .catch(() => setProcesses([]));
  }, [open]);

  const matches = processes.filter((name) =>
    name.toLowerCase().includes(query.trim().toLowerCase()),
  );

  const submit = (program: string) => {
    const name = program.trim();
    if (!name) return;
    onAdd(name.includes(".") ? name : `${name}.exe`);
    onClose();
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Add a program"
      description="Crossup runs its mappings while this program is in front. Pick a running one, or type an executable name."
      maxWidth={460}
      footer={
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <span className="hb__row-note" style={{ margin: 0 }}>
            Enter works too
          </span>
          <span className="hb__spacer" />
          <button type="button" className="appearance__reset" onClick={onClose}>
            Cancel
          </button>
        </div>
      }
    >
      <input
        className="hb__field"
        style={{ marginBottom: 10 }}
        placeholder="Type to narrow, then press Enter…"
        aria-label="Program name"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") submit(matches[0] ?? query);
        }}
      />
      <div className="mat-well" style={{ maxHeight: 260, overflowY: "auto", padding: 6, borderRadius: 10 }}>
        {matches.length === 0 && (
          <p className="hb__row-note" style={{ padding: "8px 9px", margin: 0 }}>
            No running program matches. Press Enter to add "{query}" anyway.
          </p>
        )}
        {matches.map((name) => {
          const exists = taken.has(name.toLowerCase());
          return (
            <button
              key={name}
              type="button"
              className="hb__process"
              disabled={exists}
              onClick={() => submit(name)}
            >
              <span
                aria-hidden="true"
                style={{
                  width: 5,
                  height: 5,
                  borderRadius: 999,
                  background: exists ? "var(--ink-3)" : "var(--moss)",
                }}
              />
              {name}
              {exists && (
                <span className="hb__profile-meta" style={{ marginLeft: "auto" }}>
                  added
                </span>
              )}
            </button>
          );
        })}
      </div>
    </Modal>
  );
}

// ── mapping editor ─────────────────────────────────────────────────

export function MappingDialog({
  open,
  onClose,
  view,
  initial,
  onSave,
}: {
  open: boolean;
  onClose: () => void;
  view: View;
  initial: MappingView | null;
  onSave: (mapping: { mode: Mode; source: number; dest: number }) => void;
}) {
  const [mode, setMode] = useState<Mode>(initial?.mode ?? "OPPOSITE");
  const [source, setSource] = useState<number | null>(initial?.source ?? null);
  const [dest, setDest] = useState<number | null>(initial?.dest ?? null);

  useEffect(() => {
    if (!open) return;
    setMode(initial?.mode ?? "OPPOSITE");
    setSource(initial?.source ?? null);
    setDest(initial?.dest ?? null);
  }, [open, initial]);

  const ready = source !== null && dest !== null && source !== dest;

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={initial ? "Edit mapping" : "Add mapping"}
      description={MODE_NOTES[mode]}
      maxWidth={470}
      footer={
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <button type="button" className="appearance__reset" onClick={onClose}>
            Cancel
          </button>
          <span className="hb__spacer" />
          <span style={{ opacity: ready ? 1 : 0.5, pointerEvents: ready ? "auto" : "none" }}>
            <PressDepth
              depth={3}
              tilt={5}
              disabled={!ready}
              onClick={() => {
                if (!ready) return;
                onSave({ mode, source: source!, dest: dest! });
                onClose();
              }}
            >
              {initial ? "Save" : "Add mapping"}
            </PressDepth>
          </span>
        </div>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        <div>
          <div className="appearance__label">Mode</div>
          <SegmentedControl
            label="Mapping mode"
            options={(Object.keys(MODE_LABELS) as Mode[]).map((value) => ({
              value,
              label: MODE_LABELS[value],
            }))}
            value={mode}
            onValueChange={(value) => setMode(value as Mode)}
          />
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
          <div style={{ flex: 1 }}>
            <div className="appearance__label">{mode === "REMAP" ? "From" : "Key"}</div>
            <KeyBinder
              scan={source}
              label={labelFor(view, source)}
              onChange={setSource}
            />
          </div>
          <span className="hb__arrow" aria-hidden="true" style={{ marginTop: 14 }}>
            {mode === "REMAP" ? "→" : "⇄"}
          </span>
          <div style={{ flex: 1 }}>
            <div className="appearance__label">
              {mode === "REMAP" ? "Produces" : "Opposite"}
            </div>
            <KeyBinder
              scan={dest}
              label={labelFor(view, dest)}
              onChange={setDest}
            />
          </div>
        </div>

        {source !== null && dest === source && (
          <p className="hb__row-note" style={{ margin: 0 }}>
            A key cannot be paired with itself.
          </p>
        )}
      </div>
    </Modal>
  );
}

// ── settings backup import / export ────────────────────────────────

/** The two shapes of the same settings file. */
type BackupFormat = "json" | "socd";

const BACKUP_FORMATS: { value: BackupFormat; label: string }[] = [
  { value: "json", label: "settings.json" },
  { value: "socd", label: "settings.socd" },
];

/**
 * The settings as text, taken out or put back in.
 *
 * JSON is the app's own file and holds everything: profiles, macros, the
 * clicker, the switches and the sign-in executable. A copy of it rebuilds
 * Crossup on another machine, or after a reinstall that forgot. `.socd` is the
 * original format and only carries profiles and mappings.
 */
export function SocdDialog({
  open,
  onClose,
  text,
  onImport,
  onImportJson,
}: {
  open: boolean;
  onClose: () => void;
  text: string;
  onImport: (text: string) => void;
  onImportJson: (text: string) => void;
}) {
  const [format, setFormat] = useState<BackupFormat>("json");
  const [draft, setDraft] = useState(text);
  // The JSON comes from the app's own file rather than from the window's copy
  // of it, so the backup is always the whole thing.
  const [backup, setBackup] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) setDraft(text);
  }, [open, text]);

  useEffect(() => {
    if (!open) return;
    let live = true;
    call<string>("export_settings")
      .then((contents) => {
        if (live) {
          setBackup(contents);
          setError(null);
        }
      })
      .catch((reason) => {
        if (live) setError(String(reason));
      });
    return () => {
      live = false;
    };
  }, [open]);

  const json = format === "json";
  const shown = json ? (backup ?? "") : draft;

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Settings backup"
      description="Copy this out to keep it, or paste one in to put every setting back."
      maxWidth={560}
      footer={
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <CopyButton value={shown} label="Copy" copiedLabel="Copied" />
          <span className="hb__spacer" />
          <button type="button" className="appearance__reset" onClick={onClose}>
            Cancel
          </button>
          <PressDepth
            depth={3}
            tilt={5}
            onClick={() => {
              if (json) onImportJson(backup ?? "");
              else onImport(draft);
              onClose();
            }}
          >
            Import
          </PressDepth>
        </div>
      }
    >
      <div className="blk__fields" style={{ marginBottom: 12 }}>
        <SegmentedControl
          label="Format"
          options={BACKUP_FORMATS}
          value={format}
          onValueChange={(value) => setFormat(value as BackupFormat)}
        />
      </div>

      <textarea
        className="hb__code"
        aria-label={json ? "settings.json contents" : "settings.socd contents"}
        value={shown}
        onChange={(event) =>
          json ? setBackup(event.target.value) : setDraft(event.target.value)
        }
        spellCheck={false}
      />

      {error && <p className="hb__warn">{error}</p>}
      <p className="hb__row-note">
        {json
          ? "Everything: profiles, macros, the clicker and the switches. The app also keeps this file at %APPDATA%\\dev.crossup.app\\settings.json. Replace it there with the app closed and it starts from the backup."
          : "The original Hitboxer format. Profiles and mappings only; macros and the auto clicker are not in it."}
      </p>
    </Modal>
  );
}

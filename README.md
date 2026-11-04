# Crossup

A Tauri rebuild of the input tools behind
[valignatev/hitboxer](https://github.com/valignatev/hitboxer), kept under its own
name so it is never mistaken for the original. Five input tools behind one
window, one per tab:

| Tab | What it does | Where it comes from |
| --- | --- | --- |
| **Remapper** | Keyboard remapping and the four SOCD modes, per program | a reconstruction of [valignatev/hitboxer](https://github.com/valignatev/hitboxer) v0.13 |
| **Auto clicker** | Clicking at the rate it was asked for, with duty cycle, random rate, double clicks, click points, per-monitor edge and corner failsafes, stop zones and program targets | a reimplementation of [Blur-AutoClicker](https://github.com/Blur009/Blur-AutoClicker)'s behaviour |
| **Typer** | Types a draft into whichever window is in front, at a person's pace, with mistypes, a deadline and a focus guard | the typer from QuillNot, a sibling app in this vault |

| **Macros** | A shortcut that runs a sequence of input, built as a drag-and-drop block tree | the [AutoHotkey](https://www.autohotkey.com/) idea, with a UI in front of it instead of a script |
| **Replayer** | Records mouse movement, clicks, wheel and keys, then replays them once or forever, each from a single toggle key | built here, on the app's own keyboard and mouse hooks |

The whole thing is a Tauri desktop app with React and the Interior.dev
components. The five tabs are one process and one window, and only one of them
drives the desktop at a time.

## Where each of them came from

**Remapper.** Hitboxer is written in Jai (closed beta), so there is no source to
port. That tab was rebuilt from the shipped Windows binary with **REA** (reverse
engineering agent) driving **Ghidra 12.1.4**, plus the artifact's own
`settings.socd`, log and README as documentation.

**Auto clicker.** Blur-AutoClicker publishes its source, so no REA was needed:
the engine here was written from its cadence maths and its `ClickerConfig`. Two
details were worth getting right rather than guessing at. Its random speed is a
*percentage* of the interval (`speed_randomization`, 35% by default in its own
presets), not a min/max range, so this takes a percentage too; and its accuracy
comes from scheduling against absolute deadlines rather than `now + interval`,
which is the one thing its README leads with and the one thing an auto clicker
gets wrong.

**Macros.** There is nothing to port here — AutoHotkey is a scripting language,
and what was taken from it is the idea of a hotkey that replays input. What is
*not* taken is the language: a macro is a tree of blocks, dragged into place,
nestable inside a `Repeat`, and validated before it can be saved or run.

**Typer.** Taken from the typer in QuillNot, a sibling app in this vault, and
re-cut in this app's vocabulary: a draft typed one keystroke at a time, a share
of it mistyped and corrected, a deadline that paces the whole run, and a stop
that is read between keystrokes. What is *not* carried over is QuillNot's
model-written ending ("write the rest"): Crossup has no network client, and a
button that needs an API key has no place in an input tool. What is added is
this app's own rule — the run claims the same one-run slot as the clicker and
the macros, and stands down if the window in front changes.

## What the artifact said

| Observation | Evidence |
| --- | --- |
| `hitboxer.exe`, PE32+ x86-64 GUI, 1.76 MB, SHA-256 `7ff5a565…4626bc` | REA `binary_overview`, `ev_e6d0b939…` |
| 2,013 procedures, 3,721 strings, `.text` at `0x140001000` | same overview |
| Four modes: `OPPOSITE`, `OPPOSITE_NO_REPRESS`, `NEUTRAL`, `REMAP` | strings, incl. "Expected one of OPPOSITE\|OPPOSITE_NO_REPRESS\|NEUTRAL\|REMAP" |
| Key events logged as *Pressed*, *Pressed (repeat)*, *Released*, *Releasing opposite*, *Re-pressing opposite* | strings in the hook handler |
| Remapping done with `SendInput` and hardware scan codes | decompiled `FUN_14007e1e0`; `KEYEVENTF_SCANCODE`, extended-key flag set per key |
| Profiles are per program plus a `Hitboxer Global` default | `settings.socd`, `PROFILE_START`/`PROFILE_END` |
| `mapping: <MODE> <source> <dest>` stores decimal **scan codes** (W=17, S=31, A=30, D=32) | `settings.socd` |
| Tray icon, close-to-tray, single-instance handoff, Raw Input initialisation | strings (`platform_hide_to_tray`, "Another instance …", "Failed to initialize Raw Input.") |

The mode branches were confirmed in the decompilation of `FUN_14007e1e0`
(`0x14007e1e0`, body `0x14007e1e0–0x14007eff6`, two ranges): a mode-indexed
switch where a key-up releases the held opposite (logging *Releasing opposite*),
`REMAP` never deals in opposites, and one branch returns without re-pressing —
the *no repress* mode. Evidence IDs: `ev_f1a89683…` (string search),
xrefs at `0x14007ec8d` / `0x14007ec94`, and the function dossier for
`0x14007e1e0`.

### Inferences, and what could not be established

- **Inference:** the *no repress* and *neutral* modes keep a lost direction
  blocked until it is released and pressed again. The decompiled branches show
  the release without the re-press, but the exact bookkeeping structs are Jai
  inlined data that Ghidra reports as anonymous; the state machine here was
  written to reproduce the observable behaviour and is covered by tests.
- **Inference:** the global profile is the fallback when a program profile is
  disabled.
- **Unknown:** whether the original's Raw Input path changes behaviour in games
  that ignore low-level hooks. This build uses only the `WH_KEYBOARD_LL` hook.
- **Not reproduced:** mouse/controller mappings (the original only has the enums
  for them) and the crash-reporting UI.
- **Reproduced:** single-instance handoff, via `tauri-plugin-single-instance`.
  The plugin is registered before anything else in the builder, so a second
  launch sends its arguments to the running instance and exits instead of
  installing a second `WH_KEYBOARD_LL` hook. The callback unminimises, shows and
  focuses the existing window — the same `reveal` helper the tray menu and tray
  click use, so a second launch works whether the window is minimised, behind
  other windows, or hidden to the tray.

## What is here

```
src-tauri/src/config.rs   settings model + the .socd reader/writer
src-tauri/src/picker.rs   the region picker: the window, and screen ↔ overlay pixels
src-tauri/src/autostart.rs the sign-in entry, in HKCU\...\CurrentVersion\Run
src-tauri/src/socd.rs     the four modes as a pure state machine (14 tests)
src-tauri/src/hook.rs     WH_KEYBOARD_LL, foreground program, process list
src-tauri/src/input.rs    SendInput wrappers, scan codes, monitor rects, the 1 ms timer
src-tauri/src/clicker.rs  the auto clicker engine, scheduled on a grid
src-tauri/src/macros.rs   the block tree, its validation, and the interpreter
src-tauri/src/typer.rs    the typer: its plan, its pace, and the focus guard
src-tauri/src/replay.rs   the replayer: capture from the hooks, and the replay drive
src-tauri/src/runtime.rs  one run at a time, and the global shortcuts
src-tauri/src/mouse_hotkeys.rs  mouse-button shortcuts, watched from the mouse hook
src-tauri/src/keys.rs     scan-code table with labels and extended flags
src-tauri/src/lib.rs      Tauri commands, tray, persistence, single-instance
src/app/                  the UI: five tabs, dialogs, the block builder
src/picker/               the picker's own page and styles (transparent, no app chrome)
src/theme/                UI-Colors palette engine (from the Interior skill)
src/components/interior/  the Interior.dev components, copied unchanged
```

The frontend is two Vite entries — `index.html` for the app and `overlay.html`
for the region picker — because they are two windows with nothing in common but
the styling tokens. The picker is created from Rust on demand rather than
declared in `tauri.conf.json`, so it does not exist until something asks for a
rectangle.

The engines are deliberately separate from Windows. `socd::on_key(scan, down,
repeat)` returns a decision; `clicker` and `macros` each talk to a trait
(`Machine`, `Runner`) that the real build implements with `SendInput` and the
tests implement with a recording fake and a clock they control. That is why the
four SOCD modes, the click grid and the macro interpreter can all be tested
(`cargo test`), and why the UI can run in a browser with an in-memory bridge
(`npm run dev`).

## Run it

```bash
npm install
npm run tauri dev            # desktop window
npm run tauri build          # MSI + NSIS installers
```

`src-tauri/rust-toolchain.toml` pins the MSVC toolchain: a `windows-gnu` host
fails here with "error calling dlltool" long before Tauri is involved.

Tests and checks:

```bash
cd src-tauri && cargo test   # 141 tests: SOCD modes, click grid, failsafes, macros, typer pacing,
                             #             settings format, overlay coordinate maths, sign-in
                             #             entry, mouse shortcuts
npx tsc --noEmit             # frontend types
```

## Updates

Crossup checks `https://github.com/emrisca/crossup/releases/latest/download/latest.json`
twenty seconds after launch and every six hours while it is running. When a newer
version is waiting, a row appears under the header with that version and the
first line of its notes. **Update now** downloads, verifies the signature and
installs it; **Later** hides that version until the next launch. Nothing installs
without the click, and a failed check is silent, because a machine with no
network is not a machine with a problem.

Settings has the same check on demand: it shows the installed version, a
**Check for updates** button, the reply (newer release, up to date, or what went
wrong), and the first lines of the release notes. A found update installs from
there too. The check itself still runs in Rust; the window only reads the state
and asks for another.

The keypair already exists: `.secrets/crossup-updater.key` is the private half,
which Git ignores, and its public half is in `plugins.updater.pubkey` in
`src-tauri/tauri.conf.json`. That key has no password, so a local release build
needs only the path:

```bash
TAURI_SIGNING_PRIVATE_KEY=.secrets/crossup-updater.key
npm run tauri build
```

Paste the contents of `.secrets/crossup-updater.key` into the repository secret
`TAURI_SIGNING_PRIVATE_KEY`. A key generated without a password needs no
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; set that secret only if the key is ever
replaced with one that has a password. Pushing a tag such as `v0.1.1` runs
`.github/workflows/release.yml` on a Windows runner: it installs, type checks,
builds the NSIS installer, writes `latest.json` beside it and publishes both. A
clicker or macro that is running stops when the installer takes over, which is
what the restart would have done anyway.

The installed app reads `latest.json` without a token, so the repository has to
be public. A private one turns every update check into a 404.

A version bump is three files and one tag: `package.json`,
`src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` all say the same number,
and the tag is `v` plus that number. `tauri-action` interpolates `v__VERSION__`
from the config, so a tag that drifts publishes a release no installed copy will
ever be offered.

## Using it

### Remapper

- **Profiles** — right rail. `Global` applies everywhere; add a program and its
  mappings only run while that executable is in front.
- **Mappings** — one row per key. `Opposite`, `No repress`, `Neutral` and
  `Remap` are explained in the legend under the table.
- **Presets** — WASD, arrow keys, or both, added as opposite pairs.
- **Tray** — closing hides to tray when enabled. Right-click for the menu (open,
  stop a run, quit); left-click brings the window back. The two are separate, so
  a right click opens the menu instead of raising the window.
- **Tools menu** — the header's Tools button holds one switch per tool, and it
  stays open while switches are flipped. Turning the remapper off lets every key
  through untouched and the mapping table says so in red. Shortcuts follow the
  switch, and switching a tool off stops any run it owns.
- **One instance** — launching Crossup again (shortcut, Start menu, taskbar)
  raises and focuses the window that is already running and exits, so two hooks
  can never fight over the same keys.
- **Backup and restore** — settings.json holds every setting: profiles, macros,
  the clicker, the typer, the recordings, the switches and the sign-in
  executable. A copy of it rebuilds Crossup on another machine. settings.socd is
  the original format and carries profiles and mappings only.

### Replayer

- **Recording.** Mouse movement, clicks, wheel and key presses, each stamped
  with when it happened. The recorder is fed by the hooks the remapper already
  uses, so it needs no second hook and cannot record its own replays: injected
  input is filtered out at the hook.
- **Two shortcuts, each one key.** Record and replay are separate toggles.
  Press to begin, press again to stop.
- **The shortcut is not recorded.** While recording, the keys and buttons that
  make up either shortcut are dropped instead of written down. A recording that
  ends on the chord therefore replays as it was recorded, rather than replaying
  its own stop.
- **The window hides.** Starting a recording hides Crossup, so the program
  underneath can be reached and the window is not part of the capture. The
  record shortcut brings it back, with the recording waiting to be named. A
  record shortcut is required before recording, because without one there would
  be no way back.
- **Save, keep, delete.** A finished recording is kept only once it has a name,
  and recordings live in settings.json beside the other settings.
- **Once or forever.** A replay runs a set number of passes or until the replay
  shortcut stops it, and claims the same one-run slot as every other tool.

### Typer

- **Text to type.** The draft itself, kept in settings.json, so a half-written
  paragraph is still there after a restart.
- **Pace.** Words a minute, read back as keys a second and the gap between them.
  Five characters to a word, the count every words-a-minute figure uses.
- **Pace fluctuation.** A percentage either side of the pace that each word may
  aim for, so a paragraph speeds up and eases off the way a person does. Zero
  types like a metronome; a deadline overrides it and keeps the exact pace.
- **Typo rate.** The share of characters mistyped and corrected: a neighbouring
  key, a pause while it is noticed, a backspace, then the right one.
- **Thinking pauses.** Stops between words, each up to the longest set. They
  land after a sentence's full stop, and inside a sentence that runs on, after a
  random fifteen to thirty words counted from the last pause. The estimate
  counts both.
- **Finish by.** A deadline paces the whole run so the last keystroke lands on
  time. Anything faster than 25 keys a second is clamped, and the tab says so.
- **Countdown.** Start is pressed in this window, so the run waits before its
  first keystroke. Click into the target window meanwhile.
- **The focus guard.** The window in front when Start is pressed is the one that
  gets the text. If anything else comes forward, the run stops where it stands
  rather than typing the rest into the wrong place.
- **Pause, resume, stop.** And a start/stop shortcut, both read between
  keystrokes, so a stop lands in about a frame.

### Auto clicker

- **What it presses** — a mouse button, or a key with the shift case held.
- **Rate** — a rate per second, minute, hour or day. The header line reads it
  back as clicks a second, so a rate set per day is still legible.
- **Duty cycle** — how much of each interval the button stays down; at 0 it is a
  plain click, which is the fast path.
- **Random rate** — a percentage spread either side of the interval.
- **Double click** — two clicks a cycle, a Windows double click apart.
- **Limits** — stop after N clicks, or after N seconds. Zero means run forever.
- **Click points** — “Pick a spot…” darkens the desktop and takes the next
  click as a point, wherever it lands, on any monitor. Points are served in
  turn, each its own number of clicks.
- **Edges** — stop the run when the cursor reaches a screen edge. Each side has
  its own distance and its own switch, because the bottom of a screen is usually
  under a taskbar and a side the run works along may want leaving alone.
- **Corners** — the same, for the four corners, each with its own distance.
- **Zones** — rectangles you place on the desktop, each of which stops the run,
  holds it, or carries on through a held one as the cursor enters it. “Draw a
  zone” puts a transparent sheet over every monitor: drag a rectangle on the
  screen itself and it becomes the zone, click once for a square, `Esc` or a
  right click to back out. **Draw on screen** on a zone replaces it in place,
  keeping its action, and the X/Y/W/H boxes are still there for fine tuning.
  Zones overlap, so: a **stop** wins wherever it sits in the list, a **carry on**
  beats a **pause** around it — that is how a small clear area inside a large
  held one works — and otherwise the first pause found holds.
- **Escape ×3** — three quick taps of Escape stop the run from anywhere, and it
  is always on: the key is read from its physical state, so it works while the
  cursor has run off, the hook is switched off, or a game is swallowing keys.
- **Failsafes are measured per monitor.** On a two-screen desktop the seam
  between the screens is an edge of both, not a hole in the middle of the
  desktop, so a cursor thrown sideways is caught there too. Corners are reported
  in preference to the two edges they sit in.
- **Program list** — a whitelist or blacklist of programs. A target list
  *pauses* rather than stops, because the run is meant to resume.
- A pause, from a zone or a target list, does not bank the clicks it skipped:
  the run picks its own grid up again rather than paying the time back as a
  burst.
- **Shortcut** — optional, and empty on a fresh install: the Start button runs
  it and Escape ×3 stops it, so nothing takes a global key unasked. Set one and
  it toggles or holds. Click the field and
  press the combination rather than typing it: modifiers are shown as they are
  held, `Esc` backs out, `Backspace` clears it, and a combination the shortcut
  layer cannot take is reported on the tab instead of failing silently.
- **Mouse button shortcuts.** A side button is the one key you can press without
  letting go of the mouse, so any of the five can be the shortcut: press one
  while the field is recording and it is stored as `Mouse4` (back) or `Mouse5`
  (forward), with modifiers if you hold them (`Control+Mouse4`). The press that
  starts the recording is not the one recorded, which is what makes binding
  *left* click possible at all. A wheel notch is a shortcut too: roll the wheel
  while the field is recording and it is stored as `WheelUp` or `WheelDown`,
  with modifiers if you hold them (`Control+WheelDown`). A notch is one event
  and not something that can be held, so it suits a macro, which runs on the
  press; the clicker in *hold* mode says on the tab that a notch cannot be held
  rather than accepting a shortcut that would do nothing. Keyboard shortcuts go
  through the global shortcut layer; mouse ones cannot — that layer has no mouse
  button in its vocabulary — so they are watched by a low-level mouse hook
  instead, and both arrive at the same code path. A mouse shortcut is ignored while the cursor is
  over Crossup's own window, so clicking into the app never fires it, and
  neither does clicking through the zone picker. Synthetic buttons are ignored
  too: the clicker and the macros press mouse buttons, and a shortcut has to
  answer the person's hand, not the app's own output.
- The right-hand panel shows what the run is doing: clicks, elapsed, and the
  achieved rate against the one asked for, plus whether Windows granted the
  1 ms timer. With no timer the achieved rate drops at high settings, and that
  panel is how you tell that apart from a bug.

### Macros

- **Blocks** — key, type text, click, move, scroll, wait, repeat, and click and
  drag. Drag one from the palette into the tree, or click it to append; the
  palette's drag entry is the click block in its drag mode, so it lands wherever
  a block can — inside a `Repeat` as readily as at the end. The palette stays at
  the top of the builder while a long macro scrolls under it.
- **Pressing** — a key block and a click block each choose *tap*, *hold*, *down*
  or *up*: a tap repeats, a hold presses once and waits, and down and up leave
  the key or the button down across later blocks, which is what a chord is. Only
  a tap takes a count, so the count field appears only where it means something.
- **A key block takes a mouse button** — its cap listens to buttons as well as
  keys, so a right click can sit among the keys of a chord in the same block,
  with the same tap/hold/down/up. A button has no shifted case, so the Shift box
  is not offered for it.
- **Offsets** — a move or a drag by an offset is written either as X and Y, for
  a diagonal, or as a direction and a distance in pixels, which is how a nudge
  is actually thought about: left 20 px, right 20 px, back where you started.
  A tap never keeps the button down for less than 15 ms — one frame — because a
  press and a release that arrive in the same instant are a click to Windows but
  not always to a game that reads its input once a frame.
- **Dragging** — the click block's fifth mode, *drag*, is the whole gesture in
  one block: press where the cursor is, go to a point (or by an offset), rest
  there for the block's hold time, and let go. It is the block for moving a
  window, a slider or a selection without three blocks and a hand-written wait.
- **Dropping** — the third of a block's row the pointer is over decides whether
  it lands above, below or *inside* it. A `Repeat` takes blocks in its own
  indented area, both by drop and by the “Drag blocks in here” target.
- **Reordering** — drag a block by its grip, or use the ↑/↓ buttons, which are
  there so the whole builder works from the keyboard.
- **When it runs** — a global shortcut, recorded the same way the clicker's is
  and taking a mouse button or a wheel notch just as readily, optionally only
  while one program is in front, and an on/off switch. A disabled macro keeps
  its blocks.
- **Limits** — nesting is capped at six levels, repeat counts and hold times are
  bounded, and a run has an action budget, so a typo cannot describe a loop that
  runs for a century. Anything a `Down` block left held — a key or a mouse
  button — is released on the way out, including when the run is stopped: a key
  held down types on its own, and a button held down keeps clicking whatever the
  cursor is over.
- **One at a time** — starting the clicker or a macro while either is running is
  refused, by name. The tray's “Stop clicker and macros” ends whatever it is.

## Settings

Everything the window edits is written to `settings.json` under the app config
directory (`%APPDATA%\dev.crossup.app\`) as it is edited, so closing Crossup
and opening it again brings back the profiles, the mappings, the clicker, the
zones, the macros and their shortcuts.

It is read back a section at a time. A file outlives the code that wrote it: a
field changing shape should cost that one section, not every profile and macro
along with it — which is exactly what a whole-file parse falling back to the
defaults looks like from the outside, and exactly what "it forgot everything"
turns out to be. A section that could not be read is reported in the footer log
when debug logging is on, rather than disappearing quietly.

Settings also cover:

- **Launch at sign in** — writes the `Run` entry in `HKCU`, the same switch
  Task Manager's Startup tab shows. **Choose…** points it at a specific copy of
  `Crossup.exe`, because a portable build beside an installed one, or a copy
  that has been moved, makes "the executable that is running" the wrong answer
  in a way nobody notices until the next sign-in; **Use this copy** puts it back.
  The row says which executable Windows will actually run when it differs from
  the one selected.

## Caveats

This is a key-remapping tool: it hooks every keystroke, so antivirus software
may object, and some competitive games ban SOCD cleaning outright. Use it where
you are allowed to. Running as administrator may be required for games that read
input at a higher privilege level, exactly as the original's README says.

The auto clicker and the macros press keys and move the cursor too, so they
attract the same attention from anti-cheat software — and a macro is not a
script: there are no conditions, variables or triggers beyond a shortcut. That
is the trade for having a builder instead of a language.

Of Blur-AutoClicker's features, this carries the ones that decide whether a
clicker is usable: the rate grid, duty cycle, random rate, double clicks, click
points with travel and scatter, click and time limits, per-monitor edge and
corner failsafes, stop zones, and program targets.

Zones and click points are drawn the way Blur-AutoClicker draws them: a
transparent window over the whole virtual desktop, a drag inside it, and the
rectangle that comes back is the zone. The window is a normal interactive one
rather than a click-through hook, which is why the drag is exact — nothing is
sampled from the cursor, and nothing is left installed if the pick goes wrong.

What it does not carry: `stop_when_complete`, stopping on Alt+Tab, the small
random cursor offset it adds to every click, its portable mode, crash handler
and speed-test history.

## Credits

Hitboxer was written by [valignatev](https://github.com/valignatev) and is MIT
licensed.

Blur-AutoClicker was written by [Blur009](https://github.com/Blur009) and is
GPL-3.0. Nothing was copied from it: the auto clicker here is a reimplementation
of its observable behaviour, written against this project's own model, but the
feature set is its feature set and the credit is its.

AutoHotkey is by its [contributors](https://github.com/AutoHotkey/AutoHotkey)
(GPL-2.0); only the idea of a hotkey-triggered macro is borrowed, since none of
its script code or interpreter is here.

UI components are from [Interior.dev](https://github.com/ddoemonn/interior);
the palette model is [UI-Colors](https://github.com/whosajid/UI-Colors).

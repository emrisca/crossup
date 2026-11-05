# Macro blocks — implementation plan

Status: planned, not started. Nothing in this document is implemented yet.
Scope: twenty-two blocks that fill the gaps in the current palette, grouped into four
releases. The existing engine (`Step`, `Runner`, `walk`, `validate`) and builder
(`MacrosTab.tsx`, `bridge.ts`) patterns carry over unchanged.

Already shipped, for context: Key, Text, Click, Move, Scroll, Delay, Wait for picture,
Click picture, If, Repeat.

---

## 1. Foundations (build once, six blocks depend on them)

### 1.1 `src-tauri/src/window.rs` ✅

Window enumeration and activation, kept out of `hook.rs` (input) and `picker.rs`
(one overlay window). The module is `window`, not `windows`, so it does not shadow the
`windows` crate inside the files that import from it.

| Function | Purpose |
| --- | --- |
| `visible() -> Vec<Window { hwnd, title, exe, pid }>` | `EnumWindows` + `IsWindowVisible` + `GetWindowTextW` + `GetWindowThreadProcessId`, with `exe_of_pid` lifted out of `hook::foreground_program` so both answer from the same code. Front-to-back order, untitled windows dropped, Crossup's own windows filtered by pid. |
| `find(matches, timeout_ms, stop) -> Result<Window, String>` | Polls every 120 ms, the same beat as `wait_for_picture`, and honours a stop within one beat. Zero timeout looks once. Caps the wait at `MAX_WAIT_MS`. |
| `activate(window) -> Result<(), String>` | `IsIconic` → `ShowWindow(SW_RESTORE)` → `SetForegroundWindow` → on refusal `AttachThreadInput` against the foreground thread and the target, retry, detach both → verify with `GetForegroundWindow`. |
| `foreground_program()` | The exe in front; `hook::foreground_program` now delegates here. |

Serves: Focus window, Launch's "wait for its window", While-by-program, and the window
picker. The `AttachThreadInput` fallback and the verify step are the fiddly part; without
the verify, the block after a Focus types into whatever did come forward.

### 1.2 `src-tauri/src/spawn.rs` ✅

`CreateProcessW` rather than `ShellExecuteW`, returning the new PID so Launch can wait for
*that* program's window rather than any window.

| Item | Behaviour |
| --- | --- |
| `launch(path, args) -> Result<Started { pid }, String>` | Absolute path, existence checked before Windows is asked; working directory is the program's own folder; no handles inherited; both process handles closed immediately, so the pid is all that is kept. |
| `command_line(path, args)` | The program path quoted, arguments appended verbatim — no shell, no wildcards, no re-splitting. |
| Errors | Missing file → "there is no program at “…”"; relative path → "is not a full path"; `ERROR_ELEVATION_REQUIRED` (740) → "has to run as administrator; start it yourself"; anything else → the system message. |

`Win32_Security` had to be added to the `windows` features: `CreateProcessW` names
`SECURITY_ATTRIBUTES` in its signature, so the function is gated behind it even though
nothing passes one.

### 1.3 `src-tauri/src/clipboard.rs` ✅

`set_text(text)` via `OpenClipboard`/`EmptyClipboard`/`SetClipboardData`, with a
five-attempt, 50 ms retry when another program holds the clipboard, then an honest
failure ("another program is holding the clipboard").

Details worth keeping:

- One direction only. With no variables in the engine a clipboard *read* has nowhere to
  go, and holding the clipboard open across blocks is how Ctrl+V stops working everywhere.
- `CloseClipboard` runs on every path out, including failures — a clipboard left open by
  Crossup breaks copy and paste in every other program until it exits.
- The block is allocated with `GlobalAlloc(GMEM_MOVEABLE, …)`; once `SetClipboardData`
  succeeds the memory belongs to the clipboard and is not freed, and on the failure paths
  it is. `GlobalFree` lives in `Win32::Foundation`, not `Win32::System::Memory`.
- `CF_UNICODETEXT` is written as the literal `13` rather than pulling in the `Win32_System_Ole`
  feature for one constant that has never changed.
- Tests cover the UTF-16 encoding only (empty text stays a terminated string, one
  terminator, an astral character becomes a pair). Nothing in the test suite touches the
  real clipboard, because that would overwrite whatever the person had copied.

Features added: `Win32_System_DataExchange`, `Win32_System_Memory`.

### 1.4 Engine scaffolding (`macros.rs`) ✅ (the part the foundation can honour)

The trait methods that map onto modules 1.1–1.3 are in place, each returning
`Result<(), String>` so a failure becomes `Flow::Stopped(reason)` exactly like the picture
blocks, and each taking the run's stop flag so a long wait stays interruptible:

```rust
fn focus(&mut self, target: &WindowTarget, timeout_ms: u32, stop: &AtomicBool) -> Result<(), String>;
fn launch(&mut self, path: &str, args: &str, wait_for_window: bool, timeout_ms: u32, stop: &AtomicBool) -> Result<(), String>;
fn clipboard(&mut self, text: &str) -> Result<(), String>;
```

- `WindowTarget` (`Title { contains, exact }` or `Program { exe }`) lives in `macros.rs`
  with a pure `matches(title, exe)`, so the matching rules are tested without a desktop
  and the platform side stays a lookup. An empty target matches nothing, deliberately:
  otherwise a blank field would focus the first window on the desktop.
- `launch` waits only for *its own* pid. A launcher that starts the real game as a child
  will not be found this way — that is what the separate Focus block is for.
- `Rng` (xorshift64*, `Rng::new(seed).between(min, max)`) lives in run state, seeded once
  per run from the clock and a run counter. `play_seeded(…)` exists so tests can pin it;
  a zero seed is replaced rather than accepted, since zero is a fixed point.

Deferred on purpose, to arrive with the blocks that need them rather than as stubs:
`screenshot` (needs the name and folder policy), `log` (needs the session list),
`call_macro` / `replay` / `set_tool` (need the group C/D machinery — settings lookup, the
replay driver split from `start_replay`, and the tool switch extracted from
`set_tool_enabled`). `call_depth` / `visited` in run state arrive with `call_macro`.

Action accounting: real input blocks count as actions; Note, Log and Stop count zero, so
documentation can never trip `ACTION_BUDGET`.

### 1.5 The one-run slot shapes two blocks ✅ (the replay half)

`Runtime::claim` refuses anything while a macro holds the slot, so *Run another macro*
and *Run replay* must never re-claim. The replay half is now in place:

- `replay::drive(events, passes, speed, stop) -> Result<(), String>` is public and never
  claims; it returns `Ok(())` when the passes are done and `Err("Stopped")` when the run
  was stopped, so a caller can put the answer straight into a stop reason.
- `replay::play_inside(…)` is what the Replay block will call: it refuses an empty
  recording, shows the replay badge while it plays, drives inline, and touches neither the
  claim nor the release — the macro owns the slot.
- `start` (the Replayer tab and its shortcut) now goes through the same `play_inside`, so
  there is one playback path rather than two that could drift apart.
- A test claims the slot as "macro A", plays inside it, and asserts the runtime still says
  "macro A" afterwards — the guard against a nested replay being refused by its own caller.

Still owed by the group C/D blocks (there is nowhere to put it yet):

- `Step::CallMacro` recursion rules — call-depth cap of 8, a visited-id stack that refuses
  A→B→A by name, all inside the engine's run state so the guard cannot be bypassed by a
  runner. `macros::play` can already be called with the caller's stop flag; what is missing
  is the block that fetches the other macro's steps.
- The same test for the macro half: a nested macro must not re-claim either.

---

## 2. Group A — leaf blocks, no new platform code ✅

Shipped: all ten kinds in the engine (`Step::Note`, `Log`, `RandomDelay`, `Chord`,
`ScrollAt`, `Until`, `RepeatFor`, `Stop`, `Fail`, `AssertImage`), their validation and
summaries, `Report.notes` for the log lines, `Runner::local_minutes` for the time gate,
and the grouped palette in the builder. Twenty-one new tests. Two details worth
remembering: `Until` is a **not-before gate** (a time already past today does not wait),
and a chord releases in reverse order even when the run is stopped mid-hold.

| Block | Shape | Rules |
| --- | --- | --- |
| **Note** | `Note { text }` | 0 actions; ≤ 2 000 chars; rendered dimmer than an action |
| **Random wait** | `RandomDelay { min_ms, max_ms }` | max ≥ min, both ≤ 600 000; uses the run RNG so tests are deterministic |
| **Key chord** | `Chord { scans: Vec<u16>, hold_ms }` | ≤ 4 keys, all non-zero; down in order, hold, up in reverse; releases through `Presses` so a stop mid-hold cannot leave a key down |
| **Scroll at a point** | `ScrollAt { x, y, amount }` | amount ≠ 0; moves then scrolls; 2 actions |
| **Wait until a time** | `Until { at: "HH:MM" }` | parsed at validation; sleeps in 200 ms slices so Stop still works and a clock change cannot hang it; waits for the next occurrence of that time |
| **Repeat for N seconds** | `RepeatFor { seconds, steps }` | ≤ 3 600; checks stop between passes |
| **Stop macro** | `Stop { reason }` | → `Flow::Stopped(reason or "Stopped by the macro")`, ≤ 200 chars |
| **Fail with a message** | `Fail { message }` | → `Flow::Stopped("Failed: …")`, same cap |
| **Assert picture** | `AssertImage { image, similarity }` | one look, no waiting; absent = stop with "the picture is not on screen" |
| **Log a line** | `Log { text }` | 0 actions; appended to a per-run log shown in the session panel |

---

## 3. Group B — one small platform module each ✅

Shipped: `Focus`, `Launch`, `Clipboard` and `Screenshot` as blocks, built on the foundation
modules from §1 (`window::find`/`activate`, `spawn::launch`, `clipboard::set_text`), plus a
new `screenshots` module. `pictures::capture` was split so `capture_png` is shared by pictures
and screenshots. New commands `running_windows` and `choose_program`; the builder offers the
open windows as suggestions and takes a path from the file dialog. Two details: `Launch`
checks that the program still exists at save time, and a screenshot area is picked with the
same overlay a picture is, under its own `macro-shot` purpose.

| Block | Shape | Notes |
| --- | --- | --- |
| **Focus window** | `Focus { target: FocusBy, timeout_ms }` — `FocusBy` is either a title match `Title { contains, exact }` or a program match `Program { exe }` | Case-insensitive title match, or exe match over `visible_windows()`; verify with `GetForegroundWindow` after activating. Elevated targets and UWP apps (`ApplicationFrameHost.exe`) get honest refusals rather than wrong windows. Default timeout 5 s. |
| **Launch program** | `Launch { path, args, wait_for_window, timeout_ms }` | Absolute path, existence checked at save time; args are a raw string with no shell expansion and the field says so; working directory defaults to the exe's folder; default timeout 15 s. |
| **Clipboard** | `Clipboard { text }` | Set only — with no variables, reading it has nowhere to go; pasting is a chord. |
| **Save screenshot** | `Screenshot { x, y, width, height, name }` | Reuses the `pictures` grab+encode path (extracted into a shared `capture_bytes`); writes `screenshots\{name}-{stamp}.png` under the app data folder; region chosen with the existing overlay. |

New commands: `running_windows()` (titles, for the Focus datalist) and `choose_program`
(exe picker via the dialog plugin, mirroring `choose_exe`, answering as a
`macro-program` event).

---

## 4. Group C — composition ✅

Shipped: `CallMacro { id, name }` and `Replay { id, name, passes, speed }`, with a `chain` of
running macro ids carried through the engine (`play_nested`, `State.chain`) so a cycle is
refused **by id at the block that closes it**, and a depth cap of 8 in the runner. A nested
run goes through `play_with`, the same body every entry point uses, so keys a called macro
leaves down are still released. Two additions worth knowing: a called macro's `Log` lines come
back to the caller's report, and a called macro's `Stop`/`Fail` ends the caller's run too —
anything else would make "run this macro" mean something different inside another one. The
replay block leaves `speed: 0` meaning "use the replayer's own", shows the badge through
`replay::play_inside`, and never claims the run slot. New command `block_choices` feeds both
pickers.

| Block | Shape | Notes |
| --- | --- | --- |
| **Run another macro** | `CallMacro { id, name }` | Name stored for display, id for lookup; runs under the caller's slot; depth ≤ 8; a cycle is refused by name |
| **Run replay** | `Replay { id, passes, speed }` | Uses the Replayer's saved speed unless overridden; drives inline; badge shown while it plays |

---

## 5. Group D — the one with a policy question ✅

Shipped: `Tool { tool, enabled }`, with the five switches as strings validated against the same
five the command accepts. The command and the block now share one `runtime::apply_tool`, so
"what a switch means" — saving, releasing held keys, stopping the tool's own run,
re-registering shortcuts — cannot drift between them. One refinement the plan did not spell
out: the block passes the running run's name as `exempt`, so a macro that switches *macros*
off says something about the next run rather than cutting itself short. And it only flips the
switch: switching the clicker on arms it for its shortcut instead of starting it.

**Toggle a tool** — `Tool { tool, enabled }`, where `tool` is one of `clicker`,
`remapper`, `macros`, `typer`, `replay`. Flips the switch that `set_tool_enabled` already
owns, and switching off stops that tool's run.

It deliberately does **not** *start* the clicker: starting one mid-macro means handing
over the run slot, and a macro that can silently surrender the desktop to a clicker is
worse than a macro that arms it for the next shortcut. That distinction belongs in the
README and in the block's hint text.

---

## 6. Flow blocks and runaway safety ✅

Shipped: `While { condition, steps, max_iterations, timeout_ms }`, with both caps required and
validated (1–10 000 passes, 1 s–1 hour; defaults 100 passes and 60 s). Reaching either cap is a
**loud stop** that says the check never went false, never a silent exit. The condition is the
If's, through one shared `holds()` helper, so the two blocks cannot drift apart; Stop is
checked before each pass even when the body is empty. The builder draws it as a container the
same way a Repeat is drawn — same drop target, same nesting cap — and the condition editor is
now one `ConditionFields` component used by both If and While instead of two copies of the
same markup.

**While / Repeat until** — `While { condition, steps, max_iterations, timeout_ms }`.

Both caps are **mandatory** (defaults 100 iterations / 60 s; hard caps 10 000 / 1 hour),
and hitting either one is a **loud stop**, never a silent exit. Conditions reuse the If
machinery unchanged: `Condition::Picture | Condition::Front`.

---

## 7. Builder and bridge ✅

Shipped as written, with two adjustments worth recording. The palette ended at five groups
(*Input · Time · Screen · Tools · Flow*) over **28 blocks**, with `BLOCK_GROUPS` in place of
`BLOCK_ORDER`. The fourth group is called *Tools* rather than *Desktop* because the `Tool`
block belongs there. And the per-block pattern gained a guard the plan did not ask for: the
`BlockFields` switch ends in `const unhandled: never = step`, so adding a kind to the `Step`
union without its fields case fails the typecheck instead of shipping a block with no
controls. The condition editor is now one `ConditionFields` component shared by If and While,
and the browser mock answers every new command (`running_windows`, `choose_program`,
`block_choices`).

The palette goes from 11 to 22 entries, which is too many for one wrapping row, so the
first change is a **grouped palette**: *Input · Time · Screen · Flow · Tools*, the same
`pal__item` buttons under small headings, with `BLOCK_ORDER` becoming a list of groups.

Per block, the existing pattern:

1. `Step` union member in `bridge.ts`, plus `Condition`-style nested types.
2. `BLOCK_TEMPLATES`, `BLOCK_LABELS`, `stepSummary` arms.
3. One `BlockFields` case in `MacrosTab.tsx`.
4. Mock handlers for the new commands in the browser mock.
5. Validation and `walk` arms in `macros.rs`, plus `summary()` and `check_step`.
6. A README line naming what the block does and does not do.

Only Focus, Launch and Screenshot need pickers; the rest are plain fields. All new
variants are leaves, so the `Path`/`withChildren` model for If and Repeat is untouched.

---

## 8. Tests ✅

Shipped: **225 tests** in `cargo test --lib --release`, all green. The round-trip guard now
covers every one of the 28 kinds (it was Group A only), and asserts the camelCase field names
(`timeoutMs`, `waitForWindow`, `maxIterations`, `passes`) as well as equality, because a renamed
field is a number that silently reverts to its default. `play_seeded` has its own test: the
same seed picks the same random wait twice. Per-block behaviour was covered as the plan asked
— delay ranges, chord order and release, the While cap stop reason, the assert-picture
failure, the call-depth and cycle refusals, replay passes. Platform code (activation, spawn,
clipboard, screenshot writing) is in `docs/MANUAL-CHECKLIST.md`, which is a run-through rather
than a list of wishes.

- Extend `every_kind_of_block_is_described_and_written_out` to cover every new kind — it
  is the guard against a block that saves but does not survive a settings round-trip.
- Per-block: validation boundaries, summary text, and behaviour against a scripted fake
  runner (delay ranges, chord release order, while-cap stop reason, assert-picture
  failure, call-depth guard, replay pass count).
- Determinism: `play_seeded(...)` so RandomDelay is testable.
- Platform code (window activation, spawn, clipboard, screenshot writing) cannot run
  headless and is covered by a manual checklist instead.

Verification per release, on this machine:

```
cd apps/crossup-tauri/src-tauri && cargo test --lib --release
cd apps/crossup-tauri && npm run typecheck && npm run build
```

(Debug-profile `cargo test` fails to load on this machine with
`0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND`; that is pre-existing, so `--release` is used.)

---

## 9. Sequencing and releases

| Release | Contents | Estimate |
| --- | --- | --- |
| **0.2.15** | Foundation scaffolding, Group A (10 blocks), grouped palette | ~1.5 days |
| **0.2.16** | Focus, Launch, the `running_windows` / `choose_program` commands (on top of 1.1's `window.rs`) | ~2 days |
| **0.2.17** | While/Repeat until, Assert picture, Run another macro (caps + cycle guard) | ~2 days |
| **0.2.18** | Clipboard, Save screenshot, Run replay, Toggle a tool | ~2 days |

Each release is independently useful: Group A alone makes macros readable and
debuggable, Group B makes them self-starting, Group C makes them composable.

---

## 10. Risks

- **Windows foreground lock** — activation can be refused; the fallback ladder plus an
  honest failure message is the mitigation, and the verify step is non-negotiable.
- **UWP apps** — their windows belong to `ApplicationFrameHost.exe`, so program matching
  misleads; title matching is the answer and the README says so.
- **Run-slot ownership** — the two composition blocks must never call `claim`; a review
  item with a test.
- **Runaway loops** — every loop block carries two caps and stops loudly.
- **Clipboard contention** — retry, then fail with "another program is holding the
  clipboard".
- **Elevation** — a non-elevated Crossup cannot focus or inspect elevated windows; the
  block reports that rather than targeting the wrong window.
- **Palette overload** — solved by grouping before the entry count doubles.

---

## 11. Definition of done, per block

1. Validates with a message a person can act on.
2. Survives a settings round-trip and defaults every optional field.
3. Appears in the palette under its group.
4. Has a one-line summary in the builder.
5. Has at least one test for its failure path.
6. Is described in the README in one sentence naming what it does **not** do.

---

## 12. Still avoided by design

Variables and expressions, shell execution, registry access, network calls. The Clipboard,
Launch and Screenshot blocks are the outer edge: they take a literal, not an expression,
and nothing they introduce can be read back into a later block.

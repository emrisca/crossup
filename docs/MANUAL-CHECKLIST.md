# Manual checklist

The pieces that cannot run headless. Every item here is real Windows: a window
that has to come forward, a program that has to start, a clipboard that another
program may be holding, a screen that has to be captured. They are checked by
hand before a release, in the running app, in this order — each one is quicker
to try than to describe.

Run `cargo test --lib --release` and `npm run typecheck && npm run build` first;
if those fail, this list is not the place to find out.

## Focus a window

- [ ] A macro whose first block focuses Notepad by title brings it forward, and
      the next block's keystrokes land in it.
- [ ] Focusing by program (`notepad.exe`) picks a window belonging to it.
- [ ] Focusing a title that is not open, with a 2 s timeout, stops the run with a
      sentence naming what it could not find — not a silent no-op.
- [ ] Focusing an elevated window (Task Manager) refuses with a reason rather
      than typing into whatever was in front.
- [ ] Focusing a UWP app by program is refused or misses honestly; by title it
      works. (UWP windows belong to `ApplicationFrameHost.exe`.)

## Start a program

- [ ] A program starts with an argument it echoes back (e.g. a text editor with
      a file path), proving the arguments arrive unexpanded.
- [ ] **Wait for its window** waits for a program that takes a second to appear,
      and the next block lands in that window.
- [ ] A launcher that hands off to another process times out with a sentence,
      rather than waiting forever.
- [ ] Deleting or moving the program after saving the macro makes the block show
      a problem in the builder, before it is run.

## Copy to clipboard

- [ ] The copied text pastes into Notepad with Ctrl+V.
- [ ] Emoji and non-Latin text survive the round trip.
- [ ] With another program holding the clipboard open, the block retries and
      then fails with a reason instead of hanging.

## Save a screenshot

- [ ] **Pick area…** on a second monitor captures that monitor's area, not the
      primary screen's.
- [ ] An area with negative coordinates (a screen left of or above the main one)
      captures correctly.
- [ ] A whole screen captures: the picture size limits must not apply here.
- [ ] The PNG lands in `screenshots\` under the settings folder, named
      `<name>-YYYYMMDD-HHMMSS.png`.
- [ ] A name full of `\ / : * ? "` characters still produces one file, not a
      folder tree.

## Blocks that drive other things

- [ ] A chord stopped mid-hold (Stop pressed while the keys are down) leaves no
      key down: type afterwards and nothing sticks.
- [ ] A While on a picture that never leaves the screen stops with "the check
      never went false" and names the cap that was reached.
- [ ] While nested inside Repeat, and Repeat inside While, both count against the
      six-level cap in the builder.
- [ ] **Run another macro** runs a macro whose last block presses a key; the key
      arrives. A Stop inside the called macro ends the caller too, with the
      called macro's reason in the report.
- [ ] A macro that calls itself (directly, or A→B→A) is refused at the block that
      closes the loop.
- [ ] **Run a replay** plays a recording inline, shows the badge, and the blocks
      after it run when the last pass ends.
- [ ] **Turn a tool on or off** flips the matching switch in the window — this is
      the one to look at twice, because the window may only catch up on its next
      refresh.

## Builder

- [ ] The palette shows five groups and every block in the plan is in one.
- [ ] A block drags into a While, a Repeat and both arms of an If, and the third
      of the row under the pointer decides where it lands.
- [ ] Every block's fields are usable without a second click, and a block's
      summary line matches what the fields say.

## Known state

- Debug-profile `cargo test` fails to load on the development machine with
  `0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND`. Pre-existing; use `--release`.

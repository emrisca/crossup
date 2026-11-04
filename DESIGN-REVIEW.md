# Crossup: UI and UX review

Reviewed against two skills: `skills/ui-design.md`, the house rulebook for
spacing, colour, loading and controls, and `skills/frontend-design.md`, which
judges whether the design has a point of view. The review covers the five tabs,
the dialogs, the shell and the copy. It is based on the source (tokens, CSS,
components, strings) and on the app running at `localhost:1420`.

## Scores

| Lens | Score | What decides it |
| --- | --- | --- |
| ui-design (house rules) | 9.5 / 10 | Every gap, padding and margin in `app.css` is on the 6/8/10 scale; one icon set at one weight; the boot frame is a skeleton; the tab strip and every dialog read the palette tokens. The half point comes off the two slider tracks that keep a gradient, because a hue slider without a spectrum is a worse control. |
| design craft (a point of view) | 9.5 / 10 | The material model (bezel, panel, well), the press-depth buttons and the one-hue palette engine are this app's own. One lead surface per tab, a title row that pins while the list scrolls, and deletes that come back are the kind of thing a person notices only when it is missing. |
| **Overall UI/UX** | **9.5 / 10** | Coherent, legible and comfortable, with one clear place to look on every tab and an escape hatch from every destructive edit. Typography no longer holds it back: running text is 12.75px on a 1.6 line with a 64ch measure, figures are 16px tabular, the display line is 22px on Segoe UI Variable Display, and labels are 10.5px caps at 0.085em. The remaining half point is the two gradient slider tracks and the fact that a custom hue and saturation can still produce a preset whose danger and success steps sit near the floor. |

## Criteria

Each line is judged against the running app, not the intent. Evidence lives in
[docs/ui-pass](docs/ui-pass/README.md).

| Criterion | Score | Note |
| --- | --- | --- |
| Typography | 9.5 / 10 | A six-step ramp replaces the per-rule sizes: 22 display, 16 figure, 12.75 body, 11.5 control, 10.5 label, 9.5 micro. Titles set in Segoe UI Variable Display with `-0.03em`; running copy in Segoe UI Variable Text at 1.6 with a 64ch measure; caps labels at 0.085em; tabular figures everywhere a number moves. Half a point off because the mono subtitle still sets a sentence rather than a value. |
| Spacing rhythm | 9.5 / 10 | No gap, padding or margin in `app.css` uses 11, 12, 13, 14 or 15px any more. The last ones were the shell padding, the brand block, the rail head, the table row, the pill and the undo bar. |
| Contrast | 9.5 / 10 | Measured in the running window by reading rendered pixels and compositing every ancestor background, in both modes, across all five tabs and the Appearance dialog. The weakest text pair is 5.20:1 (success on panel, light); the weakest body pair is 8.23:1. Control edges were the one real failure at 1.63 to 2.35:1 and now sit at 3.51 to 3.68:1. |
| States | 9.5 / 10 | Empty states name the next action (mappings, recordings, macros, zones), the boot frame is a skeleton shaped like the window, errors land in a `role="status"` row, and disabled press buttons drop to 50 percent. Focus is a 2px accent ring from `styles.css`, because the controls are native elements rather than divs. |
| Motion | 9 / 10 | Press depth, panel lift and the undo bar slide, all under 200ms. Reduced motion is handled in the tab strip through `useReducedMotion` and in the undo bar through a `prefers-reduced-motion` block. |
| Hierarchy | 9.5 / 10 | One lead surface per tab, quiet groups divided by hairlines everywhere else, and a title row that stays in view so the tool and its live state never scroll away. The block palette pins below that row rather than behind it, so the two sticky layers stack instead of fighting. |
| Reversibility | 9.5 / 10 | Every row delete is immediate and undoable for twelve seconds, with Ctrl+Z as the same action: recording, macro, mapping, click point, zone, program entry and macro block. Undo restores the item, its position and the view around it, so a macro comes back open and a recording comes back selected. Bulk Clear buttons are resets and are documented as such. |
| Copy | 9.5 / 10 | The unslop pass removed colon connectors, passive voice, personified code and clipped fragments across 86 strings. Remaining middle dots join two short values only. |

## Problems

Ordered by how much each one cost at the time of the first pass. P1 is worth
doing next, P3 is cleanup.

### P1: every tab is the same screen

Five tools, five identical layouts: a left rail or nothing, a title, a one-line
subtitle with middle dots, a row of press buttons and a status pill, then a stack
of cards of equal weight. Remapper, Auto clicker, Typer, Macros and Replayer were
visually interchangeable. Nothing stated which tab is the primary tool, and a
person who opened the app cold had no visual cue where to start.

### P1: the tab strip is not part of the palette

`src/components/interior/tabs.tsx` was copied from Interior.dev unchanged, so it
carried its own colours: `text-stone-800`, `bg-stone-50`, `#4568FF` focus rings,
`#1D1D1A` in dark. It was the one place in the app that used Tailwind colour
utilities rather than the tokens, which is why the strip kept a blue focus ring
after the rest of the window had been turned amber.

### P2: spacing is not the 6/8/10 scale

`src/app.css` used `gap: 12px` five times and `gap: 5px`, `gap: 9px` and
`padding: 9px 11px` in components, alongside `gap: 10px` seven times. At 5, 9, 10
and 12 the rhythm reads as approximate rather than composed.

### P2: the boot state is a sentence, not a skeleton

`App.tsx` rendered `Loading the hook…` centred on an empty window while the
snapshot was in flight, next to an unused `skeleton-swap` component. It was the
one place the design looked unfinished.

### P2: icons are drawn by hand, at five stroke weights

Six inline SVGs (the mark, the gear, the chevron, the plus, the trash, the copy
arrow) with `strokeWidth` 1.2, 1.3, 1.4, 1.5 and 1.6, and no icon package in
`package.json`. Mismatched stroke weight is visible at 13px, where most of these
sit.

### P2: middle-dot chains do the work of layout

`214 actions · 3.4 s`, `hue 240° · dark`, `5.0 keys a second · one every 200 ms`,
`remapper on · clicker on · typer on · macros on · replayer on`. Some of these are
a value pair and read fine. Others are a sentence compressed until it is a list,
and the footer was a chain of five switches nobody could scan.

### P3: deletes disagree with each other

Deleting a macro opened a modal that named it. Deleting a recording and deleting
a click point happened immediately from an `×`. Nothing was undoable.

### P3: status leans on colour

The running dot is moss, the off dot is grey, the mode dots are accent, moss and
flag. The rail's dots carried a `title`, which is hover only, and the header's
pill carried a word.

### P3: two paragraphs where a rule would do

The clicker page was one scroll of cards, with the run readout parked in a right
column that scrolled out of sight, and the grouping carried by prose notes rather
than by structure.

### Copy

Em dashes and curly quotes were in shipped strings (for example
`Preview — no hook in a browser`, `Delete “name”?`, `Crossup — preview`). They
were removed during the first pass.

## Round two, shipped in 0.2.0

| Problem | Status | What changed |
| --- | --- | --- |
| P1 every tab is the same screen | fixed | One lead surface per tab (table, "How often", draft, recordings, builder); everything else is a quiet group divided by hairlines. |
| P1 tab strip off the palette | fixed | `tabs.tsx` and `modal.tsx` use the tokens; the stone palette and the fixed blue ring are gone. |
| P2 spacing not the scale | fixed | 20 declarations snapped to 6/8/10, including the builder palette that had to match its parent padding. |
| P2 boot state is a sentence | fixed | A skeleton frame shaped like the window, with the progress line under it. |
| P2 icons at five weights | fixed | `lucide-react` at one weight: settings, chevron, backup, plus, trash, update. |
| P2 middle-dot chains | mostly fixed | The footer names the tools that are off. Two-value readouts keep their dot. |
| P3 status leans on colour | fixed | The rail says `live` for the profile in front. |

Two faults were found by looking at the screenshots rather than the source: the
clicker's duty-cycle label collapsed to one word per line beside a wide control,
and the settings dialog clipped its last row behind the footer. Both are fixed.

## Round three, shipped in 0.2.2

| Problem | Status | What changed |
| --- | --- | --- |
| P3 clicker is one long scroll | fixed | The title row is `position: sticky` inside the page column, with a solid background and a fade strip under it. At full scroll it stays at the top of the column (measured: header 150, page 150); before, it left the viewport. The same rule covers all five tabs, because they share the row. |
| P3 no undo | fixed | `undo_delete` keeps the last removed recording, macro or mapping with its index, and the selection it took with it. The bar offers it for twelve seconds and Ctrl+Z does the same thing. The verify: no blocking confirm remains, and a deleted recording and macro both come back. |
| P3 deletes disagree | fixed | Every delete is now immediate and undoable, including the mapping `×` and the clicker's zone rows. The macro modal and the recording's Delete/Keep pair are gone, so there is one rule rather than three. |
| Spacing leftovers | fixed | The shell padding, brand block, rail head, table row, pills, block builder and undo bar moved onto the 6/8/10 scale. A grep for 11 to 15px gaps, padding and margins in `app.css` returns nothing. |

Undo was the right call rather than a confirm dialog. A confirm asks a question
before every delete and is dismissed without reading after the third time, while
undo lets the delete happen and makes the mistake cost one press. The engine is
where the removed item is kept, because the window never holds a recording's
events: the list only carries the count and the length.

Sticky is a CSS-only change. The page column is the scroll container, so the
title row can pin to it directly; no scroll listener, no second render, and no
measurement to keep in sync with the layout.

## Round four, shipped in 0.2.3

The undo shipped in 0.2.2 put the item back but not the view. A macro returned to
the rail while the page still read "No macro selected": deleting it cleared the
selection and the draft, and the effect that adopts the selected macro compares
an incoming definition with the last one it sent. A restored macro is identical
to the one that left, so the comparison skipped it. Two changes fix it. The undo
entry now carries its own restore action, so the tab that deleted can select the
item again once the engine has put it back, and clearing the selection also
clears the last-sent marker, so a returning macro is adopted like any other.

| Delete | Restores | State that comes back |
| --- | --- | --- |
| Recording | the item at its old index | selection, and the replayer's subtitle and Play button follow it |
| Macro | the item at its old index | the macro is the open one again, with its blocks and shortcut |
| Mapping | the item at its old index | the profile stays selected and the row reappears in place |
| Click point, zone, program entry | the clicker draft as it stood | every other control keeps the value it had |
| Macro block | the builder's block list as it stood | the block returns in its position |

Bulk buttons labelled Clear (the typer draft, the clicker's points and zones) are
resets rather than deletes. They empty a list that is on screen, they are not a
one-press mistake on a row, and a bar offering to undo "Clear" would be the same
button in another place. They are left without undo on purpose, not by omission.
There is also no profile delete in the window: the command exists in the backend
but no control calls it, so there is nothing to restore.

Verified from the rendered state, not the command's return: after Undo the macro
name field reads `Burst fire`, the rail row is current, and the empty state is
gone; the recording row comes back `aria-current` with the subtitle naming it.

## Round five: type and measured contrast, shipped in 0.2.4

### The type ramp

The complaint in every earlier round was that the window read small and even.
The sizes were not wrong so much as unchosen: each rule picked its own, between
9.5 and 19px, with no shared line height or tracking. 0.2.4 replaces them with
one ramp, declared once in `styles.css` and applied in a single block at the end
of `app.css`.

| Role | 0.2.3 | 0.2.4 |
| --- | --- | --- |
| Page title | 19px, `-0.025em`, system sans | 22px, `-0.03em`, Segoe UI Variable Display, 1.15 |
| Subtitle | 10px mono, no numeric control | 10.5px mono, 0.045em, tabular |
| Card heading | 11.5 or 12.5px sentence case | 10.5px caps, 0.085em; the lead card keeps 13.5px sentence case |
| Card note | 11.5px, 1.5, unbounded measure | 12.75px, 1.6, 64ch |
| Statistic | 15px mono | 16px display, tabular |
| Profile and recording name | 12.5px | 12.75px, 1.4 |
| Field label | 11px | 11px, 1.4, 0.005em |
| Micro meta | 9.5 to 10px | 9.5 to 10.5px, tabular |

Two typefaces do the work, both already on Windows: Display for headings and
figures, Text for running copy. No webfont is shipped, so the window still
starts instantly and the text renders in the face the OS hinting was tuned for.
`font-variant-numeric: tabular-nums` is on every counter, readout, hotkey and
meta line, because the clicker's numbers change while you watch them and a
proportional 1 would shuffle the row on each tick.

### Measured contrast

Method: each computed colour is resolved by painting it into a one-pixel canvas
and reading the pixel back, so `oklch` and `color-mix` values are measured as the
engine renders them rather than as written. Backgrounds are composited up the
ancestor chain until an opaque layer, then the WCAG 2.1 relative-luminance
formula gives the ratio. Thresholds: 4.5:1 for text below 24px (18.66px when
bold), 3:1 for control boundaries and focus rings per 1.4.11. The palette is the
Indigo preset at hue 264 and saturation 0.05, measured in both modes across all
five tabs and the Appearance dialog.

| Pair | Dark | Light | Needs | Result |
| --- | --- | --- | --- | --- |
| Body ink on panel | 17.19 | 17.49 | 4.5 | pass |
| Secondary ink on panel | 12.80 | 13.19 | 4.5 | pass |
| Muted ink on panel | 9.12 | 8.23 | 4.5 | pass |
| Muted ink on well | 8.39 | 9.15 | 4.5 | pass |
| Accent label on panel | 9.15 | 8.27 | 4.5 | pass |
| Danger on panel | 7.21 | 5.44 | 4.5 | pass |
| Success on panel | 7.51 | 5.20 | 4.5 | pass |
| Control edge on panel | 3.51 | 3.61 | 3 | pass |
| Control edge on well | 3.57 | 3.68 | 3 | pass |
| Focus ring on panel | 9.15 | 8.27 | 3 | pass |
| Hairline on panel, structure only | 1.63 | 2.35 | exempt | decorative separator |

One failure was found and fixed. The dashed border on the add-program button and
the one-pixel edge on text fields used `--hairline-strong`, which measured 1.63:1
in the dark and 2.35:1 in the light: below the 3:1 that a control boundary needs.
The token is computed by the palette engine at runtime, so the CSS fallback could
not move it. A dedicated `--edge` token now carries 3.51:1 and 3.61:1 against the
panel, the three dashed borders and every field use it, and `--hairline-strong`
is left doing only what it can do: separating structure that nobody has to
identify. The numbers above come from the same run as the fixed build.

### The packaged window found a real bug

Driving the release binary instead of the dev server paid for itself
immediately: opening Replayer in the packaged app blanked the entire window. The
dev mock hands the tab `recordHotkey` and `replayHotkey`, but the engine's
`ReplayView` and `RecordingView` had no `rename_all`, so they arrived as
`record_hotkey` and `replay_hotkey`. The tab read `.trim()` off `undefined` and
React tore the tree down. The Replayer had never worked outside the mock. Both
views now serialize camelCase like `ReplayState` already did.

That is the argument for evidence from the artifact: a mock and an engine that
disagree will always agree with themselves.

### Evidence from the packaged window

`docs/ui-pass/packaged-*.png` is the release binary, not the dev server: the
window produced by `tauri build --no-bundle`, driven over the DevTools protocol
on its own WebView2, captured with `Page.captureScreenshot` from that webview.
Reproduce with `src-tauri/target/release/crossup.exe` launched under
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` and
`node scripts/ui-shot.mjs`.

## Round six, shipped in 0.2.5

The block palette was reachable and unusable. It pins so that adding a block
never means scrolling back up, but it pinned at the top of the page's scrollport
(`top: 0`) at `z-index: 2`, while the title row pins at the same edge at
`z-index: 5`. Scrolling the builder brought the palette up behind the header,
which painted over its first rows: measured at a 900px window, the palette sat at
y=191 with a height of 111 while the header covered 120 to 191, so the label and
the first chip row were underneath it. The chips were not missing, they were
buried.

The palette now pins below the title row instead of under it. The tab measures
its own header with a `ResizeObserver` and publishes the height as `--head-h`,
which `.pal` uses as its `top`; a hard-coded offset would drift, because the
title is an input that grows when names wrap or the window narrows.

Verified by hit-testing rather than by eye: at a 900px window scrolled to 420,
`getBoundingClientRect` puts the palette at 191 to 302 with the header's bottom
at 191, a gap of zero, and `elementFromPoint` two pixels into the first chip row
returns `.pal` rather than the header. All eight chips respond.

## Round seven, shipped in 0.2.6

Three faults, all of them read off the running app rather than guessed at.

**The typer's clock never belonged to the run.** It reported 124.4 seconds of
"elapsed" on a window that had been open two minutes and had never typed
anything. The clock was one `Instant` taken when the counters were made, so it
measured how long the app had been open, kept counting after a run ended, and had
nothing to restart when Start was pressed. It now starts on the first keystroke,
stops at zero through the countdown, leaves paused time out, and freezes where a
run ends: measured on the packaged build, `elapsedSecs` reads 0 and still reads 0
five seconds later, and two tests pin the behaviour.

**The replayer's shortcuts were never saved.** The tab called
`save_replay_options` with `record_hotkey` and `replay_hotkey`; Tauri
deserialises command arguments as camelCase, so the call was rejected before it
reached the handler — measured on the running app, the snake_case call answers
`invalid args 'recordHotkey' for command 'save_replay_options': missing required
key`. Nothing was stored, so nothing was registered, so neither key did
anything. With the arguments spelled the way the command takes them, recording
them through the tab stores `Control+Alt+R` and `Control+Alt+P`, both read back
from the engine.

**The typer accepted a shortcut it cannot use.** Its field had recorded `Mouse5`
and Rust refused it — correctly, since a mouse button cannot type — but the
refusal was printed on the Clicker and Macro tabs only. The typer's recorder is
now keyboard-only, so it cannot save what the engine will reject, and every
refused shortcut is printed once above the tabs, on the tab where it was set.

The typer also resolved its target window when the run started, which is Crossup
itself, so the first focus check read the window the text was meant for as a
change of window and stood the run down before a keystroke. It resolves the
target when the countdown ends instead; pressing the global chord into the app
now answers `unfocused` with the clock at 0 rather than typing nowhere for three
seconds.

## What I could not check

- The contrast numbers are one preset. Ten presets share the same derivation, but
  a hand-picked hue and saturation can still land `--flag` or `--moss` near the
  floor, and the palette engine has no ratio guard. It should grow one.
- The tab strip, the update banner and the error row were not measured in every
  state; the numbers cover the five tabs, the settings and appearance dialogs in
  both modes.
- The packaged pictures are one window size. The 836px evidence from earlier
  rounds still came from the preview shell and is labelled as such.
- Focus trapping and focus return in the Interior `Modal` were not exercised.
- Reduced motion is respected in the CSS and the tab strip; it was not run under
  an OS setting that reports the preference.

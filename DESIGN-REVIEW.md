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
| **Overall UI/UX** | **9.5 / 10** | Coherent, legible and comfortable, with one clear place to look on every tab and an escape hatch from every destructive edit. Typography is what holds the last half point: the app reads at 10 to 13px on the system stack. |

## Criteria

Each line is judged against the running app, not the intent. Evidence lives in
[docs/ui-pass](docs/ui-pass/README.md).

| Criterion | Score | Note |
| --- | --- | --- |
| Typography | 9 / 10 | Four sizes carry the window (21 title, 13 body, 12 controls, 10 to 11.5 labels and readouts), with `-0.03em` tracking on the title and a mono face for numbers. The gap is the system stack: text at 10px is legible but generic, and a display face would be a larger change than this pass. |
| Spacing rhythm | 9.5 / 10 | No gap, padding or margin in `app.css` uses 11, 12, 13, 14 or 15px any more. The last ones were the shell padding, the brand block, the rail head, the table row, the pill and the undo bar. |
| Contrast | 9 / 10 | The ink ladder separates title, body and note by a step each, and both modes use the same tokens. The weakest pair is `--ink-3` at 10px on `--panel`; it clears the small-text ratio but has the least room. Measured from the tokens rather than with a contrast tool. |
| States | 9.5 / 10 | Empty states name the next action (mappings, recordings, macros, zones), the boot frame is a skeleton shaped like the window, errors land in a `role="status"` row, and disabled press buttons drop to 50 percent. Focus is a 2px accent ring from `styles.css`, because the controls are native elements rather than divs. |
| Motion | 9 / 10 | Press depth, panel lift and the undo bar slide, all under 200ms. Reduced motion is handled in the tab strip through `useReducedMotion` and in the undo bar through a `prefers-reduced-motion` block. |
| Hierarchy | 9.5 / 10 | One lead surface per tab, quiet groups divided by hairlines everywhere else, and a title row that stays in view so the tool and its live state never scroll away. |
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

## What I could not check

- Contrast was judged from the token values, not measured with a contrast tool.
  The third ink step at 10px is the tightest pair in the ladder.
- Screenshots come from the same React build served at `localhost:1420`, not from
  the packaged installer. The installer is verified to build, carry its signature
  and appear in `latest.json`, but the window in the pictures is the preview
  shell, which is why the header reads "Preview".
- Focus trapping and focus return in the Interior `Modal` were not exercised.
- Reduced motion is respected in the CSS and the tab strip; it was not run under
  an OS setting that reports the preference.

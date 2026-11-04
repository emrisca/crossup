# Crossup: UI and UX review

Reviewed against two skills: `skills/ui-design.md`, which is the house rulebook for
spacing, colour, loading and components, and `skills/frontend-design.md`, which
judges whether the design has a point of view of its own. The review covers the
five tabs, the dialogs, the shell and the copy. It is based on the source (tokens,
CSS, components, strings) and on the app running at `localhost:1420`.

## Scores

| Lens | Score | What decides it |
| --- | --- | --- |
| ui-design (house rules) | 9 / 10 | The scale is enforced (6/8/10 across `app.css`), one icon set at one weight, a skeleton boot frame, and every surface including the tab strip and the modal reads the palette tokens. The remaining mark off is the two slider tracks that keep a gradient because a hue slider without a spectrum is a worse control. |
| design craft (a point of view) | 9 / 10 | The material model (bezel, panel, well), the press-depth buttons and the one-hue palette engine are genuinely this app's. The lead-surface rule now gives each tab one dominant element, so the five tools no longer read as the same screen five times. |
| **Overall UI/UX** | **9 / 10** | Coherent, legible and comfortable to use, with one clear place to look on every tab. What keeps it from a ten is scope, not craft: typography is still the system stack, and two of the review's softer notes (a sticky section header in the clicker, undo instead of a confirm) are not in yet. |

What earns the most credit:

- The surface model is legible: recessed wells for input, lifted panels for
  content, a visible frame. Nothing floats without a reason.
- The palette is derived, not picked. One hue and one chroma produce fourteen
  named colours in OKLCH, and light and dark come from the same tokens, so the
  appearance dialog is a real preview of the app rather than a swatch sheet.
- Physicality is consistent: buttons press, key caps look like caps, the switch
  is a track, controls have a shadow that reads as depth rather than decoration.
- Copy is specific. "Closing the window keeps the hook running in the tray" and
  "A tap never presses for less than 15 ms, one frame, because a game can miss a
  press and a release that arrive together" both say something a person needs.

## Problems

Ordered by how much each one costs. P1 is worth doing next, P3 is cleanup.

### P1: every tab is the same screen

Five tools, five identical layouts: a left rail or nothing, a title, a one-line
subtitle with middle dots, a row of press buttons and a status pill, then a stack
of cards of equal weight. Remapper, Auto clicker, Typer, Macros and Replayer are
visually interchangeable. Nothing states which tab is the primary tool, and a
person who opens the app cold has no visual cue where to start.

Fix: pick one dominant element per tab and let it differ. The Replayer's is the
recordings list; the Typer's is the draft; the clicker's is the run readout. Give
the dominant element a different surface weight (a full-width well, or the only
lifted card on the page) instead of giving every card the same radius and shadow.

### P1: the tab strip is not part of the palette

`src/components/interior/tabs.tsx` is copied from Interior.dev unchanged, so it
carries its own colours: `text-stone-800`, `bg-stone-50`, `#4568FF` focus rings,
`#1D1D1A` in dark. It is the one place in the app that uses Tailwind colour
utilities rather than the tokens, which is why the tab strip keeps a blue focus
ring when the rest of the window has been turned amber. Moving the hue slider to
300 and tabbing through the strip is the visible proof.

Fix: replace the stone and hex values in that component with `var(--ink)`,
`var(--panel)`, `var(--bezel)` and `var(--accent)`. It is a mechanical change and
it makes the strip obey the one control the app is proudest of.

### P2: spacing is not the 6/8/10 scale

`src/app.css` uses `gap: 12px` five times and `gap: 5px`, `gap: 9px` and
`padding: "9px 11px"` in the components, alongside `gap: 10px` seven times. The
scale is stated in the house rules for a reason: at 5, 9, 10 and 12 the rhythm
reads as approximate rather than composed, and the eye cannot tell a group from a
gap between groups.

Fix: collapse 5 to 6, 9 to 8 or 10, 12 to 10, and use a separator or a surface
change where a section break is wanted. This is a find-and-replace across one
stylesheet plus the inline styles that repeat the numbers.

### P2: the boot state is a sentence, not a skeleton

`App.tsx` renders `Loading the hook…` centred on an empty window while
`snapshot` is in flight. The app already ships `components/interior/skeleton-swap`,
so the tool is there and unused. A sentence in the middle of a blank window is
the one place the design looks unfinished.

Fix: draw the shell (header, rail, tab strip) immediately and put skeleton blocks
where the mapping table and the cards will be. Shape them like the content, no
shimmer.

### P2: icons are drawn by hand, at five stroke weights

Six inline SVGs (the mark, the gear, the chevron, the plus, the trash, the copy
arrow) with `strokeWidth` 1.2, 1.3, 1.4, 1.5 and 1.6. There is no icon package in
`package.json`. Small as it is, mismatched stroke weight is visible at 13px, which
is where most of these sit.

Fix: one set at one weight, sized 16px in dense UI and 20px in navigation. Lucide
is the default; if the Interior components must keep their own glyphs, at least
normalise the app's to one weight and one corner radius.

### P2: middle-dot chains do the work of layout

`214 actions · 3.4 s`, `hue 240° · dark`, `5.0 keys a second · one every 200 ms`,
`pass 1 of ∞`, `remapper on · clicker on · typer on · macros on · replayer on`.
Some of these are a value pair and read fine. Others are a sentence that has been
compressed until it is a list, and the footer is a chain of five switches that no
one can scan.

Fix: keep the dots for two short values. For anything longer, use two aligned
columns or a full sentence, and move the footer's tool states behind a single
"3 tools off" summary, which the header already computes.

### P3: deletes disagree with each other

Deleting a macro opens a modal that names it. Deleting a recording and deleting a
click point happen immediately from an `×`. Nothing is undoable.

Fix: one rule, applied everywhere. Either everything destructive asks, or nothing
does and the app offers an undo. The recorder's list is the cheapest place to add
the second option, since a recording is a single array entry.

### P3: status leans on colour

The running dot is moss, the off dot is grey, the mode dots are accent, moss and
flag. The rail's dots carry a `title`, which is hover only, and the header's pill
carries a word. A person who cannot separate moss from grey has the pill and
nothing else.

Fix: put the word next to the dot where the dot is the only signal, as the header
already does. Two words (`live`, `idle`) at 10px cost nothing.

### P3: two paragraphs where a rule would do

The clicker page is one scroll of cards, roughly 1,050 lines of controls, with the
run readout parked in a right column that scrolls out of sight. Settings, the
builder and the clicker all use prose notes to explain themselves, and the notes
are good, but they are also carrying the structure: a reader who skips the grey
text loses the grouping.

Fix: group the clicker into three named sections (what it presses, when it stops,
where it goes) with a sticky sub-header, and keep the notes as notes.

### Copy

Em dashes and curly quotes were in shipped strings (for example
`Preview — no hook in a browser`, `Delete “name”?`, `Crossup — preview`). They were
removed during this review: about thirty strings across eleven files now use a
comma, a colon, a full stop or straight quotes. Middle-dot chains were left in place, since they
are a deliberate part of the readouts and fixing them is a layout change rather
than a copy change.

## Round two, shipped in 0.2.0

Every P1 and P2 above is closed, and the P3s except two. Before and after
pictures for each screen are in [docs/ui-pass](docs/ui-pass/README.md).

| Problem | Status | What changed |
| --- | --- | --- |
| P1 every tab is the same screen | fixed | One lead surface per tab (table, "How often", draft, recordings, builder); everything else is a quiet group divided by hairlines. |
| P1 tab strip off the palette | fixed | `tabs.tsx` and `modal.tsx` use the tokens; the stone palette and the fixed blue focus ring are gone. |
| P2 spacing not the scale | fixed | 20 declarations snapped to 6/8/10, including the sticky builder palette that had to match its parent padding. |
| P2 boot state is a sentence | fixed | A skeleton frame shaped like the window, no shimmer, with the progress line under it. |
| P2 icons at five weights | fixed | `lucide-react` at one weight: settings, chevron, backup, plus, trash, update. |
| P2 middle-dot chains | mostly fixed | The footer names the tools that are off. Two-value readouts keep their dot. |
| P3 deletes disagree | fixed | A recording delete asks in place (`Delete` / `Keep`); the redundant second delete button is gone. |
| P3 status leans on colour | fixed | The rail says `live` for the profile in front. |
| P3 clicker is one long scroll | open | Cards are quieter and the lead card anchors the page, but there is no sticky section header yet. |
| P3 no undo | open | The app asks first instead. An undo history is a larger change than this pass. |

Two faults were found by looking at the screenshots rather than the source: the
clicker's duty-cycle label collapsed to one word per line beside a wide control,
and the settings dialog clipped its last row behind the footer. Both are fixed,
and both are visible in the pairs.

## What I could not check

- The capture of the running window failed on the last attempt (`Current display
  surface not available for capture`), so the visual judgements above rest on the
  source and on earlier interactive checks of the tabs, not a fresh screenshot.
- Contrast was judged by eye against the token values, not measured. The ink
  ladder (`#131313` / `#454545` / `#6e6e6e` on white) clears body text at the
  first two steps and is borderline for the third at 10px.
- Focus trapping and focus return in the Interior `Modal` were not exercised.
- Reduced motion is respected in the tab strip (`useReducedMotion`) and was not
  checked in the press and panel animations.

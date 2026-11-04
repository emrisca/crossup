# UI pass, 0.1.0 to 0.2.0

Screenshots and notes for the design pass that followed
[DESIGN-REVIEW.md](../../DESIGN-REVIEW.md). Every picture is the running app at
`localhost:1420` in its default preset (hue 264, dark), captured with the
window at 836 x 1216 so the pairs line up.

To reproduce: `npm run dev`, open the tab in the pair, and compare.

## Remapper

| Before | After |
| --- | --- |
| ![Remapper before](before-remapper.png) | ![Remapper after](after-remapper.png) |

The table and its empty state are still the screen. What changed is underneath:
spacing snapped to the 6/8/10 scale, the footer stopped being five switch states
in a row, the profile rail reads `live` instead of a mapping count for the
program in front, and the brand mark is a flat accent square rather than a
gradient.

## Auto clicker

| Before | After |
| --- | --- |
| ![Auto clicker before](before-clicker.png) | ![Auto clicker after](after-clicker.png) |

Three changes, one of them a real fault. The "Duty cycle" row used to collapse
its label to one word per line, because a long note and a wide control were
fighting for the same 300px; rows now wrap the control under the label instead.
"How often" became the tab's one raised surface. The side readouts kept their
own surfaces, because unboxed numbers floated next to the page.

## Typer

| After | |
| --- | --- |
| ![Typer after](after-typer.png) | |

The draft is the lifted panel on this tab, which is where the pointer and the
eye should land first. The rest of the tab is a quiet group.

## Replayer

| Before | After |
| --- | --- |
| ![Replayer before](before-replayer.png) | ![Replayer after](after-replayer.png) |

The recordings list is the surface now, and the redundant "Delete the selected
recording" button is gone. A delete asks where it happens: the `×` becomes
`Delete` and `Keep`, so the destructive step has one rule across the app.

## Settings

| Before | After |
| --- | --- |
| ![Settings before](before-settings.png) | ![Settings after](after-settings.png) |

The dialog was clipping its last row behind the footer; it is 520 wide and 86vh
tall now, so every row, including the update row, fits. The dialog also stopped
borrowing Interior.dev's stone palette and reads the app's own tokens, which is
why it no longer keeps a blue-teal cast when the hue slider moves.

## What changed, in one list

- One lead surface per tab: the mappings table, "How often", the typer draft, the
  recordings list, the macro builder. Everything else is a quiet group.
- Spacing: every `gap`, `padding` and `margin` in `app.css` snapped to 6, 8 or
  10px (20 declarations, plus the builder palette that stuck to the top).
- Palette: the tab strip and the modal read `--panel`, `--sub`, `--well`,
  `--ink`, `--hairline` and `--accent` instead of `stone-*` and `#4568FF`.
- Icons: `lucide-react` at one stroke weight, replacing six hand-drawn SVGs.
- Boot: a skeleton frame shaped like the window, instead of a sentence in the
  middle of an empty page.
- Footer: names the tools that are off ("typer off") instead of five on/off
  pairs.
- Deletes ask once, in place.
- The brand mark is flat `--accent`; preset swatches are solid colour with a
  second tone ring rather than a 135 degree gradient.

## Left alone, on purpose

- The hue and chroma slider tracks keep their gradients. They are the control's
  data, not decoration: a hue slider without a spectrum is a worse control.
- Middle-dot readouts stay where they join exactly two short values
  (`214 actions · 3.4 s`). The chains that carried whole sentences are gone with
  the footer.
- Typography is still the system stack. Swapping in a display face is a larger
  change than this pass, and the app is read at 10 to 13px, where the system
  stack is the more legible choice.

# UI pass, 0.1.0 to 0.2.2

Screenshots and notes for the design passes that followed
[DESIGN-REVIEW.md](../../DESIGN-REVIEW.md). Every picture is the running app at
`localhost:1420` in its default preset (hue 264, dark), captured at 836 x 1216
unless noted, so the pairs line up.

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
recording" button is gone.

## Settings

| Before | After |
| --- | --- |
| ![Settings before](before-settings.png) | ![Settings after](after-settings.png) |

The dialog was clipping its last row behind the footer; it is 520 wide and 86vh
tall now, so every row, including the update row, fits. The dialog also stopped
borrowing Interior.dev's stone palette and reads the app's own tokens, which is
why it no longer keeps a blue-teal cast when the hue slider moves.

## Round three: the title row pins, deletes come back

### Sticky title row

![Auto clicker scrolled with the title row pinned](after-clicker-scrolled.png)

The clicker at full scroll (the page is zoomed to 125 percent so the column
overflows at this window size). The column has scrolled 421 of 421 pixels and the
title row is still at the top of it. The measurement taken at capture time:
`headTop 150`, `pageTop 150`, `scrollTop 421` of `max 421`. Unstuck, the header
would have been 271px above the viewport.

The row is `position: sticky` on `.hb__head`, with a solid `--panel` background
and a 12px fade strip under it via `::after`. The first attempt used a gradient
behind the header instead, and the row sliding underneath showed through the
subtitle, which is visible in the discarded take: the fix was to make the header
itself opaque and move the fade below it. All five tabs use the same row, so the
tool name and its state pill stay in view everywhere.

### Undo instead of a confirm

| Recording | Macro |
| --- | --- |
| ![Undo bar after deleting a recording](undo-recording.png) | ![Undo bar after deleting a macro](undo-macro.png) |

The delete happens immediately, and the bar offers it back for twelve seconds.
Ctrl+Z does the same thing. Both screenshots show the item gone (the list is
empty, "No macro selected") with the offer on screen.

Verified after pressing Undo, not just before: the recording list went back to
one row, the subtitle went back to `Open the menu and type a line · 214 actions ·
3.4 s`, and the bar cleared. The macro path was checked the same way: the macro
list came back with the macro in it.

The engine keeps one undo slot, holding the removed item, the index it sat at and
the selection that went with it, so a recording returns to its place and stays
selected. The window never holds a recording's events, which is why the restore
lives in Rust rather than in the tab.

## Round four: undo restores the view, not just the row

| Macro after Undo | Recording after Undo |
| --- | --- |
| ![Macro returned and selected](undo-macro-restored.png) | ![Recording returned and selected](undo-recording-restored.png) |

Measured from the rendered state after pressing Undo: the macro name field reads
`Burst fire`, the rail row is current, and the empty state is gone. The recording
list holds one row, that row is `aria-current`, and the subtitle reads
`Open the menu and type a line · 214 actions · 3.4 s`.

The fault was in the tab, not the engine. The delete cleared the selection, and
the effect that adopts the selected macro skips a definition equal to the last
one it sent. A restored macro is exactly that, so the page stayed on the empty
state with the macro sitting in the rail. Clearing the last-sent marker when the
selection empties lets the restored macro be adopted.

Clicker row deletes now go through the same bar and put the whole draft back, so
a removed point, zone or program entry returns with the rest of the settings
untouched. Bulk Clear buttons are resets and carry no undo; the review says why.

The sticky header was re-measured on this build (`position: sticky`, header and
page both at 150 at full scroll), so the round-three evidence still stands.

## Criteria, scored

| Criterion | Score | Evidence |
| --- | --- | --- |
| Typography | 9 / 10 | Four sizes carry the window (21, 13, 12, 10 to 11.5), tracking `-0.03em` on titles, mono for numbers. The system stack is the ceiling. |
| Spacing rhythm | 9.5 / 10 | `grep` for 11 to 15px gaps, paddings and margins in `app.css` returns nothing; the shell, brand, rail head, table row, pills, builder and undo bar all moved on scale this pass. |
| Contrast | 9 / 10 | Three ink steps, both modes from the same tokens. `--ink-3` at 10px is the tightest pair; judged from token values, not a contrast tool. |
| States | 9.5 / 10 | Empty states name the next action, the boot frame is a skeleton, errors use `role="status"`, disabled buttons fade to 50 percent, focus is the 2px accent ring from `styles.css`. Delete now has a recovery state rather than a question. |
| Motion | 9 / 10 | Press depth, panel lift, a 160ms undo slide. Reduced motion is handled in the tab strip and in the undo bar's media query. |
| Hierarchy | 9.5 / 10 | One lead surface per tab, hairline groups elsewhere, and a title row that stays put while the controls move. |
| Reversibility | 9.5 / 10 | Every row delete is undoable: recording, macro, mapping, click point, zone, program entry and macro block. Undo puts the item back and restores the view around it, including the open macro and the selected recording. |
| Copy | 9.5 / 10 | The unslop pass rewrote 86 strings; remaining middle dots join two short values only. |

Overall 9.5 / 10, with typography as the reason it is not a ten.

## What changed, in one list

- One lead surface per tab: the mappings table, "How often", the typer draft, the
  recordings list, the macro builder. Everything else is a quiet group.
- Spacing: every `gap`, `padding` and `margin` in `app.css` on the 6/8/10 scale.
- Palette: the tab strip, the modal and the dialogs read the app's tokens
  instead of `stone-*` and `#4568FF`.
- Icons: `lucide-react` at one stroke weight, replacing six hand-drawn SVGs.
- Boot: a skeleton frame shaped like the window.
- Footer: names the tools that are off instead of five on/off pairs.
- Deletes: immediate, with one undo slot and Ctrl+Z.
- The title row pins to the top of the page column and gains a fade under it.
- The brand mark is flat `--accent`; preset swatches are solid colour with a
  second tone ring rather than a 135 degree gradient.

## Left alone, on purpose

- The hue and chroma slider tracks keep their gradients. They are the control's
  data, not decoration: a hue slider without a spectrum is a worse control.
- Middle-dot readouts stay where they join exactly two short values
  (`214 actions · 3.4 s`).
- Typography is still the system stack. The app is read at 10 to 13px, where the
  system stack is the more legible choice, and a display face is a larger change
  than these passes.

//! Macros: the AutoHotkey idea — a hotkey that runs a sequence of input — with
//! a block builder in front of it instead of a scripting language.
//!
//! A macro is a list of [`Step`]s, and a step may be a [`Step::Repeat`] holding
//! its own list, which is what lets the UI show a nestable block tree. The
//! interpreter walks it and talks to a [`Runner`], so the whole engine is
//! testable against a recording runner with no keyboard attached.
//!
//! Two guard rails matter more than they look. Nesting is depth-capped and every
//! repeat count is clamped, so a typo cannot describe a loop that runs for a
//! century; and the interpreter tracks the keys it pressed with [`PressMode::Down`]
//! and releases them on the way out, so stopping a macro halfway through a chord
//! does not leave a key stuck down.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::input::MouseButton;

/// Deepest a `Repeat` may be nested. Enforced when editing as well as running.
pub const MAX_DEPTH: usize = 6;
/// Highest repeat count a single block may carry.
pub const MAX_TIMES: u32 = 9_999;
/// Actions one run may take, counting each key press, character and click. This
/// is a runaway guard, not a limit anyone should meet: a five-minute macro of
/// typed text is a few thousand.
pub const ACTION_BUDGET: u64 = 2_000_000;

/// What a press block does with what it presses: tap it, hold it, leave it
/// down, or release one that is already down.
///
/// The key block and the click block both offer tap/hold/down/up, so they share
/// the type — and the JSON — rather than each keeping its own copy of the idea.
/// `Drag` is the click block's own fifth: [`validate`] refuses it on a key,
/// which has nowhere to go.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum PressMode {
    /// Press and release straight away.
    #[default]
    Tap,
    /// Hold the key down for `hold_ms`, then release.
    Hold,
    /// Press and leave it down, for chords that span later blocks.
    Down,
    /// Release a key that an earlier block pressed.
    Up,
    /// Press, drag the cursor somewhere and let go: the whole click-and-drag in
    /// one block. Only the click block offers it.
    Drag,
}

/// One block.
///
/// `rename_all_fields` matters here and is not decoration: it renames the fields
/// of the variants, which plain `rename_all` does not. The window is camelCase
/// throughout, so without it a hold time it sends as `holdMs` is a field this
/// never reads — the number silently falls back to its default and the box the
/// person typed it into looks broken.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Step {
    /// Press a key.
    Key {
        scan: u16,
        #[serde(default)]
        mode: PressMode,
        /// How long a `Hold` holds, in milliseconds.
        #[serde(default = "default_hold")]
        hold_ms: u32,
        /// Hold Shift with the key.
        #[serde(default)]
        uppercase: bool,
    },
    /// Type text, one character at a time.
    Text {
        text: String,
        #[serde(default)]
        per_char_ms: u32,
    },
    /// Move the cursor.
    Move {
        x: i32,
        y: i32,
        /// Move by this much rather than to this point.
        #[serde(default)]
        relative: bool,
        /// Travel time; 0 teleports.
        #[serde(default)]
        duration_ms: u32,
    },
    /// Click the mouse.
    Click {
        #[serde(default)]
        button: MouseButton,
        /// Tap it, hold it down for a moment, leave it down for a later block,
        /// or let go of one an earlier block pressed.
        #[serde(default)]
        mode: PressMode,
        /// How many taps a `Tap` makes. Only a tap repeats.
        #[serde(default = "default_count")]
        count: u32,
        /// The button's down time: how long each tap stays down, how long a
        /// single `Hold` holds, or how long a `Drag` rests where it landed
        /// before letting go.
        #[serde(default)]
        hold_ms: u32,
        /// Where a `Drag` drags to. Unused by the other modes.
        #[serde(default)]
        x: i32,
        #[serde(default)]
        y: i32,
        /// A `Drag` moves by this much rather than to this point. Relative by
        /// default, so a fresh drag block is a press and a release where the
        /// cursor already is, not a leap to the corner of the screen.
        #[serde(default = "default_relative")]
        relative: bool,
        /// How long a `Drag` takes to travel there; 0 jumps, as in a `Move`.
        #[serde(default = "default_drag_ms")]
        duration_ms: u32,
    },
    /// Scroll the wheel; negative scrolls down.
    Scroll { amount: i32 },
    /// Wait.
    Delay { ms: u32 },
    /// Run the blocks inside this one again, `times` times over.
    Repeat {
        #[serde(default = "default_times")]
        times: u32,
        #[serde(default)]
        steps: Vec<Step>,
    },
}

fn default_hold() -> u32 {
    60
}

fn default_relative() -> bool {
    true
}

fn default_drag_ms() -> u32 {
    200
}

fn default_count() -> u32 {
    1
}

fn default_times() -> u32 {
    2
}

impl Step {
    /// One short line naming this block, used by the log and by tests.
    pub fn summary(&self) -> String {
        match self {
            Step::Key { scan, mode, hold_ms, uppercase } => match mode {
                PressMode::Tap => format!("{}{}", if *uppercase { "Shift+" } else { "" }, crate::keys::label(*scan)),
                PressMode::Hold => format!(
                    "{}{} held {hold_ms} ms",
                    if *uppercase { "Shift+" } else { "" },
                    crate::keys::label(*scan)
                ),
                PressMode::Down => format!("{} down", crate::keys::label(*scan)),
                PressMode::Up => format!("{} up", crate::keys::label(*scan)),
                // A key cannot be dragged; validate refuses it. Typing it is
                // the harmless reading if one arrives here anyway.
                PressMode::Drag => format!(
                    "{}{}",
                    if *uppercase { "Shift+" } else { "" },
                    crate::keys::label(*scan)
                ),
            },
            Step::Text { text, .. } => format!("type “{}”", text.chars().take(24).collect::<String>()),
            Step::Move { x, y, relative, duration_ms } => {
                if *relative {
                    format!("move by {x},{y}")
                } else if *duration_ms > 0 {
                    format!("glide to {x},{y} over {duration_ms} ms")
                } else {
                    format!("jump to {x},{y}")
                }
            }
            Step::Click { button, mode, count, hold_ms, x, y, relative, duration_ms } => {
                let button = button.label().to_lowercase();
                match mode {
                    PressMode::Tap if *hold_ms > 0 => {
                        format!("{button} ×{count}, held {hold_ms} ms")
                    }
                    PressMode::Tap => format!("{button} ×{count}"),
                    PressMode::Hold => format!("{button} held {hold_ms} ms"),
                    PressMode::Down => format!("{button} down"),
                    PressMode::Up => format!("{button} up"),
                    PressMode::Drag => {
                        let travel = if *relative {
                            format!("by {x},{y}")
                        } else {
                            format!("to {x},{y} over {duration_ms} ms")
                        };
                        match hold_ms {
                            0 => format!("drag {button} {travel}"),
                            rest => format!("drag {button} {travel}, resting {rest} ms"),
                        }
                    }
                }
            }
            Step::Scroll { amount } => format!("scroll {amount}"),
            Step::Delay { ms } => format!("wait {ms} ms"),
            Step::Repeat { times, steps } => format!("repeat {times} × {} blocks", steps.len()),
        }
    }

}

/// A macro: a trigger, an optional target program, and the blocks to run.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Macro {
    pub id: String,
    pub name: String,
    /// Global shortcut that runs it, e.g. `Control+Alt+1`. Empty means no
    /// shortcut, so it can only be run from the window.
    #[serde(default)]
    pub hotkey: String,
    /// Run only while this program is in front. Empty means anywhere.
    #[serde(default)]
    pub only_when: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub steps: Vec<Step>,
}

/// Why a macro cannot be saved or run.
pub fn validate(macro_definition: &Macro) -> Result<(), String> {
    if macro_definition.name.trim().is_empty() {
        return Err("give the macro a name".to_string());
    }
    for step in &macro_definition.steps {
        check_step(step, 0)?;
    }
    Ok(())
}

fn check_step(step: &Step, depth: usize) -> Result<(), String> {
    let where_ = step.summary();
    match step {
        Step::Repeat { times, steps } => {
            if depth + 1 > MAX_DEPTH {
                return Err(format!(
                    "“{where_}” is nested deeper than {MAX_DEPTH} levels"
                ));
            }
            if *times == 0 {
                return Err(format!("“{where_}” needs a repeat count above zero"));
            }
            if *times > MAX_TIMES {
                return Err(format!("“{where_}” repeats more than {MAX_TIMES} times"));
            }
            for inner in steps {
                check_step(inner, depth + 1)?;
            }
        }
        Step::Key { scan, mode, hold_ms, .. } => {
            if *mode == PressMode::Drag {
                return Err(format!(
                    "“{where_}” drags, and a key has nowhere to go — drag with a click block"
                ));
            }
            if *scan == 0 {
                return Err(format!("“{where_}” has no key chosen"));
            }
            if *hold_ms > 60_000 {
                return Err(format!("“{where_}” holds for longer than a minute"));
            }
        }
        Step::Text { text, per_char_ms } => {
            if text.is_empty() {
                return Err("a text block with nothing in it does nothing".to_string());
            }
            if *per_char_ms > 5_000 {
                return Err(format!("“{where_}” types slower than one character every 5 s"));
            }
        }
        Step::Click { mode, count, hold_ms, duration_ms, .. } => {
            // Only a tap repeats, so only a tap needs a count: a `down` is one
            // press and an `up` is one release whatever the number says.
            if *mode == PressMode::Tap {
                if *count == 0 {
                    return Err(format!("“{where_}” clicks zero times"));
                }
                if *count > 100_000 {
                    return Err(format!("“{where_}” clicks more than 100 000 times"));
                }
            }
            if matches!(mode, PressMode::Tap | PressMode::Hold | PressMode::Drag)
                && *hold_ms > 60_000
            {
                return Err(format!("“{where_}” holds the button for longer than a minute"));
            }
            if *mode == PressMode::Drag && *duration_ms > 60_000 {
                return Err(format!("“{where_}” takes longer than a minute to drag"));
            }
        }
        Step::Delay { ms } => {
            if *ms > 3_600_000 {
                return Err(format!("“{where_}” waits for longer than an hour"));
            }
        }
        Step::Move { duration_ms, .. } => {
            if *duration_ms > 60_000 {
                return Err(format!("“{where_}” takes longer than a minute to travel"));
            }
        }
        Step::Scroll { amount } => {
            if *amount == 0 {
                return Err(format!("“{where_}” scrolls nowhere"));
            }
        }
    }
    Ok(())
}

/// Should this macro run, given what is in front?
pub fn should_run(macro_definition: &Macro, foreground: &str) -> bool {
    if !macro_definition.enabled {
        return false;
    }
    if macro_definition.only_when.trim().is_empty() {
        return true;
    }
    macro_definition
        .only_when
        .trim()
        .eq_ignore_ascii_case(foreground.trim())
}

/// Everything the interpreter needs from the operating system.
pub trait Runner {
    fn tap_key(&mut self, scan: u16, uppercase: bool);
    fn key(&mut self, scan: u16, down: bool, uppercase: bool);
    /// Press or release a mouse button and leave it that way. Used by the `down`
    /// and `up` click blocks, and to let go of a held button when a run ends.
    fn button(&mut self, button: MouseButton, down: bool);
    fn text(&mut self, text: &str);
    fn move_to(&mut self, x: i32, y: i32, duration_ms: u32);
    fn move_by(&mut self, dx: i32, dy: i32);
    fn click(&mut self, button: MouseButton, count: u32, hold_ms: u32);
    fn scroll(&mut self, amount: i32);
    /// Wait, unless the run is stopped. Returns false when it is.
    fn wait(&mut self, ms: u32, stop: &AtomicBool) -> bool;
}

/// Why a run ended.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub stop_reason: String,
    pub actions: u64,
}

/// Walk the blocks. Stops early — cleanly, releasing what it pressed — when the
/// stop flag goes up or the action budget runs out.
pub fn play(steps: &[Step], runner: &mut dyn Runner, stop: Arc<AtomicBool>) -> Report {
    let mut held = Presses::default();
    let mut state = State {
        actions: 0,
        reason: String::from("Ran to the end"),
    };

    match walk(steps, runner, &stop, &mut state, &mut held, 0) {
        Flow::Done => {}
        Flow::Stopped(reason) => state.reason = reason,
    }

    // Anything pressed with a `down` block and never released would stay down
    // after the macro finished, which is worse than not running it at all: a
    // key held down types on its own, and a mouse button held down keeps
    // clicking whatever the cursor happens to be over.
    held.release_all(runner);

    Report {
        stop_reason: state.reason,
        actions: state.actions,
    }
}

struct State {
    actions: u64,
    reason: String,
}

/// What a run has pressed with a `down` block and not released yet.
#[derive(Default)]
struct Presses {
    keys: Vec<(u16, bool)>,
    buttons: Vec<MouseButton>,
}

impl Presses {
    fn press_key(&mut self, scan: u16, uppercase: bool) {
        self.keys.push((scan, uppercase));
    }

    /// Forget a key that a block released on purpose, so the run does not
    /// release it a second time.
    fn release_key(&mut self, scan: u16) {
        if let Some(index) = self.keys.iter().position(|(pressed, _)| *pressed == scan) {
            self.keys.remove(index);
        }
    }

    fn press_button(&mut self, button: MouseButton) {
        if !self.buttons.contains(&button) {
            self.buttons.push(button);
        }
    }

    fn release_button(&mut self, button: MouseButton) {
        self.buttons.retain(|pressed| *pressed != button);
    }

    /// Let go of everything still held, newest first.
    fn release_all(&mut self, runner: &mut dyn Runner) {
        for button in std::mem::take(&mut self.buttons).into_iter().rev() {
            runner.button(button, false);
        }
        for (scan, uppercase) in std::mem::take(&mut self.keys).into_iter().rev() {
            runner.key(scan, false, uppercase);
        }
    }
}

enum Flow {
    Done,
    Stopped(String),
}

fn walk(
    steps: &[Step],
    runner: &mut dyn Runner,
    stop: &AtomicBool,
    state: &mut State,
    held: &mut Presses,
    depth: usize,
) -> Flow {
    if depth > MAX_DEPTH {
        return Flow::Stopped(format!("nesting deeper than {MAX_DEPTH} levels"));
    }

    for step in steps {
        if stop.load(Ordering::Relaxed) {
            return Flow::Stopped("Stopped".to_string());
        }
        if state.actions >= ACTION_BUDGET {
            return Flow::Stopped(format!(
                "stopped after {} actions",
                state.actions
            ));
        }

        match step {
            Step::Key {
                scan,
                mode,
                hold_ms,
                uppercase,
            } => {
                state.actions += 1;
                // A key block can carry a mouse button instead of a key. The
                // modes mean the same thing — tap it, hold it, leave it down —
                // and a chord often wants a right click in among the keys.
                if let Some(button) = MouseButton::from_scan(*scan) {
                    match mode {
                        // A tap is the click block's click, hold time and all,
                        // so it is not a second implementation of the same
                        // gesture.
                        PressMode::Tap | PressMode::Drag => {
                            runner.click(button, 1, *hold_ms);
                        }
                        PressMode::Hold => {
                            runner.button(button, true);
                            held.press_button(button);
                            if !runner.wait(*hold_ms, stop) {
                                return Flow::Stopped("Stopped".to_string());
                            }
                            runner.button(button, false);
                            held.release_button(button);
                        }
                        PressMode::Down => {
                            runner.button(button, true);
                            held.press_button(button);
                        }
                        PressMode::Up => {
                            runner.button(button, false);
                            held.release_button(button);
                        }
                    }
                } else {
                    match mode {
                        PressMode::Tap | PressMode::Drag => runner.tap_key(*scan, *uppercase),
                        PressMode::Hold => {
                            runner.key(*scan, true, *uppercase);
                            if !runner.wait(*hold_ms, stop) {
                                runner.key(*scan, false, *uppercase);
                                return Flow::Stopped("Stopped".to_string());
                            }
                            runner.key(*scan, false, *uppercase);
                        }
                        PressMode::Down => {
                            runner.key(*scan, true, *uppercase);
                            held.press_key(*scan, *uppercase);
                        }
                        PressMode::Up => {
                            runner.key(*scan, false, *uppercase);
                            // Released on purpose, so it is not ours to clean up.
                            held.release_key(*scan);
                        }
                    }
                }
            }
            Step::Text { text, per_char_ms } => {
                if *per_char_ms == 0 {
                    state.actions += text.chars().count() as u64;
                    runner.text(text);
                } else {
                    // One character per block keeps a stopped run from dumping
                    // the rest of the string into whatever is focused.
                    for character in text.chars() {
                        state.actions += 1;
                        runner.text(&character.to_string());
                        if !runner.wait(*per_char_ms, stop) {
                            return Flow::Stopped("Stopped".to_string());
                        }
                    }
                }
            }
            Step::Move {
                x,
                y,
                relative,
                duration_ms,
            } => {
                state.actions += 1;
                if *relative {
                    runner.move_by(*x, *y);
                } else {
                    runner.move_to(*x, *y, *duration_ms);
                }
            }
            Step::Click {
                button,
                mode,
                count,
                hold_ms,
                x,
                y,
                relative,
                duration_ms,
            } => match mode {
                // A tap is the click the block is named after: one press and
                // release per count, each down for `hold_ms`.
                PressMode::Tap => {
                    state.actions += u64::from(*count);
                    runner.click(*button, *count, *hold_ms);
                }
                // One deliberate press, held for the time given and released.
                // Waited here rather than inside the runner so a stop is
                // noticed partway through a long hold, and so the button is
                // let go of even when the run is cut short.
                PressMode::Hold => {
                    state.actions += 1;
                    runner.button(*button, true);
                    held.press_button(*button);
                    if !runner.wait(*hold_ms, stop) {
                        return Flow::Stopped("Stopped".to_string());
                    }
                    runner.button(*button, false);
                    held.release_button(*button);
                }
                PressMode::Down => {
                    state.actions += 1;
                    runner.button(*button, true);
                    held.press_button(*button);
                }
                PressMode::Up => {
                    state.actions += 1;
                    runner.button(*button, false);
                    held.release_button(*button);
                }
                // The whole gesture in one block: press where the cursor is,
                // travel, rest there, let go. The press is registered as held
                // for the same reason a `Down` is — a run cut short in the
                // middle of a drag must not leave the button down.
                PressMode::Drag => {
                    state.actions += 2;
                    runner.button(*button, true);
                    held.press_button(*button);
                    if *relative {
                        runner.move_by(*x, *y);
                    } else {
                        runner.move_to(*x, *y, *duration_ms);
                    }
                    if !runner.wait(*hold_ms, stop) {
                        runner.button(*button, false);
                        held.release_button(*button);
                        return Flow::Stopped("Stopped".to_string());
                    }
                    runner.button(*button, false);
                    held.release_button(*button);
                }
            },
            Step::Scroll { amount } => {
                state.actions += 1;
                runner.scroll(*amount);
            }
            Step::Delay { ms } => {
                state.actions += 1;
                if !runner.wait(*ms, stop) {
                    return Flow::Stopped("Stopped".to_string());
                }
            }
            Step::Repeat { times, steps } => {
                // Clamped rather than rejected: a run is not the place to
                // discover a bad number, and [`validate`] refuses them anyway.
                for _ in 0..(*times).clamp(1, MAX_TIMES) {
                    match walk(steps, runner, stop, state, held, depth + 1) {
                        Flow::Done => {}
                        Flow::Stopped(reason) => return Flow::Stopped(reason),
                    }
                }
            }
        }
    }

    Flow::Done
}

/// Shortest time a tap keeps the button down.
///
/// A synthetic down and an up in the same instant is a real click to Windows
/// and usually to a window, but not always to a game: one that samples its
/// input once a frame can see a press and a release that arrive together as
/// "not pressed". Measured here, a tap with no hold time put its down and its
/// up inside the same sampling tick, well under a millisecond apart. One frame
/// at 60 fps is the floor; a block's own time is used whenever it is longer.
pub const MIN_TAP_MS: u32 = 15;

/// How long a tap actually keeps the button down.
fn tap_hold(hold_ms: u32) -> u32 {
    hold_ms.max(MIN_TAP_MS)
}

/// The real runner: input through [`crate::input`], waits on the clock.
pub struct WindowsRunner;

impl Runner for WindowsRunner {
    fn tap_key(&mut self, scan: u16, uppercase: bool) {
        if uppercase {
            crate::input::key(0x2A, true);
        }
        crate::input::key(scan, true);
        crate::input::key(scan, false);
        if uppercase {
            crate::input::key(0x2A, false);
        }
    }

    fn key(&mut self, scan: u16, down: bool, uppercase: bool) {
        if down && uppercase {
            crate::input::key(0x2A, true);
        }
        crate::input::key(scan, down);
        if !down && uppercase {
            crate::input::key(0x2A, false);
        }
    }

    fn text(&mut self, text: &str) {
        crate::input::text(text);
    }

    fn move_to(&mut self, x: i32, y: i32, duration_ms: u32) {
        if duration_ms == 0 {
            crate::input::move_to(x, y);
            return;
        }
        let (start_x, start_y) = crate::input::cursor_position();
        let steps = (duration_ms / 8).clamp(4, 60);
        let step_ms = f64::from(duration_ms) / f64::from(steps);
        for step in 1..=steps {
            let t = f64::from(step) / f64::from(steps);
            let eased = if t < 0.5 {
                2.0 * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
            };
            let next_x = f64::from(start_x) + f64::from(x - start_x) * eased;
            let next_y = f64::from(start_y) + f64::from(y - start_y) * eased;
            crate::input::move_to(next_x.round() as i32, next_y.round() as i32);
            std::thread::sleep(std::time::Duration::from_secs_f64(step_ms / 1_000.0));
        }
        crate::input::move_to(x, y);
    }

    fn move_by(&mut self, dx: i32, dy: i32) {
        crate::input::move_by(dx, dy);
    }

    fn button(&mut self, button: MouseButton, down: bool) {
        crate::input::button(button, down);
    }

    fn click(&mut self, button: MouseButton, count: u32, hold_ms: u32) {
        let down_for = tap_hold(hold_ms);
        for _ in 0..count {
            crate::input::button(button, true);
            std::thread::sleep(std::time::Duration::from_millis(u64::from(down_for)));
            crate::input::button(button, false);
        }
    }

    fn scroll(&mut self, amount: i32) {
        crate::input::scroll(amount);
    }

    fn wait(&mut self, ms: u32, stop: &AtomicBool) -> bool {
        // Sliced so a ten-minute wait still notices the stop button.
        let mut left = ms;
        while left > 0 {
            if stop.load(Ordering::Relaxed) {
                return false;
            }
            let slice = left.min(20);
            std::thread::sleep(std::time::Duration::from_millis(u64::from(slice)));
            left -= slice;
        }
        !stop.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Records every action, and runs on a virtual clock so a `wait` costs
    /// nothing but a number.
    #[derive(Default)]
    struct Recorder {
        actions: Rc<RefCell<Vec<String>>>,
        clock: Rc<RefCell<u64>>,
    }

    impl Recorder {
        fn note(&self, line: String) {
            self.actions.borrow_mut().push(line);
        }

        fn lines(&self) -> Vec<String> {
            self.actions.borrow().clone()
        }
    }

    impl Runner for Recorder {
        fn tap_key(&mut self, scan: u16, uppercase: bool) {
            self.note(format!(
                "tap {}{}",
                if uppercase { "Shift+" } else { "" },
                crate::keys::label(scan)
            ));
        }

        fn key(&mut self, scan: u16, down: bool, uppercase: bool) {
            self.note(format!(
                "{} {}{}",
                if down { "down" } else { "up" },
                if uppercase { "Shift+" } else { "" },
                crate::keys::label(scan)
            ));
        }

        fn text(&mut self, text: &str) {
            self.note(format!("type {text:?}"));
        }

        fn move_to(&mut self, x: i32, y: i32, duration_ms: u32) {
            self.note(format!("to {x},{y} in {duration_ms} ms"));
        }

        fn move_by(&mut self, dx: i32, dy: i32) {
            self.note(format!("by {dx},{dy}"));
        }

        fn button(&mut self, button: MouseButton, down: bool) {
            self.note(format!(
                "{} {}",
                if down { "down" } else { "up" },
                button.label()
            ));
        }

        fn click(&mut self, button: MouseButton, count: u32, hold_ms: u32) {
            self.note(format!("{} ×{count} hold {hold_ms}", button.label()));
        }

        fn scroll(&mut self, amount: i32) {
            self.note(format!("scroll {amount}"));
        }

        /// Waits in real slices, the way the Windows runner does, so a stop in
        /// the middle of a long wait is noticed instead of optimistically
        /// skipped. The virtual clock still advances by the whole duration.
        fn wait(&mut self, ms: u32, stop: &AtomicBool) -> bool {
            let mut left = ms;
            while left > 0 {
                if stop.load(Ordering::Relaxed) {
                    return false;
                }
                let slice = left.min(20);
                std::thread::sleep(std::time::Duration::from_millis(u64::from(slice)));
                *self.clock.borrow_mut() += u64::from(slice);
                left -= slice;
            }
            !stop.load(Ordering::Relaxed)
        }
    }

    /// The block every other test uses: press W and let go.
    fn tap_w() -> Step {
        Step::Key {
            scan: 0x11,
            mode: PressMode::Tap,
            hold_ms: 0,
            uppercase: false,
        }
    }

    /// A macro with nothing in it yet.
    fn sample(id: &str, name: &str) -> Macro {
        Macro {
            id: id.to_string(),
            name: name.to_string(),
            hotkey: String::new(),
            only_when: String::new(),
            enabled: true,
            steps: Vec::new(),
        }
    }

    /// Run one macro and hand back what it did.
    fn run(steps: &[Step]) -> Vec<String> {
        let mut recorder = Recorder::default();
        let actions = Rc::clone(&recorder.actions);
        let stop = Arc::new(AtomicBool::new(false));
        let report = play(steps, &mut recorder, stop);
        assert_eq!(report.stop_reason, "Ran to the end");
        let lines = actions.borrow().clone();
        lines
    }

    #[test]
    fn a_tap_presses_and_a_hold_releases_after_its_time() {
        let lines = run(&[
            Step::Key {
                scan: 0x11,
                mode: PressMode::Tap,
                hold_ms: 60,
                uppercase: false,
            },
            Step::Key {
                scan: 0x1E,
                mode: PressMode::Hold,
                hold_ms: 250,
                uppercase: false,
            },
            Step::Key {
                scan: 0x11,
                mode: PressMode::Tap,
                hold_ms: 60,
                uppercase: true,
            },
        ]);

        assert_eq!(lines, vec!["tap W", "down A", "up A", "tap Shift+W"]);
    }

    #[test]
    fn a_key_block_can_hold_a_mouse_button() {
        // The same block, pointed at the right button: tap, then leave it down,
        // then let go — the chord a game menu wants, in three blocks. 0x202 is
        // the right button, above every keyboard scan.
        let lines = run(&[
            Step::Key {
                scan: 0x202,
                mode: PressMode::Tap,
                hold_ms: 40,
                uppercase: false,
            },
            Step::Key {
                scan: 0x202,
                mode: PressMode::Down,
                hold_ms: 0,
                uppercase: false,
            },
            Step::Delay { ms: 50 },
            Step::Key {
                scan: 0x202,
                mode: PressMode::Up,
                hold_ms: 0,
                uppercase: false,
            },
        ]);

        assert_eq!(lines, vec!["Right ×1 hold 40", "down Right", "up Right"]);
        assert_eq!(crate::keys::label(0x202), "Right click");
    }

    #[test]
    fn a_down_block_spans_later_blocks_until_it_is_released() {
        let lines = run(&[
            Step::Key {
                scan: 0x2A,
                mode: PressMode::Down,
                hold_ms: 0,
                uppercase: false,
            },
            Step::Delay { ms: 50 },
            Step::Click {
                button: MouseButton::Left,
                mode: PressMode::Tap,
                count: 1,
                hold_ms: 0,
                x: 0,
                y: 0,
                relative: true,
                duration_ms: 0,
            },
            Step::Key {
                scan: 0x2A,
                mode: PressMode::Up,
                hold_ms: 0,
                uppercase: false,
            },
        ]);

        assert_eq!(
            lines,
            vec!["down Left Shift", "Left ×1 hold 0", "up Left Shift"]
        );
    }

    /// A held click is one press, whatever the count says: the count belongs to
    /// a tap, and this is the block for "keep the button down for a moment".
    #[test]
    fn a_held_click_presses_once_waits_and_lets_go() {
        let recorder = Recorder::default();
        let actions = Rc::clone(&recorder.actions);
        let clock = Rc::clone(&recorder.clock);
        let mut runner = recorder;
        let stop = Arc::new(AtomicBool::new(false));
        let report = play(
            &[Step::Click {
                button: MouseButton::Right,
                mode: PressMode::Hold,
                count: 7,
                hold_ms: 300,
                x: 0,
                y: 0,
                relative: true,
                duration_ms: 0,
            }],
            &mut runner,
            stop,
        );

        assert_eq!(actions.borrow().clone(), vec!["down Right", "up Right"]);
        assert_eq!(*clock.borrow(), 300, "the hold should take its time");
        assert_eq!(report.actions, 1);
    }

    /// Down and up span blocks, which is what they are for: press, do something
    /// else with the button still down, then release.
    #[test]
    fn a_click_held_down_spans_blocks_until_its_own_up_block() {
        let lines = run(&[
            Step::Click {
                button: MouseButton::Left,
                mode: PressMode::Down,
                count: 1,
                hold_ms: 0,
                x: 0,
                y: 0,
                relative: true,
                duration_ms: 0,
            },
            Step::Delay { ms: 20 },
            Step::Click {
                button: MouseButton::Left,
                mode: PressMode::Up,
                count: 1,
                hold_ms: 0,
                x: 0,
                y: 0,
                relative: true,
                duration_ms: 0,
            },
        ]);

        // Nothing extra on the way out: the `up` block released it already.
        assert_eq!(lines, vec!["down Left", "up Left"]);
    }

    /// A stuck mouse button is worse than a stuck key, so the same net covers
    /// it: a run that is cut short must not leave one held down.
    #[test]
    fn a_stopped_run_lets_go_of_a_held_mouse_button() {
        let recorder = Recorder::default();
        let actions = Rc::clone(&recorder.actions);
        let mut runner = recorder;
        let stop = Arc::new(AtomicBool::new(false));
        {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(30));
                stop.store(true, Ordering::Relaxed);
            });
        }
        let report = play(
            &[
                Step::Click {
                    button: MouseButton::Left,
                    mode: PressMode::Down,
                    count: 1,
                    hold_ms: 0,
                    x: 0,
                    y: 0,
                    relative: true,
                    duration_ms: 0,
                },
                Step::Delay { ms: 10_000 },
            ],
            &mut runner,
            stop,
        );

        assert_eq!(report.stop_reason, "Stopped");
        assert_eq!(actions.borrow().clone(), vec!["down Left", "up Left"]);
    }

    #[test]
    fn a_press_or_release_needs_no_count_but_a_tap_does() {
        let click = |mode| Step::Click {
            button: MouseButton::Left,
            mode,
            count: 0,
            hold_ms: 0,
            x: 0,
            y: 0,
            relative: true,
            duration_ms: 0,
        };

        for mode in [PressMode::Down, PressMode::Up, PressMode::Drag] {
            let mut macro_definition = sample("m", "Test");
            macro_definition.steps = vec![click(mode)];
            assert!(
                validate(&macro_definition).is_ok(),
                "a {mode:?} is one action, so a zero count is not a mistake"
            );
        }

        let mut macro_definition = sample("m", "Test");
        macro_definition.steps = vec![click(PressMode::Tap)];
        assert!(
            validate(&macro_definition).is_err(),
            "a tap of nothing clicks nothing"
        );
    }

    #[test]
    fn every_click_mode_reads_as_what_it_does() {
        let describe = |mode, count, hold_ms| {
            Step::Click {
                button: MouseButton::Left,
                mode,
                count,
                hold_ms,
                x: 0,
                y: 0,
                relative: true,
                duration_ms: 0,
            }
            .summary()
        };

        assert_eq!(describe(PressMode::Tap, 3, 0), "left ×3");
        assert_eq!(describe(PressMode::Tap, 3, 40), "left ×3, held 40 ms");
        assert_eq!(describe(PressMode::Hold, 1, 250), "left held 250 ms");
        assert_eq!(describe(PressMode::Down, 1, 0), "left down");
        assert_eq!(describe(PressMode::Up, 1, 0), "left up");

        let drag = |relative, hold_ms| {
            Step::Click {
                button: MouseButton::Right,
                mode: PressMode::Drag,
                count: 1,
                hold_ms,
                x: 900,
                y: 500,
                relative,
                duration_ms: 250,
            }
            .summary()
        };

        assert_eq!(drag(true, 0), "drag right by 900,500");
        assert_eq!(drag(true, 40), "drag right by 900,500, resting 40 ms");
        assert_eq!(
            drag(false, 0),
            "drag right to 900,500 over 250 ms",
            "a drag to a point says how long it takes to get there"
        );
    }

    /// The safety net: a stop in the middle of a chord must not leave Shift
    /// held down for the rest of the session.
    #[test]
    fn a_stopped_run_releases_the_keys_it_pressed() {
        let recorder = Recorder::default();
        let actions = Rc::clone(&recorder.actions);
        let mut runner = recorder;
        let stop = Arc::new(AtomicBool::new(false));

        let steps = vec![
            Step::Key {
                scan: 0x11,
                mode: PressMode::Down,
                hold_ms: 0,
                uppercase: false,
            },
            Step::Key {
                scan: 0x39,
                mode: PressMode::Down,
                hold_ms: 0,
                uppercase: false,
            },
            Step::Delay { ms: 10_000 },
        ];

        // Stop it while it is waiting.
        {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(30));
                stop.store(true, Ordering::Relaxed);
            });
        }
        let report = play(&steps, &mut runner, stop);

        assert_eq!(report.stop_reason, "Stopped");
        assert_eq!(
            actions.borrow().clone(),
            vec!["down W", "down Spacebar", "up Spacebar", "up W"],
            "held keys should be released in reverse order"
        );
    }

    #[test]
    fn text_is_typed_one_character_at_a_time_when_a_gap_is_set() {
        let lines = run(&[Step::Text {
            text: "hi".to_string(),
            per_char_ms: 30,
        }]);
        assert_eq!(lines, vec!["type \"h\"", "type \"i\""]);

        let at_once = run(&[Step::Text {
            text: "hello".to_string(),
            per_char_ms: 0,
        }]);
        assert_eq!(at_once, vec!["type \"hello\""]);
    }

    #[test]
    fn nested_repeats_multiply_out() {
        let recorder = Recorder::default();
        let clock = Rc::clone(&recorder.clock);
        let mut runner = recorder;
        let stop = Arc::new(AtomicBool::new(false));

        let inner = Step::Repeat {
            times: 2,
            steps: vec![tap_w(), Step::Delay { ms: 5 }],
        };
        let report = play(
            &[Step::Repeat {
                times: 3,
                steps: vec![inner],
            }],
            &mut runner,
            stop,
        );

        // Three outer passes of two inner passes, each a tap and a wait.
        assert_eq!(report.actions, 3 * 2 * 2);
        assert_eq!(*clock.borrow(), 3 * 2 * 5, "the waits should add up");
    }

    #[test]
    fn a_repeat_of_one_still_runs_once() {
        // A zero count is refused when editing, but the interpreter must not
        // turn it into a skip either.
        let once = run(&[Step::Repeat {
            times: 0,
            steps: vec![tap_w()],
        }]);
        let twice = run(&[Step::Repeat {
            times: 2,
            steps: vec![tap_w()],
        }]);
        assert_eq!(once, vec!["tap W"]);
        assert_eq!(twice, vec!["tap W", "tap W"]);
    }

    #[test]
    fn the_report_counts_what_the_run_did() {
        let recorder = Recorder::default();
        let mut runner = recorder;
        let stop = Arc::new(AtomicBool::new(false));
        let report = play(
            &[
                Step::Click {
                    button: MouseButton::Right,
                    mode: PressMode::Tap,
                    count: 3,
                    hold_ms: 1,
                    x: 0,
                    y: 0,
                    relative: true,
                    duration_ms: 0,
                },
                Step::Text {
                    text: "abc".to_string(),
                    per_char_ms: 1,
                },
                Step::Scroll { amount: -2 },
            ],
            &mut runner,
            stop,
        );

        // Three clicks, three characters, one scroll.
        assert_eq!(report.actions, 7);
        assert_eq!(report.stop_reason, "Ran to the end");
    }

    #[test]
    fn every_kind_of_block_is_described_and_written_out() {
        let steps = vec![
            Step::Key {
                scan: 0x11,
                mode: PressMode::Hold,
                hold_ms: 120,
                uppercase: true,
            },
            Step::Move {
                x: 40,
                y: 60,
                relative: true,
                duration_ms: 0,
            },
            Step::Move {
                x: 400,
                y: 600,
                relative: false,
                duration_ms: 200,
            },
            Step::Scroll { amount: 3 },
            Step::Delay { ms: 250 },
            Step::Click {
                button: MouseButton::Left,
                mode: PressMode::Drag,
                count: 1,
                hold_ms: 0,
                x: 900,
                y: 500,
                relative: false,
                duration_ms: 250,
            },
        ];
        let lines = run(&steps);

        assert_eq!(lines[0], "down Shift+W");
        assert_eq!(lines[1], "up Shift+W");
        assert_eq!(lines[2], "by 40,60");
        assert_eq!(lines[3], "to 400,600 in 200 ms");
        assert_eq!(lines[4], "scroll 3");
        assert_eq!(lines[5], "down Left");
        assert_eq!(lines[6], "to 900,500 in 250 ms");
        assert_eq!(lines[7], "up Left");

        assert_eq!(steps[0].summary(), "Shift+W held 120 ms");
        assert_eq!(steps[1].summary(), "move by 40,60");
        assert_eq!(steps[2].summary(), "glide to 400,600 over 200 ms");
        assert_eq!(steps[3].summary(), "scroll 3");
        assert_eq!(steps[4].summary(), "wait 250 ms");
        assert_eq!(steps[5].summary(), "drag left to 900,500 over 250 ms");
    }

    #[test]
    fn a_macro_only_runs_where_it_says_it_does() {
        let mut macro_definition = sample("m1", "Burst fire");
        assert!(should_run(&macro_definition, "Anything.exe"));

        macro_definition.only_when = "Game.exe".into();
        assert!(should_run(&macro_definition, "game.exe"));
        assert!(!should_run(&macro_definition, "Discord.exe"));

        macro_definition.enabled = false;
        assert!(!should_run(&macro_definition, "Game.exe"));
    }

    #[test]
    fn nesting_is_capped_when_the_macro_is_checked() {
        let mut step = Step::Repeat {
            times: 2,
            steps: vec![Step::Delay { ms: 1 }],
        };
        for _ in 0..MAX_DEPTH + 1 {
            step = Step::Repeat {
                times: 2,
                steps: vec![step],
            };
        }

        let mut macro_definition = sample("m1", "Deep");
        macro_definition.steps = vec![step];
        let error = validate(&macro_definition).expect_err("too deep should be refused");
        assert!(error.contains("nested deeper"), "{error}");
    }

    #[test]
    fn silly_blocks_are_refused_with_the_reason() {
        let cases = [
            Step::Key {
                scan: 0,
                mode: PressMode::Tap,
                hold_ms: 0,
                uppercase: false,
            },
            Step::Text {
                text: String::new(),
                per_char_ms: 0,
            },
            Step::Click {
                button: MouseButton::Left,
                mode: PressMode::Tap,
                count: 0,
                hold_ms: 0,
                x: 0,
                y: 0,
                relative: true,
                duration_ms: 0,
            },
            // A key has nowhere to go, so a drag on one is a mistake.
            Step::Key {
                scan: 0x11,
                mode: PressMode::Drag,
                hold_ms: 0,
                uppercase: false,
            },
            Step::Scroll { amount: 0 },
            Step::Repeat {
                times: 0,
                steps: vec![],
            },
        ];

        for step in cases {
            let mut macro_definition = sample("m", "Test");
            macro_definition.steps = vec![step.clone()];
            assert!(
                validate(&macro_definition).is_err(),
                "{} should be refused",
                step.summary()
            );
        }

        assert!(validate(&sample("m", "  ")).is_err());
    }

    #[test]
    fn the_action_budget_stops_a_runaway_nest() {
        let recorder = Recorder::default();
        let mut runner = recorder;
        let stop = Arc::new(AtomicBool::new(false));
        // 2000 × 2000 × 10 = 40 million actions, far past the budget.
        let report = play(
            &[Step::Repeat {
                times: MAX_TIMES,
                steps: vec![Step::Repeat {
                    times: MAX_TIMES,
                    steps: vec![Step::Delay { ms: 0 }],
                }],
            }],
            &mut runner,
            stop,
        );

        assert!(
            report.stop_reason.contains("actions"),
            "the budget should end it, said “{}”",
            report.stop_reason
        );
    }

    /// The click-and-drag in one block: press, travel, rest where it landed for
    /// the block's own time, and let go.
    #[test]
    fn a_drag_presses_travels_rests_and_releases() {
        let recorder = Recorder::default();
        let actions = Rc::clone(&recorder.actions);
        let clock = Rc::clone(&recorder.clock);
        let mut runner = recorder;
        let stop = Arc::new(AtomicBool::new(false));
        let report = play(
            &[
                Step::Click {
                    button: MouseButton::Left,
                    mode: PressMode::Drag,
                    count: 1,
                    hold_ms: 40,
                    x: 300,
                    y: 0,
                    relative: true,
                    duration_ms: 0,
                },
                Step::Click {
                    button: MouseButton::Right,
                    mode: PressMode::Drag,
                    count: 1,
                    hold_ms: 0,
                    x: 900,
                    y: 500,
                    relative: false,
                    duration_ms: 250,
                },
            ],
            &mut runner,
            stop,
        );

        assert_eq!(
            actions.borrow().clone(),
            vec![
                "down Left",
                "by 300,0",
                "up Left",
                "down Right",
                "to 900,500 in 250 ms",
                "up Right",
            ]
        );
        assert_eq!(*clock.borrow(), 40, "the drag rests before it lets go");
        assert_eq!(report.actions, 4, "two presses and two travels");
    }

    /// A tap is allowed to ask for no time at all, and that is exactly the click
    /// a frame-based program can miss: the release must never come in the same
    /// instant as the press.
    #[test]
    fn a_tap_never_lets_go_in_the_same_instant() {
        assert_eq!(tap_hold(0), MIN_TAP_MS);
        assert_eq!(tap_hold(1), MIN_TAP_MS);
        assert_eq!(tap_hold(MIN_TAP_MS), MIN_TAP_MS);
        assert_eq!(tap_hold(500), 500, "a longer press is left alone");
    }

    /// A block has to read back in the names the window writes it with. This is
    /// the bug that made the block fields look broken: the window is camelCase,
    /// the JSON was snake_case, and every number the person typed was dropped on
    /// the way in.
    #[test]
    fn a_block_reads_the_names_the_window_writes() {
        let step: Step = serde_json::from_str(
            r#"{"kind":"click","button":"right","mode":"drag","count":1,"holdMs":40,"x":900,"y":500,"relative":false,"durationMs":250}"#,
        )
        .expect("the window's own field names");

        assert_eq!(
            step.summary(),
            "drag right to 900,500 over 250 ms, resting 40 ms",
            "the hold and the travel time have to come back, not their defaults"
        );
    }

    /// The same in the other direction, so a block saved today is read back
    /// unchanged tomorrow.
    #[test]
    fn a_block_survives_a_round_trip_through_json() {
        let step = Step::Click {
            button: MouseButton::Right,
            mode: PressMode::Drag,
            count: 1,
            hold_ms: 40,
            x: 900,
            y: 500,
            relative: false,
            duration_ms: 250,
        };
        let json = serde_json::to_string(&step).unwrap();

        for field in ["\"holdMs\":40", "\"durationMs\":250", "\"x\":900", "\"relative\":false"] {
            assert!(json.contains(field), "{json} is missing {field}");
        }
        let back: Step = serde_json::from_str(&json).unwrap();
        assert_eq!(back, step);
    }

    /// A key has nowhere to go, so a drag on a key block is refused rather than
    /// quietly typed.
    #[test]
    fn a_key_block_cannot_drag() {
        let mut macro_definition = sample("m", "Test");
        macro_definition.steps = vec![Step::Key {
            scan: 0x11,
            mode: PressMode::Drag,
            hold_ms: 0,
            uppercase: false,
        }];
        let error = validate(&macro_definition).expect_err("a key cannot be dragged");
        assert!(error.contains("drag"), "said “{error}”");
    }
}

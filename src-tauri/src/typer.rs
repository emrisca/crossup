//! The typer: types a block of text into whichever window is in front.
//!
//! The point is that a run should read like somebody at a keyboard rather than a
//! machine emptying a clipboard. So the text goes out one keystroke at a time on
//! a clock, a share of the characters are mistyped and corrected, and the gaps
//! carry a little spread instead of landing on the same millisecond.
//!
//! Two rules hold the rest together:
//!
//! 1. **One run at a time.** The typer claims the desktop through [`Runtime`],
//!    like the clicker and the macros, so the three can never fight over the
//!    keyboard.
//! 2. **Stop is checked between keystrokes, not after the text.** A run that
//!    only noticed the stop button once the whole draft was out would be worse
//!    than no stop button at all.
//!
//! The focus guard is the other half of that: the window in front when Start is
//! pressed is the one the run types into, and if anything else comes forward the
//! run stands down. Typing half a paragraph into the wrong window is the failure
//! mode that makes people distrust these tools.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::input;
use crate::runtime::RuntimeState;

/// The name a typer run is reported under in `activity`.
pub const TYPER: &str = "typer";

/// Characters to a word, the count every words-a-minute figure uses.
const CHARS_A_WORD: f64 = 5.0;
/// The fastest pace a deadline may ask for. Past this the keystrokes queue up in
/// the target window and arrive as one burst, which is the opposite of the point.
const MAX_CPS: f64 = 25.0;
/// The largest share of characters a run may mistype.
const MAX_TYPO: f64 = 0.25;
/// The pace a fresh typer starts at: an unhurried person.
const DEFAULT_WPM: f64 = 65.0;
/// The shortcut a fresh typer comes up with. A slot reading "not set" is a
/// feature nobody can use, and the typer only earns its place if it starts from
/// a window that is not this one.
pub const DEFAULT_HOTKEY: &str = "Control+Alt+T";

const BACKSPACE: u16 = 0x0e;
const TAB: u16 = 0x0f;
const ENTER: u16 = 0x1c;

// ── settings ───────────────────────────────────────────────────────

/// What to type, and how.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct TyperSettings {
    /// The draft. Kept in the settings file so a half-written paragraph is still
    /// there after a restart, the same way every other tab keeps its work.
    pub text: String,
    /// Resting pace, in words a minute.
    pub wpm: f64,
    /// Share of characters mistyped and corrected, 0 to 0.25.
    pub typo_rate: f64,
    /// Finish by this many seconds. It wins over `wpm`, because that is the
    /// whole point of asking for a deadline.
    pub deadline_secs: Option<u32>,
    /// Seconds to wait before the first keystroke, so a window can be clicked
    /// into after Start.
    pub countdown_secs: u32,
    /// The start/stop shortcut, in the same spelling the other tools use.
    pub hotkey: String,
}

impl Default for TyperSettings {
    fn default() -> Self {
        Self {
            text: String::new(),
            wpm: DEFAULT_WPM,
            typo_rate: 0.04,
            deadline_secs: None,
            countdown_secs: 3,
            hotkey: DEFAULT_HOTKEY.to_string(),
        }
    }
}

impl TyperSettings {
    /// The share of characters this run will mistype, kept inside what the
    /// engine can pace.
    fn typo(&self) -> f64 {
        self.typo_rate.clamp(0.0, MAX_TYPO)
    }

    /// The pace the run will keep, in keystrokes a second, and whether a
    /// deadline had to be slowed down to reach it.
    fn cps(&self, chars: u32) -> (f64, bool) {
        let resting = (self.wpm.clamp(5.0, 400.0) * CHARS_A_WORD) / 60.0;
        match self.deadline_secs {
            Some(secs) if chars > 0 => {
                let asked = chars as f64 / secs.max(1) as f64;
                (asked.clamp(0.5, MAX_CPS), asked > MAX_CPS)
            }
            _ => (resting.clamp(0.5, 400.0), false),
        }
    }
}

// ── the estimate ───────────────────────────────────────────────────

/// What a run will do, worked out before it starts.
///
/// It is the plan the driver follows as well as the number on screen, so the
/// tab never promises a pace the run does not keep.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TyperPlan {
    pub chars: u32,
    pub words: u32,
    /// Keystrokes a second, after any deadline.
    pub cps: f64,
    pub total_ms: u64,
    /// Whether a deadline asked for a pace faster than the engine will go.
    pub clamped: bool,
}

pub fn plan(text: &str, settings: &TyperSettings) -> TyperPlan {
    let chars = text.chars().count() as u32;
    let words = text.split_whitespace().count() as u32;
    let (cps, clamped) = settings.cps(chars);
    let char_ms = 1_000.0 / cps;
    // A correction costs three keystrokes and two of its own gaps, so it is
    // worth more time than the character it stands in for.
    let corrections = (chars as f64 * settings.typo()).round();
    let total_ms = (chars as f64 * char_ms + corrections * (char_ms * 0.9 + 170.0)).round();
    TyperPlan {
        chars,
        words,
        cps,
        total_ms: total_ms.max(0.0) as u64,
        clamped,
    }
}

// ── what the tab watches ───────────────────────────────────────────

/// A run's live numbers, polled by the tab while it is open.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// `idle`, `typing`, `paused`, `done`, `stopped` or `unfocused`.
    pub status: String,
    pub typed: u32,
    pub total: u32,
    pub planned_ms: u64,
    pub elapsed_secs: f64,
}

/// The counters a run keeps, shared by the driver thread and the commands.
/// The clock a run keeps.
///
/// It counts only while the run is typing. It used to be one `Instant` taken
/// when the counters were made, which is why a window left open for a while
/// showed that time on a tab where nothing had ever run, why pressing Start did
/// not begin it at zero, and why a finished run kept counting after it ended.
struct Timing {
    started: Instant,
    paused_for: Duration,
    paused_at: Option<Instant>,
    /// Set when the run ends, so the number stays where the run stopped.
    ended: Option<f64>,
}

impl Timing {
    /// Waiting at zero: no run yet, or the countdown before the first keystroke.
    fn waiting() -> Self {
        Self {
            started: Instant::now(),
            paused_for: Duration::ZERO,
            paused_at: None,
            ended: Some(0.0),
        }
    }

    /// Counting, from now.
    fn running() -> Self {
        Self {
            started: Instant::now(),
            paused_for: Duration::ZERO,
            paused_at: None,
            ended: None,
        }
    }

    fn elapsed(&self) -> f64 {
        if let Some(fixed) = self.ended {
            return fixed;
        }
        let mut spent = self.started.elapsed().saturating_sub(self.paused_for);
        if let Some(at) = self.paused_at {
            spent = spent.saturating_sub(at.elapsed());
        }
        spent.as_secs_f64().max(0.0)
    }

    /// Stop the clock where it is, keeping the reading.
    fn freeze(&mut self) {
        if self.ended.is_none() {
            self.ended = Some(self.elapsed());
        }
    }
}

pub struct Progress {
    status: Mutex<String>,
    typed: AtomicU32,
    /// Fixed for the length of a run, so they need no lock.
    total: u32,
    planned_ms: u64,
    timing: Mutex<Timing>,
    paused: AtomicBool,
}

impl Progress {
    fn new(status: &str, total: u32, planned_ms: u64) -> Self {
        Self {
            status: Mutex::new(status.to_string()),
            typed: AtomicU32::new(0),
            total,
            planned_ms,
            timing: Mutex::new(Timing::waiting()),
            paused: AtomicBool::new(false),
        }
    }

    pub fn view(&self) -> Status {
        let mut status = self.status.lock().unwrap().clone();
        // The flag is what the person pressed, so it decides the word on screen
        // even while the thread is still sitting in its pause loop.
        if self.paused.load(Ordering::Relaxed) && status == "typing" {
            status = "paused".to_string();
        }
        Status {
            status,
            typed: self.typed.load(Ordering::Relaxed),
            total: self.total,
            planned_ms: self.planned_ms,
            elapsed_secs: self.timing.lock().unwrap().elapsed(),
        }
    }

    /// The first keystroke is about to go: the clock starts here, not when the
    /// window opened and not when the countdown began.
    pub fn begin_typing(&self) {
        *self.timing.lock().unwrap() = Timing::running();
    }

    fn set(&self, status: &str) {
        *self.status.lock().unwrap() = status.to_string();
        // A finished run keeps the time it took rather than counting on.
        if matches!(status, "done" | "stopped" | "unfocused") {
            self.timing.lock().unwrap().freeze();
        }
    }

    fn bump(&self) {
        self.typed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn pause(&self) -> Status {
        if self.view().status == "typing" {
            self.paused.store(true, Ordering::Relaxed);
            self.timing.lock().unwrap().paused_at = Some(Instant::now());
        }
        self.view()
    }

    pub fn resume(&self) -> Status {
        let held = self.timing.lock().unwrap().paused_at.take();
        if let Some(at) = held {
            self.timing.lock().unwrap().paused_for += at.elapsed();
        }
        self.paused.store(false, Ordering::Relaxed);
        self.view()
    }

    /// Stand a run down from the outside, for a stop that arrives as a command
    /// rather than as the stop flag.
    pub fn stopped(&self) -> Status {
        self.set("stopped");
        self.view()
    }
}

/// The run the tab is watching, and the numbers of the last one.
///
/// The finished run stays current until the next one starts, so the tab goes on
/// showing what it ended on instead of blanking the moment it finishes.
pub struct Holder {
    current: Mutex<Arc<Progress>>,
}

impl Default for Holder {
    fn default() -> Self {
        Self::new()
    }
}

impl Holder {
    pub fn new() -> Self {
        Self {
            current: Mutex::new(Arc::new(Progress::new("idle", 0, 0))),
        }
    }

    pub fn view(&self) -> Status {
        self.current.lock().unwrap().view()
    }

    pub fn current(&self) -> Arc<Progress> {
        Arc::clone(&self.current.lock().unwrap())
    }

    fn begin(&self, total: u32, planned_ms: u64) -> Arc<Progress> {
        let next = Arc::new(Progress::new("typing", total, planned_ms));
        *self.current.lock().unwrap() = Arc::clone(&next);
        next
    }
}

// ── the run ────────────────────────────────────────────────────────

/// Type the draft, on a thread of its own.
pub fn start(
    runtime: &RuntimeState,
    holder: &Holder,
    settings: &TyperSettings,
) -> Result<(), String> {
    if settings.text.trim().is_empty() {
        return Err("there is no text to type".to_string());
    }
    let text = settings.text.clone();
    let estimate = plan(&text, settings);
    let stop = runtime.claim(TYPER)?;
    let progress = holder.begin(estimate.chars, estimate.total_ms);

    let characters: Vec<char> = text.chars().collect();
    let char_ms = 1_000.0 / estimate.cps;
    let typo = settings.typo();
    let countdown = settings.countdown_secs.min(30);
    let shared = Arc::clone(runtime);
    // The thread gets its own handle: the error arm below still needs the
    // holder's copy to say the run never started.
    let counters = Arc::clone(&progress);

    std::thread::Builder::new()
        .name("crossup-typer".into())
        .spawn(move || {
            let outcome = drive(&characters, char_ms, typo, countdown, &stop, &counters);
            counters.set(outcome);
            shared.release(0, 0.0);
        })
        .map_err(|error| {
            progress.stopped();
            runtime.release(0, 0.0);
            format!("could not start the typer thread: {error}")
        })?;

    Ok(())
}

/// The driver: count down, then one keystroke at a time.
///
/// Returns the word the run ended on, which is what the tab shows.
fn drive(
    characters: &[char],
    char_ms: f64,
    typo: f64,
    countdown: u32,
    stop: &AtomicBool,
    progress: &Arc<Progress>,
) -> &'static str {
    // The countdown is not decoration: Start is pressed in this window, and the
    // first keystroke has to land somewhere else.
    for _ in 0..countdown {
        if !wait(stop, 1_000.0) {
            return "stopped";
        }
    }
    // The window in front when the countdown ends is the one this run types
    // into. Taking it when the run started named Crossup itself, because that is
    // where Start and the shortcut are pressed, so the first focus check read the
    // window the text was meant for as a change of window and stood the run down
    // before a single keystroke. Crossup itself is still the wrong window: typing
    // into this app is never what anybody meant.
    let target = crate::hook::foreground_program();
    if target.is_empty() || target.eq_ignore_ascii_case(&own_program()) {
        return "unfocused";
    }
    progress.begin_typing();

    let mut roll = Roll::seeded();
    let mut checked = Instant::now();
    for character in characters {
        if stop.load(Ordering::Relaxed) {
            return "stopped";
        }
        // A paused run holds its place: nothing is typed, and the stop flag is
        // still watched, so Stop works while paused.
        while progress.paused() {
            if !wait(stop, 60.0) {
                return "stopped";
            }
        }
        if !focused(&target, &mut checked) {
            return "unfocused";
        }
        // A mistype is a wrong character, a pause while it is noticed, and a
        // backspace — the three keystrokes a person makes, at a person's speed,
        // rather than a correction done instantly.
        if roll.next() < typo && character.is_alphanumeric() {
            let wrong = wrong_for(*character, roll.next());
            input::text(&wrong.to_string());
            if !wait(stop, char_ms * 0.55) {
                return "stopped";
            }
            tap(BACKSPACE);
            if !wait(stop, 80.0) {
                return "stopped";
            }
        }
        match character {
            '\n' => tap(ENTER),
            '\t' => tap(TAB),
            other => input::text(&other.to_string()),
        }
        progress.bump();
        // A little spread either side of the pace: the same gap every time is
        // the thing that reads as a machine.
        let jitter = 0.82 + roll.next() * 0.36;
        if !wait(stop, char_ms * jitter) {
            return "stopped";
        }
    }
    "done"
}

/// Whether the run should carry on typing.
///
/// The check costs a process lookup, so it happens a few times a second rather
/// than before every keystroke. An unreadable foreground window counts as
/// focused: an elevated window cannot be named, and stopping a legitimate run
/// over that would be a bug with no explanation on screen.
fn focused(target: &str, last: &mut Instant) -> bool {
    if last.elapsed() < Duration::from_millis(200) {
        return true;
    }
    *last = Instant::now();
    let front = crate::hook::foreground_program();
    front.is_empty() || front.eq_ignore_ascii_case(target)
}

/// The name of the program this module is part of, to tell "the window the text
/// is meant for" from "the window the button was pressed in".
fn own_program() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

fn tap(scan: u16) {
    input::key(scan, true);
    input::key(scan, false);
}

/// Sleep in short slices, so a stop lands in about a frame rather than after the
/// rest of the text. Returns false when the run was stopped.
fn wait(stop: &AtomicBool, ms: f64) -> bool {
    let deadline = Instant::now() + Duration::from_secs_f64(ms.max(0.0) / 1_000.0);
    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        let slice = (deadline - now).as_millis().clamp(1, 15) as u64;
        std::thread::sleep(Duration::from_millis(slice));
    }
}

/// The wrong character to type for a mistype: a neighbour from the same row of
/// the keyboard for a letter, the digit beside it for a number, another mark for
/// punctuation.
fn wrong_for(character: char, roll: f64) -> char {
    const ROWS: [&str; 3] = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
    let lower = character.to_ascii_lowercase();
    if let Some(row) = ROWS.iter().find(|row| row.contains(lower)) {
        let letters: Vec<char> = row.chars().collect();
        let at = letters.iter().position(|letter| *letter == lower).unwrap_or(0);
        let step = if roll < 0.5 { -1 } else { 1 };
        let index = (at as i32 + step).rem_euclid(letters.len() as i32) as usize;
        let wrong = letters[index];
        return if character.is_uppercase() {
            wrong.to_ascii_uppercase()
        } else {
            wrong
        };
    }
    if let Some(digit) = character.to_digit(10) {
        return std::char::from_digit((digit + 9) % 10, 10).unwrap_or('0');
    }
    // Anything but the character itself: a "mistype" that types the same mark
    // is not a mistype, and the correction after it would be invisible.
    const MARKS: [char; 6] = ['.', ',', ';', ':', '-', '!'];
    let options: Vec<char> = MARKS
        .iter()
        .copied()
        .filter(|mark| *mark != character)
        .collect();
    options[((roll * options.len() as f64) as usize) % options.len()]
}

/// A cheap source of the "is this character mistyped" answer. Not a security
/// decision, so xorshift seeded from the clock is plenty.
struct Roll(u64);

impl Roll {
    fn seeded() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or(0x2545_F491_4F6C_DD1D);
        Self(seed | 1)
    }

    /// The next number in `0..1`.
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

// ── commands ──────────────────────────────────────────────────────

/// What the run is doing, polled by the tab.
#[tauri::command]
pub fn typer_status(holder: State<'_, Holder>) -> Status {
    holder.view()
}

#[tauri::command]
pub fn typer_pause(holder: State<'_, Holder>) -> Status {
    holder.current().pause()
}

#[tauri::command]
pub fn typer_resume(holder: State<'_, Holder>) -> Status {
    holder.current().resume()
}

/// Stop the run, and leave the status saying so.
#[tauri::command]
pub fn typer_stop(runtime: State<'_, RuntimeState>, holder: State<'_, Holder>) -> Status {
    if runtime.stop() {
        holder.current().stopped()
    } else {
        holder.view()
    }
}

/// The estimate for a draft, so the tab can show the pace before a run starts.
#[tauri::command]
pub fn typer_plan(text: String, settings: TyperSettings) -> TyperPlan {
    plan(&text, &settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deadline_sets_the_pace_and_is_clamped() {
        // 100 characters in 20 seconds is 5 a second.
        let settings = TyperSettings {
            deadline_secs: Some(20),
            typo_rate: 0.0,
            ..TyperSettings::default()
        };
        let estimate = plan(&"a".repeat(100), &settings);
        assert!((estimate.cps - 5.0).abs() < 0.001, "{estimate:?}");
        assert!(!estimate.clamped);
        assert_eq!(estimate.chars, 100);
        assert!((estimate.total_ms as f64 - 20_000.0).abs() < 1.0, "{estimate:?}");

        // 100 characters in one second is past what the engine will pace.
        let rushed = plan(
            &"a".repeat(100),
            &TyperSettings {
                deadline_secs: Some(1),
                ..TyperSettings::default()
            },
        );
        assert!(rushed.clamped);
        assert!((rushed.cps - MAX_CPS).abs() < 0.001, "{rushed:?}");
    }

    #[test]
    fn the_resting_pace_counts_five_characters_a_word() {
        let estimate = plan(
            "hello world",
            &TyperSettings {
                wpm: 60.0,
                typo_rate: 0.0,
                ..TyperSettings::default()
            },
        );
        // 60 words a minute is 300 characters a minute, which is 5 a second.
        assert!((estimate.cps - 5.0).abs() < 0.001, "{estimate:?}");
        assert_eq!(estimate.words, 2);
        assert_eq!(estimate.chars, 11);
    }

    #[test]
    fn a_mistype_is_a_neighbour_and_never_the_character_itself() {
        let settings = TyperSettings::default();
        for character in ['a', 'q', 'm', 'Z', '7', ','] {
            for step in 0..8 {
                let roll = step as f64 / 8.0;
                let wrong = wrong_for(character, roll);
                assert_ne!(
                    wrong.to_ascii_lowercase(),
                    character.to_ascii_lowercase(),
                    "{character} mistyped as itself"
                );
            }
        }
        assert!(plan("x", &settings).chars == 1);
    }

    #[test]
    fn the_clock_only_counts_while_a_run_is_typing() {
        let counters = Progress::new("idle", 10, 0);
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(counters.view().elapsed_secs, 0.0, "an idle tab kept counting");

        counters.begin_typing();
        std::thread::sleep(Duration::from_millis(40));
        let typing = counters.view().elapsed_secs;
        assert!(typing >= 0.02, "the clock did not start with the run: {typing}");

        counters.set("done");
        let finished = counters.view().elapsed_secs;
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(
            counters.view().elapsed_secs,
            finished,
            "the clock kept running after the run ended"
        );
    }

    #[test]
    fn a_pause_is_not_counted_as_typing_time() {
        let counters = Progress::new("typing", 10, 0);
        counters.begin_typing();
        std::thread::sleep(Duration::from_millis(20));
        counters.pause();
        std::thread::sleep(Duration::from_millis(120));
        let while_paused = counters.view().elapsed_secs;
        assert!(while_paused < 0.1, "paused time counted: {while_paused}");
        counters.resume();
        assert!(counters.view().elapsed_secs < 0.1);
    }

    #[test]
    fn a_typo_rate_past_the_cap_is_held_to_it() {
        let settings = TyperSettings {
            typo_rate: 9.0,
            ..TyperSettings::default()
        };
        assert!((settings.typo() - MAX_TYPO).abs() < f64::EPSILON);
    }
}

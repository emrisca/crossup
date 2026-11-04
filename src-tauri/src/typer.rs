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
/// The shortest stop a thinking pause can be, and the longest one the setting
/// may ask for. The floor keeps a pause visible instead of a stutter.
const THINK_MIN_SECS: f64 = 0.35;
const MAX_THINK_SECS: f64 = 8.0;
const DEFAULT_THINK_SECS: f64 = 2.5;
/// How long a sentence may run before the typer stops inside it: the pause
/// lands after somewhere in this many words, counted from the last pause.
/// Fifteen reads as a comma; thirty is a paragraph written without a full stop.
const LONG_SENTENCE_MIN_WORDS: u32 = 15;
const LONG_SENTENCE_MAX_WORDS: u32 = 30;
/// The largest share the pace may wander from the asked speed, either side.
const MAX_PACE_DRIFT: f64 = 0.5;
/// What a fresh typer wanders. It was a twelfth of the pace, which averaged out
/// over a sentence to a metronome; a fifth is the first share a watcher can see.
const DEFAULT_PACE_DRIFT: f64 = 0.2;
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
    /// Whether a run stops between words at all. The two switches below say
    /// where those stops land.
    pub think_pauses: bool,
    /// The longest such stop, in seconds. A run picks a shorter one at random.
    pub think_max_secs: f64,
    /// Whether a run stops after a full stop, the beat before the next thought.
    pub think_after_sentence: bool,
    /// Whether a run also stops inside a sentence that runs on, after a random
    /// fifteen to thirty words.
    pub think_in_long_sentence: bool,
    /// Whether a mistake is left standing and gone back for after a sentence or
    /// three, instead of being erased where it is made.
    pub typo_defer: bool,
    /// How far each word's pace may sit from the one on the slider, as a share
    /// either side. Zero is a metronome; a fifth is a person losing the rhythm
    /// and finding it again, and a half is the ceiling the engine allows.
    pub pace_variation: f64,
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
            think_pauses: true,
            think_max_secs: DEFAULT_THINK_SECS,
            think_after_sentence: true,
            think_in_long_sentence: true,
            typo_defer: false,
            pace_variation: DEFAULT_PACE_DRIFT,
        }
    }
}

impl TyperSettings {
    /// The share of characters this run will mistype, kept inside what the
    /// engine can pace.
    fn typo(&self) -> f64 {
        self.typo_rate.clamp(0.0, MAX_TYPO)
    }

    /// The longest thinking pause this run may take, inside what still reads as
    /// a pause rather than a hang.
    fn think_max(&self) -> f64 {
        self.think_max_secs.clamp(THINK_MIN_SECS, MAX_THINK_SECS)
    }

    /// How far this run's pace may wander either side of the asked one.
    fn pace_variation(&self) -> f64 {
        self.pace_variation.clamp(0.0, MAX_PACE_DRIFT)
    }

    /// Whether this run stops between words at all: the master switch, and at
    /// least one place for the stops to land.
    fn pausing(&self) -> bool {
        self.think_pauses && (self.think_after_sentence || self.think_in_long_sentence)
    }

    /// The pauses a draft this long is likely to take: one per sentence, plus
    /// one every nineteen words or so of the sentences still running.
    fn thinking_pauses(&self, words: u32, sentences: u32) -> f64 {
        if !self.pausing() {
            return 0.0;
        }
        let mut pauses = 0.0;
        if self.think_after_sentence {
            pauses += sentences as f64;
        }
        if self.think_in_long_sentence {
            let inside = words.saturating_sub(sentences) as f64;
            pauses += inside / long_sentence_gap_mid();
        }
        pauses
    }

    /// What those pauses are likely to cost, in seconds: the middle of the
    /// pause range, the same average the estimate has always used.
    fn thinking_secs(&self, words: u32, sentences: u32) -> f64 {
        self.thinking_pauses(words, sentences) * (THINK_MIN_SECS + self.think_max()) / 2.0
    }

    /// The pace the run will keep, in keystrokes a second, and whether a
    /// deadline had to be slowed down to reach it.
    ///
    /// Thinking comes out of a deadline's time rather than adding to it: a
    /// deadline promises the last keystroke lands on time, so the keystrokes
    /// get what the pauses leave.
    fn cps(&self, chars: u32, words: u32, sentences: u32) -> (f64, bool) {
        let resting = (self.wpm.clamp(5.0, 400.0) * CHARS_A_WORD) / 60.0;
        match self.deadline_secs {
            Some(secs) if chars > 0 => {
                let thinking = self.thinking_secs(words, sentences);
                let typing = (secs.max(1) as f64 - thinking).max(0.5);
                let asked = chars as f64 / typing;
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
    /// How much of that is the run stopping to think between words.
    pub thinking_ms: u64,
    /// Whether a deadline asked for a pace faster than the engine will go.
    pub clamped: bool,
}

/// The full stops that end a sentence rather than a decimal: a mark only counts
/// when a space or the end of the draft follows it, so "3.5" is not a sentence.
pub fn count_sentences(text: &str) -> u32 {
    let mut count = 0;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if !matches!(character, '.' | '!' | '?') {
            continue;
        }
        if characters.peek().map_or(true, |next| next.is_whitespace()) {
            count += 1;
        }
    }
    count
}

pub fn plan(text: &str, settings: &TyperSettings) -> TyperPlan {
    let chars = text.chars().count() as u32;
    let words = text.split_whitespace().count() as u32;
    let sentences = count_sentences(text);
    let (cps, clamped) = settings.cps(chars, words, sentences);
    let char_ms = 1_000.0 / cps;
    // A correction costs three keystrokes and two of its own gaps, so it is
    // worth more time than the character it stands in for.
    let corrections = (chars as f64 * settings.typo()).round();
    let thinking_ms = (settings.thinking_secs(words, sentences) * 1_000.0).round();
    // A mistake fixed where it is made costs the wrong letters, the beat where
    // it is noticed, the backspaces and the breath before the retype — about
    // four and a half key gaps, measured against the driver's own waits.
    //
    // A mistake left for later costs what it always costs a person: everything
    // written since has to come off and go back on. Two sentences is the middle
    // of the one-to-three window the run waits, and a backspace runs at a little
    // under half a key gap.
    let retyped = if settings.typo_defer {
        (2.0 * chars as f64 / sentences.max(1) as f64).min(chars as f64)
    } else {
        0.0
    };
    let corrections_ms = if settings.typo_defer {
        corrections * (retyped * char_ms * 1.45 + 560.0)
    } else {
        corrections * (char_ms * 4.5 + 235.0)
    };
    let total_ms = (chars as f64 * char_ms + corrections_ms + thinking_ms).round();
    TyperPlan {
        chars,
        words,
        cps,
        total_ms: total_ms.max(0.0) as u64,
        thinking_ms: thinking_ms.max(0.0) as u64,
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
    /// True while the run is stopped between words, thinking about the next
    /// one. It is not a pause: the person did not ask for it.
    pub thinking: bool,
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
    thinking: AtomicBool,
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
            thinking: AtomicBool::new(false),
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
            thinking: self.thinking.load(Ordering::Relaxed),
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
        // A finished run keeps the time it took rather than counting on, and it
        // is no longer thinking about anything.
        if matches!(status, "done" | "stopped" | "unfocused") {
            self.timing.lock().unwrap().freeze();
            self.thinking.store(false, Ordering::Relaxed);
        }
    }

    fn bump(&self) {
        self.typed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Say the run has stopped between words to think about the next one.
    pub fn think(&self, on: bool) {
        self.thinking.store(on, Ordering::Relaxed);
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
    let think = settings.pausing().then(|| settings.think_max());
    // A deadline promises the last keystroke lands on time, so it keeps the
    // exact pace: a wandering one would land the run early or late.
    let pace = if settings.deadline_secs.is_some() {
        0.0
    } else {
        settings.pace_variation()
    };
    let after_sentence = settings.think_after_sentence;
    let long_sentence = settings.think_in_long_sentence;
    let defer = settings.typo_defer;
    let countdown = settings.countdown_secs.min(30);
    let shared = Arc::clone(runtime);
    // The thread gets its own handle: the error arm below still needs the
    // holder's copy to say the run never started.
    let counters = Arc::clone(&progress);

    std::thread::Builder::new()
        .name("crossup-typer".into())
        .spawn(move || {
            let outcome = drive(
                &characters,
                char_ms,
                pace,
                typo,
                defer,
                think,
                after_sentence,
                long_sentence,
                countdown,
                &stop,
                &counters,
            );
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
    pace: f64,
    typo: f64,
    defer: bool,
    think: Option<f64>,
    after_sentence: bool,
    long_sentence: bool,
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
    // Each word gets its own pace, a shade either side of the asked one, so a
    // paragraph speeds up and eases off the way a person does. The share comes
    // from the tab: zero keeps a metronome, a quarter is loose.
    let mut drift = 1.0;
    // Words typed since the last pause, and how many the next one waits for.
    let mut words_since_pause = 0u32;
    let mut next_gap = long_sentence_gap(&mut roll);
    // The mistake the run left standing and means to come back for: where it
    // sits in the draft, and how many sentence ends are still to pass first.
    let mut deferred: Option<(usize, u32)> = None;
    for (index, character) in characters.iter().enumerate() {
        if index == 0 || characters[index - 1].is_whitespace() {
            drift = drift_for(roll.next(), pace);
        }
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
        // A mistake left for later is typed and walked away from. Once the
        // sentences it waits for have ended, the run goes back: everything
        // written since comes off, the wrong letter with it, and the stretch is
        // typed again.
        if let Some((at, 0)) = deferred {
            if !go_back(characters, at, index, char_ms, drift, &mut roll, stop) {
                return "stopped";
            }
            deferred = None;
        }
        // A mistype fixed where it is made is a wrong letter or a short run of
        // them, typed at the pace the rest of the word is going at, then noticed
        // — a beat longer than a key gap — and erased one backspace at a time
        // before the right letters go out again. One key swapped for another in
        // an instant reads as a machine; a burst reads as a hand ahead of an eye.
        let mut sent = false;
        if character.is_alphanumeric() && roll.next() < typo {
            if defer && deferred.is_none() {
                let wrong = wrong_for(*character, roll.next());
                input::text(&wrong.to_string());
                // One to three sentence ends from here before the run returns.
                deferred = Some((index, 1 + ((roll.next() * 3.0) as u32).min(2)));
                sent = true;
            } else {
                let burst = typo_burst(characters, index, roll.next());
                for step in 0..burst {
                    let wrong = wrong_for(characters[index + step], roll.next());
                    input::text(&wrong.to_string());
                    if !wait(stop, char_ms * (0.85 + roll.next() * 0.45) * drift) {
                        return "stopped";
                    }
                }
                // The pause where the mistake registers.
                if !wait(stop, char_ms * (0.8 + roll.next() * 1.2)) {
                    return "stopped";
                }
                for _ in 0..burst {
                    tap(BACKSPACE);
                    if !wait(stop, char_ms * (0.6 + roll.next() * 0.8)) {
                        return "stopped";
                    }
                }
                // A breath before the retype, so the correction is not one blur.
                if !wait(stop, 150.0 + roll.next() * 170.0) {
                    return "stopped";
                }
            }
        }
        if !sent {
            match character {
                '\n' => tap(ENTER),
                '\t' => tap(TAB),
                other => input::text(&other.to_string()),
            }
        }
        progress.bump();
        // A little spread either side of the word's pace, scaled by the same
        // fluctuation: the same gap every time is what reads as a machine.
        // A sentence end brings a deferred mistake one step closer; a decimal
        // point does not, the same rule the planner counts sentences by.
        if matches!(character, '.' | '!' | '?')
            && characters.get(index + 1).map_or(true, |next| next.is_whitespace())
        {
            if let Some((_, left)) = deferred.as_mut() {
                *left = left.saturating_sub(1);
            }
        }
        let gap = char_ms * drift * (1.0 - pace + roll.next() * pace * 2.0);
        if !wait(stop, gap) {
            return "stopped";
        }
        // Pauses land after the space that ends a word, where a person stops:
        // at a full stop, or after fifteen to thirty words of a sentence that
        // has not ended yet. The pause is picked from the range the settings
        // allow, and Stop still lands inside it.
        if character.is_whitespace() {
            if let Some(longest) = think {
                let ended = index > 0 && matches!(characters[index - 1], '.' | '!' | '?');
                words_since_pause += 1;
                let due = (ended && after_sentence)
                    || (!ended && long_sentence && words_since_pause >= next_gap);
                if due {
                    words_since_pause = 0;
                    next_gap = long_sentence_gap(&mut roll);
                    let held = THINK_MIN_SECS + roll.next() * (longest - THINK_MIN_SECS);
                    progress.think(true);
                    let alive = wait(stop, held * 1_000.0);
                    progress.think(false);
                    if !alive {
                        return "stopped";
                    }
                }
            }
        }
    }
    // The draft ran out before the sentences that would have taken the run back
    // to its last mistake, so it goes back now rather than leaving the text wrong.
    if let Some((at, _)) = deferred {
        if !go_back(characters, at, characters.len(), char_ms, drift, &mut roll, stop) {
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

/// How far this word's pace sits from the asked one: the middle of the range at
/// half a roll, the edges at zero and one. A share of zero keeps it at one,
/// which is the metronome.
fn drift_for(roll: f64, share: f64) -> f64 {
    if share <= 0.0 {
        return 1.0;
    }
    1.0 - share + roll * share * 2.0
}

/// The middle of the word range a long-sentence pause waits for. The estimate
/// uses the average; the run itself picks a fresh gap every time.
fn long_sentence_gap_mid() -> f64 {
    (LONG_SENTENCE_MIN_WORDS + LONG_SENTENCE_MAX_WORDS) as f64 / 2.0
}

/// How many words the run waits before stopping inside a long sentence.
fn long_sentence_gap(roll: &mut Roll) -> u32 {
    let span = LONG_SENTENCE_MAX_WORDS - LONG_SENTENCE_MIN_WORDS + 1;
    LONG_SENTENCE_MIN_WORDS + ((roll.next() * span as f64) as u32).min(span - 1)
}

/// Erase back to a mistake and type that stretch again.
///
/// This is what a person does when the error is a sentence behind them: the
/// backspaces run through everything written since, a beat passes while it is
/// read again, and then the right text goes out at the pace the run has been
/// keeping. Returns false when the run was stopped.
fn go_back(
    characters: &[char],
    at: usize,
    typed: usize,
    char_ms: f64,
    drift: f64,
    roll: &mut Roll,
    stop: &AtomicBool,
) -> bool {
    for _ in at..typed {
        tap(BACKSPACE);
        if !wait(stop, char_ms * (0.45 + roll.next() * 0.3)) {
            return false;
        }
    }
    if !wait(stop, 320.0 + roll.next() * 280.0) {
        return false;
    }
    for character in &characters[at..typed] {
        match character {
            '\n' => tap(ENTER),
            '\t' => tap(TAB),
            other => input::text(&other.to_string()),
        }
        if !wait(stop, char_ms * (0.9 + roll.next() * 0.3) * drift) {
            return false;
        }
    }
    true
}

/// How many characters a mistype takes with it: usually one, sometimes two or
/// three, and never more than the run of letters and digits that follows. The
/// retyped characters go out through the ordinary path, at the ordinary pace.
fn typo_burst(characters: &[char], index: usize, roll: f64) -> usize {
    let wanted = if roll < 0.6 {
        1
    } else if roll < 0.9 {
        2
    } else {
        3
    };
    let available = characters[index..]
        .iter()
        .take(3)
        .take_while(|character| character.is_alphanumeric())
        .count();
    wanted.min(available).max(1)
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
            think_pauses: false,
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
                think_pauses: false,
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
                think_pauses: false,
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
    fn a_mistake_left_for_later_costs_the_stretch_it_walks_back_over() {
        let text = "One sentence that runs on for a while here. Another follows it for good measure.";
        let immediate = plan(
            text,
            &TyperSettings {
                typo_rate: 0.1,
                ..TyperSettings::default()
            },
        );
        let deferred = plan(
            text,
            &TyperSettings {
                typo_rate: 0.1,
                typo_defer: true,
                ..TyperSettings::default()
            },
        );
        // The same run, longer for what it has to type twice.
        assert_eq!(deferred.chars, immediate.chars);
        assert_eq!(deferred.thinking_ms, immediate.thinking_ms);
        assert!(
            deferred.total_ms > immediate.total_ms,
            "{deferred:?} vs {immediate:?}"
        );
    }

    #[test]
    fn a_mistype_is_a_short_run_of_letters_and_never_past_the_word() {
        let word: Vec<char> = "abcdef".chars().collect();
        let spaced: Vec<char> = "ab cd".chars().collect();
        assert_eq!(typo_burst(&word, 0, 0.0), 1);
        assert_eq!(typo_burst(&word, 0, 0.7), 2);
        assert_eq!(typo_burst(&word, 0, 0.95), 3);
        // The space ends the run, whatever the roll asks for.
        assert_eq!(typo_burst(&spaced, 0, 0.95), 2);
        // Starting near the end takes only what is left.
        assert_eq!(typo_burst(&word, 5, 0.95), 1);
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
    fn thinking_is_budgeted_in_the_estimate_and_can_be_switched_off() {
        let text = "one two three four five six seven eight nine ten";
        let on = plan(
            text,
            &TyperSettings {
                wpm: 60.0,
                typo_rate: 0.0,
                ..TyperSettings::default()
            },
        );
        let off = plan(
            text,
            &TyperSettings {
                wpm: 60.0,
                typo_rate: 0.0,
                think_pauses: false,
                ..TyperSettings::default()
            },
        );
        assert!(on.thinking_ms > 0, "{on:?}");
        assert_eq!(off.thinking_ms, 0);
        assert!(on.total_ms > off.total_ms, "a pause costs no time: {on:?}");
    }

    #[test]
    fn a_deadline_absorbs_thinking_rather_than_missing_it() {
        let estimate = plan(
            "a b c d e f g h i j",
            &TyperSettings {
                deadline_secs: Some(20),
                typo_rate: 0.0,
                ..TyperSettings::default()
            },
        );
        assert!(estimate.thinking_ms > 0, "{estimate:?}");
        // The last keystroke still lands on the deadline: the pauses come out
        // of the time rather than being added after it.
        assert!(
            (estimate.total_ms as f64 - 20_000.0).abs() < 60.0,
            "{estimate:?}"
        );
    }

    #[test]
    fn the_pace_wanders_within_the_drift_and_no_further() {
        for step in 0..=32 {
            let drift = drift_for(step as f64 / 32.0, DEFAULT_PACE_DRIFT);
            assert!(
                (1.0 - DEFAULT_PACE_DRIFT - 1e-9..=1.0 + DEFAULT_PACE_DRIFT + 1e-9)
                    .contains(&drift),
                "{drift}"
            );
        }
        assert!(drift_for(0.0, DEFAULT_PACE_DRIFT) < drift_for(1.0, DEFAULT_PACE_DRIFT));
        // No fluctuation is a metronome, whatever the roll says.
        assert_eq!(drift_for(0.93, 0.0), 1.0);
    }

    #[test]
    fn the_pace_variation_is_held_to_what_the_engine_can_do() {
        let wide = TyperSettings {
            pace_variation: 9.0,
            ..TyperSettings::default()
        };
        assert!((wide.pace_variation() - MAX_PACE_DRIFT).abs() < f64::EPSILON);
        let none = TyperSettings {
            pace_variation: -1.0,
            ..TyperSettings::default()
        };
        assert_eq!(none.pace_variation(), 0.0);
    }

    #[test]
    fn a_sentence_is_a_full_stop_and_a_decimal_point_is_not() {
        assert_eq!(count_sentences("One. Two! Three? "), 3);
        assert_eq!(count_sentences("It costs 3.5 dollars. That is all"), 1);
        assert_eq!(count_sentences("no marks here"), 0);
    }

    #[test]
    fn a_long_sentence_gap_is_between_fifteen_and_thirty_words() {
        let mut roll = Roll::seeded();
        for _ in 0..64 {
            let gap = long_sentence_gap(&mut roll);
            assert!(
                (LONG_SENTENCE_MIN_WORDS..=LONG_SENTENCE_MAX_WORDS).contains(&gap),
                "{gap}"
            );
        }
    }

    #[test]
    fn each_kind_of_pause_costs_time_and_both_together_cost_more() {
        let text = "one two three. four five six. seven eight nine";
        let base = TyperSettings {
            wpm: 60.0,
            typo_rate: 0.0,
            think_after_sentence: false,
            think_in_long_sentence: false,
            ..TyperSettings::default()
        };
        let sentences = TyperSettings {
            think_after_sentence: true,
            ..base.clone()
        };
        let inside = TyperSettings {
            think_in_long_sentence: true,
            ..base.clone()
        };
        let both = TyperSettings {
            think_after_sentence: true,
            think_in_long_sentence: true,
            ..base.clone()
        };
        let none = plan(text, &base);
        let stops = plan(text, &sentences);
        let running = plan(text, &inside);
        let all = plan(text, &both);
        assert_eq!(none.thinking_ms, 0, "{none:?}");
        assert!(stops.thinking_ms > 0, "{stops:?}");
        assert!(running.thinking_ms > 0, "{running:?}");
        assert!(all.thinking_ms > stops.thinking_ms, "{all:?} {stops:?}");
    }

    #[test]
    fn a_thinking_pause_never_exceeds_its_setting() {
        let settings = TyperSettings {
            think_max_secs: 99.0,
            ..TyperSettings::default()
        };
        assert!((settings.think_max() - MAX_THINK_SECS).abs() < f64::EPSILON);
        let tiny = TyperSettings {
            think_max_secs: 0.0,
            ..TyperSettings::default()
        };
        assert!((tiny.think_max() - THINK_MIN_SECS).abs() < f64::EPSILON);
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

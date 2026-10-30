//! The auto clicker engine.
//!
//! The promise this has to keep is the one inaccurate auto clickers break:
//! *click at the rate that was asked for*. Two things make that work.
//!
//! 1. **Absolute deadlines.** Every click's due time is computed from the run's
//!    start (`start + n * interval`), never as `now + interval`. A late wake-up
//!    therefore does not push every later click late with it; the engine sends
//!    the clicks that have come due and carries on from the original grid.
//! 2. **A 1 ms timer.** Windows' default scheduler tick is about 15.6 ms, which
//!    quietly turns "click every 2 ms" into "click every 16 ms". The engine asks
//!    the kernel for a 1 ms resolution for the length of the run
//!    ([`crate::input::TimerResolution`]), then sleeps and spins to land on the
//!    grid.
//!
//! Everything Windows-shaped lives behind [`Machine`], so the whole engine runs
//! in tests against a fake clock with no mouse attached.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::input::{self, MouseButton, TimerResolution};

// ── settings ───────────────────────────────────────────────────────

/// What the clicker presses: a mouse button or a keyboard key.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum ClickInput {
    #[default]
    Mouse,
    Keyboard,
}

/// The unit a rate is expressed in, so "2 clicks per day" is expressible.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum RateUnit {
    #[default]
    Second,
    Minute,
    Hour,
    Day,
}

impl RateUnit {
    pub fn milliseconds(self) -> f64 {
        match self {
            RateUnit::Second => 1_000.0,
            RateUnit::Minute => 60_000.0,
            RateUnit::Hour => 3_600_000.0,
            RateUnit::Day => 86_400_000.0,
        }
    }

}

/// One place the clicker can click, with a per-point budget and a scatter
/// radius: click here `clicks` times, then move to the next point.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClickPoint {
    pub x: i32,
    pub y: i32,
    pub clicks: u32,
    pub radius: u32,
}

/// One watched boundary: whether the run stops there, and how many pixels in
/// from it that starts counting.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Guard {
    pub on: bool,
    pub margin: u32,
}

impl Guard {
    fn new(margin: u32) -> Self {
        Self { on: true, margin }
    }
}

impl Default for Guard {
    fn default() -> Self {
        Self::new(40)
    }
}

/// Stop the run when the cursor reaches a screen edge. This is the panic
/// button: the cursor is what you reach for when something is clicking too fast.
///
/// Each side is watched and measured on its own, because the sides are not
/// interchangeable — the bottom of the screen is usually under a taskbar, so it
/// wants a wider berth than the top, and a side the run is meant to work along
/// can be switched off entirely.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct EdgeStop {
    pub enabled: bool,
    pub top: Guard,
    pub right: Guard,
    pub bottom: Guard,
    pub left: Guard,
}

impl Default for EdgeStop {
    fn default() -> Self {
        Self {
            enabled: false,
            top: Guard::default(),
            right: Guard::default(),
            bottom: Guard::default(),
            left: Guard::default(),
        }
    }
}

/// Stop the run when the cursor reaches a screen corner: the same idea as the
/// edges, for the place a cursor ends up when it is flung at the screen.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct CornerStop {
    pub enabled: bool,
    /// How far into a corner counts as being in it. One per corner, because a
    /// corner with a dock or a second taskbar on it is not the same target as a
    /// bare one.
    pub top_left: Guard,
    pub top_right: Guard,
    pub bottom_left: Guard,
    pub bottom_right: Guard,
}

impl Default for CornerStop {
    fn default() -> Self {
        Self {
            enabled: false,
            top_left: Guard::new(50),
            top_right: Guard::new(50),
            bottom_left: Guard::new(50),
            bottom_right: Guard::new(50),
        }
    }
}

/// What a zone does to a run while the cursor is inside it.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum ZoneAction {
    /// End the run.
    #[default]
    Stop,
    /// Hold the run where it is until the cursor leaves the zone.
    Pause,
    /// Carry on inside a zone that would otherwise pause the run.
    ///
    /// This is not a start button — a run that has ended cannot be restarted
    /// from a zone, because nothing is left watching the cursor — it is a hole
    /// in a pause zone.
    Start,
}

/// A rectangle on the desktop that does something to the run when the cursor
/// enters it.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StopZone {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub action: ZoneAction,
}

impl StopZone {
    /// Is the cursor inside this zone? The far edge is excluded, so two zones
    /// sharing an edge do not both claim the pixel on the seam.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        contains(
            (self.x, self.y, self.width as i32, self.height as i32),
            (x, y),
        )
    }
}

/// Is a point inside a rectangle? Half-open, so a desktop made of two monitors
/// side by side reports the cursor on exactly one of them.
fn contains(
    (origin_x, origin_y, width, height): (i32, i32, i32, i32),
    (x, y): (i32, i32),
) -> bool {
    width > 0
        && height > 0
        && x >= origin_x
        && x < origin_x + width
        && y >= origin_y
        && y < origin_y + height
}

/// Only click while the listed programs are in front.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum TargetMode {
    /// Click only in these programs.
    #[default]
    Whitelist,
    /// Click anywhere except in these programs.
    Blacklist,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Targets {
    pub enabled: bool,
    pub mode: TargetMode,
    pub programs: Vec<String>,
}

impl Default for Targets {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: TargetMode::Whitelist,
            programs: Vec::new(),
        }
    }
}

/// Hardest rate a run may be configured for, counted as *clicks* (so a double
/// click carries twice its rate). Windows bottoms out near here; above it the
/// achieved figure stops meaning anything, which is why it is refused rather
/// than quietly missed.
pub const MAX_CPS: f64 = 1_000.0;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ClickerSettings {
    pub input: ClickInput,
    pub button: MouseButton,
    /// Scan code of the key, when `input` is `Keyboard`.
    pub key_scan: u16,
    /// Hold Shift with a letter key, for programs that read the shifted
    /// character rather than the physical key.
    pub uppercase: bool,
    pub rate_unit: RateUnit,
    /// Clicks per `rate_unit`, e.g. 12.5 clicks per second. With a double click
    /// set, each cycle carries two clicks at this rate.
    pub rate: f64,
    /// How much of the interval the button is held down, as a percentage. 0
    /// means a plain press-and-release, which is the fast path.
    pub duty: f64,
    /// Random spread on the interval, as a percentage either side.
    pub variation: f64,
    /// Stop after this many clicks. 0 means no limit.
    pub max_clicks: i64,
    /// Stop after this many seconds. 0 means no limit.
    pub time_limit_secs: f64,
    pub double_click: bool,
    pub double_click_gap_ms: u32,
    /// Click these places instead of wherever the cursor happens to be.
    pub points: Vec<ClickPoint>,
    /// How long the cursor takes to travel to a point.
    pub move_duration_ms: u32,
    /// Scatter within each point's radius, and travel rather than teleport.
    pub humanize: bool,
    pub edge_stop: EdgeStop,
    pub corner_stop: CornerStop,
    /// Rectangles that stop, pause or carry on the run as the cursor enters
    /// them. Watched only when `zones_enabled` is set.
    pub stop_zones: Vec<StopZone>,
    pub zones_enabled: bool,
    pub targets: Targets,
    /// Global shortcut that starts and stops the run, e.g. `F6`.
    pub hotkey: String,
    /// Run while the hotkey is held rather than toggling it.
    pub hold_mode: bool,
}

impl Default for ClickerSettings {
    fn default() -> Self {
        Self {
            input: ClickInput::Mouse,
            button: MouseButton::Left,
            key_scan: 0x39, // Spacebar
            uppercase: false,
            rate_unit: RateUnit::Second,
            rate: 10.0,
            duty: 0.0,
            variation: 0.0,
            max_clicks: 0,
            time_limit_secs: 0.0,
            double_click: false,
            double_click_gap_ms: 120,
            points: Vec::new(),
            move_duration_ms: 120,
            humanize: false,
            edge_stop: EdgeStop::default(),
            corner_stop: CornerStop::default(),
            stop_zones: Vec::new(),
            zones_enabled: false,
            targets: Targets::default(),
            // No shortcut out of the box. A fresh install used to claim F6
            // before the person had asked for anything, which collides with
            // whatever else on their machine already owns that key.
            hotkey: String::new(),
            hold_mode: false,
        }
    }
}

// ── pure maths, so the rate can be tested without clicking ─────────

/// Clicks per second this configuration asks for.
///
/// The rate is stored in whatever unit the person chose — two clicks a *minute*
/// is a real setting — so this is where it becomes comparable with the measured
/// rate. A double click carries two clicks per cycle, which is why it is not
/// simply the number in the box.
pub fn clicks_per_second(settings: &ClickerSettings) -> f64 {
    let rate = if settings.rate.is_finite() {
        settings.rate.max(0.0)
    } else {
        0.0
    };
    let per_cycle = if settings.double_click { 2.0 } else { 1.0 };
    rate * per_cycle * 1_000.0 / settings.rate_unit.milliseconds()
}

/// Milliseconds between click *cycles*. With a double click set, one cycle
/// carries both presses.
pub fn cycle_ms(settings: &ClickerSettings) -> f64 {
    let unit = settings.rate_unit.milliseconds();
    let rate = if settings.rate.is_finite() && settings.rate > 0.0 {
        settings.rate
    } else {
        1.0
    };
    (unit / rate).max(1.0)
}

/// How long the press is held inside a cycle, in milliseconds.
pub fn hold_ms(settings: &ClickerSettings, cycle: f64) -> f64 {
    // A duty cycle that swallowed the whole interval would leave no room to
    // release before the next press, which reads as a stuck button.
    let duty = settings.duty.clamp(0.0, 99.0);
    (cycle * duty / 100.0).max(0.0)
}

/// Milliseconds between a double click's two presses, kept inside its cycle.
pub fn double_gap_ms(settings: &ClickerSettings, cycle: f64) -> f64 {
    let room = (cycle - 1.0).max(1.0);
    f64::from(settings.double_click_gap_ms).clamp(0.0, room)
}

/// Is this a run the engine can start at all?
pub fn validate(settings: &ClickerSettings) -> Result<(), String> {
    if !settings.rate.is_finite() || settings.rate <= 0.0 {
        return Err("the rate has to be a number above zero".to_string());
    }
    let cps = clicks_per_second(settings);
    if cps > MAX_CPS {
        return Err(format!(
            "that works out to {cps:.0} clicks per second; the most Windows will pace is {MAX_CPS:.0}"
        ));
    }
    if settings.input == ClickInput::Keyboard && settings.key_scan == 0 {
        return Err("pick a key to press".to_string());
    }
    if settings.points.iter().any(|point| point.clicks == 0) {
        return Err("every click point needs at least one click".to_string());
    }
    if settings
        .stop_zones
        .iter()
        .any(|zone| zone.width == 0 || zone.height == 0)
    {
        return Err("every zone needs a width and a height".to_string());
    }
    // A shortcut is optional. It used to be required, back when it was the only
    // way to stop a run; the Stop button and the Escape failsafe both end one
    // now, so insisting on a global key would be taking a key nobody asked it to
    // take.
    Ok(())
}

/// Does the foreground program pass the target list?
pub fn target_allows(targets: &Targets, foreground: &str) -> bool {
    if !targets.enabled {
        return true;
    }
    let listed = targets
        .programs
        .iter()
        .any(|program| program.trim().eq_ignore_ascii_case(foreground.trim()));
    match targets.mode {
        TargetMode::Whitelist => listed,
        TargetMode::Blacklist => !listed,
    }
}

/// Which edge of this monitor the cursor has reached, if any.
///
/// Everything is measured from the monitor's own rectangle, so on a two-screen
/// desktop a guard fires on the inside where the screens meet as well as on the
/// outside — which is the point. A flick of the wrist leaves by whichever
/// border is nearest, and a failsafe the cursor can escape through is not one.
pub fn edge_hit(
    cursor: (i32, i32),
    monitor: (i32, i32, i32, i32),
    edge: &EdgeStop,
) -> Option<&'static str> {
    if !edge.enabled || !contains(monitor, cursor) {
        return None;
    }
    let (origin_x, origin_y, width, height) = monitor;
    let right = origin_x + width - 1;
    let bottom = origin_y + height - 1;

    if edge.top.on && cursor.1 <= origin_y + edge.top.margin as i32 {
        return Some("the cursor reached the top edge");
    }
    if edge.right.on && cursor.0 >= right - edge.right.margin as i32 {
        return Some("the cursor reached the right edge");
    }
    if edge.bottom.on && cursor.1 >= bottom - edge.bottom.margin as i32 {
        return Some("the cursor reached the bottom edge");
    }
    if edge.left.on && cursor.0 <= origin_x + edge.left.margin as i32 {
        return Some("the cursor reached the left edge");
    }
    None
}

/// Which corner of this monitor the cursor is in, if any. A corner's distance
/// is used on both axes, so it is a square rather than an L.
pub fn corner_hit(
    cursor: (i32, i32),
    monitor: (i32, i32, i32, i32),
    corner: &CornerStop,
) -> Option<&'static str> {
    if !corner.enabled || !contains(monitor, cursor) {
        return None;
    }
    let (origin_x, origin_y, width, height) = monitor;
    let right = origin_x + width - 1;
    let bottom = origin_y + height - 1;

    let dx = |guard: &Guard| origin_x + guard.margin as i32;
    let dy = |guard: &Guard| origin_y + guard.margin as i32;
    let bx = |guard: &Guard| right - guard.margin as i32;
    let by = |guard: &Guard| bottom - guard.margin as i32;

    if corner.top_left.on && cursor.0 <= dx(&corner.top_left) && cursor.1 <= dy(&corner.top_left) {
        return Some("the cursor reached the top-left corner");
    }
    if corner.top_right.on
        && cursor.0 >= bx(&corner.top_right)
        && cursor.1 <= dy(&corner.top_right)
    {
        return Some("the cursor reached the top-right corner");
    }
    if corner.bottom_left.on
        && cursor.0 <= dx(&corner.bottom_left)
        && cursor.1 >= by(&corner.bottom_left)
    {
        return Some("the cursor reached the bottom-left corner");
    }
    if corner.bottom_right.on
        && cursor.0 >= bx(&corner.bottom_right)
        && cursor.1 >= by(&corner.bottom_right)
    {
        return Some("the cursor reached the bottom-right corner");
    }
    None
}

/// The zone the cursor is in, and which action wins there.
///
/// Zones overlap, so something has to win. A **stop** wins wherever it sits in
/// the list, because a stop zone is what you draw over everything else when you
/// want a way out and cannot be relied on to be first; a **carry on** beats a
/// pause, which is what makes a small clear area inside a large held one work;
/// otherwise the first pause found holds. The index comes back so the window and
/// the log can name the zone.
pub fn zone_at(cursor: (i32, i32), zones: &[StopZone]) -> Option<(ZoneAction, usize)> {
    let mut found: Option<(ZoneAction, usize)> = None;
    for (index, zone) in zones.iter().enumerate() {
        if !zone.contains(cursor.0, cursor.1) {
            continue;
        }
        match zone.action {
            ZoneAction::Stop => return Some((ZoneAction::Stop, index)),
            ZoneAction::Start => found = Some((ZoneAction::Start, index)),
            ZoneAction::Pause => {
                if found.is_none() {
                    found = Some((ZoneAction::Pause, index));
                }
            }
        }
    }
    found
}

/// The first failsafe the cursor has tripped, in the order they are checked.
///
/// Corners come before edges, because a corner sits inside two edges and the
/// more specific message is the useful one. Both are measured against each
/// monitor in turn, and only for the monitor the cursor is actually on, so a
/// screen the cursor is not on never fires its borders.
pub fn failsafe(
    cursor: (i32, i32),
    monitors: &[(i32, i32, i32, i32)],
    settings: &ClickerSettings,
) -> Option<&'static str> {
    for monitor in monitors.iter().copied() {
        if let Some(hit) = corner_hit(cursor, monitor, &settings.corner_stop) {
            return Some(hit);
        }
    }
    for monitor in monitors.iter().copied() {
        if let Some(hit) = edge_hit(cursor, monitor, &settings.edge_stop) {
            return Some(hit);
        }
    }
    None
}

// ── the escape failsafe ────────────────────────────────────────────

/// How many taps of Escape end a run.
const ESCAPE_TAPS: usize = 3;
/// How close together they have to be. Three stray taps during five minutes of
/// clicking should not stop anything; three in two thirds of a second is
/// somebody hitting the key on purpose.
const ESCAPE_WINDOW_MS: f64 = 700.0;

/// The keyboard way out of a runaway run.
///
/// The edge and corner failsafes need the cursor, which is the thing a runaway
/// run has taken away from you — the app's own start shortcut is no help either
/// when the game swallows it. Escape is read straight from the physical key
/// state (`GetAsyncKeyState`), so it works whatever has focus, whatever the
/// keyboard hook is doing, and without stealing the key from the program in
/// front. It only counts presses that start after the run did.
pub struct EscapeGuard {
    seen: u32,
    presses: Vec<f64>,
}

impl Default for EscapeGuard {
    fn default() -> Self {
        Self {
            seen: 0,
            presses: Vec::new(),
        }
    }
}

impl EscapeGuard {
    /// Forget every press that happened before the run started.
    pub fn baseline(&mut self, presses: u32) {
        self.seen = presses;
        self.presses.clear();
    }

    /// Fold in the current counter. True when three taps landed inside the
    /// window, which is the signal to stop.
    pub fn tripped(&mut self, now: f64, presses: u32) -> bool {
        while self.seen < presses {
            self.seen += 1;
            self.presses.push(now);
        }
        self.presses.retain(|at| now - at <= ESCAPE_WINDOW_MS);
        self.presses.len() >= ESCAPE_TAPS
    }
}

/// A small deterministic generator. Tests seed it; a run seeds it from the
/// clock, so two runs never scatter the same way.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    pub fn from_clock() -> Self {
        Self::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|age| age.as_nanos() as u64)
                .unwrap_or(0x9E37_79B9_7F4A_7C15),
        )
    }

    pub fn next_u64(&mut self) -> u64 {
        // xorshift64*: enough for scattering a cursor, and nothing security shaped.
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number in `0.0..1.0`.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A number in `-1.0..1.0`.
    pub fn next_signed(&mut self) -> f64 {
        self.next_f64() * 2.0 - 1.0
    }
}

/// The interval for one cycle, with the random variation applied.
pub fn jittered_cycle_ms(settings: &ClickerSettings, rng: &mut Rng) -> f64 {
    let cycle = cycle_ms(settings);
    let spread = settings.variation.clamp(0.0, 90.0) / 100.0;
    if spread <= 0.0 {
        return cycle;
    }
    (cycle * (1.0 + rng.next_signed() * spread)).max(1.0)
}

/// Round-robin over the click points, each served its own number of clicks.
pub struct PointRotation {
    points: Vec<ClickPoint>,
    index: usize,
    served: u32,
}

impl PointRotation {
    pub fn new(points: &[ClickPoint]) -> Self {
        Self {
            points: points.to_vec(),
            index: 0,
            served: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// The place to click next, scattered inside the point's radius when
    /// `humanize` is on.
    pub fn next(&mut self, humanize: bool, rng: &mut Rng) -> Option<(i32, i32)> {
        if self.points.is_empty() {
            return None;
        }
        let point = self.points[self.index].clone();
        self.served += 1;
        if self.served >= point.clicks.max(1) {
            self.served = 0;
            self.index = (self.index + 1) % self.points.len();
        }

        if !humanize || point.radius == 0 {
            return Some((point.x, point.y));
        }
        // Uniform over the disc rather than the square, so the corners of the
        // radius box are not over-visited.
        let angle = rng.next_f64() * std::f64::consts::TAU;
        let distance = f64::from(point.radius) * rng.next_f64().sqrt();
        Some((
            point.x + (angle.cos() * distance).round() as i32,
            point.y + (angle.sin() * distance).round() as i32,
        ))
    }
}

// ── the machine: everything with an operating system in it ─────────

/// What a click actually presses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Mouse(MouseButton),
    Key { scan: u16, uppercase: bool },
}

impl Source {
    pub fn of(settings: &ClickerSettings) -> Self {
        match settings.input {
            ClickInput::Mouse => Source::Mouse(settings.button),
            ClickInput::Keyboard => Source::Key {
                scan: settings.key_scan,
                uppercase: settings.uppercase,
            },
        }
    }

    pub fn label(self) -> String {
        match self {
            Source::Mouse(button) => format!("{} mouse button", button.label()),
            Source::Key { scan, uppercase } => format!(
                "{}{}",
                if uppercase { "Shift + " } else { "" },
                crate::keys::label(scan)
            ),
        }
    }
}

/// The engine's window onto the world. The Windows implementation drives
/// `SendInput`; tests drive a recording one with a clock they control.
pub trait Machine {
    /// A monotonic millisecond clock. Only differences are meaningful.
    fn now_ms(&mut self) -> f64;
    /// Wait until the deadline. Returns false when the run was stopped.
    fn wait_until(&mut self, deadline_ms: f64, stop: &AtomicBool) -> bool;
    fn press(&mut self, source: Source);
    fn release(&mut self, source: Source);
    /// `count` complete press/release pairs in one call.
    fn batch(&mut self, source: Source, count: u64);
    /// Move the cursor; a `duration_ms` of 0 jumps straight there.
    fn move_to(&mut self, x: i32, y: i32, duration_ms: u32);
    fn cursor(&mut self) -> (i32, i32);
    /// Every monitor's rectangle. The failsafes are measured per screen, so this
    /// is the list rather than one bounding box.
    fn monitors(&mut self) -> Vec<(i32, i32, i32, i32)>;
    fn foreground(&mut self) -> String;
    /// How many times Escape has gone down since the app started. A default of
    /// zero keeps every test machine free of keyboard state.
    fn escape_presses(&mut self) -> u32 {
        0
    }
}

/// The real thing.
pub struct WindowsMachine {
    epoch: Instant,
    /// Edge-detected Escape, so holding the key is still one press.
    escape_down: bool,
    escapes: u32,
}

impl WindowsMachine {
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
            escape_down: false,
            escapes: 0,
        }
    }
}

impl Default for WindowsMachine {
    fn default() -> Self {
        Self::new()
    }
}

/// Sleep most of the way, then spin for the last fraction of the interval.
///
/// `thread::sleep` overshoots by a millisecond or more even at a 1 ms timer, and
/// at 100 CPS a millisecond is 10% of the interval, so the tail is burned
/// spinning instead. The window is a share of the interval rather than a fixed
/// millisecond, because at 500 CPS spinning for a fixed 1.5 ms would mean
/// spinning through most of every cycle: there the sleep overshoot is cheaper
/// than the CPU, and the catch-up batch keeps the rate right anyway.
fn wait_precise(deadline_ms: f64, epoch: Instant, stop: &AtomicBool) -> bool {
    let now_ms = epoch.elapsed().as_secs_f64() * 1_000.0;
    let spin = ((deadline_ms - now_ms) / 8.0).clamp(0.2, 1.5);

    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let left = deadline_ms - epoch.elapsed().as_secs_f64() * 1_000.0;
        if left <= 0.0 {
            return true;
        }
        if left > spin {
            std::thread::sleep(std::time::Duration::from_secs_f64((left - spin) / 1_000.0));
        } else {
            std::hint::spin_loop();
        }
    }
}

impl Machine for WindowsMachine {
    fn now_ms(&mut self) -> f64 {
        self.epoch.elapsed().as_secs_f64() * 1_000.0
    }

    fn wait_until(&mut self, deadline_ms: f64, stop: &AtomicBool) -> bool {
        wait_precise(deadline_ms, self.epoch, stop)
    }

    fn press(&mut self, source: Source) {
        match source {
            Source::Mouse(button) => input::button(button, true),
            Source::Key { scan, uppercase } => {
                if uppercase {
                    input::key(0x2A, true);
                }
                input::key(scan, true);
            }
        }
    }

    fn release(&mut self, source: Source) {
        match source {
            Source::Mouse(button) => input::button(button, false),
            Source::Key { scan, uppercase } => {
                input::key(scan, false);
                if uppercase {
                    input::key(0x2A, false);
                }
            }
        }
    }

    fn batch(&mut self, source: Source, count: u64) {
        let count = count as usize;
        match source {
            Source::Mouse(button) => input::click_batch(button, count),
            Source::Key { scan, uppercase } => {
                if uppercase {
                    input::key(0x2A, true);
                }
                input::key_batch(scan, count);
                if uppercase {
                    input::key(0x2A, false);
                }
            }
        }
    }

    fn move_to(&mut self, x: i32, y: i32, duration_ms: u32) {
        if duration_ms == 0 {
            input::move_to(x, y);
            return;
        }
        // An ease-in-out crawl, so a move to a distant point reads as a hand
        // rather than a teleport. Steps are capped so long moves stay brisk.
        let (start_x, start_y) = input::cursor_position();
        let steps = (duration_ms / 8).clamp(4, 60);
        let step_ms = f64::from(duration_ms) / f64::from(steps);
        let begin = Instant::now();
        for step in 1..=steps {
            let t = f64::from(step) / f64::from(steps);
            let eased = if t < 0.5 {
                2.0 * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
            };
            let next_x = f64::from(start_x) + f64::from(x - start_x) * eased;
            let next_y = f64::from(start_y) + f64::from(y - start_y) * eased;
            input::move_to(next_x.round() as i32, next_y.round() as i32);

            let target = step_ms * f64::from(step);
            let elapsed = begin.elapsed().as_secs_f64() * 1_000.0;
            if target > elapsed {
                std::thread::sleep(std::time::Duration::from_secs_f64(
                    (target - elapsed) / 1_000.0,
                ));
            }
        }
        input::move_to(x, y);
    }

    fn cursor(&mut self) -> (i32, i32) {
        input::cursor_position()
    }

    fn monitors(&mut self) -> Vec<(i32, i32, i32, i32)> {
        input::monitors()
    }

    fn foreground(&mut self) -> String {
        crate::hook::foreground_program()
    }

    fn escape_presses(&mut self) -> u32 {
        let down = input::escape_down();
        if down && !self.escape_down {
            self.escapes += 1;
        }
        self.escape_down = down;
        self.escapes
    }
}

// ── the run ────────────────────────────────────────────────────────

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub stop_reason: String,
    pub clicks: i64,
    pub elapsed_secs: f64,
    pub achieved_cps: f64,
    pub requested_cps: f64,
}

/// Milliseconds since the Unix epoch.
pub fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|age| age.as_millis() as u64)
        .unwrap_or(0)
}

/// The counters a run publishes while it is going, so the window can show the
/// click count and the rate climbing instead of waiting for the run to end.
#[derive(Default)]
pub struct Progress {
    clicks: AtomicI64,
    started_ms: AtomicU64,
}

impl Progress {
    pub fn begin(&self) {
        self.clicks.store(0, Ordering::Relaxed);
        self.started_ms.store(now_epoch_ms(), Ordering::Relaxed);
    }

    pub fn set_clicks(&self, clicks: i64) {
        self.clicks.store(clicks, Ordering::Relaxed);
    }

    pub fn clicks(&self) -> i64 {
        self.clicks.load(Ordering::Relaxed)
    }

    pub fn elapsed_secs(&self) -> f64 {
        now_epoch_ms().saturating_sub(self.started_ms.load(Ordering::Relaxed)) as f64 / 1_000.0
    }
}

/// Longest batch handed to one `SendInput` call. A batch is how the engine
/// catches up after a stall, but a huge one would ignore the stop button.
const MAX_BATCH: u64 = 64;

/// How often the failsafes and the target list are re-checked while waiting.
const CHECK_MS: f64 = 20.0;

/// Run until stopped, limited, or failed safe.
pub fn run(
    settings: &ClickerSettings,
    machine: &mut dyn Machine,
    stop: Arc<AtomicBool>,
    progress: &Progress,
) -> Outcome {
    let cycle = cycle_ms(settings);
    let hold = hold_ms(settings, cycle);
    let source = Source::of(settings);
    // The one-click-per-`SendInput` path is only for plain clicks: a duty
    // cycle, a double click or an aim at a point all need their own steps.
    let batched = hold <= 0.0 && !settings.double_click && settings.points.is_empty();
    let mut rng = Rng::from_clock();
    let mut rotation = PointRotation::new(&settings.points);
    let start = machine.now_ms();

    let mut clicks: i64 = 0;
    let mut reason = String::from("Stopped");
    let mut escape = EscapeGuard::default();
    escape.baseline(machine.escape_presses());
    // The deadline the next click is due on. It advances by one interval at a
    // time — jittered when a random rate is set — so the run keeps a grid of its
    // own instead of drifting with each slow wake-up.
    let mut due = start + jittered_cycle_ms(settings, &mut rng);
    // Whether a pause zone is holding the run right now.
    let mut held = false;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let now = machine.now_ms();
        let elapsed = (now - start) / 1_000.0;

        if settings.max_clicks > 0 && clicks >= settings.max_clicks {
            reason = format!("Reached the {}-click limit", settings.max_clicks);
            break;
        }
        if settings.time_limit_secs > 0.0 && elapsed >= settings.time_limit_secs {
            reason = format!("Reached the {} second limit", settings.time_limit_secs);
            break;
        }

        // Escape first: it is the only stop that needs neither the cursor nor
        // the keyboard hook, both of which a runaway run can take away.
        if escape.tripped(now, machine.escape_presses()) {
            reason = format!("Stopped — Escape pressed {ESCAPE_TAPS} times");
            break;
        }

        // Then the cursor failsafes: reaching for the mouse is the other way a
        // runaway run is stopped by hand.
        let cursor = machine.cursor();
        let monitors = machine.monitors();
        let zone = if settings.zones_enabled {
            zone_at(cursor, &settings.stop_zones)
        } else {
            None
        };

        // A zone is checked before the edges, because a zone is drawn on
        // purpose where the cursor is meant to go, and is the shortest way out.
        if let Some((ZoneAction::Stop, index)) = zone {
            reason = format!("Stopped — the cursor entered zone {}", index + 1);
            break;
        }
        if let Some(hit) = failsafe(cursor, &monitors, settings) {
            reason = format!("Stopped — {hit}");
            break;
        }

        // A target list pauses rather than stops, because the run is meant to
        // continue when the program comes back to the front.
        if settings.targets.enabled && !target_allows(&settings.targets, &machine.foreground()) {
            let resume = machine.now_ms() + CHECK_MS;
            if !machine.wait_until(resume, &stop) {
                reason = "Stopped while waiting for a listed program".to_string();
                break;
            }
            continue;
        }

        // A pause zone holds the run where it is. Nothing is clicked while it
        // does, and leaving it re-anchors the click grid further down: the time
        // spent held is not paid back all at once as a burst of catch-up clicks.
        if matches!(zone, Some((ZoneAction::Pause, _))) {
            held = true;
            let look_again = machine.now_ms() + CHECK_MS;
            if !machine.wait_until(look_again, &stop) {
                reason = "Stopped while held in a pause zone".to_string();
                break;
            }
            continue;
        }
        if held {
            held = false;
            due = machine.now_ms() + jittered_cycle_ms(settings, &mut rng);
        }

        if !batched {
            if !machine.wait_until(due, &stop) {
                break;
            }
            clicks += press_cycle(settings, machine, &mut rotation, &mut rng, source, hold, cycle, &stop);
            progress.set_clicks(clicks);
            due += jittered_cycle_ms(settings, &mut rng);
            continue;
        }

        // The fast path: every click this configuration has earned by now, in
        // one batch. At 200 CPS that is one `SendInput` per wake-up carrying a
        // click or two; after a stall it is the catch-up.
        let now = machine.now_ms();
        let mut owed: u64 = 0;
        while due <= now && owed < MAX_BATCH {
            owed += 1;
            due += jittered_cycle_ms(settings, &mut rng);
        }
        if owed == 0 {
            if !machine.wait_until(due, &stop) {
                break;
            }
            continue;
        }

        if settings.max_clicks > 0 {
            owed = owed.min((settings.max_clicks - clicks).max(0) as u64);
            if owed == 0 {
                reason = format!("Reached the {}-click limit", settings.max_clicks);
                break;
            }
        }

        machine.batch(source, owed);
        clicks += owed as i64;
        progress.set_clicks(clicks);
    }

    let elapsed_secs = (machine.now_ms() - start) / 1_000.0;
    Outcome {
        stop_reason: reason,
        clicks,
        elapsed_secs,
        achieved_cps: if elapsed_secs > 0.0 {
            clicks as f64 / elapsed_secs
        } else {
            0.0
        },
        requested_cps: clicks_per_second(settings),
    }
}

/// One click cycle: press, hold, release — twice over when a double click is
/// set, with the gap measured from the first release, which is what Windows
/// reads as a double click.
fn press_cycle(
    settings: &ClickerSettings,
    machine: &mut dyn Machine,
    rotation: &mut PointRotation,
    rng: &mut Rng,
    source: Source,
    hold: f64,
    cycle: f64,
    stop: &AtomicBool,
) -> i64 {
    if !rotation.is_empty() {
        if let Some((x, y)) = rotation.next(settings.humanize, rng) {
            let travel = if settings.humanize {
                settings.move_duration_ms
            } else {
                0
            };
            machine.move_to(x, y, travel);
        }
    }

    let mut clicks = 0;
    machine.press(source);
    clicks += 1;

    // The first press of a pair is a tap: the gap afterwards is what makes the
    // two read as one double click.
    let hold_first = if settings.double_click { 0.0 } else { hold };
    let release_at = machine.now_ms() + hold_first;
    if hold_first > 0.0 && !machine.wait_until(release_at, stop) {
        machine.release(source);
        return clicks;
    }
    machine.release(source);

    if settings.double_click {
        let gap = double_gap_ms(settings, cycle);
        let second_at = machine.now_ms() + gap;
        if gap > 0.0 && !machine.wait_until(second_at, stop) {
            return clicks;
        }
        machine.press(source);
        clicks += 1;
        let second_release = machine.now_ms() + hold;
        if hold > 0.0 && !machine.wait_until(second_release, stop) {
            machine.release(source);
            return clicks;
        }
        machine.release(source);
    }

    clicks
}

/// Raises the timer resolution for the length of a run and reports whether the
/// kernel agreed — the UI says so, because without it the achieved rate drops
/// at high settings and looks like a bug in the app.
pub fn with_timer<T>(body: impl FnOnce(bool) -> T) -> T {
    let timer = TimerResolution::raise();
    body(timer.raised())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Debug, PartialEq)]
    enum Event {
        Press(f64, Source),
        Release(f64, Source),
        Batch(f64, u64),
        Move(f64, i32, i32),
    }

    /// A machine with a clock the test owns, and a tape of everything sent.
    struct Fake {
        clock: Rc<RefCell<f64>>,
        /// Extra milliseconds every wait costs, modelling a scheduler that
        /// overshoots. This is what the drift correction has to survive.
        overshoot_ms: f64,
        cursor: (i32, i32),
        monitors: Vec<(i32, i32, i32, i32)>,
        foreground: String,
        log: Vec<Event>,
        /// Cursor moves to make at given clock times, so a test can drive a run
        /// into and out of a zone while it is going.
        cursor_plan: Vec<(f64, (i32, i32))>,
        /// The run is stopped once the clock passes this, so tests terminate.
        stop_at_ms: f64,
        stop: Arc<AtomicBool>,
    }

    impl Fake {
        fn new(stop_at_ms: f64) -> Self {
            Self {
                clock: Rc::new(RefCell::new(0.0)),
                overshoot_ms: 0.0,
                cursor: (960, 540),
                monitors: vec![(0, 0, 1920, 1080)],
                foreground: "Game.exe".to_string(),
                log: Vec::new(),
                cursor_plan: Vec::new(),
                stop_at_ms,
                stop: Arc::new(AtomicBool::new(false)),
            }
        }

        /// Move the cursor once the run's clock passes `at_ms`.
        fn move_cursor_at(&mut self, at_ms: f64, to: (i32, i32)) -> &mut Self {
            self.cursor_plan.push((at_ms, to));
            self
        }

        /// Run the engine over this machine and hand back the outcome.
        fn run(&mut self, settings: &ClickerSettings) -> Outcome {
            let stop = Arc::clone(&self.stop);
            run(settings, self, stop, &Progress::default())
        }

        fn times(&self, pick: impl Fn(&Event) -> Option<f64>) -> Vec<f64> {
            self.log.iter().filter_map(&pick).collect()
        }

        fn batch_times(&self) -> Vec<f64> {
            self.times(|event| match event {
                Event::Batch(at, _) => Some(*at),
                _ => None,
            })
        }

        fn click_count(&self) -> u64 {
            self.log
                .iter()
                .map(|event| match event {
                    Event::Batch(_, count) => *count,
                    Event::Press(..) => 1,
                    _ => 0,
                })
                .sum()
        }
    }

    impl Machine for Fake {
        fn now_ms(&mut self) -> f64 {
            *self.clock.borrow()
        }

        fn wait_until(&mut self, deadline_ms: f64, stop: &AtomicBool) -> bool {
            if stop.load(Ordering::Relaxed) {
                return false;
            }
            {
                let mut clock = self.clock.borrow_mut();
                // A deadline already in the past costs nothing to wait for.
                if *clock < deadline_ms {
                    *clock = deadline_ms + self.overshoot_ms;
                }
            }
            let now = *self.clock.borrow();
            let mut plan = std::mem::take(&mut self.cursor_plan);
            let mut cursor = self.cursor;
            plan.retain(|(at, target)| {
                if now >= *at {
                    cursor = *target;
                    false
                } else {
                    true
                }
            });
            self.cursor = cursor;
            self.cursor_plan = plan;
            if now >= self.stop_at_ms {
                self.stop.store(true, Ordering::Relaxed);
            }
            !stop.load(Ordering::Relaxed)
        }

        fn press(&mut self, source: Source) {
            let at = self.now_ms();
            self.log.push(Event::Press(at, source));
        }

        fn release(&mut self, source: Source) {
            let at = self.now_ms();
            self.log.push(Event::Release(at, source));
        }

        fn batch(&mut self, _source: Source, count: u64) {
            let at = self.now_ms();
            self.log.push(Event::Batch(at, count));
        }

        fn move_to(&mut self, x: i32, y: i32, _duration_ms: u32) {
            let at = self.now_ms();
            self.log.push(Event::Move(at, x, y));
        }

        fn cursor(&mut self) -> (i32, i32) {
            self.cursor
        }

        fn monitors(&mut self) -> Vec<(i32, i32, i32, i32)> {
            self.monitors.clone()
        }

        fn foreground(&mut self) -> String {
            self.foreground.clone()
        }
    }

    fn steady(rate: f64) -> ClickerSettings {
        ClickerSettings {
            rate,
            ..ClickerSettings::default()
        }
    }

    /// An edge failsafe watching all four sides at the same distance.
    fn edges(margin: u32) -> EdgeStop {
        let guard = Guard { on: true, margin };
        EdgeStop {
            enabled: true,
            top: guard,
            right: guard,
            bottom: guard,
            left: guard,
        }
    }

    /// A corner failsafe watching all four corners at the same distance.
    fn corners(span: u32) -> CornerStop {
        let guard = Guard { on: true, margin: span };
        CornerStop {
            enabled: true,
            top_left: guard,
            top_right: guard,
            bottom_left: guard,
            bottom_right: guard,
        }
    }

    fn zone(x: i32, y: i32, width: u32, height: u32, action: ZoneAction) -> StopZone {
        StopZone {
            x,
            y,
            width,
            height,
            action,
        }
    }

    #[test]
    fn the_rate_converts_to_a_cycle_and_back() {
        let settings = steady(4.0);
        assert_eq!(cycle_ms(&settings), 250.0);
        assert_eq!(clicks_per_second(&settings), 4.0);

        let slow = ClickerSettings {
            rate_unit: RateUnit::Minute,
            rate: 2.0,
            ..ClickerSettings::default()
        };
        assert_eq!(cycle_ms(&slow), 30_000.0);
        assert_eq!(clicks_per_second(&slow), 2.0 / 60.0);

        let daily = ClickerSettings {
            rate_unit: RateUnit::Day,
            rate: 1.0,
            ..ClickerSettings::default()
        };
        assert_eq!(cycle_ms(&daily), 86_400_000.0);
    }

    #[test]
    fn a_double_click_cycle_carries_two_clicks() {
        let settings = ClickerSettings {
            double_click: true,
            rate: 5.0,
            ..ClickerSettings::default()
        };
        assert_eq!(clicks_per_second(&settings), 10.0);
        assert_eq!(cycle_ms(&settings), 200.0);
    }

    #[test]
    fn the_duty_cycle_shortens_the_hold_but_never_swallows_the_interval() {
        let settings = ClickerSettings {
            duty: 25.0,
            ..steady(10.0)
        };
        assert_eq!(hold_ms(&settings, cycle_ms(&settings)), 25.0);

        // Even at the top of the range there is a gap, or the button would
        // never come back up.
        let greedy = ClickerSettings {
            duty: 100.0,
            ..steady(10.0)
        };
        assert!(hold_ms(&greedy, cycle_ms(&greedy)) < cycle_ms(&greedy));
    }

    #[test]
    fn the_engine_clicks_on_the_grid_it_promised() {
        let mut machine = Fake::new(1_050.0);
        let outcome = machine.run(&steady(10.0));

        let times = machine.batch_times();
        assert!(times.len() >= 10, "expected ten clicks, saw {times:?}");
        for (index, at) in times.iter().take(10).enumerate() {
            let expected = 100.0 * (index as f64 + 1.0);
            assert!(
                (at - expected).abs() < 0.001,
                "click {index} landed at {at}, expected {expected}"
            );
        }
        assert_eq!(outcome.requested_cps, 10.0);
    }

    /// The point of absolute deadlines: a slow scheduler costs a click its
    /// punctuality, not the rate. Every wake here is 7 ms late.
    #[test]
    fn wake_ups_that_overshoot_do_not_accumulate_drift() {
        let mut machine = Fake::new(2_500.0);
        machine.overshoot_ms = 7.0;
        let outcome = machine.run(&steady(10.0));

        let times = machine.batch_times();
        assert!(times.len() >= 20, "expected the run to keep clicking, saw {times:?}");
        for pair in times.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                (gap - 100.0).abs() < 8.0,
                "a 100 ms grid with 7 ms of overshoot should still be 100 ms apart, saw {gap}"
            );
        }
        assert!(
            outcome.achieved_cps > 9.0,
            "the achieved rate should stay near 10, got {}",
            outcome.achieved_cps
        );
    }

    #[test]
    fn a_high_rate_is_paced_rather_than_dropped() {
        let mut machine = Fake::new(1_020.0);
        let outcome = machine.run(&steady(400.0));

        assert!(
            outcome.clicks >= 380,
            "400 CPS should have managed a second's worth, got {}",
            outcome.clicks
        );
        assert!(
            (outcome.achieved_cps - 400.0).abs() < 20.0,
            "the achieved rate should be about 400, got {}",
            outcome.achieved_cps
        );
    }

    #[test]
    fn a_hold_presses_and_releases_around_each_click() {
        let mut machine = Fake::new(520.0);
        machine.run(&ClickerSettings {
            duty: 40.0,
            ..steady(10.0)
        });

        let presses = machine.times(|event| match event {
            Event::Press(at, _) => Some(*at),
            _ => None,
        });
        let releases = machine.times(|event| match event {
            Event::Release(at, _) => Some(*at),
            _ => None,
        });
        assert_eq!(presses.len(), releases.len(), "every press needs its release");
        assert!(presses.len() >= 4, "expected several clicks, saw {presses:?}");
        for (press, release) in presses.iter().zip(releases.iter()).take(4) {
            assert!(
                (release - press - 40.0).abs() < 0.001,
                "a press at {press} should hold 40 ms, released at {release}"
            );
        }
    }

    #[test]
    fn a_double_click_sends_two_presses_the_gap_apart() {
        // Long enough for one pair, not long enough for the next cycle.
        let mut machine = Fake::new(260.0);
        machine.run(&ClickerSettings {
            double_click: true,
            double_click_gap_ms: 40,
            rate: 5.0,
            ..ClickerSettings::default()
        });

        let presses = machine.times(|event| match event {
            Event::Press(at, _) => Some(*at),
            _ => None,
        });
        assert!(presses.len() >= 2, "expected a double click, saw {presses:?}");
        assert!(
            (presses[1] - presses[0] - 40.0).abs() < 0.001,
            "the pair should be 40 ms apart, was {}",
            presses[1] - presses[0]
        );
        assert_eq!(machine.click_count(), presses.len() as u64);
    }

    #[test]
    fn a_gap_wider_than_the_cycle_is_pulled_back_inside_it() {
        let settings = ClickerSettings {
            double_click: true,
            double_click_gap_ms: 5_000,
            rate: 5.0,
            ..ClickerSettings::default()
        };
        assert_eq!(double_gap_ms(&settings, cycle_ms(&settings)), 199.0);
    }

    #[test]
    fn the_click_limit_stops_the_run() {
        let mut machine = Fake::new(60_000.0);
        let outcome = machine.run(&ClickerSettings {
            max_clicks: 5,
            ..steady(10.0)
        });

        assert_eq!(outcome.clicks, 5);
        assert!(outcome.stop_reason.contains("5-click limit"), "{}", outcome.stop_reason);
    }

    #[test]
    fn the_click_limit_also_stops_a_batched_run_exactly() {
        let mut machine = Fake::new(60_000.0);
        let outcome = machine.run(&ClickerSettings {
            max_clicks: 7,
            ..steady(400.0)
        });

        assert_eq!(outcome.clicks, 7, "a batch must not overshoot the limit");
        assert!(outcome.stop_reason.contains("7-click limit"), "{}", outcome.stop_reason);
    }

    #[test]
    fn the_time_limit_stops_the_run() {
        let mut machine = Fake::new(60_000.0);
        let outcome = machine.run(&ClickerSettings {
            time_limit_secs: 1.0,
            ..steady(10.0)
        });

        assert!(outcome.stop_reason.contains("1 second limit"), "{}", outcome.stop_reason);
        assert!(outcome.elapsed_secs >= 1.0);
        assert!(outcome.clicks >= 9, "a second at 10 CPS is ten clicks, got {}", outcome.clicks);
    }

    #[test]
    fn the_achieved_rate_is_reported_against_the_one_asked_for() {
        let mut machine = Fake::new(2_040.0);
        let outcome = machine.run(&steady(10.0));

        assert_eq!(outcome.requested_cps, 10.0);
        assert!(
            (outcome.achieved_cps - 10.0).abs() < 0.5,
            "achieved {} against a requested 10",
            outcome.achieved_cps
        );
    }

    #[test]
    fn the_stop_flag_ends_the_run() {
        let mut machine = Fake::new(60_000.0);
        let stop = Arc::clone(&machine.stop);
        stop.store(true, Ordering::Relaxed);

        let outcome = machine.run(&steady(10.0));
        assert_eq!(outcome.clicks, 0);
        assert_eq!(outcome.stop_reason, "Stopped");
    }

    #[test]
    fn three_quick_escape_taps_stop_the_run() {
        let mut guard = EscapeGuard::default();
        guard.baseline(7);
        assert!(!guard.tripped(1_000.0, 7), "no press, no stop");
        assert!(!guard.tripped(1_010.0, 8));
        assert!(!guard.tripped(1_120.0, 9));
        assert!(guard.tripped(1_210.0, 10), "the third tap is the one");
    }

    #[test]
    fn slowly_spaced_escape_taps_do_not_stop_anything() {
        let mut guard = EscapeGuard::default();
        guard.baseline(0);
        assert!(!guard.tripped(1_000.0, 1));
        assert!(!guard.tripped(2_000.0, 2));
        assert!(
            !guard.tripped(3_000.0, 3),
            "three taps a second apart is not a deliberate stop"
        );
    }

    #[test]
    fn presses_from_before_the_run_are_ignored() {
        let mut guard = EscapeGuard::default();
        // Three taps were already on the counter when the user pressed start.
        guard.baseline(3);
        assert!(!guard.tripped(1_000.0, 3));
        assert!(!guard.tripped(1_050.0, 4));
        assert!(!guard.tripped(1_100.0, 5));
        assert!(guard.tripped(1_150.0, 6));
    }

    #[test]
    fn a_press_missed_between_polls_still_counts() {
        let mut guard = EscapeGuard::default();
        guard.baseline(0);
        // Two taps happened between two looks at the key state.
        assert!(!guard.tripped(1_000.0, 2));
        assert!(!guard.tripped(1_050.0, 2));
        assert!(guard.tripped(1_100.0, 3));
    }

    #[test]
    fn the_edge_failsafe_fires_on_each_side() {
        let edge = edges(30);
        let monitor = (0, 0, 1920, 1080);
        assert_eq!(edge_hit((960, 10), monitor, &edge), Some("the cursor reached the top edge"));
        assert_eq!(edge_hit((1915, 540), monitor, &edge), Some("the cursor reached the right edge"));
        assert_eq!(edge_hit((960, 1075), monitor, &edge), Some("the cursor reached the bottom edge"));
        assert_eq!(edge_hit((5, 540), monitor, &edge), Some("the cursor reached the left edge"));
        assert_eq!(edge_hit((960, 540), monitor, &edge), None);
    }

    #[test]
    fn a_side_that_is_switched_off_never_fires() {
        let mut edge = edges(30);
        edge.right.on = false;
        let monitor = (0, 0, 1920, 1080);
        assert_eq!(edge_hit((1915, 540), monitor, &edge), None);
        assert_eq!(
            edge_hit((5, 540), monitor, &edge),
            Some("the cursor reached the left edge"),
            "leaving one side off must not disturb the others"
        );

        let off = EdgeStop {
            enabled: false,
            ..edges(30)
        };
        assert_eq!(edge_hit((5, 5), monitor, &off), None);
    }

    /// The sides are not interchangeable: the bottom of a screen is usually
    /// under a taskbar, so it wants a wider berth than the top.
    #[test]
    fn each_edge_keeps_its_own_distance() {
        let edge = EdgeStop {
            enabled: true,
            top: Guard { on: true, margin: 10 },
            right: Guard { on: true, margin: 10 },
            bottom: Guard { on: true, margin: 200 },
            left: Guard { on: true, margin: 10 },
        };
        let monitor = (0, 0, 1920, 1080);
        assert_eq!(
            edge_hit((960, 930), monitor, &edge),
            Some("the cursor reached the bottom edge"),
            "150 px above a 200 px bottom guard is inside it"
        );
        assert_eq!(
            edge_hit((960, 150), monitor, &edge),
            None,
            "the same distance from a 10 px top guard is outside it"
        );
    }

    #[test]
    fn the_corner_failsafe_fires_in_each_corner() {
        let corner = corners(50);
        let monitor = (0, 0, 1920, 1080);
        assert_eq!(corner_hit((10, 10), monitor, &corner), Some("the cursor reached the top-left corner"));
        assert_eq!(corner_hit((1915, 10), monitor, &corner), Some("the cursor reached the top-right corner"));
        assert_eq!(corner_hit((10, 1075), monitor, &corner), Some("the cursor reached the bottom-left corner"));
        assert_eq!(corner_hit((1915, 1075), monitor, &corner), Some("the cursor reached the bottom-right corner"));

        // Along the middle of an edge is not a corner, and the middle of a
        // screen is neither.
        assert_eq!(corner_hit((960, 5), monitor, &corner), None);
        assert_eq!(corner_hit((960, 540), monitor, &corner), None);

        // With the failsafe off it never fires at all.
        let off = CornerStop {
            enabled: false,
            ..corners(50)
        };
        assert_eq!(corner_hit((5, 5), monitor, &off), None);
    }

    #[test]
    fn each_corner_keeps_its_own_distance_and_can_be_switched_off() {
        let corner = CornerStop {
            enabled: true,
            top_left: Guard { on: true, margin: 100 },
            top_right: Guard { on: true, margin: 20 },
            bottom_left: Guard { on: false, margin: 20 },
            bottom_right: Guard { on: true, margin: 20 },
        };
        let monitor = (0, 0, 1920, 1080);
        assert_eq!(
            corner_hit((80, 80), monitor, &corner),
            Some("the cursor reached the top-left corner"),
            "80 px in is inside a 100 px top-left guard"
        );
        assert_eq!(
            corner_hit((1840, 80), monitor, &corner),
            None,
            "the same distance in is outside a 20 px top-right guard"
        );
        assert_eq!(
            corner_hit((80, 1000), monitor, &corner),
            None,
            "a corner that is switched off never fires"
        );
    }

    /// A corner sits inside two edges, so both would fire. The corner is the
    /// useful thing to be told.
    #[test]
    fn a_corner_is_reported_rather_than_the_two_edges_it_sits_in() {
        let monitors = [(0, 0, 1920, 1080)];
        let settings = ClickerSettings {
            edge_stop: edges(40),
            corner_stop: corners(50),
            ..ClickerSettings::default()
        };
        assert_eq!(
            failsafe((5, 5), &monitors, &settings),
            Some("the cursor reached the top-left corner")
        );
        // Clear of the corners, an edge is still reported.
        assert_eq!(
            failsafe((960, 5), &monitors, &settings),
            Some("the cursor reached the top edge")
        );
    }

    /// Two screens side by side. The seam between them is not an edge of the
    /// desktop, but it is an edge of both monitors — and it is exactly where a
    /// cursor ends up when it is thrown sideways.
    #[test]
    fn the_failsafes_are_measured_per_monitor() {
        let monitors = [(0, 0, 1920, 1080), (1920, 0, 1920, 1080)];
        let settings = ClickerSettings {
            edge_stop: edges(40),
            ..ClickerSettings::default()
        };

        assert_eq!(
            failsafe((1915, 540), &monitors, &settings),
            Some("the cursor reached the right edge"),
            "the left screen's own right edge"
        );
        assert_eq!(
            failsafe((1925, 540), &monitors, &settings),
            Some("the cursor reached the left edge"),
            "the right screen's own left edge, which is the seam"
        );
        assert_eq!(
            failsafe((960, 540), &monitors, &settings),
            None,
            "the middle of a screen is not near anything"
        );
        assert_eq!(
            failsafe((5000, 540), &monitors, &settings),
            None,
            "a cursor off the desktop altogether is in no monitor's corner"
        );
    }

    #[test]
    fn an_edge_failsafe_position_ends_the_run_before_clicking() {
        let mut machine = Fake::new(5_000.0);
        machine.cursor = (2, 540);
        let outcome = machine.run(&ClickerSettings {
            edge_stop: edges(48),
            ..steady(10.0)
        });

        assert_eq!(outcome.clicks, 0);
        assert!(outcome.stop_reason.contains("left edge"), "{}", outcome.stop_reason);
    }

    #[test]
    fn a_corner_failsafe_position_ends_the_run_before_clicking() {
        let mut machine = Fake::new(5_000.0);
        machine.cursor = (1900, 1060);
        let outcome = machine.run(&ClickerSettings {
            corner_stop: corners(60),
            ..steady(10.0)
        });

        assert_eq!(outcome.clicks, 0);
        assert!(
            outcome.stop_reason.contains("bottom-right corner"),
            "{}",
            outcome.stop_reason
        );
    }

    // ── zones ──────────────────────────────────────────────────────

    #[test]
    fn zone_containment_excludes_the_far_edge() {
        let one = zone(100, 100, 200, 150, ZoneAction::Stop);
        assert!(one.contains(100, 100));
        assert!(one.contains(299, 249));
        assert!(
            !one.contains(300, 200),
            "the far edge belongs to whatever is drawn beside it"
        );
        assert!(!one.contains(99, 200));
        assert!(!one.contains(200, 250));
    }

    #[test]
    fn a_stop_zone_wins_over_a_pause_wherever_it_sits() {
        // Listed second, and still the winner: a stop zone is what you draw over
        // everything else, and cannot be relied on to be first in the list.
        let zones = [
            zone(0, 0, 100, 100, ZoneAction::Pause),
            zone(20, 20, 100, 100, ZoneAction::Stop),
        ];
        assert_eq!(zone_at((50, 50), &zones), Some((ZoneAction::Stop, 1)));
    }

    #[test]
    fn a_carry_on_zone_beats_a_pause_around_it() {
        let zones = [
            zone(0, 0, 900, 900, ZoneAction::Pause),
            zone(400, 400, 100, 100, ZoneAction::Start),
        ];
        assert_eq!(zone_at((450, 450), &zones), Some((ZoneAction::Start, 1)));
        assert_eq!(
            zone_at((100, 100), &zones),
            Some((ZoneAction::Pause, 0)),
            "outside the hole, the pause still holds"
        );
    }

    #[test]
    fn the_first_pause_holds_when_nothing_outranks_it() {
        let zones = [
            zone(0, 0, 200, 200, ZoneAction::Pause),
            zone(50, 50, 200, 200, ZoneAction::Pause),
        ];
        assert_eq!(zone_at((100, 100), &zones), Some((ZoneAction::Pause, 0)));
    }

    #[test]
    fn no_zone_holds_a_cursor_that_is_outside_them_all() {
        let zones = [
            zone(0, 0, 100, 100, ZoneAction::Pause),
            zone(200, 200, 50, 50, ZoneAction::Stop),
        ];
        assert_eq!(zone_at((500, 500), &zones), None);
        assert_eq!(zone_at((500, 500), &[]), None);
    }

    #[test]
    fn a_stop_zone_ends_the_run_before_it_clicks() {
        let mut machine = Fake::new(5_000.0);
        machine.cursor = (150, 120);
        let outcome = machine.run(&ClickerSettings {
            zones_enabled: true,
            stop_zones: vec![zone(100, 100, 200, 150, ZoneAction::Stop)],
            ..steady(10.0)
        });

        assert_eq!(outcome.clicks, 0);
        assert!(machine.batch_times().is_empty());
        assert!(
            outcome.stop_reason.contains("zone 1"),
            "the reason should name the zone, said “{}”",
            outcome.stop_reason
        );
    }

    /// The same cursor and the same zone with the watch off: nothing is looked
    /// at, and the run clicks as it otherwise would.
    #[test]
    fn zones_are_ignored_while_the_watch_is_off() {
        let mut machine = Fake::new(430.0);
        machine.cursor = (150, 120);
        let outcome = machine.run(&ClickerSettings {
            zones_enabled: false,
            stop_zones: vec![zone(100, 100, 200, 150, ZoneAction::Stop)],
            ..steady(10.0)
        });

        assert!(
            outcome.clicks >= 3,
            "expected the run to click, got {}",
            outcome.clicks
        );
    }

    /// A pause zone holds the run while the cursor is in it, and the time spent
    /// held is *not* paid back afterwards. A clicker that caught up by firing
    /// everything it owed at once would turn a pause into a burst.
    #[test]
    fn a_pause_zone_holds_the_run_without_paying_it_back() {
        let mut machine = Fake::new(1_200.0);
        machine.move_cursor_at(250.0, (150, 120));
        machine.move_cursor_at(650.0, (960, 540));

        let outcome = machine.run(&ClickerSettings {
            zones_enabled: true,
            stop_zones: vec![zone(100, 100, 200, 150, ZoneAction::Pause)],
            ..steady(10.0)
        });

        let times = machine.batch_times();
        assert!(
            times.len() >= 5,
            "expected clicks either side of the hold, saw {times:?}"
        );
        assert!(
            times.iter().all(|at| *at < 250.0 || *at > 650.0),
            "a click landed inside the hold: {times:?}"
        );
        for pair in times.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                gap >= 95.0,
                "two clicks {gap} ms apart: the hold was paid back as a burst"
            );
        }
        assert!(
            outcome.clicks < 12,
            "a 1.2 s run with 0.4 s of it held should be short of twelve clicks, got {}",
            outcome.clicks
        );
    }

    #[test]
    fn a_carry_on_zone_inside_a_pause_zone_keeps_the_run_going() {
        let mut machine = Fake::new(430.0);
        machine.cursor = (450, 450);
        let outcome = machine.run(&ClickerSettings {
            zones_enabled: true,
            stop_zones: vec![
                zone(0, 0, 900, 900, ZoneAction::Pause),
                zone(400, 400, 100, 100, ZoneAction::Start),
            ],
            ..steady(10.0)
        });

        assert!(
            outcome.clicks >= 3,
            "a carry-on zone should let the run click inside a pause, got {}",
            outcome.clicks
        );
    }

    #[test]
    fn a_whitelist_only_clicks_in_the_programs_it_lists() {
        let targets = Targets {
            enabled: true,
            mode: TargetMode::Whitelist,
            programs: vec!["Game.exe".into()],
        };
        assert!(target_allows(&targets, "game.exe"));
        assert!(!target_allows(&targets, "Discord.exe"));

        let blacklist = Targets {
            mode: TargetMode::Blacklist,
            ..targets.clone()
        };
        assert!(!target_allows(&blacklist, "Game.exe"));
        assert!(target_allows(&blacklist, "Discord.exe"));

        // A list that is switched off lets everything through.
        let off = Targets {
            enabled: false,
            ..targets
        };
        assert!(target_allows(&off, "anything at all"));
    }

    #[test]
    fn a_run_waits_out_a_program_that_is_not_on_the_list() {
        let mut machine = Fake::new(620.0);
        machine.foreground = "Discord.exe".to_string();
        let outcome = machine.run(&ClickerSettings {
            targets: Targets {
                enabled: true,
                mode: TargetMode::Whitelist,
                programs: vec!["Game.exe".into()],
            },
            ..steady(10.0)
        });

        assert_eq!(outcome.clicks, 0, "nothing should be clicked in Discord");
        assert!(machine.batch_times().is_empty());
        assert!(
            outcome.stop_reason.contains("waiting for a listed program"),
            "the reason should say what it was waiting for, said “{}”",
            outcome.stop_reason
        );
    }

    /// The same run clicks normally in the listed program.
    #[test]
    fn a_listed_program_is_clicked_as_usual() {
        let mut machine = Fake::new(430.0);
        let outcome = machine.run(&ClickerSettings {
            targets: Targets {
                enabled: true,
                mode: TargetMode::Whitelist,
                programs: vec!["game.exe".into()],
            },
            ..steady(10.0)
        });

        assert!(outcome.clicks >= 3, "expected clicks in the listed program");
    }

    #[test]
    fn click_points_take_their_turns() {
        let mut rotation = PointRotation::new(&[
            ClickPoint { x: 10, y: 20, clicks: 2, radius: 0 },
            ClickPoint { x: 300, y: 400, clicks: 1, radius: 0 },
        ]);
        let mut rng = Rng::new(7);
        assert_eq!(rotation.next(false, &mut rng), Some((10, 20)));
        assert_eq!(rotation.next(false, &mut rng), Some((10, 20)));
        assert_eq!(rotation.next(false, &mut rng), Some((300, 400)));
        assert_eq!(rotation.next(false, &mut rng), Some((10, 20)));
    }

    #[test]
    fn humanised_points_stay_inside_their_radius() {
        let mut rotation = PointRotation::new(&[ClickPoint {
            x: 100,
            y: 100,
            clicks: 1,
            radius: 12,
        }]);
        let mut rng = Rng::new(99);
        for _ in 0..500 {
            let (x, y) = rotation.next(true, &mut rng).expect("a point");
            let distance = f64::from((x - 100).pow(2) + (y - 100).pow(2)).sqrt();
            // Whole pixels cannot land exactly on a 12 px circle, so the bound
            // allows the rounding.
            assert!(
                distance <= 12.8,
                "a point landed {distance} away from the target"
            );
        }
    }

    #[test]
    fn a_run_visits_its_points_in_order() {
        // Three clicks land, so the cursor visits A, B, then A again.
        let mut machine = Fake::new(330.0);
        machine.run(&ClickerSettings {
            points: vec![
                ClickPoint { x: 100, y: 100, clicks: 1, radius: 0 },
                ClickPoint { x: 500, y: 400, clicks: 1, radius: 0 },
            ],
            ..steady(10.0)
        });

        let moves: Vec<(i32, i32)> = machine
            .log
            .iter()
            .filter_map(|event| match event {
                Event::Move(_, x, y) => Some((*x, *y)),
                _ => None,
            })
            .collect();
        assert_eq!(moves, vec![(100, 100), (500, 400), (100, 100)]);
    }

    /// The variation has to reach the *run*, not just the arithmetic: this is
    /// the random-rate feature, and without it every interval is identical.
    #[test]
    fn a_random_rate_actually_varies_the_intervals() {
        let mut machine = Fake::new(2_000.0);
        let outcome = machine.run(&ClickerSettings {
            variation: 20.0,
            ..steady(10.0)
        });

        let times = machine.batch_times();
        assert!(times.len() >= 14, "expected roughly twenty clicks, saw {}", times.len());
        let gaps: Vec<f64> = times.windows(2).map(|pair| pair[1] - pair[0]).collect();
        let mut rounded: Vec<i64> = gaps.iter().map(|gap| gap.round() as i64).collect();
        rounded.sort_unstable();
        rounded.dedup();
        assert!(
            rounded.len() > 5,
            "a ±20% variation should give varied gaps, saw {rounded:?}"
        );
        assert!(
            gaps.iter().all(|gap| (80.0..=120.0).contains(gap)),
            "a gap fell outside the band: {gaps:?}"
        );
        assert_eq!(outcome.requested_cps, 10.0);
    }

    #[test]
    fn the_variation_keeps_the_rate_within_its_band() {
        let settings = ClickerSettings {
            variation: 20.0,
            ..steady(10.0)
        };
        let mut rng = Rng::new(4);
        let mut varied = false;
        for _ in 0..400 {
            let cycle = jittered_cycle_ms(&settings, &mut rng);
            assert!(
                (80.0..=120.0).contains(&cycle),
                "a ±20% band around 100 ms produced {cycle}"
            );
            if (cycle - 100.0).abs() > 1.0 {
                varied = true;
            }
        }
        assert!(varied, "the variation never actually varied");

        let steady_run = steady(10.0);
        let mut rng = Rng::new(4);
        assert_eq!(jittered_cycle_ms(&steady_run, &mut rng), 100.0);
    }

    #[test]
    fn impossible_settings_are_refused_with_a_reason() {
        let too_fast = ClickerSettings {
            rate: 1_500.0,
            ..ClickerSettings::default()
        };
        assert!(validate(&too_fast).is_err());

        // A double click doubles the rate, so 600 per second is 1200 clicks.
        let doubled = ClickerSettings {
            rate: 600.0,
            double_click: true,
            ..ClickerSettings::default()
        };
        assert!(validate(&doubled).is_err(), "1200 clicks a second is past the ceiling");

        // The same rate as a single click is fine.
        assert!(validate(&ClickerSettings { rate: 600.0, ..ClickerSettings::default() }).is_ok());

        let zero = ClickerSettings {
            rate: 0.0,
            ..ClickerSettings::default()
        };
        assert!(validate(&zero).is_err());

        let keyboard_without_a_key = ClickerSettings {
            input: ClickInput::Keyboard,
            key_scan: 0,
            ..ClickerSettings::default()
        };
        assert!(validate(&keyboard_without_a_key).is_err());

        let no_shortcut = ClickerSettings {
            hotkey: "  ".into(),
            ..ClickerSettings::default()
        };
        assert!(
            validate(&no_shortcut).is_ok(),
            "the Start button and Escape ×3 are enough to run and to stop"
        );

        let point_without_clicks = ClickerSettings {
            points: vec![ClickPoint { x: 1, y: 1, clicks: 0, radius: 0 }],
            ..ClickerSettings::default()
        };
        assert!(validate(&point_without_clicks).is_err());

        let flat_zone = ClickerSettings {
            stop_zones: vec![zone(10, 10, 0, 40, ZoneAction::Stop)],
            ..ClickerSettings::default()
        };
        assert!(
            validate(&flat_zone).is_err(),
            "a zone with no width is a rectangle nobody can enter"
        );

        assert!(validate(&ClickerSettings::default()).is_ok());
    }

    #[test]
    fn the_source_is_described_the_way_the_ui_shows_it() {
        let mouse = ClickerSettings::default();
        assert_eq!(Source::of(&mouse).label(), "Left mouse button");

        let keyboard = ClickerSettings {
            input: ClickInput::Keyboard,
            key_scan: 0x11,
            uppercase: true,
            ..ClickerSettings::default()
        };
        assert_eq!(Source::of(&keyboard).label(), "Shift + W");
    }

    #[test]
    fn a_keyboard_clicker_holds_shift_around_its_key() {
        let mut machine = Fake::new(520.0);
        machine.run(&ClickerSettings {
            input: ClickInput::Keyboard,
            key_scan: 0x11,
            uppercase: true,
            duty: 20.0,
            ..steady(10.0)
        });

        match machine.log.first() {
            Some(Event::Press(_, source)) => assert_eq!(
                *source,
                Source::Key {
                    scan: 0x11,
                    uppercase: true
                }
            ),
            other => panic!("the machine should have been asked to press Shift+W, saw {other:?}"),
        }
    }
}

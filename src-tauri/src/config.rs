//! The settings model, and the `.socd` text format Hitboxer writes.
//!
//! The format is kept byte-compatible with the original so an existing
//! `settings.socd` can be imported rather than retyped. It is line-oriented and
//! deliberately forgiving: unknown keys are ignored, and a malformed mapping is
//! skipped rather than failing the whole file.
//!
//! ```text
//! [1] # Config version, ...
//!
//! close_to_tray: true
//! runtime_debug: false
//! PROFILE_START
//! program: Hitboxer Global
//! platform: WINDOWS
//! PROFILE_END
//! PROFILE_START
//! active: true
//! program: Overwatch.exe
//! platform: WINDOWS
//! mapping: OPPOSITE 17 31
//! PROFILE_END
//! ```

use serde::{Deserialize, Serialize};

/// How a pair of keys is treated. The names match the original's file format.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Last pressed wins; releasing the winner presses the other again.
    #[serde(rename = "OPPOSITE")]
    Opposite,
    /// Last pressed wins, but the other is not pressed again on release.
    #[serde(rename = "OPPOSITE_NO_REPRESS")]
    OppositeNoRepress,
    /// Holding both cancels them out.
    #[serde(rename = "NEUTRAL")]
    Neutral,
    /// Plain remap: the source key produces the destination key.
    #[serde(rename = "REMAP")]
    Remap,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Opposite => "OPPOSITE",
            Mode::OppositeNoRepress => "OPPOSITE_NO_REPRESS",
            Mode::Neutral => "NEUTRAL",
            Mode::Remap => "REMAP",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_uppercase().as_str() {
            "OPPOSITE" => Some(Mode::Opposite),
            "OPPOSITE_NO_REPRESS" => Some(Mode::OppositeNoRepress),
            "NEUTRAL" => Some(Mode::Neutral),
            "REMAP" => Some(Mode::Remap),
            _ => None,
        }
    }
}

/// One mapping, in hardware scan codes so it does not depend on layout.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mapping {
    pub mode: Mode,
    pub source: u16,
    pub dest: u16,
}

/// A program Crossup is active for. The global profile is stored as
/// `Hitboxer Global` — the original's name for it — so an imported
/// `settings.socd` keeps its global profile instead of gaining an empty one.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Profile {
    pub id: String,
    pub program: String,
    pub active: bool,
    pub mappings: Vec<Mapping>,
}

impl Profile {
    pub fn global() -> Self {
        Self {
            id: "global".to_string(),
            program: "Hitboxer Global".to_string(),
            active: true,
            mappings: Vec::new(),
        }
    }

    pub fn is_global(&self) -> bool {
        self.program.eq_ignore_ascii_case("Hitboxer Global")
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Settings {
    pub close_to_tray: bool,
    pub debug: bool,
    pub hook_enabled: bool,
    pub profiles: Vec<Profile>,
    /// The auto clicker's settings. Defaulted rather than required, so a file
    /// written before the clicker existed still loads.
    #[serde(default)]
    pub clicker: crate::clicker::ClickerSettings,
    /// The macro list, defaulted for the same reason.
    #[serde(default)]
    pub macros: Vec<crate::macros::Macro>,
    /// The typer's draft and pace, defaulted for the same reason.
    #[serde(default)]
    pub typer: crate::typer::TyperSettings,
    /// The replayer's recordings and its two shortcuts, defaulted for the same
    /// reason.
    #[serde(default)]
    pub replay: crate::replay::ReplaySettings,
    /// Whether the auto clicker is on. Each tool owns a switch of its own; this
    /// replaced the one global pause, which could stop the remapper without the
    /// person noticing and then persisted that way.
    #[serde(default = "on_by_default")]
    pub clicker_enabled: bool,
    /// Whether the macros are on: their shortcuts, and the Run button.
    #[serde(default = "on_by_default")]
    pub macros_enabled: bool,
    /// Whether the typer is on: its shortcut, and the Start button.
    #[serde(default = "on_by_default")]
    pub typer_enabled: bool,
    /// Whether the replayer is on: its two shortcuts, and both buttons.
    #[serde(default = "on_by_default")]
    pub replay_enabled: bool,
    /// The executable to put in the sign-in entry. Empty means "the copy that
    /// is running", which is right until somebody moves or renames it.
    #[serde(default)]
    pub autostart_exe: String,
}

/// A tool nobody has switched off yet.
fn on_by_default() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            debug: false,
            hook_enabled: true,
            profiles: vec![Profile::global()],
            clicker: crate::clicker::ClickerSettings::default(),
            macros: Vec::new(),
            typer: crate::typer::TyperSettings::default(),
            replay: crate::replay::ReplaySettings::default(),
            clicker_enabled: true,
            macros_enabled: true,
            typer_enabled: true,
            replay_enabled: true,
            autostart_exe: String::new(),
        }
    }
}

/// Read one section of the saved document, falling back to its own default.
///
/// A settings file outlives the code that wrote it: a field changes shape, and
/// the whole file stops parsing. Deserialising it in one go and falling back to
/// `Settings::default()` on any error would silently throw away every profile,
/// macro and clicker setting at once the first time one section changed shape —
/// which is exactly what a user experiences as "it forgot everything". Reading
/// a section at a time means a stale section costs only itself.
fn read_section<T: serde::de::DeserializeOwned>(
    document: &serde_json::Value,
    key: &str,
    fallback: T,
    notes: &mut Vec<String>,
) -> T {
    match document.get(key) {
        None => fallback,
        Some(value) => match serde_json::from_value(value.clone()) {
            Ok(parsed) => parsed,
            Err(error) => {
                notes.push(format!("{key} could not be read ({error}); kept its default"));
                fallback
            }
        },
    }
}

impl Settings {
    /// Rebuild the settings from a saved JSON document, keeping what still fits.
    ///
    /// Returns the settings and a note for every section that had to be dropped,
    /// so the window can say so once instead of quietly starting over.
    pub fn from_document(document: &serde_json::Value) -> (Self, Vec<String>) {
        let defaults = Self::default();
        let mut notes = Vec::new();

        let mut settings = Self {
            close_to_tray: read_section(
                document,
                "close_to_tray",
                defaults.close_to_tray,
                &mut notes,
            ),
            debug: read_section(document, "debug", defaults.debug, &mut notes),
            hook_enabled: read_section(
                document,
                "hook_enabled",
                defaults.hook_enabled,
                &mut notes,
            ),
            profiles: read_section(document, "profiles", defaults.profiles, &mut notes),
            clicker: read_section(document, "clicker", defaults.clicker, &mut notes),
            macros: read_section(document, "macros", defaults.macros, &mut notes),
            typer: read_section(document, "typer", defaults.typer, &mut notes),
            replay: read_section(document, "replay", defaults.replay, &mut notes),
            clicker_enabled: read_section(
                document,
                "clicker_enabled",
                defaults.clicker_enabled,
                &mut notes,
            ),
            macros_enabled: read_section(
                document,
                "macros_enabled",
                defaults.macros_enabled,
                &mut notes,
            ),
            typer_enabled: read_section(
                document,
                "typer_enabled",
                defaults.typer_enabled,
                &mut notes,
            ),
            replay_enabled: read_section(
                document,
                "replay_enabled",
                defaults.replay_enabled,
                &mut notes,
            ),
            autostart_exe: read_section(
                document,
                "autostart_exe",
                defaults.autostart_exe,
                &mut notes,
            ),
        };

        // The window assumes there is always a profile to show, and the global
        // one is what every other profile falls back to.
        if !settings.profiles.iter().any(|profile| profile.is_global()) {
            settings.profiles.insert(0, Profile::global());
        }

        (settings, notes)
    }
}

impl Settings {
    /// Make every shortcut slot usable.
    ///
    /// A settings file outlives the engine that wrote it. One carried `Mouse5`
    /// as the typer's shortcut, which the engine refuses because a mouse button
    /// cannot type, and the replayer's two were empty because the tab's save was
    /// rejected before it reached the handler. Left alone, both read as a feature
    /// that does nothing: the field shows a shortcut and the key never fires.
    /// Every slot now comes up with a combination that works, and anything that
    /// could never fire is replaced with a note saying what it was.
    pub fn heal_shortcuts(&mut self) -> Vec<String> {
        let mut notes = Vec::new();

        let typer_was = self.typer.hotkey.trim().to_string();
        // Asked of the same parser the shortcut layer asks: a mouse button reads
        // as a shortcut everywhere else in the app.
        let typer_usable =
            !typer_was.is_empty() && crate::mouse_hotkeys::parse(&typer_was).is_none();
        if !typer_usable {
            notes.push(if typer_was.is_empty() {
                format!(
                    "the typer had no shortcut, so it now uses {}",
                    crate::typer::DEFAULT_HOTKEY
                )
            } else {
                format!(
                    "the typer shortcut “{typer_was}” could never fire, because a mouse button cannot start typing; it now uses {}",
                    crate::typer::DEFAULT_HOTKEY
                )
            });
            self.typer.hotkey = crate::typer::DEFAULT_HOTKEY.to_string();
        }

        if self.replay.record_hotkey.trim().is_empty() {
            notes.push(format!(
                "the replayer had no record shortcut, so it now uses {}",
                crate::replay::DEFAULT_RECORD_HOTKEY
            ));
            self.replay.record_hotkey = crate::replay::DEFAULT_RECORD_HOTKEY.to_string();
        }
        if self.replay.replay_hotkey.trim().is_empty() {
            notes.push(format!(
                "the replayer had no replay shortcut, so it now uses {}",
                crate::replay::DEFAULT_REPLAY_HOTKEY
            ));
            self.replay.replay_hotkey = crate::replay::DEFAULT_REPLAY_HOTKEY.to_string();
        }
        // Two slots on one chord means one of them can never fire: the layer
        // takes the first and refuses the second with an error nobody reads.
        if self
            .replay
            .record_hotkey
            .eq_ignore_ascii_case(&self.replay.replay_hotkey)
        {
            notes.push(format!(
                "the replayer's record and replay shortcuts were both “{}”, so replay now uses {}",
                self.replay.replay_hotkey,
                crate::replay::DEFAULT_REPLAY_HOTKEY
            ));
            self.replay.replay_hotkey = crate::replay::DEFAULT_REPLAY_HOTKEY.to_string();
        }

        notes
    }
}

impl Settings {
    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    pub fn profile_mut(&mut self, id: &str) -> Option<&mut Profile> {
        self.profiles.iter_mut().find(|profile| profile.id == id)
    }

    /// The profile whose mappings apply to a foreground program.
    ///
    /// A program profile that is switched off falls through to the global one,
    /// which is how the original behaves: the global profile is always live.
    pub fn profile_for(&self, program: &str) -> Option<&Profile> {
        let lower = program.to_ascii_lowercase();
        self.profiles
            .iter()
            .find(|profile| profile.active && profile.program.to_ascii_lowercase() == lower)
            .or_else(|| self.profiles.iter().find(|profile| profile.is_global() && profile.active))
    }

    /// Write the `.socd` text the original app reads.
    pub fn to_socd(&self) -> String {
        let mut out = String::from(
            "[1] # Config version, do not delete this!!! Actually, you're not supposed to modify this by hand at all! You will regret manually editing this file!\n\n",
        );
        out.push_str(&format!("close_to_tray: {}\n", self.close_to_tray));
        out.push_str(&format!("runtime_debug: {}\n", self.debug));

        for profile in &self.profiles {
            out.push_str("PROFILE_START\n");
            if !profile.is_global() {
                out.push_str(&format!("active: {}\n", profile.active));
            }
            out.push_str(&format!("program: {}\n", profile.program));
            out.push_str("platform: WINDOWS\n");
            for mapping in &profile.mappings {
                out.push_str(&format!(
                    "mapping: {} {} {}\n",
                    mapping.mode.as_str(),
                    mapping.source,
                    mapping.dest
                ));
            }
            out.push_str("PROFILE_END\n");
        }

        out
    }

    /// Read a `.socd` file. Returns the settings plus notes about lines that
    /// were skipped, so an import can be reported rather than silently lossy.
    pub fn parse_socd(text: &str) -> (Self, Vec<String>) {
        let mut settings = Settings {
            profiles: Vec::new(),
            ..Settings::default()
        };
        let mut notes = Vec::new();
        let mut current: Option<Profile> = None;
        let mut global_written = false;

        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }

            match line {
                "PROFILE_START" => {
                    current = Some(Profile {
                        id: format!("profile-{index}"),
                        program: String::new(),
                        active: true,
                        mappings: Vec::new(),
                    });
                }
                "PROFILE_END" => {
                    if let Some(mut profile) = current.take() {
                        if profile.program.is_empty() {
                            notes.push(format!("line {}: profile without a program", index + 1));
                            continue;
                        }
                        if profile.is_global() {
                            profile.id = "global".to_string();
                            global_written = true;
                        } else {
                            profile.id = format!("profile-{}", slug(&profile.program));
                        }
                        settings.profiles.push(profile);
                    }
                }
                _ => {
                    let (key, value) = match line.split_once(':') {
                        Some(pair) => (pair.0.trim(), pair.1.trim()),
                        None => {
                            notes.push(format!("line {}: unrecognised text “{line}”", index + 1));
                            continue;
                        }
                    };

                    match key {
                        "close_to_tray" => settings.close_to_tray = value == "true",
                        "runtime_debug" => settings.debug = value == "true",
                        "program" => {
                            if let Some(profile) = current.as_mut() {
                                profile.program = value.to_string();
                            }
                        }
                        "active" => {
                            if let Some(profile) = current.as_mut() {
                                profile.active = value == "true";
                            }
                        }
                        "platform" => {}
                        "mapping" => {
                            let parts: Vec<&str> = value.split_whitespace().collect();
                            let parsed = (|| {
                                let mode = Mode::parse(parts.first()?)?;
                                Some(Mapping {
                                    mode,
                                    source: parts.get(1)?.parse().ok()?,
                                    dest: parts.get(2)?.parse().ok()?,
                                })
                            })();
                            match (parsed, current.as_mut()) {
                                (Some(mapping), Some(profile)) => profile.mappings.push(mapping),
                                _ => notes.push(format!(
                                    "line {}: bad mapping “{value}”",
                                    index + 1
                                )),
                            }
                        }
                        other => notes.push(format!("line {}: ignored “{other}”", index + 1)),
                    }
                }
            }
        }

        if !global_written {
            settings.profiles.insert(0, Profile::global());
        }
        (settings, notes)
    }
}

fn slug(program: &str) -> String {
    program
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "[1] # Config version\n\nclose_to_tray: true\nruntime_debug: false\nPROFILE_START\nprogram: Hitboxer Global\nplatform: WINDOWS\nPROFILE_END\nPROFILE_START\nactive: true\nprogram: Overwatch.exe\nplatform: WINDOWS\nmapping: OPPOSITE 17 31\nmapping: OPPOSITE 30 32\nPROFILE_END\n";

    #[test]
    fn reads_the_shipped_file_shape() {
        let (settings, notes) = Settings::parse_socd(SAMPLE);
        assert!(notes.is_empty(), "{notes:?}");
        assert!(settings.close_to_tray);
        assert!(!settings.debug);
        assert_eq!(settings.profiles.len(), 2);

        let global = &settings.profiles[0];
        assert!(global.is_global());
        assert!(global.mappings.is_empty());

        let over = &settings.profiles[1];
        assert_eq!(over.program, "Overwatch.exe");
        assert_eq!(over.mappings.len(), 2);
        assert_eq!(
            over.mappings[0],
            Mapping {
                mode: Mode::Opposite,
                source: 17,
                dest: 31
            }
        );
    }

    #[test]
    fn round_trips_through_the_file_format() {
        let (settings, _) = Settings::parse_socd(SAMPLE);
        let (again, notes) = Settings::parse_socd(&settings.to_socd());
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(settings.profiles.len(), again.profiles.len());
        assert_eq!(settings.profiles[1].mappings, again.profiles[1].mappings);
    }

    #[test]
    fn bad_mappings_are_reported_not_fatal() {
        let text = "PROFILE_START\nprogram: X.exe\nmapping: SIDEWAYS 1 2\nmapping: REMAP 30 31\nPROFILE_END\n";
        let (settings, notes) = Settings::parse_socd(text);
        assert_eq!(notes.len(), 1, "{notes:?}");
        let profile = settings
            .profiles
            .iter()
            .find(|profile| profile.program == "X.exe")
            .expect("the X.exe profile survives a bad mapping");
        assert_eq!(profile.mappings.len(), 1);
        assert_eq!(profile.mappings[0].mode, Mode::Remap);
    }

    #[test]
    fn a_missing_global_profile_is_added() {
        let text = "PROFILE_START\nprogram: X.exe\nPROFILE_END\n";
        let (settings, _) = Settings::parse_socd(text);
        assert!(settings.profiles[0].is_global());
        assert_eq!(settings.profiles.len(), 2);
    }

    /// A settings file written before the clicker and the macros existed must
    /// still load, with those sections at their defaults.
    #[test]
    fn an_older_settings_file_loads_with_the_new_sections_defaulted() {
        let older = r#"{
            "close_to_tray": true,
            "debug": false,
            "hook_enabled": true,
            "profiles": [
                { "id": "global", "program": "Hitboxer Global", "active": true, "mappings": [] }
            ]
        }"#;

        let settings: Settings = serde_json::from_str(older).expect("an older file should load");
        assert_eq!(settings.clicker.rate, 10.0);
        assert!(settings.macros.is_empty());
        assert_eq!(settings.profiles.len(), 1);
    }

    /// The case that made this worth doing: a section whose shape changed must
    /// not take the profiles, the macros and the clicker's own good fields with
    /// it. Here `edgeStop` still uses the old flat `{ margin, top: bool }` shape.
    #[test]
    fn a_section_that_changed_shape_only_costs_itself() {
        let stale = serde_json::json!({
            "close_to_tray": false,
            "debug": true,
            "hook_enabled": true,
            "autostart_exe": "C:\\Apps\\Hitboxer.exe",
            "profiles": [
                { "id": "global", "program": "Hitboxer Global", "active": true, "mappings": [] },
                { "id": "p2", "program": "Game.exe", "active": true, "mappings": [] }
            ],
            "clicker": {
                "rate": 42.0,
                "edgeStop": { "enabled": true, "margin": 24, "top": true, "left": true }
            },
            "macros": []
        });

        let (settings, notes) = Settings::from_document(&stale);

        // Everything else survived, including the preference switches.
        assert!(!settings.close_to_tray);
        assert!(settings.debug);
        assert_eq!(settings.autostart_exe, "C:\\Apps\\Hitboxer.exe", "C:\\Apps\\Hitboxer.exe");
        assert_eq!(settings.profiles.len(), 2, "both profiles should survive");
        // The clicker fell back whole, and said so once.
        assert_eq!(settings.clicker.rate, 10.0);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("clicker"), "{notes:?}");
    }

    #[test]
    fn a_partial_document_fills_in_the_rest() {
        let (settings, notes) = Settings::from_document(&serde_json::json!({}));
        assert!(notes.is_empty(), "{notes:?}");
        assert!(settings.profiles[0].is_global(), "a global profile is always there");
        assert_eq!(settings.clicker.rate, 10.0);
    }

    /// A file that names no global profile still gets one, because every other
    /// profile falls through to it.
    #[test]
    fn a_document_without_a_global_profile_gets_one_back() {
        let (settings, _) = Settings::from_document(&serde_json::json!({
            "profiles": [
                { "id": "p", "program": "Game.exe", "active": true, "mappings": [] }
            ]
        }));
        assert!(settings.profiles[0].is_global());
        assert_eq!(settings.profiles.len(), 2);
    }

    #[test]
    fn the_active_profile_wins_and_an_inactive_one_falls_back() {
        let mut settings = Settings::default();
        settings.profiles.push(Profile {
            id: "p".into(),
            program: "Game.exe".into(),
            active: false,
            mappings: vec![Mapping {
                mode: Mode::Remap,
                source: 30,
                dest: 31,
            }],
        });
        assert!(settings.profile_for("Game.exe").unwrap().is_global());
        settings.profile_mut("p").unwrap().active = true;
        assert_eq!(settings.profile_for("game.exe").unwrap().id, "p");
        assert!(settings.profile_for("Other.exe").unwrap().is_global());
    }
}

#[cfg(test)]
mod shortcut_healing {
    use super::*;

    #[test]
    fn a_legacy_mouse_shortcut_on_the_typer_is_replaced() {
        let mut settings = Settings::default();
        settings.typer.hotkey = "Mouse5".to_string();
        let notes = settings.heal_shortcuts();
        assert_eq!(settings.typer.hotkey, crate::typer::DEFAULT_HOTKEY);
        assert!(notes.iter().any(|note| note.contains("Mouse5")), "{notes:?}");
    }

    #[test]
    fn the_replayers_empty_slots_come_up_working() {
        let mut settings = Settings::default();
        settings.replay.record_hotkey = String::new();
        settings.replay.replay_hotkey = String::new();
        let notes = settings.heal_shortcuts();
        assert_eq!(
            settings.replay.record_hotkey,
            crate::replay::DEFAULT_RECORD_HOTKEY
        );
        assert_eq!(
            settings.replay.replay_hotkey,
            crate::replay::DEFAULT_REPLAY_HOTKEY
        );
        assert_eq!(notes.len(), 2, "{notes:?}");
    }

    #[test]
    fn a_working_shortcut_is_left_alone() {
        let mut settings = Settings::default();
        settings.typer.hotkey = "Control+Shift+9".to_string();
        assert!(settings.heal_shortcuts().is_empty());
        assert_eq!(settings.typer.hotkey, "Control+Shift+9");
    }

    #[test]
    fn two_tools_on_one_chord_are_split_apart() {
        let mut settings = Settings::default();
        settings.replay.replay_hotkey = crate::replay::DEFAULT_RECORD_HOTKEY.to_string();
        let notes = settings.heal_shortcuts();
        assert_eq!(
            settings.replay.replay_hotkey,
            crate::replay::DEFAULT_REPLAY_HOTKEY
        );
        assert!(notes.iter().any(|note| note.contains("both")), "{notes:?}");
    }

    #[test]
    fn the_defaults_need_no_healing() {
        let mut settings = Settings::default();
        assert!(settings.heal_shortcuts().is_empty());
        assert_eq!(settings.typer.hotkey, crate::typer::DEFAULT_HOTKEY);
    }
}

//! The SOCD engine.
//!
//! Everything here is pure: it takes key events in hardware scan codes and
//! returns what to emit, so the four modes can be tested without a keyboard or
//! a game. The Windows layer above it only feeds events in and calls `SendInput`
//! with what comes back.
//!
//! The behaviour matches the original's decompiled handler
//! (`FUN_14007e1e0` at `0x14007e1e0`, evidence `ev_f1a89683…` / `ev_e6d0b939…`):
//! key-ups log *Releasing opposite* and *Re-pressing opposite*, `REMAP` never
//! deals in opposites, and `OPPOSITE_NO_REPRESS` is its own branch that releases
//! the opposite without pressing it again.

use std::collections::HashSet;

use crate::config::{Mapping, Mode};

/// What to tell Windows, in scan codes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Down(u16),
    Up(u16),
}

/// The answer for one key event.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Decision {
    /// Swallow the physical event.
    pub consume: bool,
    /// What the game should see instead.
    pub actions: Vec<Action>,
    /// Human-readable notes for the debug log.
    pub notes: Vec<String>,
}

impl Decision {
    fn pass() -> Self {
        Self::default()
    }

    fn swallowed() -> Self {
        Self {
            consume: true,
            actions: Vec::new(),
            notes: Vec::new(),
        }
    }
}

#[derive(Default)]
pub struct Engine {
    rules: Vec<Mapping>,
    /// Physically held mapped keys, oldest first.
    held: Vec<u16>,
    /// Keys the game currently sees as down.
    visible: HashSet<u16>,
    /// Keys that must not come back until they are released and pressed again.
    blocked: HashSet<u16>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install the mappings of the active profile.
    ///
    /// Returns the events needed to avoid stuck keys when the rules change
    /// mid-press: every key we made visible is released first.
    pub fn set_rules(&mut self, rules: Vec<Mapping>) -> Vec<Action> {
        let mut actions: Vec<Action> = self.visible.iter().map(|&scan| Action::Up(scan)).collect();
        self.rules = rules;
        self.held.clear();
        self.visible.clear();
        self.blocked.clear();
        actions.sort_by_key(|action| match action {
            Action::Down(scan) | Action::Up(scan) => *scan,
        });
        actions
    }

    pub fn rules(&self) -> &[Mapping] {
        &self.rules
    }

    fn rule_for(&self, scan: u16) -> Option<&Mapping> {
        self.rules.iter().find(|rule| rule.source == scan)
    }

    /// Handle one physical key event.
    ///
    /// `repeat` marks the auto-repeat the low-level hook delivers while a key
    /// is held; it never changes state.
    pub fn on_key(&mut self, scan: u16, down: bool, repeat: bool) -> Decision {
        let Some(rule) = self.rule_for(scan).copied() else {
            // Not mapped: let it through, but remember nothing.
            return Decision::pass();
        };

        if down {
            if !repeat && !self.held.contains(&scan) {
                self.held.push(scan);
            }
        } else {
            self.held.retain(|&held| held != scan);
            // A real release is what unblocks a key.
            self.blocked.remove(&scan);
        }

        let mut decision = Decision::swallowed();

        match rule.mode {
            Mode::Remap => {
                if down {
                    if self.visible.insert(rule.dest) {
                        decision.actions.push(Action::Down(rule.dest));
                    }
                } else if self.visible.remove(&rule.dest) {
                    decision.actions.push(Action::Up(rule.dest));
                }
            }
            Mode::Opposite | Mode::OppositeNoRepress | Mode::Neutral => {
                self.resolve_pair(rule, &mut decision);
            }
        }

        decision
    }

    /// Diff the pair's desired state against what the game currently sees.
    fn resolve_pair(&mut self, rule: Mapping, decision: &mut Decision) {
        let pair = [rule.source, rule.dest];
        let held_in_pair: Vec<u16> = self
            .held
            .iter()
            .copied()
            .filter(|scan| pair.contains(scan))
            .collect();

        let desired: Vec<u16> = match rule.mode {
            Mode::Opposite => held_in_pair.last().copied().into_iter().collect(),
            Mode::OppositeNoRepress => held_in_pair
                .iter()
                .rev()
                .find(|scan| !self.blocked.contains(scan))
                .copied()
                .into_iter()
                .collect(),
            // One key held is live; two cancel, and neither returns until it is
            // released and pressed again.
            Mode::Neutral => {
                if held_in_pair.len() == 1 && !self.blocked.contains(&held_in_pair[0]) {
                    vec![held_in_pair[0]]
                } else {
                    Vec::new()
                }
            }
            Mode::Remap => Vec::new(),
        };

        // Anything of the pair the game sees but that should not be down: hide it.
        for &key in &pair {
            if self.visible.contains(&key) && !desired.contains(&key) {
                self.visible.remove(&key);
                decision.actions.push(Action::Up(key));
                // Losing a race is what blocks a key in the no-repress modes.
                if rule.mode != Mode::Opposite && self.held.contains(&key) {
                    self.blocked.insert(key);
                }
                if self.held.contains(&key) {
                    decision.notes.push(format!("releasing opposite {key}"));
                }
            }
        }

        // Anything that should be down but is not: bring it back.
        for key in desired {
            if self.visible.insert(key) {
                decision.actions.push(Action::Down(key));
                if self.held.contains(&key) {
                    decision.notes.push(format!("re-pressing opposite {key}"));
                }
            }
        }
    }

    /// Release everything the engine made visible, e.g. when the hook stops.
    pub fn release_all(&mut self) -> Vec<Action> {
        let mut actions: Vec<Action> = self.visible.drain().map(Action::Up).collect();
        self.held.clear();
        self.blocked.clear();
        actions.sort_by_key(|action| match action {
            Action::Down(scan) | Action::Up(scan) => *scan,
        });
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u16 = 30;
    const D: u16 = 32;

    fn pair(mode: Mode) -> Vec<Mapping> {
        vec![
            Mapping {
                mode,
                source: A,
                dest: D,
            },
            Mapping {
                mode,
                source: D,
                dest: A,
            },
        ]
    }

    fn engine(mode: Mode) -> Engine {
        let mut engine = Engine::new();
        engine.set_rules(pair(mode));
        engine
    }

    #[test]
    fn unmapped_keys_are_never_swallowed() {
        let mut engine = engine(Mode::Opposite);
        let decision = engine.on_key(17, true, false);
        assert!(!decision.consume);
        assert!(decision.actions.is_empty());
    }

    #[test]
    fn opposite_hands_over_and_hands_back() {
        let mut engine = engine(Mode::Opposite);

        let press_a = engine.on_key(A, true, false);
        assert!(press_a.consume);
        assert_eq!(press_a.actions, vec![Action::Down(A)]);

        let press_d = engine.on_key(D, true, false);
        assert_eq!(press_d.actions, vec![Action::Up(A), Action::Down(D)]);

        let release_d = engine.on_key(D, false, false);
        assert_eq!(
            release_d.actions,
            vec![Action::Up(D), Action::Down(A)],
            "the key still held comes back"
        );
    }

    #[test]
    fn no_repress_releases_without_pressing_again() {
        let mut engine = engine(Mode::OppositeNoRepress);
        engine.on_key(A, true, false);
        assert_eq!(
            engine.on_key(D, true, false).actions,
            vec![Action::Up(A), Action::Down(D)]
        );
        assert_eq!(
            engine.on_key(D, false, false).actions,
            vec![Action::Up(D)],
            "nothing returns until A is released and pressed again"
        );
        // A is still physically down, so its next event is a repeat: no output.
        assert!(engine.on_key(A, true, true).actions.is_empty());
        // Releasing and re-pressing A brings it back.
        engine.on_key(A, false, false);
        assert_eq!(
            engine.on_key(A, true, false).actions,
            vec![Action::Down(A)]
        );
    }

    #[test]
    fn neutral_cancels_both() {
        let mut engine = engine(Mode::Neutral);
        assert_eq!(
            engine.on_key(A, true, false).actions,
            vec![Action::Down(A)]
        );
        assert_eq!(
            engine.on_key(D, true, false).actions,
            vec![Action::Up(A)],
            "the second key cancels instead of winning"
        );
        assert!(
            engine.on_key(D, false, false).actions.is_empty(),
            "release does not resurrect the other direction"
        );
        assert!(engine.on_key(A, false, false).actions.is_empty());
        assert_eq!(
            engine.on_key(A, true, false).actions,
            vec![Action::Down(A)],
            "a fresh press works again"
        );
    }

    #[test]
    fn remap_produces_the_destination_only() {
        let mut engine = Engine::new();
        engine.set_rules(vec![Mapping {
            mode: Mode::Remap,
            source: 17,
            dest: 31,
        }]);
        assert_eq!(
            engine.on_key(17, true, false).actions,
            vec![Action::Down(31)]
        );
        assert!(
            engine.on_key(17, true, true).actions.is_empty(),
            "repeats do not re-send"
        );
        assert_eq!(
            engine.on_key(17, false, false).actions,
            vec![Action::Up(31)]
        );
    }

    #[test]
    fn changing_rules_mid_press_leaves_nothing_stuck() {
        let mut engine = engine(Mode::Opposite);
        engine.on_key(A, true, false);
        let released = engine.set_rules(pair(Mode::Neutral));
        assert_eq!(released, vec![Action::Up(A)]);
        assert!(engine.visible.is_empty());
    }

    #[test]
    fn stopping_the_hook_releases_everything() {
        let mut engine = engine(Mode::Opposite);
        engine.on_key(A, true, false);
        engine.on_key(D, true, false);
        assert_eq!(engine.release_all(), vec![Action::Up(D)]);
    }
}

//! The road entry trace: what the simulation appends to `EdgeUseManager`'s
//! edge lists inside the game's step, for two games' logs to be compared
//! (docs/HOOKS.md, "The road entry trace"). Logging only; it rides on the
//! road entry fix's two detours ([`crate::order::road`]), so nothing new is
//! hooked.
//!
//! Soak 7 of 2026-10-02 on `twomptest` (three games, no input, `51e1e4d`):
//! cat's game alone said `Diverged { step: 25950, lanes: [3] }`, one bus
//! (vehicle 198452, line-10) 0.6 m behind the others' at equal speed at step
//! 26000, a second (282046, line-12) leaving its stop about 0.6 s later by
//! 26050, the economy equal. The road fix's in-step milestone line, equal in
//! all three games for 77 milestones, differed in cat's at the one reached
//! near step 25850 (`appends=5111808 reordered=1383817` against
//! `1383824`): the simulation's own appends had already gone another way
//! somewhere in steps 25520..25850, while every vehicle's place still
//! agreed to the centimetre at the checkpoint of step 25900. A milestone
//! comes every 65,536 appends, about 330 steps; this module narrows it:
//!
//! - **At every checkpoint** (always, while the fix sorts) one line with
//!   the in-step appends since the last one: counts, persons (`Add`) and
//!   vehicles (`AddRange`) apart, an order-free hash of the appends and a
//!   hash of their sequence. The first checkpoint whose line differs
//!   between two games bounds the first append that differs to 50 steps.
//! - **In a window of steps** ([`TRACE_ENV`]) one line per in-step append:
//!   the entity, its edges, its range and bounds.
//! - **A recorder** ([`RECORD_ENV`]): the same lines for the last `n`
//!   steps kept in memory and written when the room asks for a lane dump
//!   (every game hears the ask), so the appends before a split no one could
//!   predict are in every game's log.
//!
//! For the entities [`ENTITIES_ENV`] lists (entity ids, comma-separated),
//! a traced append also lists who is on each edge it touched, after the
//! sort: the vehicles and persons ahead of and behind a watched vehicle.

use std::collections::{HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};

use crate::order::Fnv1a;

/// `from-to` (room steps, both included): every in-step append in those
/// steps is logged.
pub const TRACE_ENV: &str = "TPF3MP_HOOK_ROAD_ENTRY_TRACE";
/// A number of steps: the last that many steps' append lines are kept in
/// memory and written when the room asks for a lane dump.
pub const RECORD_ENV: &str = "TPF3MP_HOOK_ROAD_ENTRY_RECORD";
/// A list of entity ids, comma-separated; their appends list each edge's
/// entries.
pub const ENTITIES_ENV: &str = "TPF3MP_HOOK_WATCH_ENTITIES";
/// The recorder keeps at most this many steps (about 200 appends a step on
/// `twomptest`: some 800,000 lines, tens of megabytes).
pub const MAX_RECORD_STEPS: u64 = 4000;
/// Edges listed in one line; the rest are counted.
pub const MAX_EDGES_LISTED: usize = 8;
/// Entries listed per edge for a watched entity's append.
pub const MAX_ENTRIES_LISTED: usize = 16;

/// The entity ids `value` lists (commas, spaces or semicolons between
/// them); what does not read as one is left out.
pub fn parse_entities(value: Option<&str>) -> HashSet<i32> {
    value
        .unwrap_or("")
        .split([',', ' ', ';'])
        .filter_map(|word| word.trim().parse().ok())
        .collect()
}

/// Who appended: `EdgeUseManager::Add` is the persons' (from
/// `PersonMoveSystem`), `AddRange` the vehicles' (from
/// `LandVehicleMoveSystem`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Person,
    Vehicle,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Person => "person",
            Kind::Vehicle => "vehicle",
        }
    }
}

/// An edge: its entity, index and direction byte (the 12-byte `EdgeId`'s
/// meaningful bytes; the other three are padding).
pub type EdgeKey = (i32, i32, u8);

/// One edge touched and its list after the sort (`None`: unreadable).
pub type EdgeEntries = (EdgeKey, Option<Vec<Listed>>);

/// One entry of an edge's list, as listed: entity, component, back, front.
pub type Listed = (i32, i32, f32, f32);

/// One append, as the detour saw it.
#[derive(Debug, Clone, PartialEq)]
pub struct Append {
    pub kind: Kind,
    pub entity: i32,
    pub component: i32,
    /// `AddRange`'s current path index; `None` for `Add`.
    pub current: Option<i32>,
    /// `AddRange`'s `from..=to`; `None` for `Add`.
    pub range: Option<(i32, i32)>,
    /// The `{back, front}` floats, as passed (one 8-byte argument).
    pub bounds: u64,
    /// The edges appended to, in order; an unreadable one is left out.
    pub edges: Vec<EdgeKey>,
}

impl Append {
    /// FNV-1a over everything two agreeing games share: kind, entity,
    /// component, current index, range, the bounds' bits, each edge's
    /// meaningful bytes.
    pub fn hash(&self) -> u64 {
        let mut h = Fnv1a::new();
        h.write(&[match self.kind {
            Kind::Person => 1,
            Kind::Vehicle => 2,
        }]);
        h.write_u32(self.entity as u32);
        h.write_u32(self.component as u32);
        h.write_u32(self.current.unwrap_or(-1) as u32);
        let (from, to) = self.range.unwrap_or((-1, -1));
        h.write_u32(from as u32);
        h.write_u32(to as u32);
        h.write(&self.bounds.to_le_bytes());
        h.write_u32(self.edges.len() as u32);
        for (entity, index, dir) in &self.edges {
            h.write_u32(*entity as u32);
            h.write_u32(*index as u32);
            h.write(&[*dir]);
        }
        h.0
    }
}

/// The in-step appends between two checkpoint lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Digest {
    pub appends: u64,
    pub persons: u64,
    pub vehicles: u64,
    /// Edges whose list the fix put back in entity order.
    pub reordered: u64,
    /// The appends' hashes summed (wrapping): equal whatever their order.
    pub set: u64,
    /// The appends' hashes chained in order.
    pub sequence: u64,
}

impl Default for Digest {
    fn default() -> Self {
        Self {
            appends: 0,
            persons: 0,
            vehicles: 0,
            reordered: 0,
            set: 0,
            sequence: Fnv1a::new().0,
        }
    }
}

impl Digest {
    pub fn note(&mut self, append: &Append, reordered: u64) {
        let hash = append.hash();
        self.appends += 1;
        match append.kind {
            Kind::Person => self.persons += 1,
            Kind::Vehicle => self.vehicles += 1,
        }
        self.reordered += reordered;
        self.set = self.set.wrapping_add(hash);
        let mut chain = Fnv1a(self.sequence);
        chain.write(&hash.to_le_bytes());
        self.sequence = chain.0;
    }
}

/// The checkpoint line: two games' lines for one step must be equal.
pub fn checkpoint_line(step: u64, digest: &Digest) -> String {
    format!(
        "road-entry: step {step}: in-step appends={} persons={} vehicles={} reordered={} set={:016x} sequence={:016x}",
        digest.appends,
        digest.persons,
        digest.vehicles,
        digest.reordered,
        digest.set,
        digest.sequence
    )
}

fn edge_text((entity, index, dir): &EdgeKey) -> String {
    format!("{entity}/{index}/{dir}")
}

fn bounds_text(bounds: u64) -> String {
    let back = f32::from_bits(bounds as u32);
    let front = f32::from_bits((bounds >> 32) as u32);
    format!("{back:?},{front:?}")
}

/// One append's line. `step` is the room's step of the update running
/// (`None`, `-`, outside one), `index` the append's number within that
/// step, `reordered` how many of its edges the fix reordered, `entries`
/// each touched edge's list after the sort (a watched entity's append
/// only).
pub fn trace_line(
    step: Option<u64>,
    index: u64,
    append: &Append,
    reordered: u64,
    entries: Option<&[EdgeEntries]>,
) -> String {
    let step = step.map_or("-".to_owned(), |s| s.to_string());
    let current = append.current.map_or("-".to_owned(), |c| c.to_string());
    let range = append
        .range
        .map_or("-".to_owned(), |(from, to)| format!("{from}..{to}"));
    let mut edges: Vec<String> = append
        .edges
        .iter()
        .take(MAX_EDGES_LISTED)
        .map(edge_text)
        .collect();
    if append.edges.len() > MAX_EDGES_LISTED {
        edges.push(format!("+{}", append.edges.len() - MAX_EDGES_LISTED));
    }
    let mut text = format!(
        "road: step {step} #{index} {} {} comp {} cur {current} range {range} bounds {} edges {} {} reordered {reordered}",
        append.kind.name(),
        append.entity,
        append.component,
        bounds_text(append.bounds),
        append.edges.len(),
        edges.join(",")
    );
    for (edge, listed) in entries.unwrap_or(&[]) {
        text.push_str(&format!(" on {}:", edge_text(edge)));
        match listed {
            None => text.push_str(" unreadable"),
            Some(listed) => {
                text.push_str(&format!(" {}", listed.len()));
                for (entity, component, back, front) in listed.iter().take(MAX_ENTRIES_LISTED) {
                    text.push_str(&format!(" {entity}:{component}:{back:?}:{front:?}"));
                }
            }
        }
    }
    text
}

/// The trace's settings, from the environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub window: Option<(u64, u64)>,
    pub record: u64,
    pub watched: HashSet<i32>,
}

impl Config {
    /// From [`TRACE_ENV`], [`RECORD_ENV`] and [`ENTITIES_ENV`]'s values,
    /// and what to say in hook.log about them. A value that does not read
    /// leaves that part off and says so.
    pub fn from_env(
        window: Option<&str>,
        record: Option<&str>,
        entities: Option<&str>,
    ) -> (Self, Vec<String>) {
        let mut said = Vec::new();
        let mut config = Config::default();
        if let Some(value) = window.map(str::trim).filter(|v| !v.is_empty()) {
            match crate::lanedump::parse_step_range(value) {
                Some(range) => config.window = Some(range),
                None => said.push(format!(
                    "road-entry trace: {TRACE_ENV}={value} is not a step range such as 25500-25950; no window is traced"
                )),
            }
        }
        if let Some(value) = record.map(str::trim).filter(|v| !v.is_empty()) {
            match value.parse::<u64>() {
                Ok(n) if (1..=MAX_RECORD_STEPS).contains(&n) => config.record = n,
                _ => said.push(format!(
                    "road-entry trace: {RECORD_ENV}={value} is not a number of steps from 1 to {MAX_RECORD_STEPS}; nothing is recorded"
                )),
            }
        }
        config.watched = parse_entities(entities);
        let window = match config.window {
            Some((from, to)) => format!("steps {from} to {to} traced"),
            None => "no window traced".to_owned(),
        };
        let record = match config.record {
            0 => "nothing recorded".to_owned(),
            n => format!("the last {n} steps recorded for a lane dump"),
        };
        let mut watched: Vec<i32> = config.watched.iter().copied().collect();
        watched.sort_unstable();
        let watched = if watched.is_empty() {
            String::new()
        } else {
            let ids: Vec<String> = watched.iter().map(i32::to_string).collect();
            format!(", edge lists said for {}", ids.join(","))
        };
        said.push(format!(
            "road-entry trace: a digest of the in-step appends at every checkpoint; {window}, {record}{watched}"
        ));
        (config, said)
    }

    /// Whether an append in `step` gets a line (traced or recorded).
    pub fn lines_for(&self, step: Option<u64>) -> bool {
        self.record > 0
            || step.is_some_and(|s| {
                self.window
                    .is_some_and(|(from, to)| (from..=to).contains(&s))
            })
    }

    pub fn traced(&self, step: Option<u64>) -> bool {
        step.is_some_and(|s| {
            self.window
                .is_some_and(|(from, to)| (from..=to).contains(&s))
        })
    }
}

/// The last steps' lines.
#[derive(Debug, Default)]
pub struct Recorder {
    keep: u64,
    lines: VecDeque<(u64, String)>,
}

impl Recorder {
    pub fn new(keep: u64) -> Self {
        Self {
            keep,
            lines: VecDeque::new(),
        }
    }

    /// Keeps `line` of `step`, and forgets the lines more than `keep` steps
    /// before it.
    pub fn push(&mut self, step: u64, line: String) {
        if self.keep == 0 {
            return;
        }
        self.lines.push_back((step, line));
        let oldest = step.saturating_sub(self.keep - 1);
        while self.lines.front().is_some_and(|(s, _)| *s < oldest) {
            self.lines.pop_front();
        }
    }

    /// Everything kept, oldest first, and the steps it covers; empties it.
    pub fn take(&mut self) -> (Option<(u64, u64)>, Vec<String>) {
        let span = match (self.lines.front(), self.lines.back()) {
            (Some((first, _)), Some((last, _))) => Some((*first, *last)),
            _ => None,
        };
        (span, self.lines.drain(..).map(|(_, line)| line).collect())
    }
}

/// What the trace keeps while the game runs.
#[derive(Debug, Default)]
struct State {
    digest: Digest,
    /// The step the last append was in, and how many it had.
    step: Option<u64>,
    index: u64,
    recorder: Recorder,
}

static CONFIG: OnceLock<Config> = OnceLock::new();
static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state() -> std::sync::MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(|p| p.into_inner())
}

/// Reads the environment once, at install; returns the lines for hook.log.
pub fn configure_from_env() -> Vec<String> {
    let (config, said) = Config::from_env(
        std::env::var(TRACE_ENV).ok().as_deref(),
        std::env::var(RECORD_ENV).ok().as_deref(),
        std::env::var(ENTITIES_ENV).ok().as_deref(),
    );
    let keep = config.record;
    let _ = CONFIG.set(config);
    *state() = Some(State {
        recorder: Recorder::new(keep),
        ..State::default()
    });
    said
}

fn config() -> Option<&'static Config> {
    CONFIG.get()
}

/// Whether the detour should read the edges' lists for this append (a
/// watched entity's, in a step that gets lines).
pub fn wants_entries(entity: i32, step: Option<u64>) -> bool {
    config().is_some_and(|c| c.watched.contains(&entity) && c.lines_for(step))
}

/// An append inside the game's step: into the digest, and a line when the
/// window or the recorder wants one. `step` is the room's step of the
/// update running.
pub fn note(step: Option<u64>, append: &Append, reordered: u64, entries: Option<&[EdgeEntries]>) {
    let Some(config) = config() else {
        return;
    };
    let mut guard = state();
    let Some(state) = guard.as_mut() else {
        return;
    };
    state.digest.note(append, reordered);
    if state.step != step {
        state.step = step;
        state.index = 0;
    }
    state.index += 1;
    if !config.lines_for(step) {
        return;
    }
    let line = trace_line(step, state.index, append, reordered, entries);
    if let Some(step) = step
        && config.record > 0
    {
        state.recorder.push(step, line.clone());
    }
    if config.traced(step) {
        drop(guard);
        crate::log::line(&line);
    }
}

/// The checkpoint line for `step`, with the appends since the last one;
/// `None` before the trace is configured.
pub fn take_checkpoint(step: u64) -> Option<String> {
    let mut guard = state();
    let state = guard.as_mut()?;
    let digest = std::mem::take(&mut state.digest);
    Some(checkpoint_line(step, &digest))
}

/// The room asked for a lane dump: the recorded lines go to hook.log, with
/// a line before them saying what they are.
pub fn flush_recorded(why: &str) {
    let (span, lines) = {
        let mut guard = state();
        match guard.as_mut() {
            Some(state) => state.recorder.take(),
            None => return,
        }
    };
    let Some((first, last)) = span else {
        return;
    };
    crate::log::line(&format!(
        "road-entry record: {} in-step append(s) of steps {first} to {last} ({why})",
        lines.len()
    ));
    for line in lines {
        crate::log::line(&line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vehicle(entity: i32, edges: Vec<EdgeKey>) -> Append {
        Append {
            kind: Kind::Vehicle,
            entity,
            component: 7,
            current: Some(2),
            range: Some((3, 4)),
            bounds: u64::from(1.5f32.to_bits()) | (u64::from(4.25f32.to_bits()) << 32),
            edges,
        }
    }

    fn person(entity: i32) -> Append {
        Append {
            kind: Kind::Person,
            entity,
            component: 1,
            current: None,
            range: None,
            bounds: 0,
            edges: vec![(248490, 2, 1)],
        }
    }

    #[test]
    fn the_set_hash_ignores_order_and_the_sequence_does_not() {
        let (a, b) = (vehicle(198452, vec![(1, 2, 1)]), person(5));
        let mut ab = Digest::default();
        ab.note(&a, 1);
        ab.note(&b, 0);
        let mut ba = Digest::default();
        ba.note(&b, 0);
        ba.note(&a, 1);
        assert_eq!(ab.set, ba.set);
        assert_ne!(ab.sequence, ba.sequence);
        assert_eq!(
            (ab.appends, ab.persons, ab.vehicles, ab.reordered),
            (2, 1, 1, 1)
        );
        assert_eq!(
            ab,
            Digest {
                sequence: ab.sequence,
                ..ba
            }
        );
    }

    #[test]
    fn every_field_two_games_share_is_hashed() {
        let base = vehicle(198452, vec![(1, 2, 1)]);
        let changed = [
            Append {
                entity: 198453,
                ..base.clone()
            },
            Append {
                component: 8,
                ..base.clone()
            },
            Append {
                current: Some(3),
                ..base.clone()
            },
            Append {
                range: Some((3, 5)),
                ..base.clone()
            },
            Append {
                bounds: base.bounds + 1,
                ..base.clone()
            },
            Append {
                edges: vec![(1, 2, 0)],
                ..base.clone()
            },
            Append {
                edges: vec![(1, 2, 1), (1, 3, 1)],
                ..base.clone()
            },
            Append {
                kind: Kind::Person,
                ..base.clone()
            },
        ];
        for other in changed {
            assert_ne!(base.hash(), other.hash(), "{other:?}");
        }
        assert_eq!(base.hash(), base.clone().hash());
    }

    #[test]
    fn the_checkpoint_line_holds_only_what_agreeing_games_share() {
        let mut digest = Digest::default();
        digest.note(&person(5), 0);
        let line = checkpoint_line(25900, &digest);
        assert!(
            line.starts_with(
                "road-entry: step 25900: in-step appends=1 persons=1 vehicles=0 reordered=0 set="
            ),
            "{line}"
        );
        assert_eq!(
            checkpoint_line(50, &Digest::default()),
            "road-entry: step 50: in-step appends=0 persons=0 vehicles=0 reordered=0 set=0000000000000000 sequence=cbf29ce484222325"
        );
    }

    #[test]
    fn a_trace_line_names_the_append_and_a_watched_entitys_neighbours() {
        let append = vehicle(198452, vec![(247349, 4, 1), (247354, 1, 1)]);
        assert_eq!(
            trace_line(Some(25901), 3, &append, 1, None),
            "road: step 25901 #3 vehicle 198452 comp 7 cur 2 range 3..4 bounds 1.5,4.25 edges 2 247349/4/1,247354/1/1 reordered 1"
        );
        let entries = [
            (
                (247349, 4, 1),
                Some(vec![(198452, 7, 1.5, 4.25), (336853, 9, 6.0, 9.5)]),
            ),
            ((247354, 1, 1), None),
        ];
        assert_eq!(
            trace_line(None, 1, &person(5), 0, Some(&entries)),
            "road: step - #1 person 5 comp 1 cur - range - bounds 0.0,0.0 edges 1 248490/2/1 reordered 0 on 247349/4/1: 2 198452:7:1.5:4.25 336853:9:6.0:9.5 on 247354/1/1: unreadable"
        );
        let long = vehicle(1, (0..10).map(|i| (100, i, 1)).collect());
        assert!(trace_line(Some(1), 1, &long, 0, None).contains("edges 10 100/0/1,"));
        assert!(trace_line(Some(1), 1, &long, 0, None).contains("100/7/1,+2 reordered"));
    }

    #[test]
    fn the_recorder_keeps_the_last_steps_only() {
        let mut recorder = Recorder::new(3);
        for step in 10..=15 {
            recorder.push(step, format!("a{step}"));
            recorder.push(step, format!("b{step}"));
        }
        let (span, lines) = recorder.take();
        assert_eq!(span, Some((13, 15)));
        assert_eq!(lines, ["a13", "b13", "a14", "b14", "a15", "b15"]);
        assert_eq!(recorder.take(), (None, Vec::<String>::new()));
        let mut off = Recorder::new(0);
        off.push(1, "x".into());
        assert_eq!(off.take().0, None);
    }

    #[test]
    fn the_settings_read_or_say_why_not() {
        let (config, said) =
            Config::from_env(Some("25500-25950"), Some("600"), Some("198452,282046"));
        assert_eq!(config.window, Some((25500, 25950)));
        assert_eq!(config.record, 600);
        assert_eq!(config.watched, HashSet::from([198452, 282046]));
        assert_eq!(
            said,
            [
                "road-entry trace: a digest of the in-step appends at every checkpoint; steps 25500 to 25950 traced, the last 600 steps recorded for a lane dump, edge lists said for 198452,282046"
            ]
        );
        assert!(config.traced(Some(25500)) && config.traced(Some(25950)));
        assert!(!config.traced(Some(25951)) && !config.traced(None));
        assert!(config.lines_for(Some(1)), "the recorder wants every step");

        let (config, said) = Config::from_env(None, None, None);
        assert_eq!(config, Config::default());
        assert!(!config.lines_for(Some(1)));
        assert_eq!(said.len(), 1);
        assert!(said[0].contains("no window traced, nothing recorded"));

        let (config, said) = Config::from_env(Some("9-1"), Some("5000"), None);
        assert_eq!((config.window, config.record), (None, 0));
        assert_eq!(said.len(), 3);
        assert!(said[0].contains("is not a step range"));
        assert!(said[1].contains("from 1 to 4000"));
    }
}

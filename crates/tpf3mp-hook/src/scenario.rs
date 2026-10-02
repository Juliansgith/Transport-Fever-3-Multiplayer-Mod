//! The hook's test mode (docs/REGRESSION.md, "With the real game"): a game
//! started with [`ENV`] naming a scenario file plays its actor's part of
//! that scenario unattended, and every game of the room says in its
//! `hook.log` what became of every scripted action and what its world looks
//! like, for the logs of the room's games to be diffed.
//!
//! Off unless [`ENV`] is set, and [`ACTOR_ENV`] with it; a file that does
//! not read or check is refused whole and the game plays as any other
//! (fail closed). Nothing here acts outside the room's game.
//!
//! - **Actions** go the way a player's do once the mod has captured them:
//!   the step driver hands them to the room (`Session::command`), the room
//!   orders them, and every game applies them in the same update. Each game
//!   knows the whole scenario, so each names every scripted action it
//!   applies, its own or another actor's, by the item it matches: the first
//!   item not yet matched whose action is the same.
//! - **The observation**: at the first checkpoint of the room's game, and
//!   every `observe_every` checkpoints after it, the mod's game script reads
//!   a summary of the world (tpf3mp/observe.lua) and the hook writes it to
//!   the log as `scenario: observe step <n> <json>`. The first is the
//!   *baseline*: the scenario's steps count from it, and the places and ids
//!   its actions name are read from it.
//!
//! The file is JSON:
//!
//! ```json
//! { "name": "roads", "observe_every": 10,
//!   "items": [ { "at": 100, "actor": 0, "origin": "spot:#0:0",
//!                "expect": "applied", "note": "a straight road",
//!                "action": { "BuildRoad": { ... } } } ] }
//! ```
//!
//! `at` is in the room's steps after the baseline's; `actor` is the game
//! whose [`ACTOR_ENV`] says that number. `action` is an action of the
//! schema as serde writes it (`tpf3mp_proto::action::Action`, positions in
//! millimetres), where any of these placeholders may stand for a value:
//!
//! - `{"$pos": [dx, dy]}` or `[dx, dy, dz]`: a position (`Pos`), `dx` metres
//!   east and `dy` north of the item's `origin`, `dz` above it;
//!   `{"$pos2": [dx, dy]}` the same on the ground plane (`Pos2`);
//! - `{"$id": "lines+0"}`: the id the baseline's registry gives next for
//!   `lines` (also `vehicles`, `groups`, `towns`, `industries`, and
//!   `companies`, the room's roster, and `loans`, the room's loans), plus 0:
//!   the first the scenario makes;
//! - `{"$edge": "<town>:<k>"}`: the ends of the `k`th street the baseline
//!   lists in that town (`EdgeEnds`); `<town>:free:<k>` one no town
//!   building stands near, which a bulldozer takes alone;
//! - `{"$z": dz}`: the origin's height plus `dz` metres, in millimetres (a
//!   terrain cell's height);
//! - `{"$building": "<town>:<k>"}`: the `k`th town building the baseline
//!   lists nearest that town's centre (`ConstructionRef`);
//! - `{"$cell2": [dx, dy]}`: as `$pos2`, put on the corner of the terrain's
//!   4 m height cell there (a `Terraform`'s origin).
//!
//! An `origin` is `town:<town>` (the town's centre), `spot:<town>:<k>` (the
//! `k`th free flat place the baseline found near it), `water:<town>:<k>`
//! (the `k`th place on water it found near it) or `none` (absolute
//! metres, the default); `<town>` is `#<i>`, the baseline's `i`th town, or
//! the town's name.

use std::{
    collections::HashMap,
    fmt::Write as _,
    sync::{
        Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::Deserialize;
use serde_json::{Map, Value, json};
use tpf3mp_proto::{PlayerId, action::Action};

use crate::step::Handed;

/// Names the scenario file, in the game's environment.
pub const ENV: &str = tpf3mp_ipc::SCENARIO_ENV;
/// This game's actor in it, from 0.
pub const ACTOR_ENV: &str = tpf3mp_ipc::SCENARIO_ACTOR_ENV;
/// The scenario's tickets begin here, far above the mod's own: an answer to
/// one of them is the scenario's, never the GUI's.
pub const TICKET_BASE: u64 = 1 << 40;
/// Longest scenario file read.
pub const MAX_FILE: u64 = 4 * 1024 * 1024;
/// Longest observation the mod may hand over.
pub const MAX_OBSERVATION: usize = 256 * 1024;
/// Most items one scenario has.
pub const MAX_ITEMS: usize = 4096;
/// Most scripted actions handed to the room in one call of the game's step:
/// the room takes 20 a second from a player, after a burst of 40.
pub const MAX_PER_CALL: usize = 4;

/// What an item's author expects of its action.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Expect {
    /// Every game applies it.
    #[default]
    Applied,
    /// Every game refuses it, or the room does.
    Refused,
    /// Either; logged all the same.
    Any,
}

/// One action of the scenario.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    /// Steps after the baseline's.
    pub at: u64,
    pub actor: u8,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub expect: Expect,
    #[serde(default)]
    pub note: String,
    pub action: Value,
}

/// A scenario file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    #[serde(default)]
    pub comment: Value,
    /// Checkpoints between two observations after the baseline.
    #[serde(default = "default_observe_every")]
    pub observe_every: u32,
    pub items: Vec<Item>,
}

fn default_observe_every() -> u32 {
    10
}

impl Scenario {
    /// Reads and checks a scenario: every item's action must resolve against
    /// a baseline that answers every place and id with zeros, so a typo is
    /// found before any game plays it.
    pub fn parse(text: &str) -> Result<Self, String> {
        let scenario: Self =
            serde_json::from_str(text).map_err(|error| format!("not a scenario: {error}"))?;
        if scenario.items.is_empty() {
            return Err("a scenario with no items".into());
        }
        if scenario.items.len() > MAX_ITEMS {
            return Err(format!("more than {MAX_ITEMS} items"));
        }
        if scenario.observe_every == 0 {
            return Err("observe_every is at least 1".into());
        }
        let probe = Baseline::probe();
        for (index, item) in scenario.items.iter().enumerate() {
            resolve(item, &probe).map_err(|why| format!("item {index} (at {}): {why}", item.at))?;
        }
        Ok(scenario)
    }

    /// The actors it names: one more than the highest.
    pub fn actors(&self) -> usize {
        self.items
            .iter()
            .map(|item| usize::from(item.actor) + 1)
            .max()
            .unwrap_or(0)
    }
}

/// The test mode a game's environment asks for: `None` when [`ENV`] is
/// unset or empty; otherwise the runner, or why the test mode is refused.
pub fn from_env(
    get: impl Fn(&str) -> Option<String>,
    read: impl Fn(&str) -> Result<String, String>,
) -> Option<Result<Runner, String>> {
    let path = get(ENV).filter(|path| !path.is_empty())?;
    Some((|| {
        let actor = get(ACTOR_ENV)
            .ok_or_else(|| format!("{ENV} is set but {ACTOR_ENV} is not"))?
            .trim()
            .parse::<u8>()
            .map_err(|_| format!("{ACTOR_ENV} is an actor's number, 0 to 255"))?;
        let text = read(&path).map_err(|why| format!("reading {path}: {why}"))?;
        let scenario = Scenario::parse(&text).map_err(|why| format!("{path}: {why}"))?;
        Ok(Runner::new(scenario, actor))
    })())
}

/// Reads a scenario file, at most [`MAX_FILE`] bytes.
pub fn read_file(path: &str) -> Result<String, String> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut text = String::new();
    file.take(MAX_FILE + 1)
        .read_to_string(&mut text)
        .map_err(|error| error.to_string())?;
    if text.len() as u64 > MAX_FILE {
        return Err(format!("longer than {MAX_FILE} bytes"));
    }
    Ok(text)
}

/// What the scenario's places and ids are read from: the first
/// observation of the room's game.
#[derive(Debug, Clone)]
pub struct Baseline {
    pub step: u64,
    observation: Value,
    /// Answers every place and id with zeros, for checking a file.
    probe: bool,
}

impl Baseline {
    pub fn new(step: u64, observation: Value) -> Self {
        Self {
            step,
            observation,
            probe: false,
        }
    }

    fn probe() -> Self {
        Self {
            step: 0,
            observation: Value::Null,
            probe: true,
        }
    }

    fn towns(&self) -> &[Value] {
        self.observation
            .get("towns")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice)
    }

    fn town(&self, selector: &str) -> Result<&Value, String> {
        let towns = self.towns();
        let found = match selector.strip_prefix('#') {
            Some(index) => {
                let index: usize = index
                    .parse()
                    .map_err(|_| format!("no town {selector:?}: #<number>"))?;
                towns.get(index)
            }
            None => towns
                .iter()
                .find(|town| town.get("name").and_then(Value::as_str) == Some(selector)),
        };
        found.ok_or_else(|| {
            format!(
                "the baseline has no town {selector:?} ({} towns)",
                towns.len()
            )
        })
    }

    /// An origin, in metres.
    fn origin(&self, origin: Option<&str>) -> Result<[f64; 3], String> {
        let origin = origin.unwrap_or("none");
        if origin == "none" {
            return Ok([0.0; 3]);
        }
        let (kind, rest) = origin
            .split_once(':')
            .ok_or_else(|| format!("an origin {origin:?}: none, town:<town> or spot:<town>:<k>"))?;
        let (town, place) = match kind {
            "town" => (rest, None),
            "spot" | "water" => {
                let (town, k) = rest
                    .rsplit_once(':')
                    .ok_or_else(|| format!("an origin {origin:?}: {kind}:<town>:<k>"))?;
                let k: usize = k
                    .parse()
                    .map_err(|_| format!("an origin {origin:?}: {kind}:<town>:<k>"))?;
                (town, Some(k))
            }
            _ => {
                return Err(format!(
                    "an origin {origin:?}: none, town:<town>, spot:<town>:<k> or water:<town>:<k>"
                ));
            }
        };
        let list = if kind == "water" { "waters" } else { "spots" };
        if self.probe {
            return Ok([0.0; 3]);
        }
        let town = self.town(town)?;
        let at = match place {
            None => town,
            Some(k) => town
                .get(list)
                .and_then(Value::as_array)
                .and_then(|spots| spots.get(k))
                .ok_or_else(|| format!("the baseline found no {list} {k} for {origin:?}"))?,
        };
        point(at).ok_or_else(|| format!("the baseline's {origin:?} has no x, y and z"))
    }

    fn id(&self, name: &str) -> Result<u64, String> {
        let (kind, offset) = name.split_once('+').unwrap_or((name, "0"));
        let offset: u64 = offset
            .parse()
            .map_err(|_| format!("an id {name:?}: <kind>+<n>"))?;
        if !matches!(
            kind,
            "lines" | "vehicles" | "groups" | "towns" | "industries" | "companies" | "loans"
        ) {
            return Err(format!(
                "an id of kind {kind:?}: lines, vehicles, groups, towns, industries, companies or loans"
            ));
        }
        if self.probe {
            return Ok(offset);
        }
        let next = self
            .observation
            .get("next")
            .and_then(|next| next.get(kind))
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("the baseline has no next id of {kind}"))?;
        Ok(next + offset)
    }

    fn edge(&self, name: &str) -> Result<Value, String> {
        let (town, k) = name
            .rsplit_once(':')
            .ok_or_else(|| format!("an edge {name:?}: <town>:<k>"))?;
        // `<town>:free:<k>`: the `k`th street no town building stands near.
        let (town, list) = match town.strip_suffix(":free") {
            Some(town) => (town, "free_streets"),
            None => (town, "streets"),
        };
        let k: usize = k
            .parse()
            .map_err(|_| format!("an edge {name:?}: <town>:<k> or <town>:free:<k>"))?;
        let ends = if self.probe {
            [[0.0; 3], [0.0; 3]]
        } else {
            let street = self
                .town(town)?
                .get(list)
                .and_then(Value::as_array)
                .and_then(|streets| streets.get(k))
                .ok_or_else(|| format!("the baseline lists no {list} {k} in {town:?}"))?;
            let a = street.get("a").and_then(point);
            let b = street.get("b").and_then(point);
            match (a, b) {
                (Some(a), Some(b)) => [a, b],
                _ => return Err(format!("the baseline's street {name:?} has no ends")),
            }
        };
        Ok(json!({ "a": pos(ends[0]), "b": pos(ends[1]) }))
    }

    /// A town building the baseline listed near a town's centre, as a
    /// `ConstructionRef`.
    fn building(&self, name: &str) -> Result<Value, String> {
        let (town, k) = name
            .rsplit_once(':')
            .ok_or_else(|| format!("a building {name:?}: <town>:<k>"))?;
        let k: usize = k
            .parse()
            .map_err(|_| format!("a building {name:?}: <town>:<k>"))?;
        if self.probe {
            return Ok(json!({ "file": "probe.con", "at": pos([0.0; 3]) }));
        }
        let building = self
            .town(town)?
            .get("town_buildings")
            .and_then(Value::as_array)
            .and_then(|list| list.get(k))
            .ok_or_else(|| format!("the baseline lists no town building {k} in {town:?}"))?;
        let file = building
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("the baseline's town building {name:?} has no file"))?;
        let at = point(building)
            .ok_or_else(|| format!("the baseline's town building {name:?} has no place"))?;
        Ok(json!({ "file": file, "at": pos(at) }))
    }
}

/// The side of the terrain's height cells, in metres
/// (`api.engine.terrain.getBaseResolution` on build 40408).
pub const CELL: f64 = 4.0;

/// A point `{x, y, z}` or `[x, y, z]` in metres.
fn point(value: &Value) -> Option<[f64; 3]> {
    let get = |key: &str, index: usize| {
        value
            .get(key)
            .or_else(|| value.get(index))
            .and_then(Value::as_f64)
    };
    Some([get("x", 0)?, get("y", 1)?, get("z", 2).unwrap_or(0.0)])
}

/// Metres to the schema's millimetres.
#[allow(clippy::cast_possible_truncation)]
fn mm(metres: f64) -> Result<i64, String> {
    let value = (metres * 1000.0).round();
    if !value.is_finite() || value.abs() > f64::from(i32::MAX) {
        return Err(format!("{metres} m is off the map"));
    }
    Ok(value as i64)
}

fn pos(at: [f64; 3]) -> Value {
    #[allow(clippy::cast_possible_truncation)]
    let mm = |v: f64| (v * 1000.0).round() as i64;
    json!({ "x": mm(at[0]), "y": mm(at[1]), "z": mm(at[2]) })
}

/// The offsets a `$pos` names: two or three numbers.
fn offsets(value: &Value) -> Result<[f64; 3], String> {
    let list = value
        .as_array()
        .filter(|list| (2..=3).contains(&list.len()))
        .ok_or("a $pos is [dx, dy] or [dx, dy, dz], in metres")?;
    let mut out = [0.0; 3];
    for (slot, item) in out.iter_mut().zip(list) {
        *slot = item.as_f64().ok_or("a $pos is numbers")?;
    }
    Ok(out)
}

/// Replaces the placeholders in `value`.
fn expand(value: &Value, origin: [f64; 3], baseline: &Baseline) -> Result<Value, String> {
    match value {
        Value::Object(map) if map.len() == 1 => {
            let (key, inner) = map.iter().next().expect("one entry");
            match key.as_str() {
                "$pos" | "$pos2" => {
                    let d = offsets(inner)?;
                    let x = mm(origin[0] + d[0])?;
                    let y = mm(origin[1] + d[1])?;
                    if key == "$pos2" {
                        Ok(json!({ "x": x, "y": y }))
                    } else {
                        Ok(json!({ "x": x, "y": y, "z": mm(origin[2] + d[2])? }))
                    }
                }
                "$id" => {
                    let name = inner
                        .as_str()
                        .ok_or("an $id is a string, such as lines+0")?;
                    Ok(json!(baseline.id(name)?))
                }
                "$edge" => {
                    let name = inner.as_str().ok_or("an $edge is a string, such as #0:0")?;
                    baseline.edge(name)
                }
                "$building" => {
                    let name = inner
                        .as_str()
                        .ok_or("a $building is a string, such as #0:0")?;
                    baseline.building(name)
                }
                "$z" => {
                    let dz = inner.as_f64().ok_or("a $z is metres above the origin")?;
                    Ok(json!(mm(origin[2] + dz)?))
                }
                "$cell2" => {
                    let d = offsets(inner)?;
                    let corner = |v: f64| (v / CELL).floor() * CELL;
                    Ok(
                        json!({ "x": mm(corner(origin[0] + d[0]))?, "y": mm(corner(origin[1] + d[1]))? }),
                    )
                }
                _ => Ok(Value::Object(
                    [(key.clone(), expand(inner, origin, baseline)?)]
                        .into_iter()
                        .collect(),
                )),
            }
        }
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, inner) in map {
                if key.starts_with('$') {
                    return Err(format!("{key} stands alone in its object"));
                }
                out.insert(key.clone(), expand(inner, origin, baseline)?);
            }
            Ok(Value::Object(out))
        }
        Value::Array(list) => list
            .iter()
            .map(|inner| expand(inner, origin, baseline))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        other => Ok(other.clone()),
    }
}

/// The action an item stands for, against `baseline`, checked by the schema
/// (and so by its encoding, as the room would carry it).
pub fn resolve(item: &Item, baseline: &Baseline) -> Result<Action, String> {
    let origin = baseline.origin(item.origin.as_deref())?;
    let value = expand(&item.action, origin, baseline)?;
    let action: Action =
        serde_json::from_value(value).map_err(|error| format!("not an action: {error}"))?;
    action
        .to_payload()
        .map_err(|error| format!("the room cannot carry it: {error}"))?;
    Ok(action)
}

/// How a scripted action is named in every game's log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub item: usize,
    pub at: u64,
    pub kind: &'static str,
    pub expect: Expect,
}

impl Label {
    fn name(&self) -> String {
        format!("step {} action {} ({})", self.at, self.item, self.kind)
    }

    /// The outcome's line: whether it was what the item expected.
    fn outcome(&self, ok: bool, why: Option<&str>) -> String {
        let what = if ok { "applied" } else { "refused" };
        let verdict = match (self.expect, ok) {
            (Expect::Any, _) | (Expect::Applied, true) | (Expect::Refused, false) => "",
            (Expect::Applied, false) => " UNEXPECTED (expected applied)",
            (Expect::Refused, true) => " UNEXPECTED (expected refused)",
        };
        let mut line = format!("scenario: {} {what}{verdict}", self.name());
        if let Some(why) = why.filter(|why| !why.is_empty()) {
            let _ = write!(line, ": {why}");
        }
        line
    }
}

/// The scenario as one game plays it.
#[derive(Debug)]
pub struct Runner {
    scenario: Scenario,
    actor: u8,
    baseline: Option<Baseline>,
    /// Each item's action against the baseline, or why it does not resolve.
    resolved: Vec<Option<Result<Action, String>>>,
    /// This game's items handed over (or given up).
    handed: Vec<bool>,
    /// Items an ordered action matched already.
    matched: Vec<bool>,
    /// The tickets of this game's items.
    tickets: HashMap<u64, usize>,
    /// Checkpoints observed since the baseline, and the last one asked for.
    checkpoints: u64,
    asked: Option<u64>,
    /// Outcomes so far: applied, refused, unexpected, unscripted.
    applied: u64,
    refused: u64,
    unexpected: u64,
    unscripted: u64,
    done_said: bool,
}

impl Runner {
    pub fn new(scenario: Scenario, actor: u8) -> Self {
        let n = scenario.items.len();
        Self {
            scenario,
            actor,
            baseline: None,
            resolved: vec![None; n],
            handed: vec![false; n],
            matched: vec![false; n],
            tickets: HashMap::new(),
            checkpoints: 0,
            asked: None,
            applied: 0,
            refused: 0,
            unexpected: 0,
            unscripted: 0,
            done_said: false,
        }
    }

    /// The line that says the test mode is on.
    pub fn started(&self) -> String {
        let mine = self
            .scenario
            .items
            .iter()
            .filter(|item| item.actor == self.actor)
            .count();
        format!(
            "scenario: test mode on: {:?}, {} items for {} actors, {mine} of them this game's (actor {}); waiting for the room's game to observe the world",
            self.scenario.name,
            self.scenario.items.len(),
            self.scenario.actors(),
            self.actor
        )
    }

    pub fn baseline(&self) -> Option<&Baseline> {
        self.baseline.as_ref()
    }

    /// A batch ends at the checkpoint after `step`: whether the mod observes
    /// the world there. The baseline first, then every `observe_every`.
    pub fn observe_due(&mut self, step: u64) -> bool {
        if self.asked == Some(step) {
            return false;
        }
        let due = self.baseline.is_none() || {
            self.checkpoints += 1;
            self.checkpoints
                .is_multiple_of(u64::from(self.scenario.observe_every))
        };
        if due {
            self.asked = Some(step);
        }
        due
    }

    /// The mod observed the world after `step`: the log's line, and the
    /// baseline's lines the first time.
    pub fn observed(&mut self, step: u64, text: &str) -> Vec<String> {
        let mut lines = vec![format!("scenario: observe step {step} {text}")];
        if self.baseline.is_some() {
            lines.push(self.progress());
            return lines;
        }
        let observation = match serde_json::from_str::<Value>(text) {
            Ok(value) if value.is_object() => value,
            Ok(_) | Err(_) => {
                lines
                    .push("scenario: the observation is not a JSON object; no baseline yet".into());
                return lines;
            }
        };
        let baseline = Baseline::new(step, observation);
        let mut unresolved = 0;
        for (index, item) in self.scenario.items.iter().enumerate() {
            let result = resolve(item, &baseline);
            if let Err(why) = &result {
                unresolved += 1;
                lines.push(format!(
                    "scenario: step {} action {index} does not resolve against the baseline: {why}",
                    item.at
                ));
            }
            self.resolved[index] = Some(result);
        }
        lines.push(format!(
            "scenario: baseline at step {step}: {} towns; {} of {} items resolved; steps count from {step}",
            baseline.towns().len(),
            self.scenario.items.len() - unresolved,
            self.scenario.items.len()
        ));
        self.baseline = Some(baseline);
        lines
    }

    /// This game's items due before the room's step `next` runs, for the
    /// room, at most [`MAX_PER_CALL`]; and the log's lines.
    pub fn due(&mut self, next: u64) -> (Vec<Handed>, Vec<String>) {
        let mut handed = Vec::new();
        let mut lines = Vec::new();
        let Some(baseline) = &self.baseline else {
            return (handed, lines);
        };
        for (index, item) in self.scenario.items.iter().enumerate() {
            if handed.len() >= MAX_PER_CALL {
                break;
            }
            if item.actor != self.actor
                || self.handed[index]
                || baseline.step.saturating_add(item.at) > next
            {
                continue;
            }
            self.handed[index] = true;
            let label = format!("step {} action {index}", item.at);
            match &self.resolved[index] {
                Some(Ok(action)) => match action.to_payload() {
                    Ok(payload) => {
                        let ticket = TICKET_BASE + index as u64;
                        self.tickets.insert(ticket, index);
                        lines.push(format!(
                            "scenario: {label} ({}) handed to the room{}",
                            action.kind(),
                            if item.note.is_empty() {
                                String::new()
                            } else {
                                format!(": {}", item.note)
                            }
                        ));
                        handed.push((ticket, payload, None));
                    }
                    Err(error) => {
                        self.unexpected += 1;
                        lines.push(format!("scenario: {label} not handed: {error}"));
                    }
                },
                Some(Err(why)) => {
                    if item.expect != Expect::Refused {
                        self.unexpected += 1;
                    }
                    lines.push(format!(
                        "scenario: {label} not handed: it did not resolve: {why}"
                    ));
                }
                None => {}
            }
        }
        (handed, lines)
    }

    /// The label of an action the room ordered: the first item not matched
    /// yet whose action is this one, or `None` (and a line) for one the
    /// scenario does not have.
    pub fn label(&mut self, action: &Action, player: &PlayerId) -> (Option<Label>, Option<String>) {
        for (index, item) in self.scenario.items.iter().enumerate() {
            if self.matched[index] {
                continue;
            }
            if let Some(Ok(resolved)) = &self.resolved[index]
                && resolved == action
            {
                self.matched[index] = true;
                return (
                    Some(Label {
                        item: index,
                        at: item.at,
                        kind: action.kind(),
                        expect: item.expect,
                    }),
                    None,
                );
            }
        }
        self.unscripted += 1;
        let who: String = crate::lobby::hex(player).chars().take(8).collect();
        (
            None,
            Some(format!(
                "scenario: an action not in the scenario ({}) by {who}",
                action.kind()
            )),
        )
    }

    /// What became of a scripted action in this game.
    pub fn outcome(&mut self, label: &Label, ok: bool, why: Option<&str>) -> String {
        if ok {
            self.applied += 1;
        } else {
            self.refused += 1;
        }
        let line = label.outcome(ok, why);
        if line.contains("UNEXPECTED") {
            self.unexpected += 1;
        }
        line
    }

    /// One of this game's items the room refused, or could not be handed:
    /// its line, or `None` for a ticket that is not the scenario's.
    pub fn refused_by_room(&mut self, ticket: u64, why: &str) -> Option<String> {
        let index = self.tickets.remove(&ticket)?;
        let item = &self.scenario.items[index];
        let kind = match &self.resolved[index] {
            Some(Ok(action)) => action.kind(),
            _ => "?",
        };
        self.matched[index] = true;
        let label = Label {
            item: index,
            at: item.at,
            kind,
            expect: item.expect,
        };
        Some(self.outcome(&label, false, Some(why)))
    }

    /// The scenario's progress, for the log.
    pub fn progress(&mut self) -> String {
        let matched = self.matched.iter().filter(|m| **m).count();
        let total = self.scenario.items.len();
        let mut line = format!(
            "scenario: progress: {matched} of {total} items ordered; {} applied, {} refused, {} unexpected, {} unscripted",
            self.applied, self.refused, self.unexpected, self.unscripted
        );
        if matched == total && !self.done_said {
            self.done_said = true;
            line.push_str("; scenario done");
        }
        line
    }
}

// What the mod's game script and the step driver share, on the game's
// simulation threads: the driver arms and reads, the mod's natives
// (crate::lua) fill in.

/// Whether a scenario runs: the mod's natives do nothing otherwise.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// What became of a scripted action in this game: applied or not, and why.
pub type Outcome = (Label, bool, Option<String>);

struct Exchange {
    /// The checkpoint step the mod observes the world after, until it does,
    /// and whether it looks for free places too (the baseline).
    observe: Option<(u64, bool)>,
    /// Observations handed over: the step, and the text.
    observed: Vec<(u64, String)>,
    /// The batch's actions' labels, in order.
    labels: Vec<Option<Label>>,
    /// What became of the labelled ones.
    outcomes: Vec<Outcome>,
}

/// Held by every test that uses the exchange: tests run in parallel.
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

static EXCHANGE: Mutex<Exchange> = Mutex::new(Exchange {
    observe: None,
    observed: Vec::new(),
    labels: Vec::new(),
    outcomes: Vec::new(),
});

fn exchange() -> MutexGuard<'static, Exchange> {
    EXCHANGE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The step driver has a scenario.
pub fn set_active(active: bool) {
    ACTIVE.store(active, Ordering::Release);
}

pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// Before a batch: its actions' labels, and the checkpoint to observe after.
pub fn arm(labels: Vec<Option<Label>>, observe: Option<(u64, bool)>) {
    let mut exchange = exchange();
    exchange.labels = labels;
    exchange.observe = observe;
}

/// `observe()`: the step to observe the world after, once, and whether
/// with free places, or `None`.
pub fn take_observe() -> Option<(u64, bool)> {
    if !active() {
        return None;
    }
    exchange().observe.take()
}

/// `observed(text)`: the mod's observation after `step`.
pub fn observed(step: u64, text: String) {
    if active() {
        exchange().observed.push((step, text));
    }
}

/// `applied(index, ok, entity, why)`, for the scenario.
pub fn applied(index: usize, ok: bool, why: Option<String>) {
    if !active() {
        return;
    }
    let mut exchange = exchange();
    if let Some(Some(label)) = index.checked_sub(1).and_then(|i| exchange.labels.get(i)) {
        let label = label.clone();
        exchange.outcomes.push((label, ok, why));
    }
}

/// After a batch: the observations and outcomes since [`arm`].
pub fn take() -> (Vec<(u64, String)>, Vec<Outcome>) {
    let mut exchange = exchange();
    exchange.labels.clear();
    exchange.observe = None;
    (
        std::mem::take(&mut exchange.observed),
        std::mem::take(&mut exchange.outcomes),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpf3mp_proto::FixedBytes;

    const ROAD: &str = r#"{ "BuildRoad": {
        "street": "::/infrastructure/street/country/country_old_small.street_template",
        "style": null, "bus_lane": false, "tram": "None",
        "polyline": {
            "vertices": [
                { "pos": { "$pos": [0, 0] }, "resolve": "New" },
                { "pos": { "$pos": [100, 0] }, "resolve": "New" } ],
            "links": [ { "from": 0, "to": 1,
                "tangent0": { "x": 100000, "y": 0, "z": 0 },
                "tangent1": { "x": 100000, "y": 0, "z": 0 },
                "structure": "Ground", "kind": null } ],
            "removals": [], "removed_nodes": [] } } }"#;

    fn scenario(items: &str) -> String {
        format!(r#"{{ "name": "t", "observe_every": 2, "items": [ {items} ] }}"#)
    }

    fn item(at: u64, actor: u8, origin: &str, action: &str) -> String {
        format!(r#"{{ "at": {at}, "actor": {actor}, "origin": "{origin}", "action": {action} }}"#)
    }

    const OBSERVATION: &str = r#"{ "towns": [
        { "name": "Aston", "x": 1000, "y": 2000, "z": 50,
          "spots": [ { "x": 1400, "y": 2000, "z": 52 } ],
          "waters": [ { "x": 600, "y": 2000, "z": 0 } ],
          "town_buildings": [ { "file": "::/buildings/a.con", "x": 1001, "y": 2002, "z": 50 } ],
          "streets": [ { "a": [1000, 2000, 50], "b": [1010, 2000, 50] } ],
          "free_streets": [ { "a": [1100, 2000, 50], "b": [1110, 2000, 50] } ] } ],
        "next": { "lines": 7, "vehicles": 30, "groups": 12, "companies": 3, "loans": 4 } }"#;

    fn player(n: u8) -> PlayerId {
        PlayerId(FixedBytes([n; 32]))
    }

    #[test]
    fn a_scenario_reads_and_every_action_is_checked() {
        let text = scenario(&item(10, 0, "spot:#0:0", ROAD));
        let parsed = Scenario::parse(&text).unwrap();
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.actors(), 1);

        // A typo in an action fails the file, not the game later.
        let typo = ROAD.replace("\"Ground\"", "\"Grund\"");
        let error = Scenario::parse(&scenario(&item(10, 0, "none", &typo))).unwrap_err();
        assert!(error.contains("item 0"), "{error}");
        // So do an unknown field, a bad origin, a bad placeholder and no items.
        assert!(Scenario::parse(r#"{ "name": "t", "itemz": [] }"#).is_err());
        assert!(Scenario::parse(&scenario(&item(1, 0, "spot:#0", ROAD))).is_err());
        assert!(Scenario::parse(&scenario(&item(1, 0, "moon:1", ROAD))).is_err());
        let bad = ROAD.replace("[100, 0]", "[100]");
        assert!(Scenario::parse(&scenario(&item(1, 0, "none", &bad))).is_err());
        assert!(Scenario::parse(r#"{ "name": "t", "items": [] }"#).is_err());
    }

    #[test]
    fn the_test_mode_is_off_without_the_variable_and_refused_when_half_set() {
        let none = |_: &str| -> Option<String> { None };
        let unreadable = |_: &str| -> Result<String, String> { Err("no file".into()) };
        assert!(from_env(none, unreadable).is_none());
        let empty = |key: &str| (key == ENV).then(String::new);
        assert!(from_env(empty, unreadable).is_none());

        let no_actor = |key: &str| (key == ENV).then(|| "s.json".to_string());
        let error = from_env(no_actor, unreadable).unwrap().unwrap_err();
        assert!(error.contains(ACTOR_ENV), "{error}");

        let both = |key: &str| match key {
            ENV => Some("s.json".to_string()),
            ACTOR_ENV => Some("1".to_string()),
            _ => None,
        };
        assert!(from_env(both, unreadable).unwrap().is_err());
        let text = scenario(&item(10, 1, "none", ROAD));
        let runner = from_env(both, |_| Ok(text.clone())).unwrap().unwrap();
        assert!(runner.started().contains("actor 1"));
        let garbled = |key: &str| match key {
            ENV => Some("s.json".to_string()),
            ACTOR_ENV => Some("first".to_string()),
            _ => None,
        };
        assert!(from_env(garbled, |_| Ok(text.clone())).unwrap().is_err());
    }

    #[test]
    fn places_and_ids_come_from_the_baseline() {
        let baseline = Baseline::new(40, serde_json::from_str(OBSERVATION).unwrap());
        let text = scenario(&item(10, 0, "spot:Aston:0", ROAD));
        let parsed = Scenario::parse(&text).unwrap();
        let Action::BuildRoad(road) = resolve(&parsed.items[0], &baseline).unwrap() else {
            panic!("a road");
        };
        let ends: Vec<_> = road.polyline.vertices.iter().map(|v| v.pos).collect();
        assert_eq!(
            (ends[0].x, ends[0].y, ends[0].z),
            (1_400_000, 2_000_000, 52_000)
        );
        assert_eq!(ends[1].x, 1_500_000);

        assert_eq!(baseline.id("lines+2").unwrap(), 9);
        assert_eq!(baseline.id("loans+1").unwrap(), 5);
        assert_eq!(
            baseline.edge("#0:free:0").unwrap()["a"],
            json!({ "x": 1_100_000, "y": 2_000_000, "z": 50_000 })
        );
        let z = expand(&json!({ "$z": 2 }), [0.0, 0.0, 52.0], &baseline);
        assert_eq!(z.unwrap(), json!(54_000));
        assert!(baseline.id("moons+0").is_err());
        assert_eq!(
            baseline.edge("#0:0").unwrap(),
            json!({ "a": { "x": 1_000_000, "y": 2_000_000, "z": 50_000 },
                    "b": { "x": 1_010_000, "y": 2_000_000, "z": 50_000 } })
        );
        assert!(baseline.edge("#0:5").is_err());
        assert_eq!(
            baseline.building("Aston:0").unwrap(),
            json!({ "file": "::/buildings/a.con",
                    "at": { "x": 1_001_000, "y": 2_002_000, "z": 50_000 } })
        );
        assert_eq!(
            baseline.origin(Some("water:#0:0")).unwrap(),
            [600.0, 2000.0, 0.0]
        );
        let cell = expand(
            &json!({ "$cell2": [5.5, -1] }),
            [1000.0, 2001.0, 0.0],
            &baseline,
        );
        assert_eq!(cell.unwrap(), json!({ "x": 1_004_000, "y": 2_000_000 }));
        assert!(baseline.origin(Some("town:Nowhere")).is_err());
    }

    #[test]
    fn a_game_hands_over_its_own_items_once_due_and_names_everyones() {
        let assign = r#"{ "AssignLine": { "vehicles": [ { "$id": "vehicles+0" } ],
            "line": { "$id": "lines+0" }, "first_stop": null } }"#;
        let text = scenario(&format!(
            "{}, {}, {}",
            item(10, 0, "town:#0", ROAD),
            item(20, 1, "none", assign),
            item(30, 0, "none", assign)
        ));
        let mut mine = Runner::new(Scenario::parse(&text).unwrap(), 0);
        let mut theirs = Runner::new(Scenario::parse(&text).unwrap(), 1);

        // Nothing before the baseline.
        assert!(mine.due(1000).0.is_empty());
        assert!(mine.observe_due(40));
        assert!(!mine.observe_due(40), "asked once a checkpoint");
        let lines = mine.observed(40, OBSERVATION);
        assert!(lines[0].starts_with("scenario: observe step 40 {"));
        assert!(lines.iter().any(|l| l.contains("3 of 3 items resolved")));
        assert!(theirs.observe_due(40));
        theirs.observed(40, OBSERVATION);

        // Steps count from the baseline.
        assert!(mine.due(49).0.is_empty());
        let (handed, lines) = mine.due(50);
        assert_eq!(handed.len(), 1);
        assert_eq!(handed[0].0, TICKET_BASE);
        assert!(lines[0].contains("step 10 action 0 (BuildRoad) handed"));
        assert!(mine.due(55).0.is_empty(), "handed once");
        assert_eq!(theirs.due(60).0.len(), 1, "the other actor's item");

        // Every game names what the room orders the same way.
        let road = Action::from_payload(&handed[0].1).unwrap();
        let (label, _) = theirs.label(&road, &player(1));
        let label = label.unwrap();
        assert_eq!((label.item, label.at), (0, 10));
        assert_eq!(
            theirs.outcome(&label, true, None),
            "scenario: step 10 action 0 (BuildRoad) applied"
        );
        let refused = theirs.outcome(&label, false, Some("Collision"));
        assert!(refused.ends_with("refused UNEXPECTED (expected applied): Collision"));
        // Two items with the same action match in order.
        let (assign_handed, _) = mine.due(100);
        let action = Action::from_payload(&assign_handed[0].1).unwrap();
        assert_eq!(theirs.label(&action, &player(2)).0.unwrap().item, 1);
        assert_eq!(theirs.label(&action, &player(1)).0.unwrap().item, 2);
        let (none, line) = theirs.label(&action, &player(1));
        assert!(none.is_none() && line.unwrap().contains("not in the scenario"));

        // The room refused one of mine.
        let line = mine.refused_by_room(TICKET_BASE, "RateLimited").unwrap();
        assert!(line.contains("step 10 action 0 (BuildRoad) refused UNEXPECTED"));
        assert!(mine.refused_by_room(5, "not mine").is_none());
    }

    #[test]
    fn observations_follow_the_baseline_every_few_checkpoints() {
        let text = scenario(&item(0, 0, "none", ROAD));
        let mut runner = Runner::new(Scenario::parse(&text).unwrap(), 0);
        assert!(runner.observe_due(10));
        runner.observed(10, "not json");
        assert!(runner.baseline().is_none(), "a garbled one is no baseline");
        assert!(runner.observe_due(20));
        runner.observed(20, OBSERVATION);
        assert_eq!(runner.baseline().unwrap().step, 20);
        let due: Vec<bool> = (3..9).map(|n| runner.observe_due(n * 10)).collect();
        assert_eq!(due, [false, true, false, true, false, true]);
    }

    /// The scenarios the repository ships (tools/scenarios), as every game
    /// would read them.
    #[test]
    fn the_shipped_scenarios_read() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/scenarios");
        let mut read = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let text = read_file(path.to_str().unwrap()).unwrap();
            let scenario =
                Scenario::parse(&text).unwrap_or_else(|why| panic!("{}: {why}", path.display()));
            assert!(scenario.actors() >= 1);
            read += 1;
        }
        assert!(read >= 1, "no scenarios in {}", dir.display());
    }

    #[test]
    fn the_exchange_does_nothing_outside_the_test_mode() {
        let _lock = TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        set_active(false);
        observed(5, "{}".into());
        applied(1, true, None);
        assert!(take_observe().is_none());
        let (observations, outcomes) = take();
        assert!(observations.is_empty() && outcomes.is_empty());

        set_active(true);
        let label = Label {
            item: 3,
            at: 7,
            kind: "Loan",
            expect: Expect::Refused,
        };
        arm(vec![None, Some(label.clone())], Some((99, true)));
        assert_eq!(take_observe(), Some((99, true)));
        assert_eq!(take_observe(), None, "once");
        observed(99, "{}".into());
        applied(1, true, None);
        applied(2, false, Some("no".into()));
        applied(9, true, None);
        let (observations, outcomes) = take();
        assert_eq!(observations, vec![(99, "{}".to_string())]);
        assert_eq!(outcomes, vec![(label, false, Some("no".to_string()))]);
        set_active(false);
    }
}

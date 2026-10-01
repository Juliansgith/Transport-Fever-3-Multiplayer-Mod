//! The street detail tools' builds, read natively (docs/HOOKS.md, "The
//! street detail tools").
//!
//! The crosswalk tool (`UI::CrosswalkModifier`, "Crosswalk Tool") and the
//! crossing tool (`UI::LaneModifier`, "Crossing Tool": a junction's road and
//! tram lanes) tell game scripts nothing of their proposals on build 40408:
//! the game forwards `builder.proposalCreate` for six other tools only, and a
//! click of either was stopped with "no proposal seen" (hook.log,
//! 2026-10-01). Each queues its `WorldBuildProposal` itself, with its own
//! calls of `CommandList::Add`, after the factory 0x9ee860 made the command
//! with `playerInitiated` 1 (its seventh argument, `[rsp+0x30]`, is 1 at every
//! call below). So the click is counted and its apply stopped as every
//! player's build is ([`crate::builds`]), and at those calls the hook reads
//! the proposal here ([`record`]) and keeps it, as the street part game
//! scripts see a proposal in, for the GUI to take for that click
//! (`tpf3mp_native.built(n)`, as the module editor's).
//!
//! The calls (build 40408, found with `tools/tpfre`; each signature matches
//! once in the installed executable):
//!
//! - the crosswalk tool's, 0x5290af in 0x528d30 (`UI::CrosswalkModifier`'s
//!   vf2 and its input action's lambda tail-call it; its own lambda's RTTI
//!   names `UI::CrosswalkModifier::Apply(bool)`), returning to 0x5290b4;
//! - the crossing tool's three, 0x538a80, 0x5391e4 and 0x539368 in 0x5381a0
//!   (`UI::LaneModifier`'s vf2), returning 5 bytes past each.
//!
//! Both tools change junctions only: they take a node's configuration into
//! the proposal with 0xa4ad20, which looks the node up in
//! `nodeConfigsToAdd` (stride 0x80, its entity at + 0x78), and otherwise
//! names it in `nodeConfigsToRemove` (a `vector<int32>`) when it has a
//! configuration and appends a copy of it, which the tool then changes.
//!
//! What is read, from the payload (whose `Proposal` is at its start, the
//! street part first, as [`crate::modules`] reads it; `RegisterUsertypesTransport`,
//! 0x22c1ab0, binds `StreetProposal` with `nodeConfigsToAdd` at 0x60 and
//! `nodeConfigsToRemove` at 0x78, `BaseNodeLaneConnectionAndEntity` with
//! `comp` at 0 and `entity` at 0x78, and `LaneConnection` with `segment0`,
//! `lane0`, `segment1`, `lane1` at 0, 4, 8, 0xc and `withRoad`, `withTram`
//! at 0x10, 0x11; 0x1767d20 binds `BaseNodeConfig`, `TrafficLightConfig` and
//! `TrafficLightState`):
//!
//! - `BaseNodeConfig`: `laneConnections` (`vector<LaneConnection>`, 20 bytes
//!   each, at 0), `crosswalks` at 0x18, `doubleSlipSwitch` at 0x48,
//!   `trafficLightPreference` at 0x4c, `trafficLightConfig` at 0x50 (its
//!   `states` at 0, `trafficLightType` at 0x18), `userModifiedTrafficLightStates`
//!   at 0x70. `crosswalks` is a flat hash set of entities, its control
//!   bytes at its start, its slots at 8, its size at 0x10 and its capacity
//!   at 0x18 (the crosswalk tool's own lookup and clear, 0x529af0, read it
//!   so), walked whole and sorted;
//! - `TrafficLightState` (0x28 bytes): `lockedLanes` (`vector<int32>`) at 0,
//!   `duration` and `minDuration` (`f32`) at 0x18 and 0x1c, `canSkip` at 0x20.
//!
//! TF3's `BaseNodeConfig` binds no `userModifiedLaneConnections`, and the
//! crossing tool writes none: it is read as false, as game scripts read it.
//! A proposal that changes anything else (nodes, edges, stops, constructions)
//! is refused with what it changes. Every read is checked readable, every
//! vector for order and whole elements, every count capped; what does not
//! read is kept as the reason, which the GUI logs and refuses the click with
//! (fail closed).
//!
//! INFERRED, not yet seen in the game: every layout above (static only), and
//! that the crossing tool's three calls are all junction edits (its set, its
//! reset and its tram lanes' mode).

use std::sync::atomic::{AtomicUsize, Ordering};

use tpf3mp_proto::lua::LuaValue;

use crate::modules::{Memory, i32_at, read, u64_at, vector};

/// The profile's names for the tools' calls of `CommandList::Add`, and the
/// tool each is. `Add` returns 5 bytes past each.
pub const CALLS: [(&str, &str); 4] = [
    ("CrosswalkModifier::Apply/Add call", "crosswalk tool"),
    ("LaneModifier::Apply/Add call 1", "crossing tool"),
    ("LaneModifier::Apply/Add call 2", "crossing tool"),
    ("LaneModifier::Apply/Add call 3", "crossing tool"),
];

/// Every offset read, from the payload.
pub mod layout {
    pub use crate::modules::layout::{
        ADDED_NODES, ADDED_SEGMENTS, EDGE_OBJECT_SIZE, EDGE_OBJECTS_TO_ADD, EDGE_OBJECTS_TO_REMOVE,
        ENTITY_SIZE, HEAD_LEN, NODE_SIZE, REMOVED_NODES, REMOVED_SEGMENTS, SEGMENT_SIZE, TO_ADD,
        TO_REMOVE,
    };

    pub const CONFIGS_TO_ADD: usize = 0x060;
    pub const CONFIGS_TO_REMOVE: usize = 0x078;

    /// `BaseNodeLaneConnectionAndEntity`.
    pub const CONFIG_ENTRY_SIZE: usize = 0x80;
    pub const CONFIG_ENTITY: usize = 0x78;

    /// `BaseNodeConfig`.
    pub const CONFIG_SIZE: usize = 0x78;
    pub const LANE_CONNECTIONS: usize = 0x00;
    pub const CROSSWALKS: usize = 0x18;
    pub const DOUBLE_SLIP: usize = 0x48;
    pub const LIGHT_PREFERENCE: usize = 0x4c;
    pub const LIGHT_STATES: usize = 0x50;
    pub const LIGHT_TYPE: usize = 0x68;
    pub const USER_LIGHTS: usize = 0x70;

    /// The crosswalks' hash set, from its start.
    pub const SET_CONTROL: usize = 0x00;
    pub const SET_SLOTS: usize = 0x08;
    pub const SET_SIZE: usize = 0x10;
    pub const SET_CAPACITY: usize = 0x18;
    /// The control byte past the last slot.
    pub const SET_SENTINEL: u8 = 0xff;

    /// `LaneConnection`.
    pub const LANE_CONNECTION_SIZE: usize = 0x14;
    pub const SEGMENT0: usize = 0x00;
    pub const LANE0: usize = 0x04;
    pub const SEGMENT1: usize = 0x08;
    pub const LANE1: usize = 0x0c;
    pub const WITH_ROAD: usize = 0x10;
    pub const WITH_TRAM: usize = 0x11;

    /// `TrafficLightState`.
    pub const STATE_SIZE: usize = 0x28;
    pub const LOCKED_LANES: usize = 0x00;
    pub const DURATION: usize = 0x18;
    pub const MIN_DURATION: usize = 0x1c;
    pub const CAN_SKIP: usize = 0x20;
}

/// Bounds past which a read is a misread; the schema's own
/// ([`tpf3mp_proto::action`]) are checked again when the GUI hands it over.
pub const MAX_CONFIGS: usize = 64;
pub const MAX_LANE_CONNECTIONS: usize = 512;
pub const MAX_CROSSWALKS: usize = 64;
/// Largest crosswalk set capacity read: a junction has a few arms.
pub const MAX_SET_CAPACITY: usize = 1023;
pub const MAX_STATES: usize = 32;
pub const MAX_LOCKED: usize = 512;
/// Most other things a proposal may list before it is a misread.
const MAX_OTHER: usize = 4096;

/// One lane connection, as the game has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub segment0: i32,
    pub lane0: i32,
    pub segment1: i32,
    pub lane1: i32,
    pub with_road: bool,
    pub with_tram: bool,
}

/// One traffic light phase.
#[derive(Debug, Clone, PartialEq)]
pub struct Phase {
    pub locked: Vec<i32>,
    pub duration: f32,
    pub min_duration: f32,
    pub can_skip: bool,
}

/// One node's configuration as the tool set it.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub node: i32,
    pub turns: Vec<Turn>,
    /// Sorted.
    pub crosswalks: Vec<i32>,
    pub double_slip: bool,
    pub light_preference: i32,
    pub light_type: i32,
    pub phases: Vec<Phase>,
    pub user_lights: bool,
}

/// What a street detail tool queued: the configurations it removes and the
/// ones it adds.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub tool: &'static str,
    pub removed: Vec<i32>,
    pub added: Vec<Config>,
}

fn flag(raw: &[u8], at: usize, what: &str) -> Result<bool, String> {
    match raw[at] {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(format!("{what}: a flag of {other}")),
    }
}

fn f32_at(raw: &[u8], at: usize, what: &str) -> Result<f32, String> {
    let mut word = [0u8; 4];
    word.copy_from_slice(&raw[at..at + 4]);
    let value = f32::from_le_bytes(word);
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{what}: {value}"));
    }
    Ok(value)
}

/// The `int32`s of a `vector<int32>` whose head is at `offset` of `head`.
fn ints(
    memory: &dyn Memory,
    head: &[u8],
    offset: usize,
    max: usize,
    what: &str,
) -> Result<Vec<i32>, String> {
    let (begin, count) = vector(head, offset, 4, max, what)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let raw = read(memory, begin, count * 4, what)?;
    Ok((0..count).map(|i| i32_at(&raw, 4 * i)).collect())
}

/// The entities of the flat hash set whose head is at `offset` of `config`,
/// sorted.
fn entity_set(
    memory: &dyn Memory,
    config: &[u8],
    offset: usize,
    what: &str,
) -> Result<Vec<i32>, String> {
    let size = u64_at(config, offset + layout::SET_SIZE);
    if size == 0 {
        return Ok(Vec::new());
    }
    let size = usize::try_from(size)
        .ok()
        .filter(|size| *size <= MAX_CROSSWALKS)
        .ok_or_else(|| format!("{what}: {size}, more than a junction has"))?;
    let capacity = usize::try_from(u64_at(config, offset + layout::SET_CAPACITY))
        .ok()
        .filter(|c| *c <= MAX_SET_CAPACITY && (*c + 1).is_power_of_two() && *c >= size)
        .ok_or_else(|| format!("{what}: a capacity that is no table's"))?;
    let control = usize::try_from(u64_at(config, offset + layout::SET_CONTROL))
        .map_err(|_| format!("{what}: its control bytes"))?;
    let slots = usize::try_from(u64_at(config, offset + layout::SET_SLOTS))
        .map_err(|_| format!("{what}: its slots"))?;
    let bytes = read(memory, control, capacity + 1, what)?;
    if bytes[capacity] != layout::SET_SENTINEL {
        return Err(format!("{what}: no sentinel after its last slot"));
    }
    let raw = read(memory, slots, capacity * 4, what)?;
    let mut out: Vec<i32> = (0..capacity)
        .filter(|i| (bytes[*i] as i8) >= 0)
        .map(|i| i32_at(&raw, 4 * i))
        .collect();
    if out.len() != size {
        return Err(format!(
            "{what}: a set of {size} that holds {} entities",
            out.len()
        ));
    }
    out.sort_unstable();
    Ok(out)
}

fn config(memory: &dyn Memory, entry: &[u8], k: usize) -> Result<Config, String> {
    let what = |part: &str| format!("junction {k}'s {part}");
    let node = i32_at(entry, layout::CONFIG_ENTITY);
    let (begin, count) = vector(
        entry,
        layout::LANE_CONNECTIONS,
        layout::LANE_CONNECTION_SIZE,
        MAX_LANE_CONNECTIONS,
        &what("lane connections"),
    )?;
    let mut turns = Vec::with_capacity(count);
    if count > 0 {
        let raw = read(
            memory,
            begin,
            count * layout::LANE_CONNECTION_SIZE,
            &what("lane connections"),
        )?;
        for i in 0..count {
            let at = i * layout::LANE_CONNECTION_SIZE;
            turns.push(Turn {
                segment0: i32_at(&raw, at + layout::SEGMENT0),
                lane0: i32_at(&raw, at + layout::LANE0),
                segment1: i32_at(&raw, at + layout::SEGMENT1),
                lane1: i32_at(&raw, at + layout::LANE1),
                with_road: flag(&raw, at + layout::WITH_ROAD, &what("lane connection"))?,
                with_tram: flag(&raw, at + layout::WITH_TRAM, &what("lane connection"))?,
            });
        }
    }
    let crosswalks = entity_set(memory, entry, layout::CROSSWALKS, &what("crosswalks"))?;
    let (begin, count) = vector(
        entry,
        layout::LIGHT_STATES,
        layout::STATE_SIZE,
        MAX_STATES,
        &what("light phases"),
    )?;
    let mut phases = Vec::with_capacity(count);
    if count > 0 {
        let raw = read(
            memory,
            begin,
            count * layout::STATE_SIZE,
            &what("light phases"),
        )?;
        for i in 0..count {
            let state = &raw[i * layout::STATE_SIZE..(i + 1) * layout::STATE_SIZE];
            phases.push(Phase {
                locked: ints(
                    memory,
                    state,
                    layout::LOCKED_LANES,
                    MAX_LOCKED,
                    &what("light phase"),
                )?,
                duration: f32_at(state, layout::DURATION, &what("light phase's duration"))?,
                min_duration: f32_at(
                    state,
                    layout::MIN_DURATION,
                    &what("light phase's least duration"),
                )?,
                can_skip: flag(state, layout::CAN_SKIP, &what("light phase"))?,
            });
        }
    }
    Ok(Config {
        node,
        turns,
        crosswalks,
        double_slip: flag(entry, layout::DOUBLE_SLIP, &what("double slip switch"))?,
        light_preference: i32_at(entry, layout::LIGHT_PREFERENCE),
        light_type: i32_at(entry, layout::LIGHT_TYPE),
        phases,
        user_lights: flag(entry, layout::USER_LIGHTS, &what("light setting"))?,
    })
}

/// Reads the junction edit a street detail tool queued, from the
/// `WorldBuildProposal` payload at `payload`.
pub fn decode(memory: &dyn Memory, payload: usize, tool: &'static str) -> Result<Edit, String> {
    let head = read(memory, payload, layout::HEAD_LEN, "the proposal")?;
    // Nothing but junctions.
    let others = [
        ("node(s) added", layout::ADDED_NODES, layout::NODE_SIZE),
        (
            "edge(s) added",
            layout::ADDED_SEGMENTS,
            layout::SEGMENT_SIZE,
        ),
        ("node(s) removed", layout::REMOVED_NODES, layout::NODE_SIZE),
        (
            "edge(s) removed",
            layout::REMOVED_SEGMENTS,
            layout::SEGMENT_SIZE,
        ),
        (
            "stop(s) or signal(s) added",
            layout::EDGE_OBJECTS_TO_ADD,
            layout::EDGE_OBJECT_SIZE,
        ),
        (
            "stop(s) or signal(s) removed",
            layout::EDGE_OBJECTS_TO_REMOVE,
            4,
        ),
        ("construction(s) added", layout::TO_ADD, layout::ENTITY_SIZE),
        ("construction(s) removed", layout::TO_REMOVE, 4),
    ];
    let mut changes = Vec::new();
    for (what, offset, stride) in others {
        let (_, count) = vector(&head, offset, stride, MAX_OTHER, what)?;
        if count > 0 {
            changes.push(format!("{count} {what}"));
        }
    }
    if !changes.is_empty() {
        return Err(format!(
            "a build that changes more than junctions: {}",
            changes.join(", ")
        ));
    }
    let removed = ints(
        memory,
        &head,
        layout::CONFIGS_TO_REMOVE,
        MAX_CONFIGS,
        "the junction configurations removed",
    )?;
    let (begin, count) = vector(
        &head,
        layout::CONFIGS_TO_ADD,
        layout::CONFIG_ENTRY_SIZE,
        MAX_CONFIGS,
        "the junction configurations added",
    )?;
    let mut added = Vec::with_capacity(count);
    if count > 0 {
        let raw = read(
            memory,
            begin,
            count * layout::CONFIG_ENTRY_SIZE,
            "the junction configurations added",
        )?;
        for k in 0..count {
            let entry = &raw[k * layout::CONFIG_ENTRY_SIZE..(k + 1) * layout::CONFIG_ENTRY_SIZE];
            added.push(config(memory, entry, k + 1)?);
        }
    }
    if removed.is_empty() && added.is_empty() {
        return Err("a build that changes nothing".into());
    }
    Ok(Edit {
        tool,
        removed,
        added,
    })
}

fn key(name: &str) -> LuaValue {
    LuaValue::string(name)
}

fn list(items: impl IntoIterator<Item = LuaValue>) -> LuaValue {
    LuaValue::Table(
        items
            .into_iter()
            .enumerate()
            .map(|(i, item)| (LuaValue::Integer(i as i64 + 1), item))
            .collect(),
    )
}

fn int(n: i32) -> LuaValue {
    LuaValue::Integer(i64::from(n))
}

impl Config {
    fn to_lua(&self) -> LuaValue {
        let turns = self.turns.iter().map(|t| {
            LuaValue::Table(vec![
                (key("segment0"), int(t.segment0)),
                (key("lane0"), int(t.lane0)),
                (key("segment1"), int(t.segment1)),
                (key("lane1"), int(t.lane1)),
                (key("withRoad"), LuaValue::Boolean(t.with_road)),
                (key("withTram"), LuaValue::Boolean(t.with_tram)),
            ])
        });
        let states = self.phases.iter().map(|p| {
            LuaValue::Table(vec![
                (key("lockedLanes"), list(p.locked.iter().map(|i| int(*i)))),
                (key("duration"), LuaValue::Number(f64::from(p.duration))),
                (
                    key("minDuration"),
                    LuaValue::Number(f64::from(p.min_duration)),
                ),
                (key("canSkip"), LuaValue::Boolean(p.can_skip)),
            ])
        });
        let comp = LuaValue::Table(vec![
            (key("laneConnections"), list(turns)),
            (
                key("crosswalks"),
                list(self.crosswalks.iter().map(|e| int(*e))),
            ),
            (key("doubleSlipSwitch"), LuaValue::Boolean(self.double_slip)),
            (key("trafficLightPreference"), int(self.light_preference)),
            (
                key("trafficLightConfig"),
                LuaValue::Table(vec![
                    (key("trafficLightType"), int(self.light_type)),
                    (key("states"), list(states)),
                ]),
            ),
            (
                key("userModifiedTrafficLightStates"),
                LuaValue::Boolean(self.user_lights),
            ),
        ]);
        LuaValue::Table(vec![(key("entity"), int(self.node)), (key("comp"), comp)])
    }
}

impl Edit {
    /// The edit as game scripts see a proposal (`builder.proposalCreate`'s
    /// `param[1]`), its street part's `nodeConfigsToAdd` and
    /// `nodeConfigsToRemove` and every other list empty, with `junctions`
    /// naming the tool: the GUI takes it for the click and makes the action
    /// of it as of the traffic light tool's (`capture.junctions`).
    pub fn to_lua(&self) -> LuaValue {
        let empty = || LuaValue::Table(Vec::new());
        let street = LuaValue::Table(vec![
            (key("addedNodes"), empty()),
            (key("addedSegments"), empty()),
            (key("removedNodes"), empty()),
            (key("removedSegments"), empty()),
            (key("edgeObjectsToAdd"), empty()),
            (key("edgeObjectsToRemove"), empty()),
            (
                key("nodeConfigsToRemove"),
                list(self.removed.iter().map(|e| int(*e))),
            ),
            (
                key("nodeConfigsToAdd"),
                list(self.added.iter().map(Config::to_lua)),
            ),
        ]);
        LuaValue::Table(vec![
            (key("junctions"), LuaValue::string(self.tool)),
            (key("toAdd"), empty()),
            (key("toRemove"), empty()),
            (key("proposal"), street),
        ])
    }

    /// One line for the log.
    pub fn summary(&self) -> String {
        let configs: Vec<String> = self
            .added
            .iter()
            .map(|c| {
                format!(
                    "+cfg{}{{tl={} lc={} cw={} phases={} dss={} um=false/{}}}",
                    c.node,
                    c.light_preference,
                    c.turns.len(),
                    c.crosswalks.len(),
                    c.phases.len(),
                    c.double_slip,
                    c.user_lights
                )
            })
            .collect();
        format!(
            "{}: removes the configuration of {:?}; adds {}",
            self.tool,
            self.removed,
            if configs.is_empty() {
                "none".to_owned()
            } else {
                configs.join(" ")
            }
        )
    }
}

/// Where `Add` returns to from each tool's calls; 0 where the profile does
/// not name one.
static RETURNS: [AtomicUsize; 4] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];

/// Notes the `index`th of [`CALLS`] at `call` (its absolute address): `Add`
/// returns 5 bytes past it.
pub fn set_call(index: usize, call: usize) {
    if let Some(slot) = RETURNS.get(index) {
        slot.store(call + 5, Ordering::Release);
    }
}

/// The tool whose call of `Add` returns to `return_address`, if it is one
/// of the street detail tools'.
pub fn tool_at(return_address: usize) -> Option<&'static str> {
    if return_address == 0 {
        return None;
    }
    RETURNS
        .iter()
        .zip(CALLS)
        .find(|(slot, _)| slot.load(Ordering::Acquire) == return_address)
        .map(|(_, (_, tool))| tool)
}

/// Keeps what a street detail tool queued at click `click` (the count before
/// it) for the GUI, as [`crate::modules::record`] keeps the module editor's:
/// the edit, or why the room cannot carry it, said as the tool's. Logs one
/// line.
pub fn record(memory: &dyn Memory, click: u64, payload: usize, tool: &'static str) {
    let read = decode(memory, payload, tool);
    match &read {
        Ok(edit) => crate::log::line(&format!(
            "junction tool: click {click} queued {}",
            edit.summary()
        )),
        Err(why) => crate::log::line(&format!(
            "junction tool: click {click} of the {tool} cannot go to the room: {why}"
        )),
    }
    crate::modules::keep(
        click,
        read.map(|edit| edit.to_lua())
            .map_err(|why| format!("junction tool: the {tool}'s change: {why}")),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Memory laid out by a test: regions by base address.
    #[derive(Default)]
    struct Fake {
        regions: Vec<(usize, Vec<u8>)>,
        next: usize,
    }

    impl Memory for Fake {
        fn read(&self, address: usize, len: usize) -> Option<Vec<u8>> {
            self.regions.iter().find_map(|(base, bytes)| {
                let offset = address.checked_sub(*base)?;
                bytes
                    .get(offset..offset.checked_add(len)?)
                    .map(<[u8]>::to_vec)
            })
        }
    }

    impl Fake {
        fn new() -> Self {
            Self {
                regions: Vec::new(),
                next: 0x200_0000,
            }
        }
        fn alloc(&mut self, bytes: Vec<u8>) -> usize {
            let at = self.next;
            self.next += ((bytes.len() + 0xfff) & !0xfff) + 0x1000;
            self.regions.push((at, bytes));
            at
        }
        fn bytes(&mut self, at: usize) -> &mut Vec<u8> {
            &mut self.regions.iter_mut().find(|(b, _)| *b == at).unwrap().1
        }
    }

    fn put_u64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn put_i32(bytes: &mut [u8], at: usize, value: i32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put_vector(bytes: &mut [u8], at: usize, base: usize, count: usize, stride: usize) {
        let end = if count == 0 { 0 } else { base + count * stride };
        put_u64(bytes, at, if count == 0 { 0 } else { base as u64 });
        put_u64(bytes, at + 8, end as u64);
        put_u64(bytes, at + 16, end as u64);
    }
    fn put_ints(fake: &mut Fake, bytes: &mut [u8], at: usize, ints: &[i32]) {
        if ints.is_empty() {
            return;
        }
        let mut raw = vec![0u8; ints.len() * 4];
        for (i, v) in ints.iter().enumerate() {
            put_i32(&mut raw, 4 * i, *v);
        }
        let base = fake.alloc(raw);
        put_vector(bytes, at, base, ints.len(), 4);
    }

    /// A flat hash set of `entities` in a table of 8 slots, as the game
    /// keeps one (empty slots 0x80, the sentinel after the last).
    fn put_set(fake: &mut Fake, bytes: &mut [u8], at: usize, entities: &[(usize, i32)]) {
        if entities.is_empty() {
            return;
        }
        let capacity = 7usize;
        let mut control = vec![0x80u8; capacity + 1 + 16];
        control[capacity] = 0xff;
        let mut slots = vec![0u8; capacity * 4];
        for (slot, entity) in entities {
            control[*slot] = 0x11;
            put_i32(&mut slots, 4 * slot, *entity);
        }
        let control = fake.alloc(control);
        let slots = fake.alloc(slots);
        put_u64(bytes, at + layout::SET_CONTROL, control as u64);
        put_u64(bytes, at + layout::SET_SLOTS, slots as u64);
        put_u64(bytes, at + layout::SET_SIZE, entities.len() as u64);
        put_u64(bytes, at + layout::SET_CAPACITY, capacity as u64);
    }

    /// The crosswalk tool's click on node 58, which had a configuration: its
    /// own removed, and a copy added with crosswalks on edges 900 and 77 (in
    /// that order in the set's table), two turns and two light phases.
    fn crosswalk_click() -> (Fake, usize) {
        let mut fake = Fake::new();
        let mut entry = vec![0u8; layout::CONFIG_ENTRY_SIZE];
        let mut turns = vec![0u8; 2 * layout::LANE_CONNECTION_SIZE];
        for (i, (s0, l0, s1, l1, road, tram)) in [(77, 0, 900, 1, 1u8, 0u8), (900, 0, 77, 1, 1, 1)]
            .into_iter()
            .enumerate()
        {
            let at = i * layout::LANE_CONNECTION_SIZE;
            put_i32(&mut turns, at + layout::SEGMENT0, s0);
            put_i32(&mut turns, at + layout::LANE0, l0);
            put_i32(&mut turns, at + layout::SEGMENT1, s1);
            put_i32(&mut turns, at + layout::LANE1, l1);
            turns[at + layout::WITH_ROAD] = road;
            turns[at + layout::WITH_TRAM] = tram;
        }
        let turns_at = fake.alloc(turns);
        put_vector(
            &mut entry,
            layout::LANE_CONNECTIONS,
            turns_at,
            2,
            layout::LANE_CONNECTION_SIZE,
        );
        put_set(
            &mut fake,
            &mut entry,
            layout::CROSSWALKS,
            &[(2, 900), (5, 77)],
        );
        entry[layout::DOUBLE_SLIP] = 0;
        put_i32(&mut entry, layout::LIGHT_PREFERENCE, 2);
        put_i32(&mut entry, layout::LIGHT_TYPE, 1);
        entry[layout::USER_LIGHTS] = 1;
        let mut states = vec![0u8; 2 * layout::STATE_SIZE];
        for (i, (locked, duration, least, skip)) in
            [(vec![0, 1], 20.0f32, 5.0f32, 0u8), (vec![2], 15.5, 4.25, 1)]
                .into_iter()
                .enumerate()
        {
            let at = i * layout::STATE_SIZE;
            put_ints(
                &mut fake,
                &mut states[at..at + layout::STATE_SIZE],
                layout::LOCKED_LANES,
                &locked,
            );
            states[at + layout::DURATION..at + layout::DURATION + 4]
                .copy_from_slice(&duration.to_le_bytes());
            states[at + layout::MIN_DURATION..at + layout::MIN_DURATION + 4]
                .copy_from_slice(&least.to_le_bytes());
            states[at + layout::CAN_SKIP] = skip;
        }
        let states_at = fake.alloc(states);
        put_vector(
            &mut entry,
            layout::LIGHT_STATES,
            states_at,
            2,
            layout::STATE_SIZE,
        );
        put_i32(&mut entry, layout::CONFIG_ENTITY, 58);
        let entry_at = fake.alloc(entry);
        let mut head = vec![0u8; layout::HEAD_LEN];
        put_vector(
            &mut head,
            layout::CONFIGS_TO_ADD,
            entry_at,
            1,
            layout::CONFIG_ENTRY_SIZE,
        );
        put_ints(&mut fake, &mut head, layout::CONFIGS_TO_REMOVE, &[58]);
        let payload = fake.alloc(head);
        (fake, payload)
    }

    #[test]
    fn a_crosswalk_click_reads_as_the_configuration_the_tool_set() {
        let (fake, payload) = crosswalk_click();
        let edit = decode(&fake, payload, "crosswalk tool").unwrap();
        assert_eq!(edit.removed, [58]);
        let c = &edit.added[0];
        assert_eq!(c.node, 58);
        assert_eq!(c.crosswalks, [77, 900], "the set walked whole, sorted");
        assert_eq!(
            c.turns[1],
            Turn {
                segment0: 900,
                lane0: 0,
                segment1: 77,
                lane1: 1,
                with_road: true,
                with_tram: true
            }
        );
        assert_eq!((c.light_preference, c.light_type), (2, 1));
        assert_eq!(c.phases[1].locked, [2]);
        assert_eq!(
            (c.phases[1].duration, c.phases[1].min_duration),
            (15.5, 4.25)
        );
        assert!(c.phases[1].can_skip && c.user_lights && !c.double_slip);
        assert_eq!(
            edit.summary(),
            "crosswalk tool: removes the configuration of [58]; adds \
             +cfg58{tl=2 lc=2 cw=2 phases=2 dss=false um=false/true}"
        );
        let lua = edit.to_lua();
        assert_eq!(
            lua.get("junctions"),
            Some(&LuaValue::string("crosswalk tool"))
        );
        let street = lua.get("proposal").unwrap();
        let first = |v: &LuaValue| match v {
            LuaValue::Table(entries) => entries[0].1.clone(),
            _ => panic!(),
        };
        assert_eq!(
            first(street.get("nodeConfigsToRemove").unwrap()),
            LuaValue::Integer(58)
        );
        let added = first(street.get("nodeConfigsToAdd").unwrap());
        assert_eq!(added.get("entity"), Some(&LuaValue::Integer(58)));
        let comp = added.get("comp").unwrap();
        assert_eq!(
            comp.get("crosswalks"),
            Some(&list([int(77), int(900)])),
            "{comp:?}"
        );
        let light = comp.get("trafficLightConfig").unwrap();
        assert_eq!(
            first(
                first(light.get("states").unwrap())
                    .get("lockedLanes")
                    .unwrap()
            ),
            LuaValue::Integer(0)
        );
    }

    #[test]
    fn a_click_that_changes_more_or_does_not_read_is_a_reason_not_a_guess() {
        // An edge added besides.
        let (mut fake, payload) = crosswalk_click();
        let edge = fake.alloc(vec![0u8; layout::SEGMENT_SIZE]);
        put_vector(
            fake.bytes(payload),
            layout::ADDED_SEGMENTS,
            edge,
            1,
            layout::SEGMENT_SIZE,
        );
        assert_eq!(
            decode(&fake, payload, "crossing tool").unwrap_err(),
            "a build that changes more than junctions: 1 edge(s) added"
        );
        // A crosswalk set that claims more than its table holds.
        let (mut fake, payload) = crosswalk_click();
        let entry = u64_at(fake.bytes(payload), layout::CONFIGS_TO_ADD) as usize;
        put_u64(fake.bytes(entry), layout::CROSSWALKS + layout::SET_SIZE, 3);
        assert!(
            decode(&fake, payload, "crosswalk tool")
                .unwrap_err()
                .contains("a set of 3 that holds 2 entities")
        );
        // A flag that is no flag, and a duration that is not a number.
        let (mut fake, payload) = crosswalk_click();
        let entry = u64_at(fake.bytes(payload), layout::CONFIGS_TO_ADD) as usize;
        fake.bytes(entry)[layout::DOUBLE_SLIP] = 7;
        assert!(
            decode(&fake, payload, "crosswalk tool")
                .unwrap_err()
                .contains("a flag of 7")
        );
        // Nothing at all, and memory that does not read.
        let mut fake = Fake::new();
        let payload = fake.alloc(vec![0u8; layout::HEAD_LEN]);
        assert_eq!(
            decode(&fake, payload, "crosswalk tool").unwrap_err(),
            "a build that changes nothing"
        );
        assert!(decode(&Fake::new(), 0x1234, "crosswalk tool").is_err());
    }

    #[test]
    fn only_the_tools_own_calls_are_read_and_a_click_is_kept_for_the_gui() {
        let _serial = crate::modules::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        set_call(0, 0x1_4052_90af);
        set_call(3, 0x1_4053_9368);
        assert_eq!(tool_at(0x1_4052_90b4), Some("crosswalk tool"));
        assert_eq!(tool_at(0x1_4053_936d), Some("crossing tool"));
        assert_eq!(tool_at(0x1_4052_90af), None, "the call itself");
        assert_eq!(tool_at(0), None);
        let (fake, payload) = crosswalk_click();
        record(&fake, 501, payload, "crosswalk tool");
        record(&Fake::new(), 502, 0x1234, "crossing tool");
        assert!(matches!(
            crate::modules::take(501),
            Some(Ok(LuaValue::Table(_)))
        ));
        assert!(matches!(
            crate::modules::take(502),
            Some(Err(why)) if why.starts_with("junction tool: the crossing tool's change: ")
        ));
    }
}

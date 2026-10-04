//! The network lane's edge rows read natively (docs/HOOKS.md, "The network
//! lane read natively"): the rows `tpf3mp/lanes.lua` builds from every
//! street and track edge at a checkpoint, the same text, from the engine's
//! memory instead of a `getComponent` call and a dozen field reads an edge.
//!
//! [`ENV`] sets it: off (the default), `compare`, where the mod reads both
//! and logs whether they agree but hashes its own, or `on`, where the mod
//! hashes these rows and reads its own only when they did not read. The
//! junction rows of the lane stay the mod's.
//!
//! Where it reads (investigation/TF3_NATIVE_NETWORK_2026-10-04.md; the
//! offsets are the build's, in its native bundle):
//!
//! - the engine, `[CGameTime+8]`, from the `CGameTime` the game's own step
//!   called its speed getter on; only while that step runs, so only inside
//!   the game script's `postUpdate`, where the mod reads its lanes, and
//!   never on another thread's call;
//! - the `BaseEdge` pool, the one of the engine's pools whose vtable is
//!   `CompVec<BaseEdge>`'s, its type id its index there (and its own
//!   record of it, which must agree);
//! - every entity whose component bits hold that type id, its data index
//!   from its component list, its `BaseEdge` from the pool's dense vector
//!   or its pages.
//!
//! Fail closed: anything that does not read as the layout says (a pointer
//! out of order, a count past its bound, an entity whose bits and list
//! disagree, a number that is not finite) fails the whole read with why,
//! and the mod reads its own. Nothing is written, nothing of the game's is
//! called.

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::build_data::native::netread as layout;
use crate::modules::{Memory, i32_at, res_name, u64_at};

/// `off` (unset), `compare` or `on`.
pub const ENV: &str = "TPF3MP_HOOK_NATIVE_NETWORK";

/// Most component pools, entities, lane configs an edge and components an
/// entity read; past any of them the read fails.
pub const MAX_POOLS: usize = 4096;
pub const MAX_ENTITIES: usize = 1 << 23;
pub const MAX_LANES: usize = 256;
pub const MAX_COMPONENTS: usize = 256;
/// Type ids the component bits hold: 16 bytes an entity.
const MAX_TYPE: usize = layout::BITS_PER_ENTITY * 8;

/// What [`ENV`] asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    Compare,
    On,
}

impl Mode {
    /// From [`ENV`]'s value; anything else than `compare` or `on` is off,
    /// with why when it was set to something.
    pub fn from_env(value: Option<&str>) -> (Self, Option<String>) {
        match value.map(str::trim).filter(|v| !v.is_empty()) {
            None => (Self::Off, None),
            Some(v) if v.eq_ignore_ascii_case("compare") => (Self::Compare, None),
            Some(v) if v.eq_ignore_ascii_case("on") => (Self::On, None),
            Some(v) if v.eq_ignore_ascii_case("off") => (Self::Off, None),
            Some(v) => (
                Self::Off,
                Some(format!(
                    "{ENV}={v} is none of off, compare and on; the native network read is off"
                )),
            ),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Compare => "compare",
            Self::On => "on",
        }
    }
}

/// [`ENV`]'s mode, read once; the first call logs a value that is not one.
pub fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| {
        let value = std::env::var(ENV).ok();
        let (mode, why) = Mode::from_env(value.as_deref());
        if let Some(why) = why {
            crate::log::line(&why);
        }
        mode
    })
}

/// The running game's memory, each range checked through the per-thread
/// region cache ([`crate::image::Readable`]) before it is copied.
pub struct Process;

impl Memory for Process {
    fn read(&self, address: usize, len: usize) -> Option<Vec<u8>> {
        if address == 0 || !crate::image::readable_cached(address, len) {
            return None;
        }
        let mut bytes = vec![0u8; len];
        // SAFETY: `len` bytes at `address` are committed readable memory,
        // checked just above in this update's epoch, and `bytes` has room
        // for them.
        unsafe { std::ptr::copy_nonoverlapping(address as *const u8, bytes.as_mut_ptr(), len) };
        Some(bytes)
    }
}

/// The network lane of the world the game's step is running now, from the
/// game's memory: `Err(why)` outside the step, without the image, or when
/// the edges do not read.
pub fn read_now() -> Result<Network, String> {
    let game_time = crate::install::game_time_now();
    if game_time == 0 {
        return Err("no game step is running".into());
    }
    let image = image_base();
    if image == 0 {
        return Err("the game's image was not found".into());
    }
    let memory = Process;
    let head = read(&memory, game_time, layout::GAME_TIME_ENGINE + 8, "the CGameTime")?;
    let engine = usize::try_from(u64_at(&head, layout::GAME_TIME_ENGINE))
        .map_err(|_| "the engine's address".to_string())?;
    network(&memory, engine, image)
}

#[cfg(windows)]
fn image_base() -> usize {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    static BASE: OnceLock<usize> = OnceLock::new();
    // SAFETY: a null name asks for the process's own executable, which
    // stays loaded for the life of the process.
    *BASE.get_or_init(|| unsafe { GetModuleHandleW(std::ptr::null()) } as usize)
}

#[cfg(not(windows))]
fn image_base() -> usize {
    0
}

/// `len` bytes at `address`, or why not; nothing to read for none.
fn read(memory: &dyn Memory, address: usize, len: usize, what: &str) -> Result<Vec<u8>, String> {
    if len == 0 {
        return Ok(Vec::new());
    }
    crate::modules::read(memory, address, len, what)
}

/// A `std::vector`'s `{begin, end}` at `offset` of `head`: its begin and
/// its count of `stride`-byte elements, at most `max`.
fn vector(head: &[u8], offset: usize, stride: usize, max: usize, what: &str) -> Result<(usize, usize), String> {
    let begin = u64_at(head, offset);
    let end = u64_at(head, offset + 8);
    if end < begin {
        return Err(format!("{what}: its end is before its begin"));
    }
    if begin == 0 && end != 0 {
        return Err(format!("{what}: no begin but an end"));
    }
    let bytes = usize::try_from(end - begin).map_err(|_| format!("{what}: its size"))?;
    if bytes % stride != 0 {
        return Err(format!("{what}: a size that is not a whole number of elements"));
    }
    let count = bytes / stride;
    if count > max {
        return Err(format!("{what}: {count} elements, more than {max}"));
    }
    let begin = usize::try_from(begin).map_err(|_| format!("{what}: its begin"))?;
    Ok((begin, count))
}

fn f32_at(bytes: &[u8], offset: usize) -> f32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[offset..offset + 4]);
    f32::from_le_bytes(word)
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[offset..offset + 4]);
    u32::from_le_bytes(word)
}

/// A component pool: where its elements of `size` bytes are.
struct Pool {
    dense: (usize, usize),
    pages: (usize, usize),
    paged_slots: usize,
    size: usize,
}

impl Pool {
    fn read(memory: &dyn Memory, pool: usize, size: usize, what: &str) -> Result<Self, String> {
        let head = read(memory, pool, layout::POOL_HEAD, what)?;
        let dense = vector(&head, layout::POOL_DENSE, size, MAX_ENTITIES, what)?;
        let pages = vector(&head, layout::POOL_PAGES, layout::PAGE_ENTRY, MAX_ENTITIES, what)?;
        let paged_slots = usize::try_from(u64_at(&head, layout::POOL_PAGED_SLOTS))
            .ok()
            .filter(|n| *n <= MAX_ENTITIES)
            .ok_or_else(|| format!("{what}: its paged slots"))?;
        Ok(Self {
            dense,
            pages,
            paged_slots,
            size,
        })
    }

    /// The address of the element at data index `index`.
    fn element(&self, memory: &dyn Memory, index: i32) -> Result<usize, String> {
        let index = u32::try_from(index).map_err(|_| format!("a negative data index {index}"))?;
        if index < layout::PAGED_FROM {
            let i = index as usize;
            if i >= self.dense.1 {
                return Err(format!("data index {i} past the pool's {} elements", self.dense.1));
            }
            return Ok(self.dense.0 + i * self.size);
        }
        let i = (index - layout::PAGED_FROM) as usize;
        let page = i / layout::PAGE_SLOTS;
        if i >= self.paged_slots || page >= self.pages.1 {
            return Err(format!("paged slot {i} past the pool's pages"));
        }
        let entry = read(memory, self.pages.0 + page * layout::PAGE_ENTRY, 8, "a pool page")?;
        let data = usize::try_from(u64_at(&entry, 0)).map_err(|_| "a pool page's address".to_string())?;
        if data == 0 {
            return Err(format!("paged slot {i} on a page that is not there"));
        }
        Ok(data + (i % layout::PAGE_SLOTS) * self.size)
    }
}

//// The type id of the pool whose vtable is at `vtable`: its index among
/// the engine's pools (`sub_94770` appends a type's pool as it registers
/// the type with the id `pools.size() + 1`, stored less one). Every entity
/// read through it must list that id ([`Store::index`]).
fn type_id(memory: &dyn Memory, engine: usize, vtable: usize, what: &str) -> Result<(usize, usize), String> {
    let head = read(memory, engine + layout::POOLS, 16, "the engine's pools")?;
    let (begin, count) = vector(&head, 0, 8, MAX_POOLS, "the engine's pools")?;
    let pools = read(memory, begin, count * 8, "the engine's pools")?;
    let mut found = None;
    for i in 0..count {
        let pool = usize::try_from(u64_at(&pools, i * 8)).unwrap_or(0);
        if pool == 0 {
            continue;
        }
        let Some(head) = memory.read(pool, 16) else {
            continue;
        };
        if usize::try_from(u64_at(&head, 0)).ok() == Some(vtable) {
            if found.is_some() {
                return Err(format!("two pools of {what}"));
            }
            found = Some((i, pool));
        }
    }
    let (id, pool) = found.ok_or_else(|| format!("no pool of {what}"))?;
    if id >= MAX_TYPE {
        return Err(format!("{what}'s type id {id} is past the component bits"));
    }
    Ok((id, pool))
}

/// Entities a scan reads the component bits of at once.
const CHUNK: usize = 1 << 16;

/// The engine's entities: their table and their component bits.
struct Store<'m> {
    memory: &'m dyn Memory,
    engine: usize,
    image: usize,
    records: usize,
    entities: usize,
    bits: usize,
}

/// One component type: its id and its pool.
struct Kind {
    id: usize,
    pool: Pool,
    name: &'static str,
}

impl<'m> Store<'m> {
    fn new(memory: &'m dyn Memory, engine: usize, image: usize) -> Result<Self, String> {
        let head = read(memory, engine, layout::BITS + 8, "the engine")?;
        let (records, entities) = vector(
            &head,
            layout::ENTITIES,
            layout::ENTITY_RECORD,
            MAX_ENTITIES,
            "the entity table",
        )?;
        let bits = usize::try_from(u64_at(&head, layout::BITS)).map_err(|_| "the component bits".to_string())?;
        if entities > 0 && bits == 0 {
            return Err("entities without component bits".into());
        }
        Ok(Self {
            memory,
            engine,
            image,
            records,
            entities,
            bits,
        })
    }

    /// The component type whose pool's vtable is at `vtable` (an RVA), its
    /// elements `size` bytes.
    fn kind(&self, vtable: usize, size: usize, name: &'static str) -> Result<Kind, String> {
        let (id, pool) = type_id(self.memory, self.engine, self.image + vtable, name)?;
        let pool = Pool::read(self.memory, pool, size, name)?;
        Ok(Kind { id, pool, name })
    }

    /// The data index of `kind` in `entity`'s component list: `None` for a
    /// removed entity, `Err` when the list lacks it or lists it twice.
    fn index(&self, entity: usize, kind: &Kind) -> Result<Option<i32>, String> {
        let record = read(
            self.memory,
            self.records + entity * layout::ENTITY_RECORD,
            layout::ENTITY_RECORD,
            "an entity's record",
        )?;
        let (pairs, count) = vector(
            &record,
            0,
            layout::COMPONENT_PAIR,
            MAX_COMPONENTS,
            "an entity's components",
        )?;
        let list = read(self.memory, pairs, count * layout::COMPONENT_PAIR, "an entity's components")?;
        // A removed entity keeps one pair {-1, -1} (`sub_4f7db0`).
        if count == 1 && i32_at(&list, 0) < 0 {
            return Ok(None);
        }
        let mut index = None;
        for p in 0..count {
            let id = i32_at(&list, p * layout::COMPONENT_PAIR);
            if usize::try_from(id).ok() == Some(kind.id) {
                if index.is_some() {
                    return Err(format!("entity {entity} lists {} twice", kind.name));
                }
                index = Some(i32_at(&list, p * layout::COMPONENT_PAIR + 4));
            }
        }
        index
            .map(Some)
            .ok_or_else(|| format!("entity {entity} has the {0} bit but no {0}", kind.name))
    }

    /// Whether `entity`'s component bits hold `kind`.
    fn has(&self, entity: usize, kind: &Kind) -> Result<bool, String> {
        if entity >= self.entities {
            return Ok(false);
        }
        let word = read(
            self.memory,
            self.bits + entity * layout::BITS_PER_ENTITY + (kind.id / 64) * 8,
            8,
            "the component bits",
        )?;
        Ok(u64_at(&word, 0) >> (kind.id % 64) & 1 == 1)
    }

    /// `entity`'s `kind`, its bytes; `None` when it has none or is removed.
    fn component(&self, entity: usize, kind: &Kind) -> Result<Option<Vec<u8>>, String> {
        if !self.has(entity, kind)? {
            return Ok(None);
        }
        let Some(index) = self.index(entity, kind)? else {
            return Ok(None);
        };
        let at = kind.pool.element(self.memory, index)?;
        Ok(Some(read(self.memory, at, kind.pool.size, kind.name)?))
    }

    /// Every entity whose component bits hold `kind`, in id order.
    fn with(&self, kind: &Kind) -> Result<Vec<usize>, String> {
        let word = (kind.id / 64) * 8;
        let bit = kind.id % 64;
        let mut out = Vec::new();
        let mut first = 0;
        while first < self.entities {
            let n = CHUNK.min(self.entities - first);
            let bits = read(
                self.memory,
                self.bits + first * layout::BITS_PER_ENTITY,
                n * layout::BITS_PER_ENTITY,
                "the component bits",
            )?;
            for i in 0..n {
                if u64_at(&bits, i * layout::BITS_PER_ENTITY + word) >> bit & 1 == 1 {
                    out.push(first + i);
                }
            }
            first += n;
        }
        Ok(out)
    }
}

/// What a junction row needs of an edge: its nodes and its network.
#[derive(Debug, Clone, Copy)]
struct EdgeEnds {
    node0: usize,
    node1: usize,
    street: bool,
}

/// The network lane's rows, read natively: the edges' rows, and apart, the
/// junctions' (which can fail on their own).
pub struct Network {
    pub edges: Vec<String>,
    pub junctions: Result<Vec<Junction>, String>,
}

/// A junction's row but for the two names only the game's Lua gives: its
/// traffic light preference (an enum value) and its light's resource
/// (`-1` for the default). The row is `head|preference|light|tail`
/// (tpf3mp/junctions.lua, `rows`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Junction {
    pub head: String,
    pub preference: i32,
    pub light: i32,
    pub tail: String,
}

/// The edge rows of the world in `engine`, as lanes.lua makes them
/// (`lanes.edgeRow`), unsorted. `image` is where the game's executable is
/// loaded.
pub fn edge_rows(memory: &dyn Memory, engine: usize, image: usize) -> Result<Vec<String>, String> {
    let store = Store::new(memory, engine, image)?;
    let edges = store.kind(layout::BASE_EDGE_POOL_VTABLE, layout::BASE_EDGE_SIZE, "BaseEdge")?;
    Ok(read_edges(&store, &edges)?.0)
}

type Ends = HashMap<usize, EdgeEnds>;

/// The edges' rows and their ends, by entity.
fn read_edges(store: &Store, kind: &Kind) -> Result<(Vec<String>, Ends), String> {
    let mut rows = Vec::new();
    let mut ends = HashMap::new();
    for entity in store.with(kind)? {
        let Some(index) = store.index(entity, kind)? else {
            continue;
        };
        let at = kind.pool.element(store.memory, index)?;
        let edge = read(store.memory, at, layout::BASE_EDGE_SIZE, "a BaseEdge")?;
        rows.push(edge_row(store.memory, &edge, at).map_err(|why| format!("entity {entity}: {why}"))?);
        let node = |offset: usize| {
            usize::try_from(i32_at(&edge, offset)).map_err(|_| format!("entity {entity}: a negative node"))
        };
        let street = match i32_at(&edge, layout::EDGE_ROAD_TYPE) {
            layout::ROAD_TYPE_STREET => true,
            layout::ROAD_TYPE_TRACK => false,
            other => return Err(format!("entity {entity}: road type {other}")),
        };
        ends.insert(
            entity,
            EdgeEnds {
                node0: node(layout::EDGE_NODE0)?,
                node1: node(layout::EDGE_NODE1)?,
                street,
            },
        );
    }
    Ok((rows, ends))
}

/// The network lane of the world in `engine`, read natively: `Err` when
/// the edges did not read, the junctions' own `Err` when only they did not.
pub fn network(memory: &dyn Memory, engine: usize, image: usize) -> Result<Network, String> {
    let store = Store::new(memory, engine, image)?;
    let edges = store.kind(layout::BASE_EDGE_POOL_VTABLE, layout::BASE_EDGE_SIZE, "BaseEdge")?;
    let (rows, ends) = read_edges(&store, &edges)?;
    let junctions = read_junctions(&store, &ends);
    Ok(Network {
        edges: rows,
        junctions,
    })
}

/// C's `%.0f`, as `string.format` makes it. Only finite numbers.
pub fn fixed0(v: f64) -> Result<String, String> {
    if !v.is_finite() {
        return Err(format!("a number that is not finite ({v})"));
    }
    Ok(format!("{v:.0}"))
}

/// junctions.lua's `pointKey`: a position in millimetres.
fn point_key(p: [f32; 3]) -> Result<String, String> {
    Ok(format!(
        "{},{},{}",
        fixed0(f64::from(p[0]) * 1000.0)?,
        fixed0(f64::from(p[1]) * 1000.0)?,
        fixed0(f64::from(p[2]) * 1000.0)?
    ))
}

fn flag(raw: &[u8], at: usize, what: &str) -> Result<bool, String> {
    match raw[at] {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(format!("{what} reads {other}")),
    }
}

/// What makes the junctions' rows: the nodes' positions and the edges'
/// keys, each read and made once.
struct Junctions<'s, 'm> {
    store: &'s Store<'m>,
    ends: &'s Ends,
    nodes: Kind,
    positions: HashMap<usize, [f32; 3]>,
    keys: HashMap<usize, String>,
}

impl Junctions<'_, '_> {
    fn position(&mut self, node: usize) -> Result<[f32; 3], String> {
        if let Some(p) = self.positions.get(&node) {
            return Ok(*p);
        }
        let raw = self
            .store
            .component(node, &self.nodes)?
            .ok_or_else(|| format!("node {node} has no position"))?;
        let p = [
            f32_at(&raw, layout::NODE_POSITION),
            f32_at(&raw, layout::NODE_POSITION + 4),
            f32_at(&raw, layout::NODE_POSITION + 8),
        ];
        if p.iter().any(|v| !v.is_finite()) {
            return Err(format!("node {node}: an invalid junction position"));
        }
        self.positions.insert(node, p);
        Ok(p)
    }

    /// junctions.lua's `edgeKey`: the edge's network and its nodes' places.
    fn edge_key(&mut self, edge: i32) -> Result<String, String> {
        let edge = usize::try_from(edge).map_err(|_| format!("a junction names edge {edge}"))?;
        if let Some(k) = self.keys.get(&edge) {
            return Ok(k.clone());
        }
        let e = *self
            .ends
            .get(&edge)
            .ok_or_else(|| format!("a junction edge {edge} no longer exists"))?;
        let mut a = point_key(self.position(e.node0)?)?;
        let mut b = point_key(self.position(e.node1)?)?;
        if a.as_bytes() > b.as_bytes() {
            std::mem::swap(&mut a, &mut b);
        }
        let k = format!("{}:{a}>{b}", if e.street { "Street" } else { "Track" });
        self.keys.insert(edge, k.clone());
        Ok(k)
    }
}

/// Every junction's row parts, as junctions.lua's `rows` makes them: the
/// nodes of the edges in `ends` that have a `BaseNodeConfig`, in id order.
fn read_junctions(store: &Store, ends: &Ends) -> Result<Vec<Junction>, String> {
    let nodes = store.kind(layout::BASE_NODE_POOL_VTABLE, layout::BASE_NODE_SIZE, "BaseNode")?;
    let configs = store.kind(
        layout::BASE_NODE_CONFIG_POOL_VTABLE,
        layout::BASE_NODE_CONFIG_SIZE,
        "BaseNodeConfig",
    )?;
    // Each node's network: a street edge at it makes it a street node, as
    // getNodeStreetSegments being first does in junctions.lua.
    let mut street_node = HashMap::<usize, bool>::new();
    for e in ends.values() {
        for n in [e.node0, e.node1] {
            *street_node.entry(n).or_insert(false) |= e.street;
        }
    }
    let mut j = Junctions {
        store,
        ends,
        nodes,
        positions: HashMap::new(),
        keys: HashMap::new(),
    };
    let mut out = Vec::new();
    for node in store.with(&configs)? {
        let Some(&street) = street_node.get(&node) else {
            // In neither network's node map: junctions.lua never reads it.
            continue;
        };
        let Some(raw) = store.component(node, &configs)? else {
            continue;
        };
        let mut lanes = Vec::new();
        let (turns_at, turns) = vector(
            &raw,
            layout::CONFIG_TURNS,
            layout::TURN_SIZE,
            MAX_LANES,
            "a junction's turns",
        )?;
        let turn_bytes = read(store.memory, turns_at, turns * layout::TURN_SIZE, "a junction's turns")?;
        for t in 0..turns {
            let turn = &turn_bytes[t * layout::TURN_SIZE..(t + 1) * layout::TURN_SIZE];
            let incoming = j.edge_key(i32_at(turn, layout::TURN_SEGMENT0))?;
            let outgoing = j.edge_key(i32_at(turn, layout::TURN_SEGMENT1))?;
            lanes.push(format!(
                "{incoming}:{}>{outgoing}:{}:{}:{}",
                i32_at(turn, layout::TURN_LANE0),
                i32_at(turn, layout::TURN_LANE1),
                flag(turn, layout::TURN_ROAD, "a turn's road flag")?,
                flag(turn, layout::TURN_TRAM, "a turn's tram flag")?,
            ));
        }
        for e in crate::junctions::crosswalk_ids(store.memory, &raw)? {
            lanes.push(format!("walk:{}", j.edge_key(e)?));
        }
        let mut sorted = lanes.clone();
        sorted.sort_unstable();
        let (phases_at, phases) = vector(
            &raw,
            layout::CONFIG_PHASES,
            layout::PHASE_SIZE,
            MAX_LANES,
            "a junction's phases",
        )?;
        let phase_bytes = read(store.memory, phases_at, phases * layout::PHASE_SIZE, "a junction's phases")?;
        let mut phase_rows = Vec::with_capacity(phases);
        for i in 0..phases {
            let phase = &phase_bytes[i * layout::PHASE_SIZE..(i + 1) * layout::PHASE_SIZE];
            let (locked_at, count) = vector(phase, layout::PHASE_LOCKED, 4, MAX_LANES, "a phase's locked lanes")?;
            let locked_bytes = read(store.memory, locked_at, count * 4, "a phase's locked lanes")?;
            let mut locked = Vec::with_capacity(count);
            for k in 0..count {
                let lane = usize::try_from(i32_at(&locked_bytes, k * 4))
                    .ok()
                    .and_then(|l| lanes.get(l))
                    .ok_or("traffic phase references no lane")?;
                locked.push(lane.as_str());
            }
            locked.sort_unstable();
            phase_rows.push(format!(
                "{}/{}/{}:{}",
                fixed3(f64::from(f32_at(phase, layout::PHASE_DURATION)))?,
                fixed3(f64::from(f32_at(phase, layout::PHASE_MINIMUM)))?,
                flag(phase, layout::PHASE_SKIP, "a phase's skip flag")?,
                locked.join(","),
            ));
        }
        let head = format!(
            "{}:{}|{}",
            if street { "Street" } else { "Track" },
            point_key(j.position(node)?)?,
            sorted.join(";")
        );
        let tail = format!(
            "{}|{}|{}",
            flag(&raw, layout::CONFIG_DOUBLE_SLIP, "a junction's double slip flag")?,
            flag(&raw, layout::CONFIG_CUSTOM_PHASES, "a junction's custom phases flag")?,
            phase_rows.join(";")
        );
        out.push(Junction {
            head,
            preference: i32_at(&raw, layout::CONFIG_PREFERENCE),
            light: i32_at(&raw, layout::CONFIG_LIGHT_TYPE),
            tail,
        });
    }
    Ok(out)
}

/// One edge's row from its `BaseEdge` (`edge`, read at `at`).
fn edge_row(memory: &dyn Memory, edge: &[u8], at: usize) -> Result<String, String> {
    let point = |offset: usize| -> Result<String, String> {
        let mut parts = Vec::with_capacity(3);
        for k in 0..3 {
            parts.push(lua_number(q01(f32_at(edge, offset + 4 * k)))?);
        }
        Ok(parts.join(","))
    };
    let (mut a, mut b) = (point(layout::EDGE_POSITION0)?, point(layout::EDGE_POSITION1)?);
    let reversed = a.as_bytes() > b.as_bytes();
    if reversed {
        std::mem::swap(&mut a, &mut b);
    }
    let template = res_name(memory, at + layout::EDGE_ROAD_TEMPLATE, "the edge's road template")?;
    let (lanes_at, lanes) = vector(
        edge,
        layout::EDGE_LANE_CONFIGS,
        layout::LANE_CONFIG_SIZE,
        MAX_LANES,
        "the edge's lane configs",
    )?;
    let configs = read(memory, lanes_at, lanes * layout::LANE_CONFIG_SIZE, "the edge's lane configs")?;
    let mut lane_rows = Vec::with_capacity(lanes);
    for i in 0..lanes {
        let c = &configs[i * layout::LANE_CONFIG_SIZE..(i + 1) * layout::LANE_CONFIG_SIZE];
        let sign = if reversed { -1.0 } else { 1.0 };
        let forward = match c[layout::LANE_FORWARD] {
            0 => false,
            1 => true,
            other => return Err(format!("a lane's forward flag reads {other}")),
        };
        let modes = u32_at(c, layout::LANE_MODES);
        let modes: String = (0..16).map(|m| if modes >> m & 1 == 1 { '1' } else { '0' }).collect();
        lane_rows.push(format!(
            "{}/{}/{}/{}/{}/{modes}",
            fixed3(f64::from(f32_at(c, layout::LANE_SPEED)))?,
            fixed3(f64::from(f32_at(c, layout::LANE_WIDTH)))?,
            fixed3(f64::from(f32_at(c, layout::LANE_HEIGHT)))?,
            fixed3(f64::from(f32_at(c, layout::LANE_OFFSET)) * sign)?,
            forward != reversed,
        ));
    }
    lane_rows.sort_unstable();
    Ok(format!("{a}>{b}:{template}|lanes:{}", lane_rows.join(";")))
}

/// lanes.lua's `q01`: `math.floor(v * 10 + 0.5) / 10`, in doubles.
fn q01(v: f32) -> f64 {
    (f64::from(v) * 10.0 + 0.5).floor() / 10.0
}

/// A number as the game's Lua (5.2) makes it text, `tostring` and `%s`:
/// C's `%.14g`. Only finite numbers.
pub fn lua_number(v: f64) -> Result<String, String> {
    if !v.is_finite() {
        return Err(format!("a number that is not finite ({v})"));
    }
    const P: i32 = 14;
    if v == 0.0 {
        return Ok(if v.is_sign_negative() { "-0" } else { "0" }.into());
    }
    // The exponent of the value rounded to P significant digits, as
    // `%.13e` gives it.
    let e = format!("{:.*e}", (P - 1) as usize, v);
    let (mantissa, exponent) = e.split_once('e').ok_or("a number's exponent")?;
    let x: i32 = exponent.parse().map_err(|_| "a number's exponent")?;
    let text = if (-4..P).contains(&x) {
        let precision = usize::try_from(P - 1 - x).unwrap_or(0);
        trim_zeros(format!("{v:.precision$}"))
    } else {
        format!("{}e{}{:02}", trim_zeros(mantissa.to_string()), if x < 0 { '-' } else { '+' }, x.abs())
    };
    Ok(text)
}

fn trim_zeros(mut text: String) -> String {
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    text
}

/// C's `%.3f`, as `string.format` makes it. Only finite numbers.
pub fn fixed3(v: f64) -> Result<String, String> {
    if !v.is_finite() {
        return Err(format!("a number that is not finite ({v})"));
    }
    Ok(format!("{v:.3}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Memory made of blocks at addresses.
    #[derive(Default)]
    struct Fake {
        blocks: BTreeMap<usize, Vec<u8>>,
        next: usize,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                blocks: BTreeMap::new(),
                next: 0x10_0000,
            }
        }
        fn alloc(&mut self, bytes: Vec<u8>) -> usize {
            let at = self.next;
            self.next += (bytes.len().max(1) + 0xfff) & !0xfff;
            self.blocks.insert(at, bytes);
            at
        }
        fn put(&mut self, at: usize, bytes: Vec<u8>) {
            self.blocks.insert(at, bytes);
        }
    }

    impl Memory for Fake {
        fn read(&self, address: usize, len: usize) -> Option<Vec<u8>> {
            let (base, block) = self.blocks.range(..=address).next_back()?;
            let start = address - base;
            block.get(start..start + len).map(<[u8]>::to_vec)
        }
    }

    fn put_u64(bytes: &mut [u8], at: usize, v: u64) {
        bytes[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    fn put_f32(bytes: &mut [u8], at: usize, v: f32) {
        bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// An MSVC `std::string` holding `text`, inline or on the heap.
    fn string(fake: &mut Fake, text: &str) -> Vec<u8> {
        let mut s = vec![0u8; 0x20];
        if text.len() < 16 {
            s[..text.len()].copy_from_slice(text.as_bytes());
            put_u64(&mut s, 0x18, 15);
        } else {
            let heap = fake.alloc(text.as_bytes().to_vec());
            put_u64(&mut s, 0, heap as u64);
            put_u64(&mut s, 0x18, text.len() as u64);
        }
        put_u64(&mut s, 0x10, text.len() as u64);
        s
    }

    struct Lane {
        speed: f32,
        width: f32,
        height: f32,
        forward: bool,
        modes: u32,
        offset: f32,
    }

    struct Edge {
        p0: [f32; 3],
        p1: [f32; 3],
        template: (&'static str, &'static str),
        lanes: Vec<Lane>,
    }

    const IMAGE: usize = 0x1_4000_0000;

    /// An engine with `edges` as entities 1, 3, 4, ... (entity 0 a person,
    /// entity 2 a removed edge), the BaseEdge pool at type id `edge_type`,
    /// the edges from `paged` on in pages.
    fn engine(fake: &mut Fake, edges: &[Edge], edge_type: usize, paged: usize) -> usize {
        let mut data = Vec::new();
        let mut page = vec![0u8; layout::PAGE_SLOTS * layout::BASE_EDGE_SIZE];
        let mut indices = Vec::new();
        for (i, e) in edges.iter().enumerate() {
            let mut b = vec![0u8; layout::BASE_EDGE_SIZE];
            for k in 0..3 {
                put_f32(&mut b, layout::EDGE_POSITION0 + 4 * k, e.p0[k]);
                put_f32(&mut b, layout::EDGE_POSITION1 + 4 * k, e.p1[k]);
            }
            let mut configs = vec![0u8; e.lanes.len() * layout::LANE_CONFIG_SIZE];
            for (j, l) in e.lanes.iter().enumerate() {
                let c = &mut configs[j * layout::LANE_CONFIG_SIZE..];
                put_f32(c, layout::LANE_SPEED, l.speed);
                put_f32(c, layout::LANE_WIDTH, l.width);
                put_f32(c, layout::LANE_HEIGHT, l.height);
                c[layout::LANE_FORWARD] = u8::from(l.forward);
                c[layout::LANE_MODES..layout::LANE_MODES + 4].copy_from_slice(&l.modes.to_le_bytes());
                put_f32(c, layout::LANE_OFFSET, l.offset);
            }
            let len = configs.len();
            let at = if len == 0 { 0 } else { fake.alloc(configs) };
            put_u64(&mut b, layout::EDGE_LANE_CONFIGS, at as u64);
            put_u64(&mut b, layout::EDGE_LANE_CONFIGS + 8, (at + len) as u64);
            put_u64(&mut b, layout::EDGE_LANE_CONFIGS + 16, (at + len) as u64);
            let first = string(fake, e.template.0);
            let second = string(fake, e.template.1);
            b[layout::EDGE_ROAD_TEMPLATE..layout::EDGE_ROAD_TEMPLATE + 0x20].copy_from_slice(&first);
            b[layout::EDGE_ROAD_TEMPLATE + 0x20..layout::EDGE_ROAD_TEMPLATE + 0x40].copy_from_slice(&second);
            if i < paged {
                indices.push((data.len() / layout::BASE_EDGE_SIZE) as u32);
                data.extend_from_slice(&b);
            } else {
                let slot = i - paged;
                page[slot * layout::BASE_EDGE_SIZE..(slot + 1) * layout::BASE_EDGE_SIZE].copy_from_slice(&b);
                indices.push(layout::PAGED_FROM + slot as u32);
            }
        }
        let dense_len = data.len();
        let dense = fake.alloc(data);
        let page_at = fake.alloc(page);
        let mut page_table = vec![0u8; layout::PAGE_ENTRY];
        put_u64(&mut page_table, 0, page_at as u64);
        let page_table_at = fake.alloc(page_table);
        let mut pool = vec![0u8; layout::POOL_HEAD];
        put_u64(&mut pool, 0, (IMAGE + layout::BASE_EDGE_POOL_VTABLE) as u64);
        put_u64(&mut pool, layout::POOL_DENSE, dense as u64);
        put_u64(&mut pool, layout::POOL_DENSE + 8, (dense + dense_len) as u64);
        put_u64(&mut pool, layout::POOL_PAGES, page_table_at as u64);
        put_u64(&mut pool, layout::POOL_PAGES + 8, (page_table_at + layout::PAGE_ENTRY) as u64);
        put_u64(&mut pool, layout::POOL_PAGED_SLOTS, (edges.len() - paged) as u64);
        let pool_at = fake.alloc(pool);
        // Another pool, of something else, before it.
        let mut other = vec![0u8; layout::POOL_HEAD];
        put_u64(&mut other, 0, (IMAGE + 0x100) as u64);
        let other_at = fake.alloc(other);
        let mut pools = vec![0u8; (edge_type + 1) * 8];
        put_u64(&mut pools, 0, other_at as u64);
        put_u64(&mut pools, edge_type * 8, pool_at as u64);
        let pools_len = pools.len();
        let pools_at = fake.alloc(pools);
        // Entities: 0 a person, then the edges, with a removed edge at 2.
        let mut records: Vec<Vec<(i32, i32)>> = vec![vec![(0, 7)]];
        let mut bits: Vec<u128> = vec![1];
        for (i, index) in indices.iter().enumerate() {
            if i == 1 {
                // Removed, its bits left as they were.
                records.push(vec![(-1, -1)]);
                bits.push(1 << edge_type);
            }
            records.push(vec![(0, 3), (edge_type as i32, *index as i32)]);
            bits.push(1 | 1u128 << edge_type);
        }
        let mut table = vec![0u8; records.len() * layout::ENTITY_RECORD];
        for (e, pairs) in records.iter().enumerate() {
            let mut list = Vec::new();
            for (t, d) in pairs {
                list.extend_from_slice(&t.to_le_bytes());
                list.extend_from_slice(&d.to_le_bytes());
            }
            let len = list.len();
            let at = fake.alloc(list);
            put_u64(&mut table, e * 24, at as u64);
            put_u64(&mut table, e * 24 + 8, (at + len) as u64);
            put_u64(&mut table, e * 24 + 16, (at + len) as u64);
        }
        let table_len = table.len();
        let table_at = fake.alloc(table);
        let bits_bytes: Vec<u8> = bits.iter().flat_map(|b| b.to_le_bytes()).collect();
        let bits_at = fake.alloc(bits_bytes);
        let mut head = vec![0u8; 0x100];
        put_u64(&mut head, layout::POOLS, pools_at as u64);
        put_u64(&mut head, layout::POOLS + 8, (pools_at + pools_len) as u64);
        put_u64(&mut head, layout::ENTITIES, table_at as u64);
        put_u64(&mut head, layout::ENTITIES + 8, (table_at + table_len) as u64);
        put_u64(&mut head, layout::BITS, bits_at as u64);
        fake.alloc(head)
    }

    fn lane(speed: f32, offset: f32, forward: bool, modes: u32) -> Lane {
        Lane {
            speed,
            width: 3.5,
            height: 0.0,
            forward,
            modes,
            offset,
        }
    }

    fn sample() -> Vec<Edge> {
        vec![
            Edge {
                p0: [10.04, -3.06, 0.0],
                p1: [100.0, 2.25, 1.0],
                template: ("", "street/town_medium_new.lua"),
                lanes: vec![lane(13.888_889, -2.0, false, 0b1), lane(13.888_889, 2.0, true, 0b11)],
            },
            Edge {
                p0: [500.15, 200.0, 12.3456],
                p1: [400.0, 199.95, 12.0],
                template: ("mymod_1", "street/x.lua"),
                lanes: vec![lane(0.0625, 0.0, true, 0x8000), lane(22.2, 1.0625, false, 0)],
            },
            Edge {
                p0: [-0.04, 0.05, -0.05],
                p1: [0.0, 0.0, 0.0],
                template: ("", ""),
                lanes: vec![],
            },
            Edge {
                p0: [123_456.78, -98_765.43, 1e-5],
                p1: [123_456.7, -98_765.4, 2.5],
                template: ("", "track/standard.lua"),
                lanes: vec![lane(83.333_33, -0.7175, true, 1 << 11)],
            },
        ]
    }

    /// lanes.lua's own row, run in Lua, for each of `edges`.
    fn lua_rows(edges: &[Edge]) -> Vec<String> {
        let lua = mlua::Lua::new();
        let scripts = concat!(env!("CARGO_MANIFEST_DIR"), "/../../mod/tpf3mp_1/content/scripts/");
        lua.load(format!("package.path = {:?} .. '?.lua;' .. package.path", scripts))
            .exec()
            .unwrap();
        let lanes: mlua::Table = lua.load("return require('tpf3mp.lanes')").eval().unwrap();
        let row: mlua::Function = lanes.get("edgeRow").unwrap();
        let mut out = Vec::new();
        for e in edges {
            let edge = lua.create_table().unwrap();
            let vec = |p: [f32; 3]| {
                let t = lua.create_table().unwrap();
                t.set("x", f64::from(p[0])).unwrap();
                t.set("y", f64::from(p[1])).unwrap();
                t.set("z", f64::from(p[2])).unwrap();
                t
            };
            edge.set("position0", vec(e.p0)).unwrap();
            edge.set("position1", vec(e.p1)).unwrap();
            let template = if e.template.1.is_empty() {
                String::new()
            } else {
                format!("{}::/{}", e.template.0, e.template.1)
            };
            edge.set("roadTemplate", template).unwrap();
            let configs = lua.create_table().unwrap();
            for (i, l) in e.lanes.iter().enumerate() {
                let c = lua.create_table().unwrap();
                c.set("speed", f64::from(l.speed)).unwrap();
                c.set("width", f64::from(l.width)).unwrap();
                c.set("height", f64::from(l.height)).unwrap();
                c.set("offset", f64::from(l.offset)).unwrap();
                c.set("forward", l.forward).unwrap();
                let modes = lua.create_table().unwrap();
                for m in 0..16 {
                    modes.set(m, l.modes >> m & 1 == 1).unwrap();
                }
                c.set("transportModes", modes).unwrap();
                configs.set(i + 1, c).unwrap();
            }
            edge.set("laneConfigs", configs).unwrap();
            out.push(row.call::<String>(edge).unwrap());
        }
        out
    }

    #[test]
    fn rows_read_as_the_mods_lua_makes_them() {
        let edges = sample();
        for (edge_type, paged) in [(5, edges.len()), (64, 1), (127, 0)] {
            let mut fake = Fake::new();
            let engine = engine(&mut fake, &edges, edge_type, paged);
            let native = edge_rows(&fake, engine, IMAGE).unwrap();
            assert_eq!(native, lua_rows(&edges), "type {edge_type}, paged from {paged}");
        }
    }

    #[test]
    fn numbers_print_as_lua_prints_them() {
        let lua = mlua::Lua::new();
        let g: mlua::Function = lua.load("return function(v) return tostring(v) end").eval().unwrap();
        let f: mlua::Function = lua
            .load("return function(v) return string.format('%.3f', v) end")
            .eval()
            .unwrap();
        let mut samples = vec![
            0.0, -0.0, 0.1, -0.1, 1.0, 12.5, -3.1, 1e-5, 1.25e-5, 123_456.7, -98_765.4, 1e14, 1e15,
            123_456_789_012_345.0, 0.000_1, 0.000_099_99, 0.0625, 1.0625, 2.5, 0.0005, 0.0015, 0.0025,
            -0.0005, 13.888_889_312_744_14, 83.333_33, 1e100, -1e-100, 5e-324,
        ];
        // Every float32 lanes.lua reads at 0.1 m, and many others.
        let mut x: u32 = 12345;
        for _ in 0..20_000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let v = f32::from_bits(x);
            if v.is_finite() {
                samples.push(f64::from(v));
                samples.push(q01(v));
            }
            samples.push(f64::from(x % 100_000) / 16.0 - 3000.0);
        }
        let z: mlua::Function = lua
            .load("return function(v) return string.format('%.0f', v) end")
            .eval()
            .unwrap();
        for v in samples {
            let lua_z: String = z.call(v * 1000.0).unwrap();
            assert_eq!(fixed0(v * 1000.0).unwrap(), lua_z, "%.0f of {:e}", v * 1000.0);
            let lua_g: String = g.call(v).unwrap();
            assert_eq!(lua_number(v).unwrap(), lua_g, "tostring({v:e})");
            let lua_f: String = f.call(v).unwrap();
            assert_eq!(fixed3(v).unwrap(), lua_f, "%.3f of {v:e}");
        }
    }

    #[test]
    fn a_layout_that_does_not_read_fails_the_read() {
        let edges = sample();
        // No pool of BaseEdge: another image.
        let mut fake = Fake::new();
        let at = engine(&mut fake, &edges, 5, 4);
        assert!(edge_rows(&fake, at, IMAGE + 0x1000).unwrap_err().contains("no pool"));
        // A lane's flag that is no bool.
        let mut fake = Fake::new();
        let mut odd = sample();
        odd.truncate(1);
        let at = engine(&mut fake, &odd, 5, 1);
        let pools = u64_at(&fake.read(at + layout::POOLS, 8).unwrap(), 0) as usize;
        let pool = u64_at(&fake.read(pools + 5 * 8, 8).unwrap(), 0) as usize;
        let dense = u64_at(&fake.read(pool + layout::POOL_DENSE, 8).unwrap(), 0) as usize;
        let edge = fake.read(dense, layout::BASE_EDGE_SIZE).unwrap();
        let configs = u64_at(&edge, layout::EDGE_LANE_CONFIGS) as usize;
        let mut c = fake.read(configs, 2 * layout::LANE_CONFIG_SIZE).unwrap();
        c[layout::LANE_FORWARD] = 7;
        fake.put(configs, c);
        assert!(edge_rows(&fake, at, IMAGE).unwrap_err().contains("forward flag"));
        // A number that is not finite.
        let mut fake = Fake::new();
        let mut nan = sample();
        nan[0].p1[1] = f32::NAN;
        let at = engine(&mut fake, &nan, 5, 4);
        assert!(edge_rows(&fake, at, IMAGE).unwrap_err().contains("not finite"));
        // An entity whose bits say BaseEdge and whose list does not.
        let mut fake = Fake::new();
        let at = engine(&mut fake, &edges, 5, 4);
        let bits = u64_at(&fake.read(at + layout::BITS, 8).unwrap(), 0) as usize;
        fake.blocks.get_mut(&bits).unwrap()[0] |= 1 << 5;
        assert!(edge_rows(&fake, at, IMAGE).unwrap_err().contains("no BaseEdge"));
    }

    /// One pool of a world built for a test: its vtable, type id, element
    /// size and elements by entity, all dense.
    struct Spec {
        vtable: usize,
        id: usize,
        size: usize,
        elements: Vec<(usize, Vec<u8>)>,
    }

    /// An engine with the pools of `specs` and `entities` entities.
    fn build(fake: &mut Fake, specs: &[Spec], entities: usize) -> usize {
        let top = specs.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        let mut pools = vec![0u8; top * 8];
        let mut lists: Vec<Vec<(i32, i32)>> = vec![Vec::new(); entities];
        let mut bits = vec![0u128; entities];
        for spec in specs {
            let mut data = Vec::new();
            for (i, (entity, bytes)) in spec.elements.iter().enumerate() {
                assert_eq!(bytes.len(), spec.size);
                data.extend_from_slice(bytes);
                lists[*entity].push((spec.id as i32, i as i32));
                bits[*entity] |= 1 << spec.id;
            }
            let len = data.len();
            let dense = fake.alloc(data);
            let mut pool = vec![0u8; layout::POOL_HEAD];
            put_u64(&mut pool, 0, (IMAGE + spec.vtable) as u64);
            put_u64(&mut pool, layout::POOL_DENSE, dense as u64);
            put_u64(&mut pool, layout::POOL_DENSE + 8, (dense + len) as u64);
            let at = fake.alloc(pool);
            put_u64(&mut pools, spec.id * 8, at as u64);
        }
        let pools_len = pools.len();
        let pools_at = fake.alloc(pools);
        let mut table = vec![0u8; entities * layout::ENTITY_RECORD];
        for (e, pairs) in lists.iter().enumerate() {
            let mut list = Vec::new();
            for (t, d) in pairs {
                list.extend_from_slice(&t.to_le_bytes());
                list.extend_from_slice(&d.to_le_bytes());
            }
            let len = list.len();
            let at = if len == 0 { 0 } else { fake.alloc(list) };
            put_u64(&mut table, e * 24, at as u64);
            put_u64(&mut table, e * 24 + 8, (at + len) as u64);
            put_u64(&mut table, e * 24 + 16, (at + len) as u64);
        }
        let table_len = table.len();
        let table_at = fake.alloc(table);
        let bits_at = fake.alloc(bits.iter().flat_map(|b| b.to_le_bytes()).collect());
        let mut head = vec![0u8; 0x100];
        put_u64(&mut head, layout::POOLS, pools_at as u64);
        put_u64(&mut head, layout::POOLS + 8, (pools_at + pools_len) as u64);
        put_u64(&mut head, layout::ENTITIES, table_at as u64);
        put_u64(&mut head, layout::ENTITIES + 8, (table_at + table_len) as u64);
        put_u64(&mut head, layout::BITS, bits_at as u64);
        fake.alloc(head)
    }

    struct Turn {
        from: usize,
        lane_in: i32,
        to: usize,
        lane_out: i32,
        road: bool,
        tram: bool,
    }

    struct Phase {
        locked: Vec<i32>,
        duration: f32,
        minimum: f32,
        skip: bool,
    }

    struct Config {
        node: usize,
        turns: Vec<Turn>,
        crosswalks: Vec<usize>,
        preference: i32,
        light: i32,
        double_slip: bool,
        custom: bool,
        phases: Vec<Phase>,
    }

    /// Nodes 10-13 and 20 (by position), edges 1-3 (1 and 2 streets, 3 a
    /// track) and 4 (a track from 13 to 20), and the junction configs.
    fn junction_world() -> (Vec<(usize, [f32; 3])>, Vec<(usize, usize, usize, bool)>, Vec<Config>) {
        let nodes = vec![
            (10, [0.0, 0.0, 0.0]),
            (11, [100.0004, -20.0005, 3.25]),
            (12, [200.5, 0.0015, -0.0004]),
            (13, [300.0, 300.0, 1.0]),
            (20, [-50.0, 7.0, 0.0]),
            (30, [9.0, 9.0, 9.0]),
        ];
        let edges = vec![(1, 10, 11, true), (2, 11, 12, true), (3, 12, 13, false), (4, 20, 13, false)];
        let configs = vec![
            Config {
                node: 11,
                turns: vec![
                    Turn { from: 1, lane_in: 0, to: 2, lane_out: 1, road: true, tram: false },
                    Turn { from: 2, lane_in: 1, to: 1, lane_out: 0, road: true, tram: true },
                ],
                crosswalks: vec![2, 1],
                preference: 2,
                light: -1,
                double_slip: false,
                custom: true,
                phases: vec![
                    Phase { locked: vec![2, 0], duration: 30.0, minimum: 5.5, skip: true },
                    Phase { locked: vec![3, 1], duration: 25.0625, minimum: 0.0, skip: false },
                ],
            },
            Config {
                node: 12,
                turns: vec![],
                crosswalks: vec![],
                preference: 0,
                light: 3,
                double_slip: true,
                custom: false,
                phases: vec![],
            },
            Config {
                node: 13,
                turns: vec![Turn { from: 3, lane_in: 0, to: 4, lane_out: 0, road: false, tram: false }],
                crosswalks: vec![],
                preference: 1,
                light: -1,
                double_slip: false,
                custom: false,
                phases: vec![],
            },
            // At no edge: junctions.lua never reads it.
            Config {
                node: 30,
                turns: vec![],
                crosswalks: vec![],
                preference: 0,
                light: -1,
                double_slip: false,
                custom: false,
                phases: vec![],
            },
        ];
        (nodes, edges, configs)
    }

    /// A phmap flat_hash_set<int> of `ids` in that slot order, as
    /// `crate::junctions` reads it, written into `raw` at 0x18.
    fn crosswalk_set(fake: &mut Fake, raw: &mut [u8], ids: &[usize]) {
        if ids.is_empty() {
            return;
        }
        let capacity = 7;
        let mut tags = vec![0x80u8; capacity + 1];
        tags[capacity] = 0xff;
        let mut slots = vec![0u8; capacity * 4];
        for (i, id) in ids.iter().enumerate() {
            tags[i] = 0x11;
            slots[i * 4..i * 4 + 4].copy_from_slice(&(*id as i32).to_le_bytes());
        }
        put_u64(raw, 0x18, fake.alloc(tags) as u64);
        put_u64(raw, 0x20, fake.alloc(slots) as u64);
        put_u64(raw, 0x28, ids.len() as u64);
        put_u64(raw, 0x30, capacity as u64);
    }

    fn vector_of(fake: &mut Fake, raw: &mut [u8], at: usize, bytes: Vec<u8>) {
        let len = bytes.len();
        let begin = if len == 0 { 0 } else { fake.alloc(bytes) };
        put_u64(raw, at, begin as u64);
        put_u64(raw, at + 8, (begin + len) as u64);
        put_u64(raw, at + 16, (begin + len) as u64);
    }

    fn junction_engine(fake: &mut Fake) -> usize {
        let (nodes, edges, configs) = junction_world();
        let mut edge_elements = Vec::new();
        for (entity, n0, n1, street) in &edges {
            let mut b = vec![0u8; layout::BASE_EDGE_SIZE];
            b[layout::EDGE_NODE0..layout::EDGE_NODE0 + 4].copy_from_slice(&(*n0 as i32).to_le_bytes());
            b[layout::EDGE_NODE1..layout::EDGE_NODE1 + 4].copy_from_slice(&(*n1 as i32).to_le_bytes());
            let road = if *street { layout::ROAD_TYPE_STREET } else { layout::ROAD_TYPE_TRACK };
            b[layout::EDGE_ROAD_TYPE..layout::EDGE_ROAD_TYPE + 4].copy_from_slice(&road.to_le_bytes());
            let p = |n: usize| nodes.iter().find(|(e, _)| *e == n).unwrap().1;
            for k in 0..3 {
                put_f32(&mut b, layout::EDGE_POSITION0 + 4 * k, p(*n0)[k]);
                put_f32(&mut b, layout::EDGE_POSITION1 + 4 * k, p(*n1)[k]);
            }
            edge_elements.push((*entity, b));
        }
        let node_elements = nodes
            .iter()
            .map(|(entity, p)| {
                let mut b = vec![0u8; layout::BASE_NODE_SIZE];
                for k in 0..3 {
                    put_f32(&mut b, layout::NODE_POSITION + 4 * k, p[k]);
                }
                (*entity, b)
            })
            .collect();
        let mut config_elements = Vec::new();
        for c in &configs {
            let mut raw = vec![0u8; layout::BASE_NODE_CONFIG_SIZE];
            let mut turns = Vec::new();
            for t in &c.turns {
                let mut b = vec![0u8; layout::TURN_SIZE];
                b[layout::TURN_SEGMENT0..][..4].copy_from_slice(&(t.from as i32).to_le_bytes());
                b[layout::TURN_LANE0..][..4].copy_from_slice(&t.lane_in.to_le_bytes());
                b[layout::TURN_SEGMENT1..][..4].copy_from_slice(&(t.to as i32).to_le_bytes());
                b[layout::TURN_LANE1..][..4].copy_from_slice(&t.lane_out.to_le_bytes());
                b[layout::TURN_ROAD] = u8::from(t.road);
                b[layout::TURN_TRAM] = u8::from(t.tram);
                turns.extend(b);
            }
            vector_of(fake, &mut raw, layout::CONFIG_TURNS, turns);
            crosswalk_set(fake, &mut raw, &c.crosswalks);
            let mut phases = Vec::new();
            for p in &c.phases {
                let mut b = vec![0u8; layout::PHASE_SIZE];
                vector_of(fake, &mut b, layout::PHASE_LOCKED, p.locked.iter().flat_map(|l| l.to_le_bytes()).collect());
                put_f32(&mut b, layout::PHASE_DURATION, p.duration);
                put_f32(&mut b, layout::PHASE_MINIMUM, p.minimum);
                b[layout::PHASE_SKIP] = u8::from(p.skip);
                phases.extend(b);
            }
            vector_of(fake, &mut raw, layout::CONFIG_PHASES, phases);
            raw[layout::CONFIG_DOUBLE_SLIP] = u8::from(c.double_slip);
            raw[layout::CONFIG_PREFERENCE..][..4].copy_from_slice(&c.preference.to_le_bytes());
            raw[layout::CONFIG_LIGHT_TYPE..][..4].copy_from_slice(&c.light.to_le_bytes());
            raw[layout::CONFIG_CUSTOM_PHASES] = u8::from(c.custom);
            config_elements.push((c.node, raw));
        }
        let specs = [
            Spec { vtable: layout::BASE_EDGE_POOL_VTABLE, id: 3, size: layout::BASE_EDGE_SIZE, elements: edge_elements },
            Spec { vtable: layout::BASE_NODE_POOL_VTABLE, id: 9, size: layout::BASE_NODE_SIZE, elements: node_elements },
            Spec {
                vtable: layout::BASE_NODE_CONFIG_POOL_VTABLE,
                id: 70,
                size: layout::BASE_NODE_CONFIG_SIZE,
                elements: config_elements,
            },
        ];
        build(fake, &specs, 40)
    }

    /// junctions.lua's own `rows` over a fake `api` holding the same world,
    /// and its `rowsFromParts` over the hook's parts.
    fn lua_junctions(parts: &[Junction]) -> (Vec<String>, Vec<String>) {
        let (nodes, edges, configs) = junction_world();
        let lua = mlua::Lua::new();
        let scripts = concat!(env!("CARGO_MANIFEST_DIR"), "/../../mod/tpf3mp_1/content/scripts/");
        lua.load(format!("package.path = {:?} .. '?.lua;' .. package.path", scripts))
            .exec()
            .unwrap();
        let mut world = String::from("local nodes, edges, configs = {}, {}, {}\n");
        for (e, p) in &nodes {
            world += &format!(
                "nodes[{e}] = {{ position = {{ x = {}, y = {}, z = {} }} }}\n",
                f64::from(p[0]),
                f64::from(p[1]),
                f64::from(p[2])
            );
        }
        for (e, n0, n1, street) in &edges {
            world += &format!("edges[{e}] = {{ node0 = {n0}, node1 = {n1}, street = {street} }}\n");
        }
        for c in &configs {
            let turns: Vec<String> = c
                .turns
                .iter()
                .map(|t| {
                    format!(
                        "{{ segment0 = {}, lane0 = {}, segment1 = {}, lane1 = {}, withRoad = {}, withTram = {} }}",
                        t.from, t.lane_in, t.to, t.lane_out, t.road, t.tram
                    )
                })
                .collect();
            let walks: Vec<String> = c.crosswalks.iter().map(ToString::to_string).collect();
            let phases: Vec<String> = c
                .phases
                .iter()
                .map(|p| {
                    let locked: Vec<String> = p.locked.iter().map(ToString::to_string).collect();
                    format!(
                        "{{ lockedLanes = {{ {} }}, duration = {}, minDuration = {}, canSkip = {} }}",
                        locked.join(", "),
                        f64::from(p.duration),
                        f64::from(p.minimum),
                        p.skip
                    )
                })
                .collect();
            world += &format!(
                "configs[{}] = {{ laneConnections = {{ {} }}, crosswalks = {{ {} }}, trafficLightPreference = {}, \
                 doubleSlipSwitch = {}, userModifiedTrafficLightStates = {}, \
                 trafficLightConfig = {{ trafficLightType = {}, states = {{ {} }} }} }}\n",
                c.node,
                turns.join(", "),
                walks.join(", "),
                c.preference,
                c.double_slip,
                c.custom,
                c.light,
                phases.join(", ")
            );
        }
        world += r#"
local function segments(street)
  return function(node)
    local out = {}
    for e, edge in pairs(edges) do
      if edge.street == street and (edge.node0 == node or edge.node1 == node) then out[#out + 1] = e end
    end
    table.sort(out)
    return out
  end
end
local function nodeMap(street)
  return function()
    local out = {}
    for e, edge in pairs(edges) do
      if edge.street == street then out[edge.node0] = { e } out[edge.node1] = { e } end
    end
    return out
  end
end
api = {
  type = {
    ComponentType = { BASE_NODE_CONFIG = "config", BASE_NODE = "node", BASE_EDGE = "edge" },
    enum = { TrafficLightPreference = { AUTO = 0, YES = 1, NO = 2 } },
  },
  res = { trafficLightTypeRep = { getName = function(i) return "lights/type" .. i .. ".lua" end } },
  engine = {
    getComponent = function(id, kind)
      if kind == "config" then return configs[id] end
      if kind == "node" then return nodes[id] end
      if kind == "edge" then return edges[id] end
    end,
    system = { streetSystem = {
      getNode2StreetEdgeMap = nodeMap(true), getNode2TrackEdgeMap = nodeMap(false),
      getNodeStreetSegments = segments(true), getNodeTrackSegments = segments(false),
    } },
  },
}
"#;
        lua.load(&world).exec().unwrap();
        let junctions: mlua::Table = lua.load("return require('tpf3mp.junctions')").eval().unwrap();
        let api: mlua::Table = lua.globals().get("api").unwrap();
        let rows: Vec<String> = junctions.get::<mlua::Function>("rows").unwrap().call(api.clone()).unwrap();
        let table = lua.create_table().unwrap();
        let list = |items: Vec<mlua::Value>| lua.create_sequence_from(items).unwrap();
        table
            .set("heads", list(parts.iter().map(|j| mlua::Value::String(lua.create_string(&j.head).unwrap())).collect()))
            .unwrap();
        table
            .set("tails", list(parts.iter().map(|j| mlua::Value::String(lua.create_string(&j.tail).unwrap())).collect()))
            .unwrap();
        table
            .set("preferences", list(parts.iter().map(|j| mlua::Value::Number(f64::from(j.preference))).collect()))
            .unwrap();
        table
            .set("lights", list(parts.iter().map(|j| mlua::Value::Number(f64::from(j.light))).collect()))
            .unwrap();
        let made: Vec<String> = junctions
            .get::<mlua::Function>("rowsFromParts")
            .unwrap()
            .call((api, table))
            .unwrap();
        (rows, made)
    }

    #[test]
    fn junction_rows_read_as_the_mods_lua_makes_them() {
        let mut fake = Fake::new();
        let engine = junction_engine(&mut fake);
        let network = network(&fake, engine, IMAGE).unwrap();
        let parts = network.junctions.unwrap();
        assert_eq!(parts.len(), 3, "the node at no edge is left out");
        let (rows, made) = lua_junctions(&parts);
        assert_eq!(rows.len(), 3);
        assert_eq!(made, rows);
    }

    #[test]
    fn a_junction_that_does_not_read_fails_the_junctions_alone() {
        let mut fake = Fake::new();
        let engine = junction_engine(&mut fake);
        // Edge 2 names a road type that is neither.
        let pools = u64_at(&fake.read(engine + layout::POOLS, 8).unwrap(), 0) as usize;
        let pool = u64_at(&fake.read(pools + 3 * 8, 8).unwrap(), 0) as usize;
        let dense = u64_at(&fake.read(pool + layout::POOL_DENSE, 8).unwrap(), 0) as usize;
        fake.blocks.get_mut(&dense).unwrap()[layout::BASE_EDGE_SIZE + layout::EDGE_ROAD_TYPE] = 7;
        assert!(network(&fake, engine, IMAGE).err().unwrap().contains("road type 7"));
        // A phase locking a lane its junction does not have.
        let mut fake = Fake::new();
        let engine = junction_engine(&mut fake);
        let pools = u64_at(&fake.read(engine + layout::POOLS, 8).unwrap(), 0) as usize;
        let pool = u64_at(&fake.read(pools + 70 * 8, 8).unwrap(), 0) as usize;
        let dense = u64_at(&fake.read(pool + layout::POOL_DENSE, 8).unwrap(), 0) as usize;
        let raw = fake.read(dense, layout::BASE_NODE_CONFIG_SIZE).unwrap();
        let phases = u64_at(&raw, layout::CONFIG_PHASES) as usize;
        let locked = u64_at(&fake.read(phases, 8).unwrap(), 0) as usize;
        fake.blocks.get_mut(&locked).unwrap()[0] = 9;
        let network = network(&fake, engine, IMAGE).unwrap();
        assert_eq!(network.edges.len(), 4);
        assert!(network.junctions.unwrap_err().contains("references no lane"));
    }

    #[test]
    fn the_mode_is_off_unless_asked() {
        assert_eq!(Mode::from_env(None).0, Mode::Off);
        assert_eq!(Mode::from_env(Some(" compare ")).0, Mode::Compare);
        assert_eq!(Mode::from_env(Some("ON")).0, Mode::On);
        let (mode, why) = Mode::from_env(Some("yes"));
        assert_eq!(mode, Mode::Off);
        assert!(why.unwrap().contains(ENV));
    }
}

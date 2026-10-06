//! The parcel walk's probe: would a query per box, instead of the union of
//! the boxes, find everything the game's walk acts on? (docs/HOOKS.md,
//! "The parcel walk's probe"; investigation/TF3_PARCEL_COLLISION_2026-10-06.md.)
//!
//! CHECK ONLY, off unless [`ENV`] is `1`: it changes nothing. Four calls
//! inside `parcel_util::UpdateParcelCollision`'s walk are redirected to
//! counters, each of which calls the game's callee with the arguments it
//! was given and returns what it returned:
//!
//! - the node visitor's node dereference: every octree node the walk
//!   visits, and whether its loose box meets some box plus the margin;
//! - the visitor's `GetComponentDataIndex` of each entity's
//!   `BoundingVolume`: every entity the walk tests, whether it is in the
//!   union box (the game's own test) and whether it meets some box plus the
//!   margin;
//! - the ParcelSystem's visit of a street's parcels: every street the walk
//!   hands on, and the time its parcels take;
//! - the element test of each parcel: a parcel with a non-empty result is
//!   the only kind the walk writes (its elements' collision flags, its
//!   change notes, in the walk's order). For each such parcel, whether its
//!   street, and its street's node, meet some box plus the margin, and the
//!   margin that would have kept both.
//!
//! A per-box walk (a node or entity is visited only when it meets some box
//! plus the margin) keeps the game's visit order, so it writes exactly
//! what the game writes if and only if it loses no acted-on parcel: the
//! `lost` counts must stay 0. The code alone does not prove that (a
//! parcel's elements are not bounded by its street's volume anywhere in the
//! walk), so only the game can show it.
//!
//! The aggregates go into the `perf: sim` line, once a window.

#![allow(unsafe_code)]
#![cfg_attr(not(all(windows, target_arch = "x86_64")), allow(dead_code))]

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub use crate::build_data::native::simperf::{
    BOUNDING_VOLUME_STRIDE, ENGINE_STORAGES, NODE_BOX, NODE_ENTITIES, PARCEL_MARGIN,
    PROBE_INDEX_CALL, PROBE_INDEX_CALLEE, PROBE_NODE_CALL, PROBE_NODE_CALLEE, PROBE_PARCEL_CALL,
    PROBE_PARCEL_CALLEE, PROBE_STREET_CALL, PROBE_STREET_CALLEE, STORAGE_DENSE, STORAGE_PAGE_ENTRY,
    STORAGE_PAGE_LEN, STORAGE_PAGED_FROM, STORAGE_PAGES,
};

/// `1` turns the probe on (with the timers, `TPF3MP_HOOK_PERF` not `0`).
pub const ENV: &str = "TPF3MP_HOOK_PARCEL_PROBE";

/// A box as the walk takes it: `{min x, min y, max x, max y}`.
pub type Box2 = [f32; 4];
/// A volume as the octree keeps it: `{min x, y, z, max x, y, z}`.
pub type Aabb = [f32; 6];

/// The walk's query for `boxes`, as `UpdateParcelCollision` builds it
/// (`0x931346`..`0x9313fb`): the boxes' union (`vminss`/`vmaxss`, the box
/// first, so a NaN keeps what was there), widened by `margin` on every
/// side, z unbounded. `None` for no boxes (the game asserts).
pub fn union_query(boxes: &[Box2], margin: f32) -> Option<Aabb> {
    let first = boxes.first()?;
    let mut u = *first;
    for b in boxes {
        u[0] = if b[0] < u[0] { b[0] } else { u[0] };
        u[1] = if b[1] < u[1] { b[1] } else { u[1] };
        u[2] = if b[2] > u[2] { b[2] } else { u[2] };
        u[3] = if b[3] > u[3] { b[3] } else { u[3] };
    }
    Some(box_query(&u, margin))
}

/// One box's query, widened as the walk widens the union: a per-box walk
/// would test this. Each one lies inside [`union_query`] (the same f32
/// subtraction and addition of the margin, both monotone).
pub fn box_query(b: &Box2, margin: f32) -> Aabb {
    [
        b[0] - margin,
        b[1] - margin,
        f32::MIN,
        b[2] + margin,
        b[3] + margin,
        f32::MAX,
    ]
}

/// The walk's overlap test of a query and a volume, compare for compare
/// (the descent `0x925e67`, the visitor `0x92631d`): apart only when
/// `q.min > v.max` or `v.min >= q.max` on some axis; a NaN never parts.
pub fn meets(q: &Aabb, v: &Aabb) -> bool {
    !(q[0] > v[3] || v[0] >= q[3] || q[1] > v[4] || v[1] >= q[4] || q[2] > v[5] || v[2] >= q[5])
}

/// Whether `v` meets some box's query.
pub fn meets_any(queries: &[Aabb], v: &Aabb) -> bool {
    queries.iter().any(|q| meets(q, v))
}

/// How far, in metres, `v` lies outside box `b` in x or y: the margin a
/// per-box query of `b` needs to reach it (0 when they overlap).
pub fn gap(v: &Aabb, b: &Box2) -> f64 {
    let d = [
        f64::from(b[0]) - f64::from(v[3]),
        f64::from(v[0]) - f64::from(b[2]),
        f64::from(b[1]) - f64::from(v[4]),
        f64::from(v[1]) - f64::from(b[3]),
    ];
    d.into_iter().fold(0.0, |m, x| if x > m { x } else { m })
}

/// The margin a per-box query needs to reach `v` from its nearest box.
pub fn needed(v: &Aabb, boxes: &[Box2]) -> f64 {
    boxes
        .iter()
        .map(|b| gap(v, b))
        .fold(f64::INFINITY, f64::min)
}

/// What one walk saw.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Counts {
    pub walks: u64,
    /// Nodes the walk visited, and those meeting some box's query.
    pub nodes: u64,
    pub nodes_near: u64,
    /// Entities the walk tested (every one in a visited node), those in
    /// near nodes (what a per-box walk would test), those in the union
    /// query, and those meeting some box's query.
    pub entities: u64,
    pub entities_in_near_nodes: u64,
    pub entities_union: u64,
    pub entities_near: u64,
    /// Streets handed to the ParcelSystem, and those meeting some box's
    /// query.
    pub streets: u64,
    pub streets_near: u64,
    /// Parcels tested, and those the walk acts on.
    pub parcels: u64,
    pub acted: u64,
    /// Acted-on parcels a per-box walk would lose: their street, or their
    /// street's node, meets no box's query.
    pub lost_street: u64,
    pub lost_node: u64,
    /// The largest margin, in centimetres, an acted-on parcel's street and
    /// node needed; `None` while none was acted on.
    pub needed_cm: Option<u64>,
    /// Time in the streets' parcel visits, all and far ones.
    pub street_nanos: u64,
    pub far_street_nanos: u64,
}

impl Counts {
    fn add(&mut self, o: &Counts) {
        self.walks += o.walks;
        self.nodes += o.nodes;
        self.nodes_near += o.nodes_near;
        self.entities += o.entities;
        self.entities_in_near_nodes += o.entities_in_near_nodes;
        self.entities_union += o.entities_union;
        self.entities_near += o.entities_near;
        self.streets += o.streets;
        self.streets_near += o.streets_near;
        self.parcels += o.parcels;
        self.acted += o.acted;
        self.lost_street += o.lost_street;
        self.lost_node += o.lost_node;
        self.needed_cm = match (self.needed_cm, o.needed_cm) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        self.street_nanos += o.street_nanos;
        self.far_street_nanos += o.far_street_nanos;
    }

    /// The probe's part of the `perf: sim` line.
    pub fn line(&self) -> String {
        let margin = match self.needed_cm {
            Some(cm) => format!("{:.2} m", cm as f64 / 100.0),
            None => "none acted".to_owned(),
        };
        format!(
            "; parcel probe {} walks: nodes {} (near {}), entities {} (in near nodes {}, \
             in union {}, near {}), streets {} (near {}), parcels {} (acted {}, lost {} by \
             street, {} by node at {} m; margin needed {}), street work {:.2} ms (far {:.2} ms)",
            self.walks,
            self.nodes,
            self.nodes_near,
            self.entities,
            self.entities_in_near_nodes,
            self.entities_union,
            self.entities_near,
            self.streets,
            self.streets_near,
            self.parcels,
            self.acted,
            self.lost_street,
            self.lost_node,
            PARCEL_MARGIN,
            margin,
            self.street_nanos as f64 / 1e6,
            self.far_street_nanos as f64 / 1e6,
        )
    }
}

/// One walk in progress on this thread.
#[derive(Debug, Default)]
pub struct Walk {
    boxes: Vec<Box2>,
    union: Option<Aabb>,
    queries: Vec<Aabb>,
    /// The node being visited, and the entity last tested in it.
    node: Option<(Aabb, bool)>,
    entity: Option<(Aabb, bool)>,
    counts: Counts,
}

impl Walk {
    pub fn new(boxes: &[Box2]) -> Self {
        Self {
            boxes: boxes.to_vec(),
            union: union_query(boxes, PARCEL_MARGIN),
            queries: boxes.iter().map(|b| box_query(b, PARCEL_MARGIN)).collect(),
            counts: Counts {
                walks: 1,
                ..Counts::default()
            },
            ..Self::default()
        }
    }

    /// The visitor reached a node with this loose box and entities.
    pub fn node(&mut self, aabb: Aabb, entities: u64) {
        let near = meets_any(&self.queries, &aabb);
        self.counts.nodes += 1;
        self.counts.entities_in_near_nodes += if near { entities } else { 0 };
        self.counts.nodes_near += u64::from(near);
        self.node = Some((aabb, near));
        self.entity = None;
    }

    /// The visitor tests an entity with this volume.
    pub fn entity(&mut self, aabb: Aabb) {
        let near = meets_any(&self.queries, &aabb);
        self.counts.entities += 1;
        self.counts.entities_near += u64::from(near);
        self.counts.entities_union += u64::from(self.union.is_some_and(|q| meets(&q, &aabb)));
        self.entity = Some((aabb, near));
    }

    /// The street last tested is handed on; returns whether it is near.
    pub fn street(&mut self) -> bool {
        let near = self.entity.is_some_and(|(_, near)| near);
        self.counts.streets += 1;
        self.counts.streets_near += u64::from(near);
        near
    }

    /// The street's parcels took `nanos`.
    pub fn street_done(&mut self, near: bool, nanos: u64) {
        self.counts.street_nanos += nanos;
        if !near {
            self.counts.far_street_nanos += nanos;
        }
    }

    /// A parcel of the current street was tested; `acted` when its result
    /// is not empty.
    pub fn parcel(&mut self, acted: bool) {
        self.counts.parcels += 1;
        if !acted {
            return;
        }
        self.counts.acted += 1;
        let (street, street_near) = self.entity.unwrap_or(([f32::NAN; 6], false));
        let (node, node_near) = self.node.unwrap_or(([f32::NAN; 6], false));
        self.counts.lost_street += u64::from(!street_near);
        self.counts.lost_node += u64::from(!node_near);
        let need = needed(&street, &self.boxes).max(needed(&node, &self.boxes));
        let cm = if need.is_finite() {
            (need * 100.0).ceil().clamp(0.0, u64::MAX as f64) as u64
        } else {
            u64::MAX
        };
        self.counts.needed_cm = Some(self.counts.needed_cm.map_or(cm, |c| c.max(cm)));
    }

    pub fn counts(&self) -> &Counts {
        &self.counts
    }
}

thread_local! {
    /// The walk this thread is in, while the probe is on.
    static WALK: RefCell<Option<Walk>> = const { RefCell::new(None) };
}

/// Whether the redirects are in.
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// The window's sums, as [`Counts`] (behind a lock: a walk ends at most a
/// few dozen times a second).
static WINDOW: std::sync::Mutex<Option<Counts>> = std::sync::Mutex::new(None);

/// Whether the probe is in.
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// A walk begins on this thread over `boxes` (from the timer around
/// `UpdateParcelCollision`); returns the walk it interrupts, if any, for
/// [`end`].
pub fn begin(boxes: &[Box2]) -> Option<Walk> {
    if !installed() {
        return None;
    }
    WALK.with(|w| {
        w.try_borrow_mut()
            .ok()
            .and_then(|mut w| w.replace(Walk::new(boxes)))
    })
}

/// The walk ended: its counts go into the window; `outer` is put back.
pub fn end(outer: Option<Walk>) {
    if !installed() {
        return;
    }
    let walk = WALK.with(|w| {
        w.try_borrow_mut()
            .ok()
            .and_then(|mut w| std::mem::replace(&mut *w, outer))
    });
    if let Some(walk) = walk {
        let mut window = WINDOW.lock().unwrap_or_else(|p| p.into_inner());
        window
            .get_or_insert_with(Counts::default)
            .add(walk.counts());
    }
}

/// Takes the window's sums; `None` while the probe is not in.
pub fn take() -> Option<Counts> {
    if !installed() {
        return None;
    }
    let mut window = WINDOW.lock().unwrap_or_else(|p| p.into_inner());
    Some(window.take().unwrap_or_default())
}

/// Runs `f` on this thread's walk, when there is one.
fn with_walk(f: impl FnOnce(&mut Walk)) {
    WALK.with(|w| {
        if let Ok(mut w) = w.try_borrow_mut()
            && let Some(walk) = w.as_mut()
        {
            f(walk);
        }
    });
}

// The callees, set before their calls are redirected.
static NODE_FN: AtomicUsize = AtomicUsize::new(0);
static INDEX_FN: AtomicUsize = AtomicUsize::new(0);
static STREET_FN: AtomicUsize = AtomicUsize::new(0);
static PARCEL_FN: AtomicUsize = AtomicUsize::new(0);

type NodeFn = unsafe extern "system-unwind" fn(usize) -> usize;
type IndexFn = unsafe extern "system-unwind" fn(usize, usize, usize) -> usize;
type StreetFn = unsafe extern "system-unwind" fn(usize, usize, usize);
type ParcelFn = unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize) -> usize;

/// Six floats at `at`.
///
/// # Safety
///
/// 24 readable bytes at `at`.
unsafe fn aabb_at(at: usize) -> Aabb {
    // SAFETY: the caller's.
    unsafe { std::ptr::read_unaligned(at as *const Aabb) }
}

/// The pointer-sized value at `at`.
///
/// # Safety
///
/// Eight readable bytes at `at`.
unsafe fn word(at: usize) -> usize {
    // SAFETY: the caller's.
    unsafe { std::ptr::read_unaligned(at as *const usize) }
}

/// The visitor's node dereference: the node it returns.
unsafe extern "system-unwind" fn node_hook(iterator: usize) -> usize {
    // SAFETY: set before the call was redirected: the game's dereference.
    let original = unsafe { std::mem::transmute::<usize, NodeFn>(NODE_FN.load(Ordering::Acquire)) };
    // SAFETY: the game's own call, with its own argument.
    let node = unsafe { original(iterator) };
    with_walk(|walk| {
        // SAFETY: the node component the visitor reads next, at the same
        // offsets (its box was read by the descent before this call, its
        // entity vector is read right after it).
        let (aabb, begin, end) = unsafe {
            (
                aabb_at(node + NODE_BOX),
                word(node + NODE_ENTITIES),
                word(node + NODE_ENTITIES + 8),
            )
        };
        walk.node(aabb, (end.wrapping_sub(begin) / 4) as u64);
    });
    node
}

/// The record a storage keeps for `index`, as the visitor finds it
/// (`0x9262ce`..`0x926315`).
///
/// # Safety
///
/// `engine` and `ty` as the visitor passed them, and `index` what the
/// lookup returned for them: the visitor reads the same memory right after.
unsafe fn bounding_volume(engine: usize, ty: usize, index: i32) -> usize {
    let ty = ty as u32 as i32 as isize;
    // SAFETY: the caller's; the reads the visitor makes.
    unsafe {
        let storage = word(word(engine + ENGINE_STORAGES).wrapping_add_signed(ty * 8));
        if index < STORAGE_PAGED_FROM {
            word(storage + STORAGE_DENSE)
                .wrapping_add_signed(index as isize * BOUNDING_VOLUME_STRIDE as isize)
        } else {
            let i = index.wrapping_sub(STORAGE_PAGED_FROM);
            let (page, slot) = (i / STORAGE_PAGE_LEN, i % STORAGE_PAGE_LEN);
            let pages = word(storage + STORAGE_PAGES);
            word(pages.wrapping_add_signed(page as isize * STORAGE_PAGE_ENTRY as isize))
                .wrapping_add_signed(slot as isize * BOUNDING_VOLUME_STRIDE as isize)
        }
    }
}

/// The visitor's lookup of an entity's `BoundingVolume`: its index.
unsafe extern "system-unwind" fn index_hook(engine: usize, entity: usize, ty: usize) -> usize {
    // SAFETY: set before the call was redirected: the game's lookup.
    let original =
        unsafe { std::mem::transmute::<usize, IndexFn>(INDEX_FN.load(Ordering::Acquire)) };
    // SAFETY: the game's own call, with its own arguments.
    let index = unsafe { original(engine, entity, ty) };
    with_walk(|walk| {
        // SAFETY: the record the visitor reads right after this call.
        let aabb = unsafe { aabb_at(bounding_volume(engine, ty, index as u32 as i32)) };
        walk.entity(aabb);
    });
    index
}

/// The ParcelSystem's visit of one street's parcels.
unsafe extern "system-unwind" fn street_hook(system: usize, street: usize, visit: usize) {
    // SAFETY: set before the call was redirected: the game's visit.
    let original =
        unsafe { std::mem::transmute::<usize, StreetFn>(STREET_FN.load(Ordering::Acquire)) };
    let mut near = None;
    with_walk(|walk| near = Some(walk.street()));
    let start = near.map(|_| std::time::Instant::now());
    // SAFETY: the game's own call, with its own arguments.
    unsafe { original(system, street, visit) };
    if let (Some(near), Some(start)) = (near, start) {
        let nanos = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        with_walk(|walk| walk.street_done(near, nanos));
    }
}

/// The element test of one parcel; `result` its vector of flags.
unsafe extern "system-unwind" fn parcel_hook(
    context: usize,
    parcel: usize,
    r8: usize,
    boxes: usize,
    result: usize,
) -> usize {
    // SAFETY: set before the call was redirected: the game's test.
    let original =
        unsafe { std::mem::transmute::<usize, ParcelFn>(PARCEL_FN.load(Ordering::Acquire)) };
    // SAFETY: the game's own call, with its own arguments.
    let out = unsafe { original(context, parcel, r8, boxes, result) };
    with_walk(|walk| {
        // SAFETY: the caller's local vector (begin, end), which it reads
        // right after this call.
        let (begin, end) = unsafe { (word(result), word(result + 8)) };
        walk.parcel(begin != end);
    });
    out
}

/// What [`install`] needs: each site's address and its callee's.
#[derive(Debug, Clone, Copy)]
pub struct Sites {
    pub node: (usize, usize),
    pub index: (usize, usize),
    pub street: (usize, usize),
    pub parcel: (usize, usize),
}

impl Sites {
    /// From the profile's targets and the callees' RVAs at `base`.
    pub fn resolve(
        resolved: &tpf3mp_hookcore::profile::ResolvedProfile,
        base: u64,
    ) -> Result<Self, String> {
        let site = |name: &str| {
            resolved
                .get(name)
                .map(|t| t.address as usize)
                .ok_or_else(|| format!("the profile has no {name:?}"))
        };
        let callee = |rva: u64| base.wrapping_add(rva) as usize;
        Ok(Self {
            node: (site(PROBE_NODE_CALL)?, callee(PROBE_NODE_CALLEE)),
            index: (site(PROBE_INDEX_CALL)?, callee(PROBE_INDEX_CALLEE)),
            street: (site(PROBE_STREET_CALL)?, callee(PROBE_STREET_CALLEE)),
            parcel: (site(PROBE_PARCEL_CALL)?, callee(PROBE_PARCEL_CALLEE)),
        })
    }
}

/// Whether [`ENV`] asks for the probe.
pub fn wanted() -> Result<bool, String> {
    crate::bigmap::switch(std::env::var(ENV).ok().as_deref())
}

/// Redirects the four calls, all or none.
///
/// # Safety
///
/// `sites` are the walk's calls of their callees, which no thread runs
/// during this call.
pub unsafe fn install_at(sites: Sites) -> Result<String, String> {
    use tpf3mp_hookcore::detour::CallRedirect;
    NODE_FN.store(sites.node.1, Ordering::Release);
    INDEX_FN.store(sites.index.1, Ordering::Release);
    STREET_FN.store(sites.street.1, Ordering::Release);
    PARCEL_FN.store(sites.parcel.1, Ordering::Release);
    let hooks: [((usize, usize), *const u8); 4] = [
        (sites.node, node_hook as *const u8),
        (sites.index, index_hook as *const u8),
        (sites.street, street_hook as *const u8),
        (sites.parcel, parcel_hook as *const u8),
    ];
    let mut done = Vec::new();
    for ((site, callee), to) in hooks {
        // SAFETY: the caller's; each hook has its callee's ABI and calls it.
        match unsafe { CallRedirect::install(site as *mut u8, callee, to) } {
            Ok(redirect) => done.push(redirect),
            Err(error) => {
                // Dropping the ones made restores their calls.
                drop(done);
                return Err(format!("the call at {site:#x}: {error}"));
            }
        }
    }
    for redirect in done {
        std::mem::forget(redirect);
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(format!(
        "CHECK ONLY, changes nothing; calls at {:#x}, {:#x}, {:#x}, {:#x}",
        sites.node.0, sites.index.0, sites.street.0, sites.parcel.0
    ))
}

/// Installs the probe when [`ENV`] asks for it and the walk's timer is in;
/// returns the line for hook.log.
pub fn install(
    resolved: &tpf3mp_hookcore::profile::ResolvedProfile,
    base: u64,
    timer_in: bool,
) -> String {
    match wanted() {
        Ok(false) => return format!("perf: sim parcel probe: off ({ENV} is not 1)"),
        Err(why) => return format!("perf: sim parcel probe: off, {ENV}: {why}"),
        Ok(true) => {}
    }
    if !timer_in {
        return "perf: sim parcel probe: absent, the parcel-collision timer is not in".to_owned();
    }
    // SAFETY: the profile resolved the sites; no game thread runs yet;
    // CallRedirect checks each call reaches its callee.
    let outcome = Sites::resolve(resolved, base).and_then(|sites| unsafe { install_at(sites) });
    match outcome {
        Ok(how) => format!("perf: sim parcel probe: in ({how})"),
        Err(why) => format!("perf: sim parcel probe: absent, {why}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_union_query_is_the_walks() {
        assert_eq!(union_query(&[], 50.0), None);
        let q = union_query(&[[0.0, 0.0, 10.0, 10.0], [100.0, -5.0, 110.0, 5.0]], 50.0).unwrap();
        assert_eq!(q, [-50.0, -55.0, f32::MIN, 160.0, 60.0, f32::MAX]);
        // A NaN in a later box keeps what the union had (vminss/vmaxss with
        // the box first).
        let q = union_query(&[[0.0, 0.0, 1.0, 1.0], [f32::NAN; 4]], 0.0).unwrap();
        assert_eq!(q[..2], [0.0, 0.0]);
        assert_eq!(q[3..5], [1.0, 1.0]);
    }

    #[test]
    fn every_box_query_lies_in_the_union_query() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 2_000_000) as f32 / 7.0 - 140_000.0
        };
        for _ in 0..2_000 {
            let boxes: Vec<Box2> = (0..1 + (next().abs() as usize % 6))
                .map(|_| {
                    let (x, y) = (next(), next());
                    [x, y, x + next().abs() / 50.0, y + next().abs() / 50.0]
                })
                .collect();
            let u = union_query(&boxes, PARCEL_MARGIN).unwrap();
            for b in &boxes {
                let q = box_query(b, PARCEL_MARGIN);
                assert!(u[0] <= q[0] && u[1] <= q[1] && q[3] <= u[3] && q[4] <= u[4]);
                // So whatever meets a box's query meets the union's.
                let v = [q[3], q[4], 0.0, q[3] + 1.0, q[4] + 1.0, 1.0];
                assert!(!meets(&q, &v) || meets(&u, &v));
            }
        }
    }

    #[test]
    fn meeting_is_the_walks_compare() {
        let q = [0.0, 0.0, f32::MIN, 10.0, 10.0, f32::MAX];
        // Touching the low side meets (q.min > v.max is false) ...
        assert!(meets(&q, &[-5.0, -5.0, 0.0, 0.0, 0.0, 0.0]));
        // ... touching the high side does not (v.min >= q.max).
        assert!(!meets(&q, &[10.0, 5.0, 0.0, 12.0, 6.0, 0.0]));
        assert!(!meets(&q, &[11.0, 5.0, 0.0, 12.0, 6.0, 0.0]));
        assert!(!meets(&q, &[2.0, -9.0, 0.0, 3.0, -0.5, 0.0]));
        // Any height meets the unbounded z.
        assert!(meets(&q, &[2.0, 2.0, 1e30, 3.0, 3.0, 2e30]));
        // A NaN never parts.
        assert!(meets(&q, &[f32::NAN; 6]));
    }

    #[test]
    fn the_needed_margin_is_the_gap_to_the_nearest_box() {
        let boxes = [[0.0, 0.0, 10.0, 10.0], [100.0, 0.0, 110.0, 10.0]];
        // 30 m right of the first box, 60 m left of the second.
        let v = [40.0, 2.0, 0.0, 40.0, 3.0, 0.0];
        assert_eq!(needed(&v, &boxes), 30.0);
        // Overlapping one: none.
        assert_eq!(needed(&[5.0, 5.0, 0.0, 6.0, 6.0, 0.0], &boxes), 0.0);
        // Diagonal: the larger axis gap.
        assert_eq!(
            needed(&[-20.0, -70.0, 0.0, -12.0, -40.0, 0.0], &boxes),
            40.0
        );
    }

    fn aabb(x: f32, y: f32, size: f32) -> Aabb {
        [x, y, 0.0, x + size, y + size, 10.0]
    }

    #[test]
    fn a_walk_counts_what_a_per_box_walk_would_keep_and_lose() {
        // Two boxes 10 km apart: the union spans both.
        let mut walk = Walk::new(&[[0.0, 0.0, 10.0, 10.0], [10_000.0, 0.0, 10_010.0, 10.0]]);
        // A node near the first box, holding a near street with an acted
        // parcel and a far street (in the union) with an idle parcel.
        walk.node(aabb(-100.0, -100.0, 300.0), 2);
        walk.entity(aabb(20.0, 0.0, 5.0));
        let near = walk.street();
        assert!(near);
        walk.parcel(true);
        walk.street_done(near, 1_000);
        walk.entity(aabb(150.0, 0.0, 5.0));
        let far = walk.street();
        assert!(!far);
        walk.parcel(false);
        walk.street_done(far, 3_000);
        // A far node, between the boxes, whose street's parcel is acted on:
        // a per-box walk would lose it.
        walk.node(aabb(5_000.0, 0.0, 128.0), 1);
        walk.entity(aabb(5_000.0, 0.0, 10.0));
        let far = walk.street();
        walk.parcel(true);
        walk.street_done(far, 500);
        let c = walk.counts();
        assert_eq!(
            (c.walks, c.nodes, c.nodes_near, c.entities_in_near_nodes),
            (1, 2, 1, 2)
        );
        assert_eq!((c.entities, c.entities_union, c.entities_near), (3, 3, 1));
        assert_eq!((c.streets, c.streets_near), (3, 1));
        assert_eq!(
            (c.parcels, c.acted, c.lost_street, c.lost_node),
            (3, 2, 1, 1)
        );
        // The far street is 4,990 m from the first box and 4,990 m from
        // the second; its node 4,862 m from the second.
        assert_eq!(c.needed_cm, Some(499_000));
        assert_eq!((c.street_nanos, c.far_street_nanos), (4_500, 3_500));
    }

    #[test]
    fn the_probe_line_says_what_was_kept_and_lost() {
        let mut c = Counts {
            walks: 85,
            nodes: 9_000,
            nodes_near: 700,
            entities: 400_000,
            entities_in_near_nodes: 30_000,
            entities_union: 250_000,
            entities_near: 9_000,
            streets: 20_000,
            streets_near: 900,
            parcels: 60_000,
            acted: 300,
            lost_street: 0,
            lost_node: 0,
            needed_cm: Some(1_234),
            street_nanos: 500_000_000,
            far_street_nanos: 470_000_000,
        };
        assert_eq!(
            c.line(),
            "; parcel probe 85 walks: nodes 9000 (near 700), entities 400000 (in near nodes \
             30000, in union 250000, near 9000), streets 20000 (near 900), parcels 60000 \
             (acted 300, lost 0 by street, 0 by node at 50 m; margin needed 12.34 m), \
             street work 500.00 ms (far 470.00 ms)"
        );
        c.needed_cm = None;
        assert!(
            c.line().contains("margin needed none acted"),
            "{}",
            c.line()
        );
        // Summing keeps the largest margin.
        let mut sum = Counts::default();
        sum.add(&Counts {
            needed_cm: Some(5),
            walks: 1,
            ..Counts::default()
        });
        sum.add(&Counts {
            needed_cm: None,
            walks: 2,
            ..Counts::default()
        });
        assert_eq!((sum.walks, sum.needed_cm), (3, Some(5)));
    }

    /// The four hooks hand the game's callees their arguments, all of them,
    /// and give back what they returned; inside a walk they count, outside
    /// one they only pass through.
    #[test]
    fn the_hooks_pass_everything_through_and_count() {
        use std::sync::Mutex;
        static SEEN: Mutex<Vec<(&str, [usize; 5])>> = Mutex::new(Vec::new());
        // A fake engine: storages[3] -> a storage whose dense array holds
        // two volumes and whose page 1 holds one at slot 2.
        let volumes: Vec<Aabb> = vec![aabb(1_000.0, 1_000.0, 5.0), aabb(2.0, 2.0, 1.0)];
        let mut page = vec![[0f32; 6]; 32];
        page[2] = aabb(4.0, 4.0, 1.0);
        let pages: Vec<[usize; 2]> = vec![[0, 0], [page.as_ptr() as usize, 0]];
        let mut storage = [0usize; 0x88 / 8 + 1];
        storage[STORAGE_DENSE / 8] = volumes.as_ptr() as usize;
        storage[STORAGE_PAGES / 8] = pages.as_ptr() as usize;
        let storages = [0usize, 0, 0, storage.as_ptr() as usize];
        let mut engine = [0usize; ENGINE_STORAGES / 8 + 1];
        engine[ENGINE_STORAGES / 8] = storages.as_ptr() as usize;
        let engine = engine.as_ptr() as usize;
        // A node: box at +8, its entity vector (three ints) at +0x20.
        let ids = [7i32, 8, 9];
        let mut node = [0u8; 0xc0];
        let node_box = aabb(-10.0, -10.0, 100.0);
        node[8..32].copy_from_slice(bytemuck_aabb(&node_box));
        node[0x20..0x28].copy_from_slice(&(ids.as_ptr() as usize).to_le_bytes());
        node[0x28..0x30].copy_from_slice(&(ids.as_ptr() as usize + 12).to_le_bytes());
        let node = node.as_ptr() as usize;

        static NODE: AtomicUsize = AtomicUsize::new(0);
        static INDICES: Mutex<Vec<usize>> = Mutex::new(Vec::new());
        unsafe extern "system-unwind" fn game_node(it: usize) -> usize {
            SEEN.lock().unwrap().push(("node", [it, 0, 0, 0, 0]));
            NODE.load(Ordering::Relaxed)
        }
        unsafe extern "system-unwind" fn game_index(e: usize, id: usize, ty: usize) -> usize {
            SEEN.lock().unwrap().push(("index", [e, id, ty, 0, 0]));
            INDICES.lock().unwrap().remove(0)
        }
        unsafe extern "system-unwind" fn game_street(s: usize, id: usize, f: usize) {
            SEEN.lock().unwrap().push(("street", [s, id, f, 0, 0]));
        }
        unsafe extern "system-unwind" fn game_parcel(
            a: usize,
            b: usize,
            c: usize,
            d: usize,
            e: usize,
        ) -> usize {
            SEEN.lock().unwrap().push(("parcel", [a, b, c, d, e]));
            0x55
        }
        NODE.store(node, Ordering::Relaxed);
        *INDICES.lock().unwrap() = vec![0, 1, 0x4000_0000 + 32 + 2, 1];
        NODE_FN.store(game_node as *const () as usize, Ordering::Release);
        INDEX_FN.store(game_index as *const () as usize, Ordering::Release);
        STREET_FN.store(game_street as *const () as usize, Ordering::Release);
        PARCEL_FN.store(game_parcel as *const () as usize, Ordering::Release);
        INSTALLED.store(true, Ordering::Release);
        let _ = take();

        // Outside a walk: through, nothing counted.
        // SAFETY: the fakes take what the hooks pass.
        unsafe {
            assert_eq!(node_hook(0x11), node);
            assert_eq!(index_hook(engine, 7, 3), 0);
        }
        // A walk over one box at the origin.
        let outer = begin(&[[0.0, 0.0, 10.0, 10.0]]);
        assert!(outer.is_none());
        let acted: [usize; 3] = [0x100, 0x108, 0x110];
        let idle: [usize; 3] = [0x100, 0x100, 0x110];
        // SAFETY: as above; the result vectors are readable.
        unsafe {
            assert_eq!(node_hook(0x22), node);
            // Entity 8, the dense volume at (2, 2): near.
            assert_eq!(index_hook(engine, 8, 3 | 0xdead_0000_0000), 1);
            street_hook(0x33, 8, 0x44);
            assert_eq!(game_parcel_via_hook(&acted), 0x55);
            // Entity 9, page 1 slot 2 at (4, 4): near, idle parcel.
            assert_eq!(index_hook(engine, 9, 3), 0x4000_0000 + 32 + 2);
            street_hook(0x33, 9, 0x44);
            assert_eq!(game_parcel_via_hook(&idle), 0x55);
        }
        end(outer);
        let c = take().unwrap();
        assert_eq!(
            (c.walks, c.nodes, c.nodes_near, c.entities_in_near_nodes),
            (1, 1, 1, 3)
        );
        assert_eq!(
            (c.entities, c.entities_near, c.streets, c.streets_near),
            (2, 2, 2, 2)
        );
        assert_eq!(
            (c.parcels, c.acted, c.lost_street, c.lost_node),
            (2, 1, 0, 0)
        );
        assert_eq!(c.needed_cm, Some(0));
        let seen = SEEN.lock().unwrap().clone();
        assert_eq!(
            seen,
            vec![
                ("node", [0x11, 0, 0, 0, 0]),
                ("index", [engine, 7, 3, 0, 0]),
                ("node", [0x22, 0, 0, 0, 0]),
                ("index", [engine, 8, 3 | 0xdead_0000_0000, 0, 0]),
                ("street", [0x33, 8, 0x44, 0, 0]),
                ("parcel", [1, 2, 3, 4, acted.as_ptr() as usize]),
                ("index", [engine, 9, 3, 0, 0]),
                ("street", [0x33, 9, 0x44, 0, 0]),
                ("parcel", [1, 2, 3, 4, idle.as_ptr() as usize]),
            ]
        );
        INSTALLED.store(false, Ordering::Release);
    }

    /// Calls the parcel hook as the functor does, with `result`.
    unsafe fn game_parcel_via_hook(result: &[usize; 3]) -> usize {
        // SAFETY: the caller's.
        unsafe { parcel_hook(1, 2, 3, 4, result.as_ptr() as usize) }
    }

    fn bytemuck_aabb(v: &Aabb) -> &[u8] {
        // SAFETY: six f32s are 24 plain bytes.
        unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), 24) }
    }
}

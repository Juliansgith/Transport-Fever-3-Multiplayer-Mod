//! The octree at depth 12: ids for the level stock 32-bit ids cannot
//! number, and the renderer's level decoder taught to read them.
//!
//! A tree of depth `d` has levels `0..d`, the root counting as one; the
//! descent (0xae33c0) numbers a child `8·id + 1 + octant` in 32 bits. At
//! depth 11 the deepest level, 10, ends at id 1,227,133,512; depth 12's
//! level 11 would run from 1,227,133,513 to 9,817,068,104, past 32 bits,
//! and the renderer's `CalcOctreeLevel` (0x818b30), whose level thresholds
//! wrap to a step of 0 after level 10, would loop forever on any of them
//! (investigation/TF3_BIGMAPS_256KM_2026-10-05.md, "Depth 12").
//!
//! tpf2-bigmap (silver2127, MIT; no code taken) gave levels 11 and 12
//! ranges of ids drawn from counters. A counter's ids depend on the
//! process's history (which world it loaded before, how often), so two
//! games of a room would number their deep nodes differently. Here a
//! level-11 node's id is a pure function of its box instead, as a stock
//! id is a pure function of its path: `0x50000000 + zslot·2²² + y·2¹¹ +
//! x`, the node's 128 m cell in the root's 2,048 by 2,048 grid and one of
//! 128 height slots, z from −8,192 to +8,192 m (TF3's terrain spans −100 to
//! about 3,200 m). Every game of a room with the same tree has the same ids,
//! whoever reads them. A node outside the height band is clamped into it
//! (and counted): its id may then repeat another's, which only the
//! renderer's per-node caches key by.
//!
//! What it touches, each only on a tree of depth 12 (the descent reads the
//! depth from its own `EcsOctree`), so any shallower world runs the game's
//! code unchanged:
//!
//! - a detour of the descent that replaces the incoming id of a level-11
//!   node, before the game stores it;
//! - a splice before the decoder's loop: an id at or past `0x50000000`
//!   (no stock id reaches it) answers level 11 and skips the loop.
//!
//! Level 11 is the deepest level of a depth-12 tree, so its nodes are
//! leaves: no id is ever derived from theirs.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use tpf3mp_hookcore::detour::{InlineDetour, SavedRegs, Splice};
use tpf3mp_hookcore::profile::ResolvedProfile;

pub use crate::build_data::native::bigmap::{
    DECODER, DECODER_STOLEN, DESCENT, DESCENT_CHECKS, DESCENT_START, DESCENT_START_CALL,
    DESCENT_START_CHECKS, ECS_OCTREE_BOX, ECS_OCTREE_DEPTH,
};
use crate::log;

/// The depth this module numbers.
pub const DEPTH: i32 = 12;
/// Its deepest level, the one stock ids cannot number.
pub const DEEP_LEVEL: i32 = DEPTH - 1;
/// The first id of the deep level: past every stock id (`0x49249248`).
pub const LEVEL11_BASE: i32 = 0x5000_0000;
/// The deep level's cells a side: 2¹¹.
pub const CELLS: i64 = 1 << 11;
/// Height slots, centred on z = 0.
pub const Z_SLOTS: i64 = 128;
/// The last stock id at depth 11 (level 10's last).
pub const STOCK_LAST_ID: i64 = 0x4924_9248;

/// A level-11 node's id from the root's box (min x, y, z, max x, y, z)
/// and the node's minimum corner, and whether its height was clamped.
pub fn level11_id(root: [f32; 6], node_min: [f32; 3]) -> (i32, bool) {
    let mut clamped = false;
    let mut cell = |axis: usize| -> i64 {
        let size = (f64::from(root[axis + 3]) - f64::from(root[axis])) / CELLS as f64;
        let index = ((f64::from(node_min[axis]) - f64::from(root[axis])) / size).floor();
        if !index.is_finite() {
            clamped = true;
            return 0;
        }
        index as i64
    };
    let (x, y, z) = (cell(0), cell(1), cell(2));
    let fit = |v: i64, n: i64, clamped: &mut bool| {
        if !(0..n).contains(&v) {
            *clamped = true;
        }
        v.clamp(0, n - 1)
    };
    let x = fit(x, CELLS, &mut clamped);
    let y = fit(y, CELLS, &mut clamped);
    let z = fit(z - (CELLS / 2 - Z_SLOTS / 2), Z_SLOTS, &mut clamped);
    let id = i64::from(LEVEL11_BASE) + (z << 22) + (y << 11) + x;
    (i32::try_from(id).unwrap_or(i32::MAX), clamped)
}

/// The level the patched decoder gives an id: 11 at or past
/// [`LEVEL11_BASE`]; otherwise the game's own loop's answer.
pub fn decoded_level(id: i32) -> Option<i32> {
    if id >= LEVEL11_BASE {
        Some(DEEP_LEVEL)
    } else {
        stock_level(id, 64)
    }
}

/// The game's decoder loop, as its code runs it (32-bit wrapping), with a
/// bound on the iterations: `None` where it would not have ended.
pub fn stock_level(id: i32, bound: u32) -> Option<i32> {
    if id < 0 {
        return None; // the game asserts `nodeIndex >= 0`
    }
    let (mut sum, mut step, mut level) = (0i32, 1i32, 0i32);
    if id < step {
        return Some(level);
    }
    for _ in 0..bound {
        sum = sum.wrapping_add(step);
        level += 1;
        step = step.wrapping_mul(8);
        if id < sum.wrapping_add(step) {
            return Some(level);
        }
    }
    None
}

/// The stock id a node gets from its path of octants, in 64 bits.
pub fn stock_id(path: &[i64]) -> i64 {
    path.iter().fold(0, |id, octant| 8 * id + 1 + octant)
}

/// `detail::EcsOctreeIterator`'s descent: ten arguments, returns `rdx`.
type DescentFn = unsafe extern "system" fn(
    usize,
    usize,
    i32,
    i32,
    i32,
    usize,
    usize,
    i32,
    *const f32,
    i32,
) -> usize;

static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static NUMBERED: AtomicU64 = AtomicU64::new(0);
static CLAMPED: AtomicU64 = AtomicU64::new(0);

/// The id a node at `remaining` depth left gets in the tree `octree`
/// points at, where depth 12 needs one; `None` keeps the game's.
///
/// # Safety
///
/// `octree` is the descent's `EcsOctree` and `node_box` its node's box,
/// as the game passes them.
unsafe fn deep_id(octree: usize, remaining: i32, node_box: *const f32) -> Option<i32> {
    // SAFETY: the EcsOctree the game passed, which the descent reads too.
    let depth = unsafe { std::ptr::read_unaligned((octree + ECS_OCTREE_DEPTH) as *const i32) };
    if depth != DEPTH || depth - remaining != DEEP_LEVEL || node_box.is_null() {
        return None;
    }
    // SAFETY: as above; the node's box is six floats the descent reads.
    let root = unsafe { std::ptr::read_unaligned((octree + ECS_OCTREE_BOX) as *const [f32; 6]) };
    let min = unsafe { std::ptr::read_unaligned(node_box.cast::<[f32; 3]>()) };
    let (id, clamped) = level11_id(root, min);
    let n = NUMBERED.fetch_add(1, Ordering::Relaxed);
    if clamped && CLAMPED.fetch_add(1, Ordering::Relaxed) == 0 {
        log::line(&format!(
            "big maps: octree depth 12: a node at {min:?} is outside the height band (±8,192 m); its id {id:#x} is clamped (renderer caches only)"
        ));
    }
    if n == 0 {
        log::line(&format!(
            "big maps: octree depth 12: first level-11 node numbered {id:#x}"
        ));
    }
    Some(id)
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn descent(
    octree: usize,
    out: usize,
    parent: i32,
    id: i32,
    octant: i32,
    position: usize,
    extent: usize,
    remaining: i32,
    node_box: *const f32,
    node: i32,
) -> usize {
    // SAFETY: the game's own arguments.
    let id = unsafe { deep_id(octree, remaining, node_box) }.unwrap_or(id);
    let original = ORIGINAL.load(Ordering::Acquire);
    // SAFETY: the trampoline to the game's descent, with its arguments.
    unsafe {
        std::mem::transmute::<usize, DescentFn>(original)(
            octree, out, parent, id, octant, position, extent, remaining, node_box, node,
        )
    }
}

/// Before the decoder's loop: a deep id gets level 11 and skips it
/// (`r9d` becomes the level the stolen `mov r12d,r9d` copies, and `r8d`
/// the step the stolen `cmp edx,r8d` makes `jl` take past the loop).
unsafe extern "system" fn decoder_hook(regs: *mut SavedRegs) {
    // SAFETY: the stub's block, held until the hook returns.
    let regs = unsafe { &mut *regs };
    let id = regs.rdx as u32 as i32;
    if id >= LEVEL11_BASE {
        regs.r9 = DEEP_LEVEL as u64;
        regs.r8 = 0x7fff_ffff;
    }
}

/// What a check of the game's bytes found.
fn check(at: usize, checks: &[(usize, &[u8])], what: &str) -> Result<(), String> {
    for (offset, bytes) in checks {
        let address = at + offset;
        if !crate::image::readable(address, bytes.len()) {
            return Err(format!("{what}+{offset:#x} is unreadable"));
        }
        // SAFETY: readable, checked just above.
        let found = unsafe { std::slice::from_raw_parts(address as *const u8, bytes.len()) };
        if found != *bytes {
            return Err(format!(
                "{what}+{offset:#x} is not the code depth 12 was read on"
            ));
        }
    }
    Ok(())
}

/// The installed pieces, kept for the life of the game.
pub struct Installed {
    pub descent: InlineDetour,
    pub decoder: Splice,
}

/// Checks the descent, its starter and the decoder, then installs the
/// decoder's splice and the descent's detour; on any failure, nothing.
pub fn install(resolved: &ResolvedProfile) -> Result<Installed, String> {
    let at = |name: &str| -> Result<usize, String> {
        let target = resolved
            .get(name)
            .ok_or_else(|| format!("the profile has no {name:?}"))?;
        usize::try_from(target.address).map_err(|_| "an address past usize".to_owned())
    };
    let (descent_at, start_at, decoder_at) = (at(DESCENT)?, at(DESCENT_START)?, at(DECODER)?);
    check(descent_at, &DESCENT_CHECKS, DESCENT)?;
    check(start_at, &DESCENT_START_CHECKS, DESCENT_START)?;
    let call = start_at + DESCENT_START_CALL;
    if !crate::image::readable(call, 5) {
        return Err(format!("{DESCENT_START}'s call is unreadable"));
    }
    // SAFETY: five readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(call as *const u8, 5) };
    let rel = i32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
    if bytes[0] != 0xE8 || (call + 5).wrapping_add_signed(rel as isize) != descent_at {
        return Err(format!("{DESCENT_START} does not call {DESCENT}"));
    }
    // SAFETY: the decoder's `mov r12d,r9d; cmp edx,r8d`, checked by the
    // splice; only the loop's back edge branches into the function past
    // them, and nothing runs the renderer yet.
    let decoder = unsafe {
        Splice::install(
            decoder_at as *mut u8,
            &DECODER_STOLEN,
            DECODER_STOLEN.len(),
            decoder_hook,
        )
    }
    .map_err(|error| format!("{DECODER}: {error}"))?;
    // SAFETY: the descent the profile resolved and its body checked; no
    // world exists yet; `descent` has its ABI.
    match unsafe { InlineDetour::install(descent_at as *mut u8, descent as *const u8) } {
        Ok(detour) => {
            ORIGINAL.store(detour.trampoline() as usize, Ordering::Release);
            Ok(Installed {
                descent: detour,
                decoder,
            })
        }
        Err(error) => {
            // SAFETY: as above.
            let _ = unsafe { decoder.detach() };
            Err(format!("{DESCENT}: {error}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: [f32; 6] = [
        -131_072.0, -131_072.0, -131_072.0, 131_072.0, 131_072.0, 131_072.0,
    ];

    #[test]
    fn deep_ids_are_the_cell_and_never_a_stock_id() {
        assert_eq!(
            level11_id(ROOT, [-131_072.0, -131_072.0, -8_192.0]),
            (LEVEL11_BASE, false)
        );
        assert_eq!(
            level11_id(
                ROOT,
                [131_072.0 - 128.0, 131_072.0 - 128.0, 8_192.0 - 128.0]
            ),
            (0x6fff_ffff, false)
        );
        assert_eq!(
            level11_id(ROOT, [0.0, 0.0, 0.0]).0,
            LEVEL11_BASE + (64 << 22) + (1024 << 11) + 1024
        );
        // Out of the height band: clamped, still a deep id.
        let (id, clamped) = level11_id(ROOT, [0.0, 0.0, 9_000.0]);
        assert!(clamped && id >= LEVEL11_BASE);
        assert!(i64::from(LEVEL11_BASE) > STOCK_LAST_ID);
        // Every cell of a row is its own id.
        let mut seen = std::collections::HashSet::new();
        for x in 0..CELLS {
            for z in [-8_192.0f32, 0.0, 3_200.0] {
                let corner = -131_072.0 + x as f32 * 128.0;
                assert!(seen.insert(level11_id(ROOT, [corner, 640.0, z]).0));
            }
        }
    }

    #[test]
    fn the_stock_decoder_ends_at_depth_11_and_loops_past_it() {
        // Level L starts at (8^L - 1) / 7.
        for level in 0..=10 {
            let first = ((8i64.pow(level) - 1) / 7) as i32;
            assert_eq!(stock_level(first, 64), Some(level as i32));
            if level > 0 {
                assert_eq!(stock_level(first - 1, 64), Some(level as i32 - 1));
            }
        }
        assert_eq!(stock_level(STOCK_LAST_ID as i32, 64), Some(10));
        // The first id depth 12's level 11 would get by the stock rule, and
        // anything past it: the loop never ends.
        assert_eq!(stock_level((STOCK_LAST_ID + 1) as i32, 1_000_000), None);
        assert_eq!(stock_level(LEVEL11_BASE, 1_000_000), None);
        // A stock level-11 id is past 32 bits.
        assert!(stock_id(&[7; 11]) > i64::from(u32::MAX));
        assert_eq!(stock_id(&[7; 10]), STOCK_LAST_ID);
        // Patched: deep ids answer 11, every stock id as before.
        assert_eq!(decoded_level(LEVEL11_BASE), Some(11));
        assert_eq!(decoded_level(0x6fff_ffff), Some(11));
        assert_eq!(decoded_level(STOCK_LAST_ID as i32), Some(10));
    }

    #[test]
    fn nothing_installs_without_the_targets() {
        let empty = ResolvedProfile {
            name: "empty".into(),
            targets: Vec::new(),
            absent_optional: Vec::new(),
        };
        assert!(
            install(&empty)
                .err()
                .unwrap()
                .contains("the profile has no")
        );
    }
}

/// The game's own descent, its starter and the renderer's decoder,
/// relocated from the executable and run on a depth-12 tree, with the ECS
/// calls the descent makes answered by a model of the component store
/// (crate::bigmap::original).
#[cfg(all(test, windows, target_arch = "x86_64"))]
#[allow(clippy::unwrap_used, clippy::vec_box)]
mod original_tests {
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    use tpf3mp_hookcore::profile::ResolvedTarget;

    use super::*;
    use crate::bigmap::original::{Exe, Page};

    static SERIAL: Mutex<()> = Mutex::new(());

    const DESCENT_RVA: u64 = 0xae33c0;
    const DESCENT_LEN: usize = 1421;
    const START_RVA: u64 = 0xae3950;
    const START_LEN: usize = 309;
    const DECODER_FN_RVA: u64 = 0x818b30;
    const DECODER_FN_LEN: usize = 374;
    const DECODER_SITE: usize = 0x2b;
    const NODE: usize = 0xc0;
    const TYPE_INDEX: i32 = 7;

    thread_local! {
        /// The node components, by entity; boxed so pointers stay put.
        static STORE: RefCell<Vec<Box<[u8; NODE]>>> = const { RefCell::new(Vec::new()) };
    }

    fn node_ptr(entity: i32) -> *mut u8 {
        STORE.with(|s| s.borrow_mut()[entity as usize].as_mut_ptr())
    }

    extern "system" fn create(_engine: usize, out: *mut i32) -> *mut i32 {
        let entity = STORE.with(|s| {
            let mut s = s.borrow_mut();
            s.push(Box::new([0; NODE]));
            s.len() as i32 - 1
        });
        // SAFETY: the descent's own out slot.
        unsafe { *out = entity };
        out
    }
    extern "system" fn add(_engine: usize, entity: i32, data: *const u8, _r9: usize, _flag: usize) {
        // SAFETY: the descent's 0xc0-byte node data, into the model.
        unsafe { std::ptr::copy_nonoverlapping(data, node_ptr(entity), NODE) };
    }
    extern "system" fn index_of(_engine: usize, entity: i32, type_index: i32) -> i32 {
        assert_eq!(type_index, TYPE_INDEX);
        entity
    }
    extern "system" fn comp_at(_vec: usize, type_index: i32, index: i32) -> *mut u8 {
        assert_eq!(type_index, TYPE_INDEX);
        node_ptr(index)
    }
    extern "system" fn comp_of(_engine: usize, entity: i32, type_index: i32) -> *mut u8 {
        assert_eq!(type_index, TYPE_INDEX);
        node_ptr(entity)
    }
    extern "system" fn nothing(rcx: usize) -> usize {
        rcx
    }
    extern "system" fn fail() {
        std::process::abort();
    }

    fn addr(f: *const ()) -> u64 {
        f as u64
    }

    /// A near jump through a slot holding `f`.
    fn jump(page: &mut Page, f: u64) -> usize {
        let slot = page.data(&f.to_le_bytes());
        let at = page.code(&[0xFF, 0x25, 0, 0, 0, 0]);
        let rel = (slot as i64 - (at as i64 + 6)) as i32;
        // SAFETY: the jump's displacement, in the page just written.
        unsafe {
            std::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), (at + 2) as *mut u8, 4)
        };
        at
    }

    struct Rig {
        _page: Page,
        resolved: ResolvedProfile,
        start: usize,
        decoder: usize,
    }

    fn rig(exe: &Exe) -> Rig {
        let mut page = Page::new();
        let stubs: HashMap<u64, usize> = [
            (0x2bb3760, addr(create as *const ())),
            (0x2bb2360, addr(nothing as *const ())),
            (0x2bb2ab0, addr(nothing as *const ())),
            (0xade7b0, addr(add as *const ())),
            (0xa4b90, addr(index_of as *const ())),
            (0xadf100, addr(comp_at as *const ())),
            (0x2d91c0, addr(comp_of as *const ())),
            (0x74f20, addr(nothing as *const ())),
            (0x318426c, addr(nothing as *const ())),
            (0x3184140, addr(nothing as *const ())),
            (0xaa0c0, addr(fail as *const ())),
            (0x303d380, addr(fail as *const ())),
            (0xdf160, addr(fail as *const ())),
        ]
        .into_iter()
        .map(|(rva, f)| (rva, jump(&mut page, f)))
        .collect();
        let fail_slot = page.data(&addr(fail as *const ()).to_le_bytes());
        let mut data = HashMap::new();
        for (rva, len) in [
            (0x3676640u64, 4usize),
            (0x3ce3a38, 8),
            (0x3672620, 8),
            (0x3689e00, 16),
            (0x3689e58, 16),
            (0x3700280, 16),
            (0x36dffd0, 16),
            (0x36e0000, 16),
            (0x36e0068, 16),
        ] {
            data.insert(rva, page.data(exe.bytes(rva, len)));
        }
        // The descent's IAT call of _invalid_parameter_noinfo_noreturn.
        let code = exe.bytes(DESCENT_RVA, DESCENT_LEN);
        let at = 0x351;
        assert_eq!(code[at..at + 2], [0xFF, 0x15]);
        let disp = i32::from_le_bytes(code[at + 2..at + 6].try_into().unwrap());
        let iat = (DESCENT_RVA as i64 + at as i64 + 6 + i64::from(disp)) as u64;
        let descent_copy = page.here();
        let map = |target: u64| -> Option<usize> {
            if target == DESCENT_RVA {
                return Some(descent_copy);
            }
            if target == iat {
                return Some(fail_slot);
            }
            stubs.get(&target).or_else(|| data.get(&target)).copied()
        };
        let (descent, offsets) = page.relocate(exe, DESCENT_RVA, DESCENT_LEN, &map);
        assert_eq!(descent, descent_copy);
        for (offset, _) in DESCENT_CHECKS {
            assert!(
                offsets.contains(&(offset, offset)),
                "the relocated descent keeps +{offset:#x}"
            );
        }
        let (start, _) = page.relocate(exe, START_RVA, START_LEN, &map);
        let (decoder, offsets) = page.relocate(exe, DECODER_FN_RVA, DECODER_FN_LEN, &map);
        let site = offsets
            .iter()
            .find(|(old, _)| *old == DECODER_SITE)
            .unwrap()
            .1;
        let target = |name: &str, address: usize| ResolvedTarget {
            name: name.into(),
            address: address as u64,
            image_index: 0,
            required: false,
        };
        Rig {
            _page: page,
            resolved: ResolvedProfile {
                name: "relocated".into(),
                targets: vec![
                    target(DESCENT, descent),
                    target(DESCENT_START, start),
                    target(DECODER, decoder + site),
                ],
                absent_optional: Vec::new(),
            },
            start,
            decoder,
        }
    }

    /// The EcsOctree the starter reads, at `depth` with root entity 0.
    #[repr(C)]
    struct EcsOctree {
        engine: usize,
        root: [f32; 6],
        depth: i32,
        root_entity: i32,
        type_index: i32,
        pad: [u8; 0x14],
    }

    type Start =
        unsafe extern "system" fn(*const EcsOctree, *mut [u8; 0x18], *const [f32; 6]) -> usize;

    /// A fresh tree at `depth`, ±`half`: the root node alone.
    fn tree(depth: i32, half: f32) -> EcsOctree {
        STORE.with(|s| {
            let mut s = s.borrow_mut();
            s.clear();
            let mut root = Box::new([0u8; NODE]);
            root[0..4].copy_from_slice(&(-1i32).to_le_bytes());
            for (i, v) in [-half, -half, -half, half, half, half].iter().enumerate() {
                root[8 + 4 * i..12 + 4 * i].copy_from_slice(&v.to_le_bytes());
            }
            s.push(root);
        });
        EcsOctree {
            engine: 0x1000,
            root: [-half, -half, -half, half, half, half],
            depth,
            root_entity: 0,
            type_index: TYPE_INDEX,
            pad: [0; 0x14],
        }
    }

    /// Inserts each box through the game's starter; the node each lands in.
    fn insert(rig: &Rig, octree: &EcsOctree, boxes: &[[f32; 6]]) -> Vec<i32> {
        // SAFETY: the relocated starter, the game's own ABI.
        let start = unsafe { std::mem::transmute::<usize, Start>(rig.start) };
        boxes
            .iter()
            .map(|b| {
                let mut out = [0u8; 0x18];
                // SAFETY: the starter's arguments, alive for the call.
                unsafe { start(octree, &mut out, b) };
                i32::from_le_bytes(out[8..12].try_into().unwrap())
            })
            .collect()
    }

    type Node = (i32, i32, i32, [f32; 6]);

    /// Every node: (entity, parent, id, box).
    fn nodes() -> Vec<Node> {
        STORE.with(|s| {
            s.borrow()
                .iter()
                .enumerate()
                .map(|(entity, n)| {
                    let int = |at: usize| i32::from_le_bytes(n[at..at + 4].try_into().unwrap());
                    let f = |at: usize| f32::from_le_bytes(n[at..at + 4].try_into().unwrap());
                    (
                        entity as i32,
                        int(0),
                        int(4),
                        [f(8), f(12), f(16), f(20), f(24), f(28)],
                    )
                })
                .collect()
        })
    }

    fn level(parents: &HashMap<i32, i32>, mut entity: i32) -> i32 {
        let mut level = 0;
        while parents[&entity] >= 0 {
            entity = parents[&entity];
            level += 1;
        }
        level
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % 1_000_000) as f32 / 1_000_000.0
        }
    }

    /// Objects over a 1000 x 186 map (±128,000 by ±23,808 m), heights 0 to
    /// 3,200 m: 2 m ones that reach the leaves, and every tenth 600 m.
    fn objects(seed: u64, n: usize) -> Vec<[f32; 6]> {
        let mut rng = Rng(seed);
        (0..n)
            .map(|i| {
                let x = rng.next() * 256_000.0 - 128_000.0;
                let y = rng.next() * 47_616.0 - 23_808.0;
                let z = rng.next() * 3_200.0;
                let size = if i % 10 == 0 { 600.0 } else { 2.0 };
                [x, y, z, x + size, y + size, z + size]
            })
            .collect()
    }

    type Decode = unsafe extern "system" fn(usize, i32, *mut [usize; 3]);

    /// The level the relocated decoder gives `id`: it copies the node
    /// lists of its level and deeper, the first one flagged; each level's
    /// list here holds its own number.
    fn decode(rig: &Rig, id: i32) -> i32 {
        let mut levels: Vec<Vec<i32>> = (0..12).map(|l| vec![l]).collect();
        let lists: Vec<[usize; 3]> = levels
            .iter_mut()
            .map(|l| {
                let p = l.as_mut_ptr() as usize;
                [p, p + 4, p + 4]
            })
            .collect();
        let mut skip = vec![0usize; 0x120 / 8];
        skip[0x108 / 8] = lists.as_ptr() as usize;
        skip[0x110 / 8] = lists.as_ptr() as usize + lists.len() * 24;
        let mut out = [0u64; 64];
        let p = out.as_mut_ptr() as usize;
        let mut vector = [p, p, p + 64 * 8];
        // SAFETY: the relocated decoder on a skip manager of 12 levels.
        let decode = unsafe { std::mem::transmute::<usize, Decode>(rig.decoder) };
        unsafe { decode(skip.as_ptr() as usize, id, &mut vector) };
        assert!(vector[1] > p, "the decoder copied nothing for {id:#x}");
        assert_eq!(
            out[0] >> 32 & 0xff,
            1,
            "the first entry is the node's own level"
        );
        (out[0] & 0xffff_ffff) as i32
    }

    #[test]
    fn the_games_descent_builds_a_depth_12_tree_with_deep_ids_and_the_decoder_reads_them() {
        let Some(exe) = Exe::load() else { return };
        let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        let rig = rig(&exe);
        // Depth 11, the game's code unpatched.
        let octree = tree(11, 65_536.0);
        let small: Vec<[f32; 6]> = objects(1, 500)
            .into_iter()
            .map(|b| b.map(|v| v / 2.0))
            .collect();
        insert(&rig, &octree, &small);
        let stock11 = nodes();
        // Depth 12 unpatched: level 11's ids are past what the decoder ends on.
        let octree = tree(12, 131_072.0);
        insert(&rig, &octree, &objects(1, 2_000));
        let parents: HashMap<i32, i32> = nodes().iter().map(|n| (n.0, n.1)).collect();
        let wrapped = nodes()
            .iter()
            .filter(|n| level(&parents, n.0) == 11)
            .filter(|n| stock_level(n.2, 1_000_000).is_none())
            .count();
        assert!(
            wrapped > 1_000,
            "stock level-11 ids the decoder cannot end on: {wrapped}"
        );

        let installed = install(&rig.resolved).unwrap();
        // Depth 11 patched: the same tree and the same ids.
        let octree = tree(11, 65_536.0);
        insert(&rig, &octree, &small);
        let same = nodes();
        assert_eq!(same.len(), stock11.len());
        for (a, b) in same.iter().zip(&stock11) {
            assert_eq!(
                (a.0, a.1, a.2, a.3.map(f32::to_bits)),
                (b.0, b.1, b.2, b.3.map(f32::to_bits))
            );
        }
        for &(_, _, id, _) in same.iter().step_by(37) {
            assert_eq!(
                Some(decode(&rig, id)),
                stock_level(id, 64),
                "stock id {id:#x}"
            );
        }

        // Depth 12 patched.
        let octree = tree(12, 131_072.0);
        let boxes = objects(1, 3_000);
        let leaves = insert(&rig, &octree, &boxes);
        let all = nodes();
        let parents: HashMap<i32, i32> = all.iter().map(|n| (n.0, n.1)).collect();
        let mut ids = HashSet::new();
        let mut deep = 0;
        for &(entity, _, id, b) in &all {
            let level = level(&parents, entity);
            assert!(level <= 11, "no level past 11");
            assert!(id >= 0, "{entity}: {id:#x}");
            assert!(ids.insert(id), "id {id:#x} twice");
            // A node stores its loose box: its cell grown by half a cell
            // each way (the root here is the test's own, its cell).
            let tight = 262_144.0 / (1u32 << level) as f32;
            let size = b[3] - b[0];
            let cell_min = [b[0], b[1], b[2]].map(|v| v + tight / 2.0);
            if level > 0 {
                assert_eq!(size, 2.0 * tight, "level {level}");
            }
            if level == 11 {
                deep += 1;
                assert_eq!(tight, 128.0, "128 m leaves");
                assert_eq!((id, false), level11_id(octree.root, cell_min));
            } else {
                assert!(i64::from(id) <= STOCK_LAST_ID);
                assert_eq!(stock_level(id, 64), Some(level));
            }
            assert_eq!(decode(&rig, id), level, "the decoder on {id:#x}");
        }
        assert!(deep > 2_000, "{deep} level-11 nodes");
        for (i, &leaf) in leaves.iter().enumerate() {
            // 2 m objects reach the 128 m leaves; 600 m ones stop where a
            // loose box holds them.
            let got = level(&parents, leaf);
            if i % 10 == 0 {
                assert!((7..=9).contains(&got), "object {i}: level {got}");
            } else {
                assert_eq!(got, 11, "object {i}");
            }
        }
        // Another game: other worlds first, then the same objects in the
        // reverse order. Every node box gets the same id.
        let by_box = |nodes: &[Node]| -> HashMap<[u32; 6], i32> {
            nodes.iter().map(|n| (n.3.map(f32::to_bits), n.2)).collect()
        };
        let first = by_box(&all);
        let octree = tree(12, 131_072.0);
        insert(&rig, &octree, &objects(9, 300));
        let octree = tree(12, 131_072.0);
        let mut reversed = boxes.clone();
        reversed.reverse();
        insert(&rig, &octree, &reversed);
        assert_eq!(by_box(&nodes()), first, "ids are a function of the node");

        // SAFETY: nothing runs the relocated code now.
        unsafe { installed.descent.detach() }.unwrap();
        unsafe { installed.decoder.detach() }.unwrap();
    }

    #[test]
    fn a_site_that_differs_installs_nothing() {
        let Some(exe) = Exe::load() else { return };
        let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        let rig = rig(&exe);
        for name in [DESCENT, DESCENT_START, DECODER] {
            let mut resolved = rig.resolved.clone();
            resolved
                .targets
                .iter_mut()
                .find(|t| t.name == name)
                .unwrap()
                .address += 1;
            assert!(install(&resolved).is_err(), "{name}");
        }
        // Nothing was left installed: depth 12's level 11 keeps stock ids.
        let octree = tree(12, 131_072.0);
        insert(&rig, &octree, &objects(1, 50));
        let parents: HashMap<i32, i32> = nodes().iter().map(|n| (n.0, n.1)).collect();
        assert!(
            nodes()
                .iter()
                .filter(|n| level(&parents, n.0) == 11)
                .all(
                    |n| level11_id(octree.root, [n.3[0] + 64.0, n.3[1] + 64.0, n.3[2] + 64.0]).0
                        != n.2
                )
        );
    }
}

//! The engine-copy checker (docs/HOOKS.md, "The engine-copy checker"): a
//! read-only diagnostic, off unless [`ENV`] is set.
//!
//! The game keeps two engines (`CGame+0x1f0`: `gameStates[2]` at `+0x78`,
//! `simIdx` at `+0x98`), simulates one a frame and copies it into the other
//! (`GameState::Replicate` `0x255de0`). The copy is not whole: the
//! platform-decision flag (`MovePath +0x70`) does not come across, and which
//! engine runs a room's step follows each game's frames, so the flag split
//! the room on `twomptest` (`crate::order::decision_sync`). The copy's
//! per-type functions (`GetReplicaCompCopyFns` in `GameState.cpp`) copy 19
//! component types; `MovePath`'s copies only its bytes `+0x4c..+0x74`, and
//! only of the components in the contiguous store, none of the paged ones.
//! Other fields may be left behind the same way.
//!
//! This checker says which. At the first update of a room's step on the
//! other engine than the update before (the seeds' `ecs::Engine::Update`
//! detour, on the simulation thread before any system of the update runs,
//! after `decision-sync`'s own copy), the engine about to simulate has just
//! received the copy of the one that simulated last, which nothing has
//! touched since. Both should hold the same world. For each transport
//! vehicle (the transport vehicle loop's node list) and the stations and
//! lines their `TransportVehicle`s name, it reads each component of
//! [`COMPONENTS`] in both engines through the game's own const getter and
//! compares them byte for byte, by dword:
//!
//! - three 8-byte words that are a vector (begin, end, capacity) in each
//!   copy, or null in one, are compared by the vector's length;
//! - an 8-byte word that holds a heap address in both copies is masked (the
//!   copies' heaps differ legitimately); a run of two or more such words
//!   starting there, a vector's begin and end, is compared by the distance
//!   between the first two instead;
//! - the bytes of [`MASKS`] are not compared: padding the game's own copy
//!   skips, and `MovePath`'s per-copy snapshot;
//! - a component one engine has and the other does not is a difference too;
//! - a component at one address in both engines (a shared page) is equal.
//!
//! Each difference is aggregated per component and offset, and said when it
//! is new, whenever the number of entities it differs in changes, and else
//! every [`LOG_EVERY_STEPS`] room steps (the [`WATCHED`] decision flag every
//! check it differs, first); one that stops differing is said once:
//!
//! ```text
//! copycheck: step S component C offset +0xNN differs in K of N vehicles (e.g. vehicle V (slot I): engine0=0x..., engine1=0x...; engine1 ran the update before)
//! copycheck: step S component C offset +0xNN now equal in all N vehicles
//! ```
//!
//! It writes nothing. Every read is checked readable first (the hook's
//! region cache); the getters are found by their code (the shape of
//! `0x52bbc0`, the const getter `decision-sync` uses, which reads the
//! engine's tables and never copies a page; the mutable getters, which copy
//! a shared page on write, have another shape and are never called); work
//! is bounded to [`MAX_PER_CHECK`] entities of each kind a check, in turn
//! through the list; a panic switches it off for the game.

#![allow(unsafe_code)]
// Elsewhere the seeds' detour is not installed, so the checker never runs.
#![cfg_attr(not(all(windows, target_arch = "x86_64")), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::image::Readable as Probe;
use crate::log;

/// `1` (or `on`) turns the checker on, at every change of engine; a number
/// above 1 checks every that many changes; unset, `0` or `off`, off.
pub const ENV: &str = "TPF3MP_PROBE_ENGINE_COPY";
/// Entities of each kind compared in one check, at most; a longer list is
/// taken in turn, this many a check.
pub const MAX_PER_CHECK: usize = 256;
/// A difference already said is said again at most this often (room steps).
pub const LOG_EVERY_STEPS: u64 = 500;
/// Fields always said first, and counted in every `alive` line: the
/// platform-decision flag.
pub const WATCHED: &[(&str, usize)] = &[("MovePath", 0x70)];
/// Difference lines one check says at most; the rest wait for later checks.
pub const MAX_LINES_PER_CHECK: usize = 64;
/// Checks between two `alive` lines.
pub const ALIVE_EVERY: u64 = 1 << 8;
/// `TransportVehicle`'s line and current station (the vehicle watcher's).
const TV_LINE: usize = 0xb8;
const TV_STATION: usize = 0xc0;

/// What [`ENV`] says: `None` off, `Some(n)` a check every n-th engine change.
pub fn every(value: Option<&str>) -> Option<u64> {
    match value.map(str::trim)? {
        "" | "0" | "off" => None,
        "on" => Some(1),
        n => n.parse::<u64>().ok().filter(|n| *n >= 1),
    }
}

/// Which entities a component is compared for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// The transport vehicle loop's vehicles: road vehicles, trains, ships
    /// and aircraft.
    Vehicle,
    /// The stations the vehicles' `TransportVehicle +0xc0` names.
    Station,
    /// The lines the vehicles' `TransportVehicle +0xb8` names.
    Line,
}

impl Kind {
    fn one(self) -> &'static str {
        match self {
            Self::Vehicle => "vehicle",
            Self::Station => "station",
            Self::Line => "line",
        }
    }
}

/// A component the checker compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
    /// Its name in `ecs::component`.
    pub name: &'static str,
    pub kind: Kind,
}

/// The components compared: the land vehicles' first (`MovePath`,
/// `TransportVehicle`, `LandVehicle`, `CarriageList`), then the ships' and
/// aircraft's, the stations' and the lines'.
pub const COMPONENTS: &[Component] = &[
    Component {
        name: "MovePath",
        kind: Kind::Vehicle,
    },
    Component {
        name: "TransportVehicle",
        kind: Kind::Vehicle,
    },
    Component {
        name: "LandVehicle",
        kind: Kind::Vehicle,
    },
    Component {
        name: "CarriageList",
        kind: Kind::Vehicle,
    },
    Component {
        name: "Ship",
        kind: Kind::Vehicle,
    },
    Component {
        name: "Aircraft",
        kind: Kind::Vehicle,
    },
    Component {
        name: "MovePathAircraft",
        kind: Kind::Vehicle,
    },
    Component {
        name: "Station",
        kind: Kind::Station,
    },
    Component {
        name: "Line",
        kind: Kind::Line,
    },
];

/// Why a byte range of a component is not compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// Bytes no field holds: the game's own copy and assignment of the
    /// component skip them, so each engine keeps whatever its allocation
    /// left there.
    Padding,
    /// A field each engine keeps for itself.
    Scratch,
}

/// A byte range `[start, end)` of a component of `size` bytes not compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mask {
    pub component: &'static str,
    pub size: usize,
    pub start: usize,
    pub end: usize,
    pub why: Why,
}

const fn pad(component: &'static str, size: usize, start: usize, end: usize) -> Mask {
    Mask {
        component,
        size,
        start,
        end,
        why: Why::Padding,
    }
}

/// The ranges not compared, from build 40408's own code (docs/HOOKS.md, "The
/// engine-copy checker"):
///
/// - padding: the bytes the game's replication skips in its field by field
///   moves and assignments (`ReplicaCompVec<MovePath>::vf4` `0x259ae0`,
///   `TransportVehicle`'s assignment `0x2018c0`, `CarriageList`'s
///   `0x1e72d0`, `Line`'s `0x1e7930`): byte fields' tails and the gaps
///   before 8-byte members;
/// - `MovePath +0x74..+0xa0`, scratch: a snapshot of `+0x4c..+0x74` that
///   `LandVehicleMoveSystem` (`0xabc974`) takes while `+0x9c` is clear, and
///   that flag; no simulation system was found reading it (the
///   interpolation's start, INFERRED), and its values in an engine are
///   that engine's own batches'.
///
/// A component of another size than the one listed is compared whole.
pub const MASKS: &[Mask] = &[
    pad("MovePath", 0xa0, 0x24, 0x28),
    pad("MovePath", 0xa0, 0x39, 0x3c),
    Mask {
        component: "MovePath",
        size: 0xa0,
        start: 0x74,
        end: 0xa0,
        why: Why::Scratch,
    },
    pad("TransportVehicle", 0x1e8, 0x04, 0x08),
    pad("TransportVehicle", 0x1e8, 0x54, 0x58),
    pad("TransportVehicle", 0x1e8, 0xa1, 0xa8),
    pad("TransportVehicle", 0x1e8, 0xad, 0xb0),
    pad("TransportVehicle", 0x1e8, 0xb5, 0xb8),
    pad("TransportVehicle", 0x1e8, 0xc9, 0xd0),
    pad("TransportVehicle", 0x1e8, 0x194, 0x198),
    pad("TransportVehicle", 0x1e8, 0x1b1, 0x1b4),
    pad("TransportVehicle", 0x1e8, 0x1bd, 0x1c0),
    pad("TransportVehicle", 0x1e8, 0x1c9, 0x1cc),
    pad("TransportVehicle", 0x1e8, 0x1e4, 0x1e8),
    pad("CarriageList", 0x20, 0x19, 0x20),
    pad("Line", 0x28, 0x21, 0x24),
];

/// The ranges of [`MASKS`] for `component` at `size` bytes.
pub fn masks_for(component: &str, size: usize) -> Vec<Mask> {
    MASKS
        .iter()
        .filter(|m| m.component == component && m.size == size)
        .copied()
        .collect()
}

/// The type descriptor's decorated name of `ecs::component::<name>`, as it
/// stands at the descriptor's `+0x10`.
pub fn decorated(name: &str) -> Vec<u8> {
    format!(".?AU{name}@component@ecs@@\0").into_bytes()
}

/// The const component getter's code up to its contiguous/paged branch:
/// `ecs::Engine`'s `Get<T>(Entity) const` as `0x52bbc0` (`MovePath`'s) is,
/// the same for every `T` but the type descriptor, the call and jump
/// displacements and the element size after the branch. The mutable
/// getters (`0x2832a0`) save other registers, and copy a shared page.
pub const GETTER_PATTERN: &str = "48 89 5C 24 10 48 89 74 24 18 57 48 83 EC 30 48 63 DA \
     48 8D 05 ?? ?? ?? ?? 48 8B F1 48 89 44 24 40 48 8D 54 24 20 48 83 C1 48 \
     4C 8D 44 24 40 E8 ?? ?? ?? ?? 48 8B 46 60 48 03 46 48 48 39 44 24 20 \
     0F 84 ?? ?? ?? ?? 48 8B 44 24 28 44 8B 40 08 41 83 E8 01 0F 88 ?? ?? ?? ?? \
     49 63 F8 48 8B CF 48 8B C7 48 C1 E8 06 83 E1 3F 48 8D 14 58 \
     48 8B 86 C0 00 00 00 48 8B 04 D0 48 0F A3 C8 73 ?? 8B D3 48 8B CE \
     E8 ?? ?? ?? ?? 48 8B 4E 78 4C 8B ?? F9 3D 00 00 00 40 7C ??";
/// Bytes read of a getter candidate: its pattern and its branch.
pub const GETTER_READ: usize = 0x120;

/// A pattern: each byte, or `None` for any.
pub fn pattern(text: &str) -> Vec<Option<u8>> {
    text.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).ok())
        .collect()
}

fn matches(code: &[u8], pattern: &[Option<u8>]) -> bool {
    code.len() >= pattern.len()
        && pattern
            .iter()
            .zip(code)
            .all(|(want, have)| want.is_none_or(|want| want == *have))
}

fn rel32(code: &[u8], at: usize, base: u64) -> Option<u64> {
    let rel = i32::from_le_bytes(code.get(at..at + 4)?.try_into().ok()?);
    Some(
        base.wrapping_add(at as u64 + 4)
            .wrapping_add_signed(i64::from(rel)),
    )
}

/// A const getter, as its code says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Getter {
    /// The type descriptor its `lea` names.
    pub descriptor: u64,
    /// The two calls it makes (the engine's type map's find, the entity's
    /// slot): the same in every getter.
    pub calls: [u64; 2],
    /// The component's size, from the contiguous branch's arithmetic.
    pub size: usize,
}

/// The getter at `at` whose code is `code` ([`GETTER_READ`] bytes), if it
/// is one.
pub fn getter(at: u64, code: &[u8], pattern: &[Option<u8>]) -> Option<Getter> {
    if !matches(code, pattern) {
        return None;
    }
    let jl = pattern.len();
    let disp = i8::from_le_bytes([*code.get(jl - 1)?]);
    let branch = jl.checked_add_signed(isize::from(disp))?;
    if branch < jl {
        return None;
    }
    Some(Getter {
        descriptor: rel32(code, 0x15, at)?,
        calls: [rel32(code, 0x30, at)?, rel32(code, 0x85, at)?],
        size: element_size(code.get(branch..)?)?,
    })
}

/// `lea r, [base + index * scale]`'s SIB byte with base and index both
/// `rax` (`r * k`): `k`.
fn times(sib: u8) -> Option<usize> {
    match sib {
        0x40 => Some(3),
        0x80 => Some(5),
        0xC0 => Some(9),
        _ => None,
    }
}

/// The element size the contiguous branch multiplies the slot by
/// (`cdqe`, then the size's arithmetic, then the add of the store's
/// begin), or `None` for a form not known.
pub fn element_size(branch: &[u8]) -> Option<usize> {
    const ADD: [u8; 4] = [0x49, 0x03, 0x40, 0x68]; // add rax, [r8+0x68]
    let rest = branch.strip_prefix(&[0x48, 0x98])?; // cdqe
    let ends = |tail: &[u8]| tail.starts_with(&ADD);
    let shl = |tail: &[u8]| match tail {
        [0x48, 0xC1, 0xE0, s, after @ ..] if *s < 16 && ends(after) => Some(1usize << s),
        _ => None,
    };
    match rest {
        _ if ends(rest) => Some(1),
        [0x48, 0x6B, 0xC0, n, after @ ..] if ends(after) && *n > 0 && *n < 0x80 => {
            Some(usize::from(*n))
        }
        [0x48, 0x69, 0xC0, a, b, c, d, after @ ..] if ends(after) => {
            let n = u32::from_le_bytes([*a, *b, *c, *d]);
            (1..0x10_000).contains(&n).then_some(n as usize)
        }
        [0x48, 0x8D, 0x04, sib, after @ ..] => {
            let k = times(*sib)?;
            if ends(after) {
                Some(k)
            } else {
                shl(after).map(|s| k * s)
            }
        }
        // lea rcx, [rax+rax*k]; mov rax, [r9+0x68]; lea rax, [rax+rcx*s]
        [
            0x48,
            0x8D,
            0x0C,
            sib,
            0x49,
            0x8B,
            0x41,
            0x68,
            0x48,
            0x8D,
            0x04,
            scale,
            ..,
        ] => {
            let k = times(*sib)?;
            let s = match scale {
                0x48 => 2,
                0x88 => 4,
                0xC8 => 8,
                _ => return None,
            };
            Some(k * s)
        }
        _ => shl(rest),
    }
}

/// Whether an 8-byte word looks like a heap address: a canonical user
/// address above 4 GiB, 4-aligned. In the game it must also be readable.
pub fn pointer_like(value: u64) -> bool {
    (0x1_0000_0000..0x8000_0000_0000).contains(&value) && value & 3 == 0
}

/// What differs at one offset of a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum What {
    /// The dword at the offset (its values).
    Value,
    /// The distance from the heap address at the offset to the one after
    /// it (a vector's length in bytes).
    Length,
    /// The component itself: one engine has it, the other not (1 or 0).
    Presence,
}

/// One field that differs between the copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldDiff {
    pub offset: usize,
    pub what: What,
    /// The value in engine 0, and in engine 1.
    pub a: u64,
    pub b: u64,
}

/// One component compared.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Compared {
    pub diffs: Vec<FieldDiff>,
    /// The 8-byte words masked as heap addresses, with whether their
    /// values differed.
    pub masked: Vec<(usize, bool)>,
}

/// Compares a component's two copies `a` and `b` (engine 0's and 1's): by
/// dword, except the words `pointer` says hold a heap address in both (only
/// when `words` says the component is laid out in 8-byte words), compared
/// as vector lengths where two or more begin a run, and the bytes `masks`
/// covers (a dword partly masked is compared by its other bytes).
pub fn compare(
    a: &[u8],
    b: &[u8],
    words: bool,
    masks: &[Mask],
    pointer: &mut dyn FnMut(u64) -> bool,
) -> Compared {
    let n = a.len().min(b.len());
    let mut out = Compared::default();
    let mut skip = vec![false; n];
    let word = |bytes: &[u8], at: usize| {
        u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or_default())
    };
    if words {
        let count = n / 8;
        // Vectors first: three words that are a vector's begin, end and
        // capacity in each copy, or all null (never allocated), and not
        // null in both: compared by length (an empty vector with a
        // capacity equals an unallocated one).
        let mut vector = vec![false; count];
        let mut i = 0;
        while i + 3 <= count {
            let at = i * 8;
            let mut length = |bytes: &[u8]| -> Option<Option<u64>> {
                let (b, e, c) = (word(bytes, at), word(bytes, at + 8), word(bytes, at + 16));
                if b == 0 && e == 0 && c == 0 {
                    return Some(None);
                }
                let shaped = pointer_like(b)
                    && pointer_like(e)
                    && pointer_like(c)
                    && b <= e
                    && e <= c
                    && c - b < 1 << 32;
                (shaped && pointer(b)).then(|| Some(e - b))
            };
            match (length(a), length(b)) {
                (Some(la), Some(lb)) if la.is_some() || lb.is_some() => {
                    let (la, lb) = (la.unwrap_or(0), lb.unwrap_or(0));
                    vector[i..i + 3].fill(true);
                    skip[at..at + 24].fill(true);
                    out.masked
                        .push((at, word(a, at) != word(b, at) || la != lb));
                    if la != lb {
                        out.diffs.push(FieldDiff {
                            offset: at,
                            what: What::Length,
                            a: la,
                            b: lb,
                        });
                    }
                    i += 3;
                }
                _ => i += 1,
            }
        }
        let heap: Vec<bool> = (0..count)
            .map(|i| {
                let (x, y) = (word(a, i * 8), word(b, i * 8));
                !vector[i] && pointer_like(x) && pointer_like(y) && pointer(x) && pointer(y)
            })
            .collect();
        for i in 0..count {
            if !heap[i] {
                continue;
            }
            let at = i * 8;
            let (x, y) = (word(a, at), word(b, at));
            skip[at..at + 8].fill(true);
            out.masked.push((at, x != y));
            let starts_run = i == 0 || !heap[i - 1];
            if starts_run && i + 1 < count && heap[i + 1] {
                let la = word(a, at + 8).wrapping_sub(x);
                let lb = word(b, at + 8).wrapping_sub(y);
                if la != lb {
                    out.diffs.push(FieldDiff {
                        offset: at,
                        what: What::Length,
                        a: la,
                        b: lb,
                    });
                }
            }
        }
    }
    for m in masks {
        if m.start < n {
            skip[m.start..m.end.min(n)].fill(true);
        }
    }
    let dword = |bytes: &[u8], at: usize| {
        let mut four = [0u8; 4];
        let end = (at + 4).min(n);
        four[..end - at].copy_from_slice(&bytes[at..end]);
        u64::from(u32::from_le_bytes(four))
    };
    let mut at = 0;
    while at < n {
        let end = (at + 4).min(n);
        if (at..end).any(|i| !skip[i] && a[i] != b[i]) {
            out.diffs.push(FieldDiff {
                offset: at,
                what: What::Value,
                a: dword(a, at),
                b: dword(b, at),
            });
        }
        at = end;
    }
    out.diffs.sort_by_key(|d| (d.offset, d.what));
    out
}

/// Where a component stands in an engine: in the contiguous store at a slot,
/// in a page, or not known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Slot(usize),
    Paged,
    Unknown,
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Slot(i) => write!(f, "slot {i}"),
            Self::Paged => write!(f, "paged"),
            Self::Unknown => write!(f, "store unknown"),
        }
    }
}

/// An example of a difference: the entity, its place, the two values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Example {
    pub entity: i32,
    pub place: Place,
    pub a: u64,
    pub b: u64,
}

/// A difference's key: component (index into [`COMPONENTS`]), offset, what.
pub type Key = (usize, usize, What);

/// One check's differences.
#[derive(Debug, Clone, Default)]
pub struct Tally {
    /// Per key: the entities that differ there, and the first.
    pub diffs: BTreeMap<Key, (u64, Example)>,
    /// Per component: how many entities were compared.
    pub compared: Vec<u64>,
    /// Per component: compared at one address in both engines.
    pub shared: Vec<u64>,
    /// Per component: the masked words, with how many differed.
    pub masked: Vec<BTreeMap<usize, (u64, u64)>>,
    /// Per component: an example of a masked word that differed.
    pub masked_example: Vec<Option<(usize, Example)>>,
    /// Per component: entities in the contiguous store, paged, unknown.
    pub places: Vec<[u64; 3]>,
    /// Per component: its size in bytes (0 when not known).
    pub sizes: Vec<usize>,
}

impl Tally {
    pub fn new(components: usize) -> Self {
        Self {
            diffs: BTreeMap::new(),
            compared: vec![0; components],
            shared: vec![0; components],
            masked: vec![BTreeMap::new(); components],
            masked_example: vec![None; components],
            places: vec![[0; 3]; components],
            sizes: vec![0; components],
        }
    }

    /// One entity's component, compared.
    pub fn add(&mut self, component: usize, entity: i32, place: Place, compared: &Compared) {
        self.compared[component] += 1;
        self.places[component][match place {
            Place::Slot(_) => 0,
            Place::Paged => 1,
            Place::Unknown => 2,
        }] += 1;
        for d in &compared.diffs {
            let entry = self.diffs.entry((component, d.offset, d.what)).or_insert((
                0,
                Example {
                    entity,
                    place,
                    a: d.a,
                    b: d.b,
                },
            ));
            entry.0 += 1;
        }
        for &(at, differs) in &compared.masked {
            let entry = self.masked[component].entry(at).or_insert((0, 0));
            entry.0 += 1;
            if differs {
                entry.1 += 1;
            }
        }
    }

    /// A component one engine has and the other not.
    pub fn presence(&mut self, component: usize, entity: i32, a: bool, b: bool) {
        self.compared[component] += 1;
        let entry = self.diffs.entry((component, 0, What::Presence)).or_insert((
            0,
            Example {
                entity,
                place: Place::Unknown,
                a: u64::from(a),
                b: u64::from(b),
            },
        ));
        entry.0 += 1;
    }

    /// One entity's component at one address in both engines.
    pub fn same(&mut self, component: usize) {
        self.compared[component] += 1;
        self.shared[component] += 1;
    }

    /// Fields that differed, in all.
    pub fn fields(&self) -> usize {
        self.diffs.len()
    }
}

/// What the log has been told: per key, the room step it was last said,
/// and the masked words said per component.
#[derive(Debug, Default)]
pub struct Reporter {
    said: BTreeMap<Key, u64>,
    /// Per key said: how many entities differed when it was last said.
    counts: BTreeMap<Key, u64>,
    masked_said: Vec<BTreeSet<usize>>,
    layout_said: Vec<bool>,
}

fn offset_text(offset: usize, what: What) -> String {
    match what {
        What::Value => format!("offset +{offset:#04x}"),
        What::Length => format!("offset +{offset:#04x} (vector length, bytes)"),
        What::Presence => "presence".into(),
    }
}

impl Reporter {
    /// The lines one check says: the differences due (new, or last said
    /// [`LOG_EVERY_STEPS`] or more steps ago), at most
    /// [`MAX_LINES_PER_CHECK`]; then, once per component, its layout and
    /// the words masked as heap addresses (and words masked later, once).
    /// `ran` is the engine (0 or 1) that ran the update before.
    pub fn lines(&mut self, step: u64, ran: u8, tally: &Tally) -> Vec<String> {
        let components = tally.compared.len();
        self.masked_said.resize_with(components, BTreeSet::new);
        self.layout_said.resize(components, false);
        let mut lines = Vec::new();
        let mut held = 0usize;
        // Due: a watched field, a new one, one whose count changed, one last
        // said LOG_EVERY_STEPS ago; in that order of priority.
        let mut due: Vec<(u8, Key, u64, Example)> = Vec::new();
        for (&key, &(count, ex)) in &tally.diffs {
            let (component, offset, what) = key;
            let watched = what == What::Value
                && COMPONENTS
                    .get(component)
                    .is_some_and(|c| WATCHED.contains(&(c.name, offset)));
            let rank = match (self.said.get(&key), self.counts.get(&key)) {
                _ if watched => 0,
                (None, _) => 1,
                (Some(_), last) if last != Some(&count) => 2,
                (Some(&at), _) if step >= at.saturating_add(LOG_EVERY_STEPS) => 3,
                _ => continue,
            };
            due.push((rank, key, count, ex));
        }
        due.sort_by_key(|(rank, key, _, _)| (*rank, *key));
        for (_, key, count, ex) in due {
            if lines.len() >= MAX_LINES_PER_CHECK {
                held += 1;
                continue;
            }
            let (component, offset, what) = key;
            self.said.insert(key, step);
            self.counts.insert(key, count);
            let c = COMPONENTS.get(component);
            let (name, kind) = c.map_or(("?", Kind::Vehicle), |c| (c.name, c.kind));
            lines.push(format!(
                "copycheck: step {step} component {name} {} differs in {count} of {} {}s (e.g. {} {} ({}): engine0={:#x}, engine1={:#x}; engine{ran} ran the update before)",
                offset_text(offset, what),
                tally.compared[component],
                kind.one(),
                kind.one(),
                ex.entity,
                ex.place,
                ex.a,
                ex.b,
            ));
        }
        // Fields said differing that no longer do, in a check that compared
        // their component.
        let gone: Vec<Key> = self
            .counts
            .keys()
            .filter(|key| {
                !tally.diffs.contains_key(key) && tally.compared.get(key.0).is_some_and(|n| *n > 0)
            })
            .copied()
            .collect();
        for key in gone {
            self.counts.remove(&key);
            self.said.remove(&key);
            let (component, offset, what) = key;
            let c = COMPONENTS.get(component);
            let (name, kind) = c.map_or(("?", Kind::Vehicle), |c| (c.name, c.kind));
            lines.push(format!(
                "copycheck: step {step} component {name} {} now equal in all {} {}s",
                offset_text(offset, what),
                tally.compared[component],
                kind.one(),
            ));
        }
        if held > 0 {
            lines.push(format!(
                "copycheck: step {step}: {held} more differing field(s) held for later checks (at most {MAX_LINES_PER_CHECK} lines a check)"
            ));
        }
        for component in 0..components {
            if tally.compared[component] == 0 {
                continue;
            }
            let c = COMPONENTS.get(component);
            let (name, kind) = c.map_or(("?", Kind::Vehicle), |c| (c.name, c.kind));
            if !self.layout_said[component] {
                self.layout_said[component] = true;
                let [slot, paged, unknown] = tally.places[component];
                let size = tally.sizes.get(component).copied().unwrap_or(0);
                let ranges = |why: Why| {
                    let r: Vec<String> = masks_for(name, size)
                        .iter()
                        .filter(|m| m.why == why)
                        .map(|m| format!("+{:#04x}..+{:#04x}", m.start, m.end))
                        .collect();
                    if r.is_empty() {
                        "none".to_string()
                    } else {
                        r.join(" ")
                    }
                };
                lines.push(format!(
                    "copycheck: component {name}: {} {}s compared at step {step}: {} at one address in both engines, {slot} in the contiguous store, {paged} paged, {unknown} store unknown; not compared: padding {} (bytes the game's own copy skips), scratch {}",
                    tally.compared[component],
                    kind.one(),
                    tally.shared[component],
                    ranges(Why::Padding),
                    ranges(Why::Scratch),
                ));
            }
            let new: Vec<(usize, (u64, u64))> = tally.masked[component]
                .iter()
                .filter(|(at, _)| !self.masked_said[component].contains(at))
                .map(|(&at, &n)| (at, n))
                .collect();
            if new.is_empty() {
                continue;
            }
            let first = self.masked_said[component].is_empty();
            let words: Vec<String> = new
                .iter()
                .map(|(at, (n, differ))| format!("+{at:#04x} ({n}, {differ} differing)"))
                .collect();
            let example = tally.masked_example[component]
                .filter(|(at, _)| new.iter().any(|(n, _)| n == at))
                .map(|(at, ex)| {
                    format!(
                        "; e.g. {} {} +{at:#04x}: engine0={:#x}, engine1={:#x}",
                        kind.one(),
                        ex.entity,
                        ex.a,
                        ex.b
                    )
                })
                .unwrap_or_default();
            lines.push(format!(
                "copycheck: component {name}: {} as heap addresses, not compared (a readable address above 4 GiB in both copies, which each engine's own heap holds; a run of them is compared by the distance between its first two): {}{example}",
                if first { "masks" } else { "also masks" },
                words.join(" "),
            ));
            self.masked_said[component].extend(new.iter().map(|(at, _)| *at));
        }
        lines
    }
}

/// The next window of at most `max` of `list`, from `cursor`, which moves
/// on past it.
pub fn window<T: Copy>(list: &[T], max: usize, cursor: &mut usize) -> Vec<T> {
    if list.len() <= max {
        *cursor = 0;
        return list.to_vec();
    }
    let start = *cursor % list.len();
    *cursor = (start + max) % list.len();
    list.iter().cycle().skip(start).take(max).copied().collect()
}

// ---------- in the game ----------

type GetterFn = unsafe extern "system" fn(usize, i32) -> usize;

struct State {
    every: u64,
    getters: Vec<Found>,
    /// The engine the last room's update ran on.
    last: usize,
    /// The two engines, numbered in the order met (since the last load).
    engines: [usize; 2],
    changes: u64,
    checks: u64,
    vehicles: Vec<i32>,
    cursors: [usize; 3],
    reporter: Reporter,
    nanos: u64,
    max_nanos: u64,
}

static ON: AtomicBool = AtomicBool::new(false);
static BROKEN: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<State> = Mutex::new(State {
    every: 1,
    getters: Vec::new(),
    last: 0,
    engines: [0; 2],
    changes: 0,
    checks: 0,
    vehicles: Vec::new(),
    cursors: [0; 3],
    reporter: Reporter {
        said: BTreeMap::new(),
        counts: BTreeMap::new(),
        masked_said: Vec::new(),
        layout_said: Vec::new(),
    },
    nanos: 0,
    max_nanos: 0,
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Code of the game's image, `len` bytes at `at`, if readable.
fn code(at: usize, len: usize) -> Option<&'static [u8]> {
    if !crate::image::readable(at, len) {
        return None;
    }
    // SAFETY: `len` readable bytes of the game's mapped image, only read.
    Some(unsafe { std::slice::from_raw_parts(at as *const u8, len) })
}

/// The game's `.text` in memory: address and length.
fn text(base: usize) -> Option<(usize, usize)> {
    let header = code(base, 0x1000)?;
    let pe = tpf3mp_hookcore::pe::PeHeaders::parse(header).ok()?;
    let text = pe.section(".text")?;
    Some((
        base.checked_add(text.virtual_address as usize)?,
        text.virtual_size as usize,
    ))
}

/// A component's getter found in the code: its address and element size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Found {
    /// Index into [`COMPONENTS`].
    pub component: usize,
    pub address: usize,
    pub size: usize,
}

/// Finds each component's const getter in `text` (at `text_at`): every
/// match of the pattern whose type descriptor's name (`name_at(address,
/// len)`) is a wanted component's, the first for each, all with the same
/// two calls.
pub fn scan(
    text: &[u8],
    text_at: usize,
    name_at: &dyn Fn(u64, usize) -> Option<Vec<u8>>,
) -> Vec<Found> {
    let pattern = pattern(GETTER_PATTERN);
    let prefix: Vec<u8> = pattern.iter().take(0x15).map_while(|b| *b).collect();
    let names: Vec<Vec<u8>> = COMPONENTS.iter().map(|c| decorated(c.name)).collect();
    let mut found: Vec<Found> = Vec::new();
    let mut calls: Option<[u64; 2]> = None;
    let mut at = 0;
    while let Some(i) = text[at..]
        .windows(prefix.len())
        .position(|w| w == prefix.as_slice())
    {
        let start = at + i;
        at = start + 1;
        let Some(window) = text.get(start..start + GETTER_READ) else {
            break;
        };
        let address = text_at + start;
        let Some(g) = getter(address as u64, window, &pattern) else {
            continue;
        };
        let Some(component) = names.iter().position(|name| {
            name_at(g.descriptor + 0x10, name.len()).is_some_and(|bytes| bytes == *name)
        }) else {
            continue;
        };
        if found.iter().any(|f| f.component == component) {
            continue;
        }
        if *calls.get_or_insert(g.calls) != g.calls {
            continue;
        }
        found.push(Found {
            component,
            address,
            size: g.size,
        });
    }
    found.sort_by_key(|f| f.component);
    found
}

/// Turns the checker on if [`ENV`] says so (`base` the game's image), and
/// says what it found.
pub fn install(base: usize) -> String {
    ON.store(false, Ordering::Release);
    let Some(every) = every(std::env::var(ENV).ok().as_deref()) else {
        return format!("copycheck: the two engine copies are not compared ({ENV}=1 turns it on)");
    };
    let Some((at, len)) = text(base) else {
        return "copycheck: off, the game's .text is not readable; nothing is read".into();
    };
    let Some(text) = code(at, len) else {
        return "copycheck: off, the game's .text is not readable; nothing is read".into();
    };
    let started = Instant::now();
    let name_at = |at: u64, len: usize| {
        usize::try_from(at)
            .ok()
            .and_then(|at| code(at, len))
            .map(<[u8]>::to_vec)
    };
    let getters = scan(text, at, &name_at);
    let ms = started.elapsed().as_millis();
    if !getters
        .iter()
        .any(|f| COMPONENTS[f.component].name == "MovePath")
    {
        return "copycheck: off, MovePath's const getter is not in the code; nothing is read"
            .into();
    }
    let listed: Vec<String> = getters
        .iter()
        .map(|f| {
            format!(
                "{} ({:#x} bytes, getter +{:#x})",
                COMPONENTS[f.component].name,
                f.size,
                f.address - base
            )
        })
        .collect();
    let missing: Vec<&str> = COMPONENTS
        .iter()
        .enumerate()
        .filter(|(i, _)| !getters.iter().any(|f| f.component == *i))
        .map(|(_, c)| c.name)
        .collect();
    let mut s = state();
    s.every = every;
    s.getters = getters;
    s.last = 0;
    s.engines = [0; 2];
    drop(s);
    ON.store(true, Ordering::Release);
    format!(
        "copycheck: on, read only: the two engine copies compared at the first update of a room's step on the other engine{} (after decision-sync's copy), at most {MAX_PER_CHECK} entities of each kind a check; components {}; not found: {}; getters found in {ms} ms",
        if every > 1 {
            format!(", every {every} changes of engine")
        } else {
            String::new()
        },
        listed.join(", "),
        if missing.is_empty() {
            "none".to_string()
        } else {
            missing.join(", ")
        },
    )
}

/// A world was loaded: its engines start from it, numbered afresh.
pub fn reset() {
    if !ON.load(Ordering::Acquire) {
        return;
    }
    let mut s = state();
    s.last = 0;
    s.engines = [0; 2];
    s.vehicles.clear();
}

/// At the transport vehicle loop's first record: the vehicles.
pub fn note_list(rbp: u64, begin: u64) {
    if !ON.load(Ordering::Acquire) || BROKEN.load(Ordering::Relaxed) || !crate::order::in_step() {
        return;
    }
    if let Some(vehicles) = crate::order::decision_sync::read_list(rbp, begin) {
        state().vehicles = vehicles;
    }
}

/// Before an update of `engine`, inside the room's step, after
/// `decision-sync`: compares the engines when the update before ran on the
/// other one.
pub fn before_update(engine: usize) {
    if !ON.load(Ordering::Acquire) || BROKEN.load(Ordering::Relaxed) || engine == 0 {
        return;
    }
    if !crate::order::in_step() {
        return;
    }
    let Some(step) = crate::seeds::current_step() else {
        return;
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut s = state();
        let before = std::mem::replace(&mut s.last, engine);
        if before == 0 || before == engine {
            return;
        }
        s.changes += 1;
        if !s.changes.is_multiple_of(s.every) {
            return;
        }
        check(&mut s, step, before, engine);
    }));
    if result.is_err() {
        BROKEN.store(true, Ordering::Release);
        log::line("copycheck: panicked on the game's thread; switched off for this game");
    }
}

/// The number (0 or 1) of `engine`, numbering it if new.
fn number(engines: &mut [usize; 2], engine: usize) -> u8 {
    if let Some(i) = engines.iter().position(|e| *e == engine) {
        return i as u8;
    }
    if let Some(i) = engines.iter().position(|e| *e == 0) {
        engines[i] = engine;
        return i as u8;
    }
    // A third engine without a load: start the numbering again.
    *engines = [engine, 0];
    0
}

/// Whether `entity` is within `engine`'s tables, as the getter reads them
/// without a bound: its components list (`GetComponentDataIndex`
/// `0xa4b90`: 24-byte vectors from `[engine+0x90]`, the vector's end at
/// `+0x98`) and its two words of the has-component bits (from
/// `[engine+0xc0]`). An entity outside them is not asked for.
pub fn entity_in_tables(
    entity: i32,
    lists: (u64, u64),
    bits: u64,
    readable: &mut dyn FnMut(u64, usize) -> bool,
) -> bool {
    let Ok(e) = u64::try_from(entity) else {
        return false;
    };
    let (begin, end) = lists;
    if end <= begin || (end - begin) % 24 != 0 || e >= (end - begin) / 24 {
        return false;
    }
    readable(begin + e * 24, 24) && bits != 0 && readable(bits + e * 16, 16)
}

fn entity_ok(probe: &mut Probe, engine: usize, entity: i32) -> bool {
    let engine = engine as u64;
    let (Some(begin), Some(end), Some(bits)) = (
        probe.read::<u64>(engine + 0x90),
        probe.read::<u64>(engine + 0x98),
        probe.read::<u64>(engine + 0xc0),
    ) else {
        return false;
    };
    entity_in_tables(entity, (begin, end), bits, &mut |at, len| {
        usize::try_from(at).is_ok_and(|at| probe.readable(at, len))
    })
}

/// The contiguous store's `[begin, end)` of the component at `ptr` in
/// `engine`: the engine's stores are a vector at `+0x78`, each with its
/// contiguous elements at `+0x68..+0x70` (the getter's and the copy's own
/// reads).
fn store_of(probe: &mut Probe, engine: usize, ptr: usize) -> Option<(usize, usize)> {
    let begin = probe.read::<u64>(engine as u64 + 0x78)?;
    let end = probe.read::<u64>(engine as u64 + 0x80)?;
    if end < begin || (end - begin) % 8 != 0 || (end - begin) / 8 > 4096 {
        return None;
    }
    let ptr = ptr as u64;
    (0..(end - begin) / 8).find_map(|i| {
        let store = probe.read::<u64>(begin + i * 8)?;
        if store == 0 {
            return None;
        }
        let b = probe.read::<u64>(store + 0x68)?;
        let e = probe.read::<u64>(store + 0x70)?;
        (b <= ptr && ptr < e).then_some((b as usize, e as usize))
    })
}

/// The watched fields' counts in one check, for the `alive` line.
pub fn watched_text(tally: &Tally) -> String {
    WATCHED
        .iter()
        .filter_map(|&(name, offset)| {
            let component = COMPONENTS.iter().position(|c| c.name == name)?;
            let n = *tally.compared.get(component)?;
            let k = tally
                .diffs
                .get(&(component, offset, What::Value))
                .map_or(0, |d| d.0);
            Some(format!(", {name} +{offset:#04x} differs in {k} of {n}"))
        })
        .collect()
}

fn check(s: &mut State, step: u64, before: usize, engine: usize) {
    let started = Instant::now();
    let n_before = number(&mut s.engines, before);
    let n_engine = number(&mut s.engines, engine);
    if n_before == n_engine {
        return;
    }
    // Engine 0's and engine 1's, in that order.
    let (e0, e1) = if n_before == 0 {
        (before, engine)
    } else {
        (engine, before)
    };
    let mut probe = Probe::new();
    let getters = s.getters.clone();
    let call = |getter: usize, engine: usize, entity: i32| -> usize {
        // SAFETY: a const getter of the game's (its code checked at
        // install): it reads the engine's component tables and answers a
        // pointer or null, writing nothing; both engines live while the
        // room's world does (a load resets `last`), and no system of either
        // runs now (the start of an update on the simulation thread).
        unsafe { std::mem::transmute::<usize, GetterFn>(getter)(engine, entity) }
    };
    let vehicles = window(&s.vehicles, MAX_PER_CHECK, &mut s.cursors[0]);
    // The stations and lines the vehicles name, from the engine that ran.
    let (mut stations, mut lines) = (BTreeSet::new(), BTreeSet::new());
    if let Some(tv) = getters
        .iter()
        .find(|f| COMPONENTS[f.component].name == "TransportVehicle")
    {
        for &v in &vehicles {
            if !entity_ok(&mut probe, before, v) {
                continue;
            }
            let p = call(tv.address, before, v);
            if p == 0 || !probe.readable(p, tv.size.max(TV_STATION + 4)) {
                continue;
            }
            for (set, at) in [(&mut lines, TV_LINE), (&mut stations, TV_STATION)] {
                if let Some(id) = probe.read::<i32>((p + at) as u64)
                    && id >= 0
                {
                    set.insert(id);
                }
            }
        }
    }
    let stations: Vec<i32> = stations.into_iter().collect();
    let lines: Vec<i32> = lines.into_iter().collect();
    let stations = window(&stations, MAX_PER_CHECK, &mut s.cursors[1]);
    let lines = window(&lines, MAX_PER_CHECK, &mut s.cursors[2]);
    let mut tally = Tally::new(COMPONENTS.len());
    let mut pages: BTreeMap<u64, bool> = BTreeMap::new();
    for f in &getters {
        let c = COMPONENTS[f.component];
        let entities = match c.kind {
            Kind::Vehicle => &vehicles,
            Kind::Station => &stations,
            Kind::Line => &lines,
        };
        let masks = masks_for(c.name, f.size);
        tally.sizes[f.component] = f.size;
        let mut store: Option<(usize, usize)> = None;
        let mut store_tries = 0;
        for &entity in entities {
            if !entity_ok(&mut probe, e0, entity) || !entity_ok(&mut probe, e1, entity) {
                continue;
            }
            let (pa, pb) = (call(f.address, e0, entity), call(f.address, e1, entity));
            if pa == 0 && pb == 0 {
                continue;
            }
            if pa == 0 || pb == 0 {
                tally.presence(f.component, entity, pa != 0, pb != 0);
                continue;
            }
            if pa == pb {
                tally.same(f.component);
                continue;
            }
            if !probe.readable(pa, f.size) || !probe.readable(pb, f.size) {
                continue;
            }
            // Where engine 0's copy stands (its store found once a check).
            if store.is_none() && store_tries < 4 {
                store_tries += 1;
                store = store_of(&mut probe, e0, pa);
            }
            let place = match store {
                Some((b, e)) if b <= pa && pa < e => Place::Slot((pa - b) / f.size.max(1)),
                Some(_) => Place::Paged,
                None => Place::Unknown,
            };
            // SAFETY: `f.size` readable bytes at each, checked just above;
            // only read, while nothing writes either (as for the getter).
            let (a, b) = unsafe {
                (
                    std::slice::from_raw_parts(pa as *const u8, f.size),
                    std::slice::from_raw_parts(pb as *const u8, f.size),
                )
            };
            // Readable or not, by 64 KiB page, asked once a check (a
            // system call each in Sandboxie).
            let mut heap = |v: u64| {
                *pages
                    .entry(v >> 16)
                    .or_insert_with(|| usize::try_from(v).is_ok_and(|v| probe.readable(v, 1)))
            };
            let compared = compare(a, b, f.size % 8 == 0, &masks, &mut heap);
            if tally.masked_example[f.component].is_none()
                && let Some(&(at, _)) = compared.masked.iter().find(|(_, differs)| *differs)
            {
                let word = |bytes: &[u8]| {
                    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or([0; 8]))
                };
                tally.masked_example[f.component] = Some((
                    at,
                    Example {
                        entity,
                        place,
                        a: word(a),
                        b: word(b),
                    },
                ));
            }
            tally.add(f.component, entity, place, &compared);
        }
    }
    let mut out = s.reporter.lines(step, n_before, &tally);
    s.checks += 1;
    let nanos = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
    s.nanos = s.nanos.saturating_add(nanos);
    s.max_nanos = s.max_nanos.max(nanos);
    if s.checks == 1 || s.checks.is_multiple_of(ALIVE_EVERY) {
        out.push(format!(
            "copycheck: alive, checks={} engine changes={} at step {step}: {} vehicles, {} stations, {} lines compared, {} differing field(s){}; {} us a check on average, {} us at most",
            s.checks,
            s.changes,
            vehicles.len(),
            stations.len(),
            lines.len(),
            tally.fields(),
            watched_text(&tally),
            s.nanos / s.checks / 1000,
            s.max_nanos / 1000,
        ));
    }
    for line in out {
        log::line(&line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        text.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    #[test]
    fn the_switch_reads_on_a_number_or_off() {
        assert_eq!(every(None), None);
        assert_eq!(every(Some("0")), None);
        assert_eq!(every(Some("off")), None);
        assert_eq!(every(Some("1")), Some(1));
        assert_eq!(every(Some("on")), Some(1));
        assert_eq!(every(Some(" 8 ")), Some(8));
        assert_eq!(every(Some("x")), None);
    }

    #[test]
    fn the_element_size_comes_from_the_branchs_arithmetic() {
        // The contiguous branches of build 40408's const getters.
        let cases = [
            ("48 98 48 8D 04 80 48 C1 E0 05 49 03 40 68", 0xa0), // MovePath
            ("48 98 48 69 C0 E8 01 00 00 49 03 40 68", 0x1e8),   // TransportVehicle
            ("48 98 48 8D 0C 40 49 8B 41 68 48 8D 04 88", 12),   // LandVehicle, Ship
            ("48 98 48 C1 E0 05 49 03 40 68", 0x20),             // CarriageList
            ("48 98 48 C1 E0 04 49 03 40 68", 0x10),             // Aircraft
            ("48 98 48 69 C0 38 02 00 00 49 03 40 68", 0x238),   // MovePathAircraft
            ("48 98 48 6B C0 58 49 03 40 68", 0x58),             // Station
            ("48 98 48 8D 0C 80 49 8B 41 68 48 8D 04 C8", 40),   // Line
            ("48 98 48 6B C0 78 49 03 40 68", 0x78),             // StockList
            ("48 98 49 03 40 68", 1),                            // Carriage
        ];
        for (bytes, size) in cases {
            assert_eq!(element_size(&hex(bytes)), Some(size), "{bytes}");
        }
        // Anything else is not a size.
        assert_eq!(element_size(&hex("48 98 48 8D 04 81 49 03 40 68")), None);
        assert_eq!(element_size(&hex("90 48 C1 E0 05 49 03 40 68")), None);
        assert_eq!(element_size(&hex("48 98 48 C1 E0 05 49 03 41 68")), None);
    }

    /// MovePath's const getter at 0x52bbc0 in build 40408, its first
    /// `GETTER_READ` bytes.
    fn movepath_getter() -> Vec<u8> {
        let mut code = hex(
            "48 89 5c 24 10 48 89 74 24 18 57 48 83 ec 30 48 63 da 48 8d 05 5f ee 7b 03 \
             48 8b f1 48 89 44 24 40 48 8d 54 24 20 48 83 c1 48 4c 8d 44 24 40 e8 7c dd b6 ff \
             48 8b 46 60 48 03 46 48 48 39 44 24 20 0f 84 af 00 00 00 48 8b 44 24 28 44 8b 40 08 \
             41 83 e8 01 0f 88 9c 00 00 00 49 63 f8 48 8b cf 48 8b c7 48 c1 e8 06 83 e1 3f \
             48 8d 14 58 48 8b 86 c0 00 00 00 48 8b 04 d0 48 0f a3 c8 73 77 8b d3 48 8b ce \
             e8 47 8f b7 ff 48 8b 4e 78 4c 8b 04 f9 3d 00 00 00 40 7c 40",
        );
        code.resize(0x98 + 0x40, 0xCC);
        code.extend(hex("48 98 48 8D 04 80 48 C1 E0 05 49 03 40 68"));
        code.resize(GETTER_READ, 0xCC);
        code
    }

    #[test]
    fn a_const_getter_names_its_type_its_calls_and_its_size() {
        let pattern = pattern(GETTER_PATTERN);
        assert_eq!(pattern.len(), 0x98);
        let at = 0x1_4052_bbc0;
        let g = getter(at, &movepath_getter(), &pattern).expect("a getter");
        // The type descriptor 0x143ceaa38, sub_99970 and sub_a4b90.
        assert_eq!(g.descriptor, 0x1_43ce_aa38);
        assert_eq!(g.calls, [0x1_4009_9970, 0x1_400a_4b90]);
        assert_eq!(g.size, 0xa0);
    }

    #[test]
    fn a_mutable_getter_or_other_code_is_not_taken() {
        let pattern = pattern(GETTER_PATTERN);
        // The mutable getter's prologue (0x2832a0) saves rbp, not rsi.
        let mut code = movepath_getter();
        code[..11].copy_from_slice(&hex("48 89 5C 24 18 48 89 6C 24 20 56"));
        assert_eq!(getter(0x1000, &code, &pattern), None);
        // An unknown size: refused.
        let mut code = movepath_getter();
        code[0x98 + 0x40 + 2] = 0x90;
        assert_eq!(getter(0x1000, &code, &pattern), None);
        // A branch backwards: refused.
        let mut code = movepath_getter();
        code[0x97] = 0xF0;
        assert_eq!(getter(0x1000, &code, &pattern), None);
        // Too short.
        assert_eq!(getter(0x1000, &movepath_getter()[..0x90], &pattern), None);
    }

    fn words(values: &[u64]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    #[test]
    fn heap_addresses_are_masked_and_a_run_compared_by_length() {
        // +0x00 a vector (begin, end, capacity), +0x18 a value, +0x20 a
        // lone address, +0x28 a flag.
        let a = words(&[
            0x2_0000_1000,
            0x2_0000_1040,
            0x2_0000_1080,
            7,
            0x2_0000_9000,
            0x0000_0001_0000_0000 | 1,
        ]);
        let b = words(&[
            0x3_0000_5000,
            0x3_0000_5040,
            0x3_0000_5100,
            7,
            0x3_0000_7000,
            0x0000_0001_0000_0000,
        ]);
        let mut heap = |_: u64| true;
        let c = compare(&a, &b, true, &[], &mut heap);
        // Begin, end, capacity and the lone address are masked; the
        // lengths agree (0x40 each) though the capacities do not.
        // The vector is masked as one, at its begin.
        assert_eq!(c.masked, vec![(0, true), (0x20, true)]);
        assert_eq!(
            c.diffs,
            vec![FieldDiff {
                offset: 0x28,
                what: What::Value,
                a: 1,
                b: 0
            }]
        );
        // A vector whose length differs is said at its begin.
        let mut b2 = b.clone();
        b2[8..16].copy_from_slice(&0x3_0000_5048u64.to_le_bytes());
        let c = compare(&a, &b2, true, &[], &mut heap);
        assert!(c.diffs.contains(&FieldDiff {
            offset: 0,
            what: What::Length,
            a: 0x40,
            b: 0x48
        }));
    }

    #[test]
    fn a_word_masked_needs_an_address_in_both_copies_that_is_readable() {
        // An address in one copy, null in the other: a difference, not a mask.
        let a = words(&[0x2_0000_1000, 0]);
        let b = words(&[0, 0]);
        let mut heap = |_: u64| true;
        let c = compare(&a, &b, true, &[], &mut heap);
        assert!(c.masked.is_empty());
        // Both dwords of the word differ.
        assert_eq!(c.diffs.len(), 2);
        assert_eq!((c.diffs[0].offset, c.diffs[0].a), (0, 0x1000));
        assert_eq!((c.diffs[1].offset, c.diffs[1].a), (4, 2));
        // Pointer-like but unreadable (two ints, say): compared as values.
        let a = words(&[0x5_0000_1234]);
        let b = words(&[0x5_0000_1238]);
        let mut nowhere = |_: u64| false;
        let c = compare(&a, &b, true, &[], &mut nowhere);
        assert!(c.masked.is_empty());
        assert_eq!(c.diffs[0].what, What::Value);
        // Not laid out in words (a 12-byte component): never masked.
        let c = compare(&a, &b, false, &[], &mut heap);
        assert!(c.masked.is_empty());
        // Small values and kernel-looking ones are never addresses.
        assert!(!pointer_like(0x1234));
        assert!(!pointer_like(0xffff_8000_0000_0000));
        assert!(!pointer_like(0x2_0000_1001));
        assert!(pointer_like(0x2_0000_1000));
    }

    #[test]
    fn the_decision_flag_differs_at_0x70_unless_scratch() {
        let mut a = vec![0u8; 0xa0];
        let mut b = vec![0u8; 0xa0];
        a[0x70] = 1;
        // An odd tail: a 0xa2-byte component's last two bytes count too.
        a.extend([0, 9]);
        b.extend([0, 8]);
        let mut heap = |_: u64| false;
        let c = compare(&a, &b, false, &[], &mut heap);
        assert_eq!(
            c.diffs,
            vec![
                FieldDiff {
                    offset: 0x70,
                    what: What::Value,
                    a: 1,
                    b: 0
                },
                FieldDiff {
                    offset: 0xa0,
                    what: What::Value,
                    a: 0x900,
                    b: 0x800
                },
            ]
        );
        let scratch = Mask {
            component: "MovePath",
            size: 0xa2,
            start: 0x70,
            end: 0x74,
            why: Why::Scratch,
        };
        let c = compare(&a, &b, false, &[scratch], &mut heap);
        assert_eq!(c.diffs.len(), 1);
        assert_eq!(c.diffs[0].offset, 0xa0);
    }

    fn flag_diff(flag_a: u64) -> Compared {
        Compared {
            diffs: vec![FieldDiff {
                offset: 0x70,
                what: What::Value,
                a: flag_a,
                b: 1 - flag_a,
            }],
            masked: vec![(0x18, true)],
        }
    }

    #[test]
    fn differences_are_aggregated_per_component_and_offset() {
        let mp = 0; // MovePath
        let mut t = Tally::new(COMPONENTS.len());
        t.add(mp, 217708, Place::Paged, &flag_diff(1));
        t.add(mp, 192508, Place::Slot(3), &flag_diff(0));
        t.add(mp, 5, Place::Slot(4), &Compared::default());
        t.same(mp);
        t.presence(1, 9, true, false);
        assert_eq!(t.compared[mp], 4);
        assert_eq!(t.shared[mp], 1);
        assert_eq!(t.places[mp], [2, 1, 0]);
        assert_eq!(t.fields(), 2);
        let (count, ex) = t.diffs[&(mp, 0x70, What::Value)];
        assert_eq!(count, 2);
        // The first entity met is the example.
        assert_eq!(
            (ex.entity, ex.place, ex.a, ex.b),
            (217708, Place::Paged, 1, 0)
        );
        assert_eq!(t.masked[mp][&0x18], (2, 2));
        assert_eq!(t.diffs[&(1, 0, What::Presence)].0, 1);
    }

    #[test]
    fn a_difference_is_said_when_new_then_at_most_every_500_steps() {
        let mut t = Tally::new(COMPONENTS.len());
        // MovePath +0x44 (not a watched field).
        let mut d = flag_diff(1);
        d.diffs[0].offset = 0x44;
        t.add(0, 217708, Place::Paged, &d);
        t.masked_example[0] = Some((
            0x18,
            Example {
                entity: 217708,
                place: Place::Paged,
                a: 0x2_0000_1000,
                b: 0x3_0000_1000,
            },
        ));
        let mut r = Reporter::default();
        let lines = r.lines(3300, 1, &t);
        assert_eq!(
            lines[0],
            "copycheck: step 3300 component MovePath offset +0x44 differs in 1 of 1 vehicles (e.g. vehicle 217708 (paged): engine0=0x1, engine1=0x0; engine1 ran the update before)"
        );
        // Once per component: its layout and its masked words.
        assert!(lines[1].starts_with("copycheck: component MovePath: 1 vehicles compared at step 3300: 0 at one address in both engines, 0 in the contiguous store, 1 paged"));
        assert!(lines[1].ends_with(
            "not compared: padding none (bytes the game's own copy skips), scratch none"
        ));
        assert!(lines[2].starts_with("copycheck: component MovePath: masks as heap addresses"));
        assert!(lines[2].contains("+0x18 (1, 1 differing)"));
        assert!(
            lines[2]
                .ends_with("e.g. vehicle 217708 +0x18: engine0=0x200001000, engine1=0x300001000")
        );
        assert_eq!(lines.len(), 3);
        // Not again before 500 steps; then again.
        assert!(r.lines(3301, 0, &t).is_empty());
        assert!(r.lines(3799, 0, &t).is_empty());
        let again = r.lines(3800, 0, &t);
        assert_eq!(again.len(), 1);
        assert!(again[0].contains("engine0 ran the update before"));
        // A word masked later is said once, as an addition.
        t.masked[0].insert(0x20, (1, 0));
        let more = r.lines(3801, 0, &t);
        assert_eq!(more.len(), 1);
        assert!(more[0].starts_with("copycheck: component MovePath: also masks as heap addresses"));
        assert!(more[0].contains("+0x20 (1, 0 differing)"));
        assert!(r.lines(3802, 0, &t).is_empty());
    }

    #[test]
    fn a_check_says_at_most_64_lines_and_holds_the_rest() {
        let mut t = Tally::new(COMPONENTS.len());
        let diffs: Vec<FieldDiff> = (0..70)
            .map(|i| FieldDiff {
                offset: i * 4,
                what: What::Value,
                a: 1,
                b: 2,
            })
            .collect();
        t.add(
            1,
            7,
            Place::Slot(0),
            &Compared {
                diffs,
                masked: Vec::new(),
            },
        );
        let mut r = Reporter::default();
        let lines = r.lines(10, 0, &t);
        let said = lines.iter().filter(|l| l.contains(" differs in ")).count();
        assert_eq!(said, MAX_LINES_PER_CHECK);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("6 more differing field(s) held"))
        );
        // The held ones come at the next check, the said ones do not.
        let next = r.lines(11, 0, &t);
        assert_eq!(
            next.iter().filter(|l| l.contains(" differs in ")).count(),
            6
        );
        assert!(next.iter().all(|l| !l.contains("held")));
    }

    #[test]
    fn a_long_list_is_taken_in_turn() {
        let list: Vec<i32> = (0..5).collect();
        let mut cursor = 0;
        assert_eq!(window(&list, 2, &mut cursor), vec![0, 1]);
        assert_eq!(window(&list, 2, &mut cursor), vec![2, 3]);
        assert_eq!(window(&list, 2, &mut cursor), vec![4, 0]);
        assert_eq!(window(&list, 9, &mut cursor), list);
        assert_eq!(cursor, 0);
    }

    #[test]
    fn engines_are_numbered_in_the_order_met() {
        let mut engines = [0; 2];
        assert_eq!(number(&mut engines, 0xA), 0);
        assert_eq!(number(&mut engines, 0xB), 1);
        assert_eq!(number(&mut engines, 0xA), 0);
        // A third without a load starts again.
        assert_eq!(number(&mut engines, 0xC), 0);
        assert_eq!(number(&mut engines, 0xA), 1);
    }

    #[test]
    fn every_component_has_a_decorated_name() {
        assert_eq!(decorated("MovePath"), b".?AUMovePath@component@ecs@@\0");
        assert_eq!(
            decorated("MovePath"),
            crate::order::decision_sync::TYPE_NAME
        );
        assert!(COMPONENTS.iter().any(|c| c.kind == Kind::Station));
        assert!(COMPONENTS.iter().any(|c| c.kind == Kind::Line));
    }

    /// The scan over the installed game's own code (by file offset, at its
    /// RVAs): every component of [`COMPONENTS`] found, at the getters and
    /// sizes `tpfre` showed for build 40408. `TPF3MP_GAME_EXE` names the
    /// executable; run with `--ignored`.
    #[test]
    #[ignore = "reads the installed game"]
    fn the_scan_finds_the_const_getters_in_the_game() {
        let exe = std::env::var("TPF3MP_GAME_EXE").expect("TPF3MP_GAME_EXE");
        let file = std::fs::read(exe).expect("the game");
        let pe = tpf3mp_hookcore::pe::PeHeaders::parse(&file).expect("a PE");
        let at_rva = |rva: u64, len: usize| -> Option<Vec<u8>> {
            pe.sections.iter().find_map(|s| {
                let va = u64::from(s.virtual_address);
                let off = rva.checked_sub(va)?;
                (off < u64::from(s.virtual_size)).then_some(())?;
                let start = s.pointer_to_raw_data as usize + off as usize;
                file.get(start..start + len).map(<[u8]>::to_vec)
            })
        };
        let text = pe.section(".text").expect(".text");
        let bytes = &file[text.pointer_to_raw_data as usize..][..text.size_of_raw_data as usize];
        let found = scan(bytes, text.virtual_address as usize, &at_rva);
        let got: Vec<(&str, usize, usize)> = found
            .iter()
            .map(|f| (COMPONENTS[f.component].name, f.address, f.size))
            .collect();
        assert_eq!(
            got,
            vec![
                ("MovePath", 0x52bbc0, 0xa0),
                ("TransportVehicle", 0x2827e0, 0x1e8),
                ("LandVehicle", 0x281890, 12),
                ("CarriageList", 0x281650, 0x20),
                ("Ship", 0x281ea0, 12),
                ("Aircraft", 0x280c90, 0x10),
                ("MovePathAircraft", 0x76aad0, 0x238),
                ("Station", 0x282470, 0x58),
                ("Line", 0x76a9c0, 40),
            ]
        );
    }

    #[test]
    fn an_entity_outside_the_engines_tables_is_not_asked_for() {
        let mut all = |_: u64, _: usize| true;
        // Ten entities' lists at 0x1000.
        let lists = (0x1000, 0x1000 + 10 * 24);
        assert!(entity_in_tables(9, lists, 0x9000, &mut all));
        assert!(!entity_in_tables(10, lists, 0x9000, &mut all));
        assert!(!entity_in_tables(-1, lists, 0x9000, &mut all));
        // A list that is not one of 24-byte entries, or empty: nothing.
        assert!(!entity_in_tables(0, (0x1000, 0x1010), 0x9000, &mut all));
        assert!(!entity_in_tables(0, (0x1000, 0x1000), 0x9000, &mut all));
        // No bits, or the entity's bits unreadable: nothing.
        assert!(!entity_in_tables(1, lists, 0, &mut all));
        let mut not_bits = |at: u64, _: usize| at < 0x9000;
        assert!(!entity_in_tables(1, lists, 0x9000, &mut not_bits));
    }

    #[test]
    fn padding_is_not_compared_but_the_field_beside_it_is() {
        // TransportVehicle +0x1c8: a byte field, then three bytes of
        // padding (round A: engine0=0x1, engine1=0x20202001).
        let masks = masks_for("TransportVehicle", 0x1e8);
        let mut a = vec![0u8; 0x1e8];
        let mut b = vec![0u8; 0x1e8];
        a[0x1c8] = 1;
        b[0x1c8..0x1cc].copy_from_slice(&0x2020_2001u32.to_le_bytes());
        // +0x04: padding whole (engine1=0x6628726f).
        b[0x04..0x08].copy_from_slice(&0x6628_726fu32.to_le_bytes());
        let mut heap = |_: u64| false;
        assert!(compare(&a, &b, true, &masks, &mut heap).diffs.is_empty());
        // The byte field itself differing is said.
        b[0x1c8] = 0;
        let c = compare(&a, &b, true, &masks, &mut heap);
        assert_eq!(c.diffs.len(), 1);
        assert_eq!((c.diffs[0].offset, c.diffs[0].a), (0x1c8, 1));
        // MovePath: the snapshot +0x74..+0xa0 and padding +0x24 are not
        // compared, the flag +0x70 is.
        let masks = masks_for("MovePath", 0xa0);
        let mut a = vec![0u8; 0xa0];
        let b = vec![0u8; 0xa0];
        a[0x24] = 0x3d;
        a[0x78] = 7;
        a[0x98] = 1;
        assert!(compare(&a, &b, true, &masks, &mut heap).diffs.is_empty());
        a[0x70] = 1;
        let c = compare(&a, &b, true, &masks, &mut heap);
        assert_eq!(c.diffs.len(), 1);
        assert_eq!(c.diffs[0].offset, 0x70);
        // Another size: nothing masked.
        assert!(masks_for("MovePath", 0xa8).is_empty());
    }

    #[test]
    fn every_padding_range_lies_inside_its_component() {
        for m in MASKS {
            assert!(m.start < m.end && m.end <= m.size, "{m:?}");
            assert!(COMPONENTS.iter().any(|c| c.name == m.component), "{m:?}");
        }
    }

    #[test]
    fn a_field_is_said_again_when_its_count_changes_and_when_it_is_equal_again() {
        let tv = 1;
        let diff = |offset| Compared {
            diffs: vec![FieldDiff {
                offset,
                what: What::Value,
                a: 3,
                b: 4,
            }],
            masked: Vec::new(),
        };
        let mut r = Reporter::default();
        let mut t = Tally::new(COMPONENTS.len());
        t.add(tv, 7, Place::Slot(0), &diff(0x1b8));
        let said = |lines: &[String]| lines.iter().filter(|l| l.contains(" differs in ")).count();
        assert_eq!(said(&r.lines(10, 0, &t)), 1);
        // The same count: not again.
        assert_eq!(said(&r.lines(11, 1, &t)), 0);
        // Another vehicle differs too: said, with the new count.
        t.add(tv, 8, Place::Slot(1), &diff(0x1b8));
        let lines = r.lines(12, 0, &t);
        assert_eq!(said(&lines), 1);
        assert!(lines[0].contains("+0x1b8 differs in 2 of 2 vehicles"));
        // Equal again: said once.
        let mut t = Tally::new(COMPONENTS.len());
        t.add(tv, 7, Place::Slot(0), &Compared::default());
        let lines = r.lines(13, 1, &t);
        assert_eq!(
            lines,
            vec![
                "copycheck: step 13 component TransportVehicle offset +0x1b8 now equal in all 1 vehicles"
            ]
        );
        assert!(r.lines(14, 0, &t).is_empty());
    }

    #[test]
    fn the_decision_flag_is_said_first_every_check_it_differs() {
        let mut t = Tally::new(COMPONENTS.len());
        let many: Vec<FieldDiff> = (0..100)
            .map(|i| FieldDiff {
                offset: i * 4,
                what: What::Value,
                a: 1,
                b: 2,
            })
            .collect();
        // TransportVehicle sorts after MovePath anyway; put the flag last
        // among MovePath's by giving MovePath 100 new fields too.
        t.add(
            0,
            217708,
            Place::Slot(5),
            &Compared {
                diffs: many.clone(),
                masked: Vec::new(),
            },
        );
        let mut r = Reporter::default();
        let lines = r.lines(3201, 1, &t);
        assert!(lines[0].contains("component MovePath offset +0x70 differs in 1 of 1"));
        // Said again the next check though its count is the same.
        let lines = r.lines(3202, 0, &t);
        assert!(lines[0].contains("component MovePath offset +0x70 differs"));
        assert!(watched_text(&t).contains("MovePath +0x70 differs in 1 of 1"));
        let empty = Tally::new(COMPONENTS.len());
        assert!(watched_text(&empty).contains("MovePath +0x70 differs in 0 of 0"));
    }

    #[test]
    fn an_empty_vector_equals_an_unallocated_one_and_lengths_are_compared() {
        // TransportVehicle +0x198 in round A: an empty vector with room for
        // two entries in one engine, never allocated in the other.
        let mut a = words(&[0x1d0_5464_4f30, 0x1d0_5464_4f30, 0x1d0_5464_4f50, 9]);
        let b = words(&[0, 0, 0, 9]);
        let mut heap = |_: u64| true;
        let c = compare(&a, &b, true, &[], &mut heap);
        assert!(c.diffs.is_empty(), "{c:?}");
        assert_eq!(c.masked, vec![(0, true)]);
        // One entry in it: a length that differs.
        a[8..16].copy_from_slice(&0x1d0_5464_4f40u64.to_le_bytes());
        let c = compare(&a, &b, true, &[], &mut heap);
        assert_eq!(
            c.diffs,
            vec![FieldDiff {
                offset: 0,
                what: What::Length,
                a: 0x10,
                b: 0
            }]
        );
        // Three nulls in both copies: plain equal words, nothing masked.
        let z = words(&[0, 0, 0]);
        let c = compare(&z, &z, true, &[], &mut heap);
        assert!(c.diffs.is_empty() && c.masked.is_empty());
        // Not a vector's shape (end before begin): compared as words.
        let x = words(&[0x2_0000_1040, 0x2_0000_1000, 0x2_0000_1080]);
        let c = compare(&x, &z, true, &[], &mut heap);
        assert!(!c.diffs.is_empty());
        assert!(c.diffs.iter().all(|d| d.what == What::Value));
    }
}

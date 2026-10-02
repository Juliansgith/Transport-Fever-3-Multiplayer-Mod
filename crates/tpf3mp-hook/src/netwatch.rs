//! The network watcher (logging only; it changes nothing): when a watched
//! construction's transport network changes, and how its edges are ordered
//! (docs/HOOKS.md, "Path ties", the lane permutation of construction
//! 362201).
//!
//! In the round of 2026-10-02 (`1908df3`) the route search found the same
//! costs on construction 362201's lanes in two games, but under other lane
//! indices: its edge list was permuted between the games, though the path
//! through it at step 921 used the same indices in both. So the list was
//! rebuilt in between, in another order in each game. With
//! `TPF3MP_HOOK_WATCH_PATH_ENTITIES` set, at the start of every room's
//! update, the watched entities' `TransportNetwork` component (the const
//! getter found in the code as `crate::copycheck` finds its getters) is read
//! in the engine about to simulate, and one line is said when its edge list
//! differs from the last one said for that entity:
//!
//! ```text
//! net: step <s> entity <e> edges <n>: <i>:<fingerprint> ...
//! ```
//!
//! An edge's fingerprint hashes its 0x70 bytes as 8-byte words, leaving out
//! any word that holds a heap address. Two games whose lines differ only in
//! the order of the same fingerprints have the same edges under other
//! indices; the first step of a change names when the list was rebuilt.

#![allow(unsafe_code)]
#![cfg_attr(not(all(windows, target_arch = "x86_64")), allow(dead_code))]

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::copycheck::{
    GETTER_PATTERN, GETTER_READ, decorated, entity_in_tables, getter, pattern, pointer_like,
};
use crate::image::Readable as Probe;

/// The type whose getter is wanted.
pub const COMPONENT: &str = "TransportNetwork";
/// Where the component keeps its edges: a vector at `+0x18`, 0x70 bytes an
/// edge (`FindNextFreeTerminal`, `0xb85706..0xb8570e`).
pub const EDGES: u64 = 0x18;
pub const EDGE_LEN: u64 = 0x70;
const MAX_EDGES: u64 = 4096;

static GETTER: AtomicUsize = AtomicUsize::new(0);

struct State {
    entities: HashSet<i32>,
    said: HashMap<i32, Vec<u32>>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

type GetterFn = unsafe extern "system" fn(usize, i32) -> usize;

/// One edge's fingerprint: FNV-1a over its 8-byte words, a heap address
/// left out.
pub fn fingerprint(edge: &[u8]) -> u32 {
    let mut hash = crate::order::Fnv1a::new();
    for word in edge.as_chunks::<8>().0 {
        if pointer_like(u64::from_le_bytes(*word)) {
            hash.write(&[0xff; 8]);
        } else {
            hash.write(word);
        }
    }
    hash.0 as u32
}

/// The line for an edge list.
pub fn line(step: u64, entity: i32, prints: &[u32]) -> String {
    let mut text = format!("net: step {step} entity {entity} edges {}:", prints.len());
    for (i, print) in prints.iter().enumerate() {
        text.push_str(&format!(" {i}:{print:08x}"));
    }
    text
}

fn code(at: usize, len: usize) -> Option<&'static [u8]> {
    if !crate::image::readable(at, len) {
        return None;
    }
    // SAFETY: `len` readable bytes of the game's mapped image, only read.
    Some(unsafe { std::slice::from_raw_parts(at as *const u8, len) })
}

/// Turns the watcher on when the environment lists entities (`base` the
/// game's image), and says what it found.
pub fn install(base: usize) -> String {
    let entities = crate::order::claims::parse_entities(
        std::env::var(crate::order::path_ties::WATCH_ENTITIES_ENV)
            .ok()
            .as_deref(),
    );
    if entities.is_empty() {
        return "net: no construction's network is watched".into();
    }
    let Some(header) = code(base, 0x1000) else {
        return "net: off, the game's image is not readable".into();
    };
    let Ok(pe) = tpf3mp_hookcore::pe::PeHeaders::parse(header) else {
        return "net: off, the game's image is not readable".into();
    };
    let Some(section) = pe.section(".text") else {
        return "net: off, no .text".into();
    };
    let text_at = base + section.virtual_address as usize;
    let Some(text) = code(text_at, section.virtual_size as usize) else {
        return "net: off, the game's .text is not readable".into();
    };
    let pattern = pattern(GETTER_PATTERN);
    let prefix: Vec<u8> = pattern.iter().take(0x15).map_while(|b| *b).collect();
    let name = decorated(COMPONENT);
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
        let named = usize::try_from(g.descriptor + 0x10)
            .ok()
            .and_then(|d| code(d, name.len()))
            .is_some_and(|bytes| bytes == name.as_slice());
        if named {
            GETTER.store(address, Ordering::Release);
            let count = entities.len();
            *STATE.lock().unwrap_or_else(|p| p.into_inner()) = Some(State {
                entities,
                said: HashMap::new(),
            });
            return format!(
                "net: watching {count} construction(s)' {COMPONENT} (getter +{:#x}, {:#x} bytes)",
                address - base,
                g.size
            );
        }
    }
    format!("net: off, {COMPONENT}'s const getter is not in the code")
}

/// A world was loaded: everything is said again.
pub fn reset() {
    if let Some(state) = STATE.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
        state.said.clear();
    }
}

/// Before an update of `engine`, in the room's step.
pub fn before_update(engine: usize) {
    let getter = GETTER.load(Ordering::Acquire);
    if getter == 0 || engine == 0 || !crate::order::in_step() {
        return;
    }
    let Some(step) = crate::seeds::current_step() else {
        return;
    };
    let _ = std::panic::catch_unwind(|| {
        let mut guard = STATE.lock().unwrap_or_else(|p| p.into_inner());
        let Some(state) = guard.as_mut() else {
            return;
        };
        let mut probe = Probe::new();
        let entities: Vec<i32> = state.entities.iter().copied().collect();
        for entity in entities {
            let e = engine as u64;
            let (Some(begin), Some(end), Some(bits)) = (
                probe.read::<u64>(e + 0x90),
                probe.read::<u64>(e + 0x98),
                probe.read::<u64>(e + 0xc0),
            ) else {
                return;
            };
            if !entity_in_tables(entity, (begin, end), bits, &mut |at, len| {
                usize::try_from(at).is_ok_and(|at| probe.readable(at, len))
            }) {
                continue;
            }
            // SAFETY: the game's const getter, found by its code and the
            // type its lea names, asked only for an entity within the
            // engine's tables; it reads and answers a pointer or null.
            let get: GetterFn = unsafe { std::mem::transmute::<usize, GetterFn>(getter) };
            let component = unsafe { get(engine, entity) } as u64;
            if component == 0 {
                continue;
            }
            let (Some(first), Some(last)) = (
                probe.read::<u64>(component + EDGES),
                probe.read::<u64>(component + EDGES + 8),
            ) else {
                continue;
            };
            if last < first
                || (last - first) % EDGE_LEN != 0
                || (last - first) / EDGE_LEN > MAX_EDGES
            {
                continue;
            }
            let mut prints = Vec::new();
            for i in 0..(last - first) / EDGE_LEN {
                let Some(edge) = probe.read::<[u8; EDGE_LEN as usize]>(first + i * EDGE_LEN) else {
                    return;
                };
                prints.push(fingerprint(&edge));
            }
            if state.said.get(&entity) != Some(&prints) {
                crate::log::line(&line(step, entity, &prints));
                state.said.insert(entity, prints);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edges_fingerprint_leaves_heap_addresses_out_and_the_line_lists_them_in_order() {
        let mut a = [0u8; EDGE_LEN as usize];
        a[..8].copy_from_slice(&0x02f0_a981_d8c0_u64.to_le_bytes());
        a[8..12].copy_from_slice(&1.5f32.to_le_bytes());
        let mut b = a;
        b[..8].copy_from_slice(&0x01e2_a0f3_a050_u64.to_le_bytes());
        assert_eq!(fingerprint(&a), fingerprint(&b));
        b[8..12].copy_from_slice(&2.5f32.to_le_bytes());
        assert_ne!(fingerprint(&a), fingerprint(&b));
        assert_eq!(
            line(921, 362_201, &[0xab, 0xcd]),
            "net: step 921 entity 362201 edges 2: 0:000000ab 1:000000cd"
        );
    }
}

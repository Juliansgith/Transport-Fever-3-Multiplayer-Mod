//! Shared native demand scheduling, with optional bounded observations.
//! Diagnostics never alter the scheduler's comparisons or return values.
#![allow(unsafe_code)]

use crate::{image::Readable, log};
use std::{
    cell::{Cell, RefCell},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
};
use tpf3mp_hookcore::{detour::InlineDetour, profile::ResolvedProfile};

pub(super) mod native {
    include!(concat!(env!("OUT_DIR"), "/industry_schedule_native.rs"));
}
static ORIGINAL: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
static DETOURS: Mutex<Vec<InlineDetour>> = Mutex::new(Vec::new());
static SENDER: OnceLock<SyncSender<Record>> = OnceLock::new();
static DROPPED: AtomicUsize = AtomicUsize::new(0);
static COUNT: AtomicUsize = AtomicUsize::new(0);

pub(super) fn install_fix(resolved: &ResolvedProfile) -> Result<(), String> {
    let (name, expected) = native::SITES[0];
    let address = resolved
        .get(name)
        .ok_or_else(|| format!("missing {name}"))?
        .address;
    for (i, byte) in expected.iter().enumerate() {
        if read::<u8>(address + i as u64) != Some(*byte) {
            return Err(format!("{name}: compiled ABI mismatch"));
        }
    }
    // SAFETY: the exact build and compiled entry bytes are checked while the
    // game is suspended. This thunk preserves the scheduler's native ABI.
    let detour =
        unsafe { InlineDetour::install(address as *mut u8, schedule as *const () as *const u8) }
            .map_err(|error| format!("{name}: {error}"))?;
    ORIGINAL[0].store(detour.trampoline() as usize, Ordering::Release);
    DETOURS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(detour);
    Ok(())
}

thread_local! {
    static RECORD: RefCell<Option<Record>> = const { RefCell::new(None) };
    static TOWN: Cell<i32> = const { Cell::new(-1) };
}

#[derive(Clone, Copy, Debug)]
struct Set {
    values: [i32; 64],
    len: usize,
}
#[derive(Clone, Copy, Debug)]
struct Change {
    town: i32,
    left: Option<Set>,
    right: Option<Set>,
}
struct Record {
    time: i64,
    before: Option<i32>,
    after: Option<i32>,
    comparisons: usize,
    changes: Vec<Change>,
    truncated: bool,
}

fn read<T: Copy>(at: u64) -> Option<T> {
    Readable::new().read(at)
}

// Canonical integer values, never pointer addresses or hash-table order.
fn set(at: u64) -> Option<Set> {
    let control = read::<u64>(at)?;
    let data = read::<u64>(at.checked_add(8)?)?;
    let size = read::<u64>(at.checked_add(16)?)?;
    let capacity = read::<u64>(at.checked_add(24)?)?;
    if size > 64 || capacity > 4095 || size > capacity {
        return None;
    }
    let mut out = Set {
        values: [0; 64],
        len: 0,
    };
    for slot in 0..capacity {
        if read::<i8>(control.checked_add(slot)?)? >= 0 {
            if out.len == out.values.len() {
                return None;
            }
            out.values[out.len] = read::<i32>(data.checked_add(slot.checked_mul(4)?)?)?;
            out.len += 1;
        }
    }
    if out.len as u64 != size {
        return None;
    }
    out.values[..out.len].sort_unstable();
    Some(out)
}

pub(super) fn install(resolved: &ResolvedProfile) -> Result<(), String> {
    // Validate every compiled ABI before installing any detour.
    let mut addresses = Vec::new();
    for (name, expected) in native::SITES.iter().skip(1) {
        let address = resolved
            .get(name)
            .ok_or_else(|| format!("missing {name}"))?
            .address;
        for (i, byte) in expected.iter().enumerate() {
            if read::<u8>(address + i as u64) != Some(*byte) {
                return Err(format!("{name}: compiled schedule ABI mismatch"));
            }
        }
        addresses.push(address);
    }
    let hooks = [
        town as *const () as *const u8,
        equal as *const () as *const u8,
    ];
    let mut detours = Vec::new();
    for (index, address) in addresses.iter().enumerate() {
        // SAFETY: exact build, signature and compiled prologue checked above;
        // installation occurs while the game is suspended.
        let detour = unsafe { InlineDetour::install(*address as *mut u8, hooks[index]) }
            .map_err(|error| format!("industry schedule probe: {error}"))?;
        ORIGINAL[index + 1].store(detour.trampoline() as usize, Ordering::Release);
        detours.push(detour);
    }
    let (sender, receiver) = mpsc::sync_channel::<Record>(64);
    std::thread::Builder::new().name("tpf3mp-industry-schedule".into()).spawn(move || {
        for record in receiver {
            let changes = record.changes.iter().map(|change| {
                let show = |set: &Option<Set>| set.as_ref().map(|set| format!("{:?}", &set.values[..set.len])).unwrap_or_else(|| "unreadable".into());
                format!("{}:{}=>{}", change.town, show(&change.left), show(&change.right))
            }).collect::<Vec<_>>().join(";");
            log::line(&format!("industry-schedule probe: time={} pending={:?}->{:?} comparisons={} changes=[{}] truncated={} dropped={}",
                record.time, record.before, record.after, record.comparisons, changes, record.truncated, DROPPED.swap(0, Ordering::Relaxed)));
        }
    }).map_err(|error| format!("industry schedule logger: {error}"))?;
    SENDER
        .set(sender)
        .map_err(|_| "industry schedule installed twice".to_owned())?;
    DETOURS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .extend(detours);
    Ok(())
}

fn observe(body: impl FnOnce()) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)).is_err() {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

unsafe extern "system" fn schedule(captures: usize, time: i64) {
    super::shared::invoke(captures, |captures| {
        // SAFETY: sharing preserves the exact native capture layout and keeps
        // its state owned for the whole original call.
        unsafe { observed_schedule(captures, time) };
    });
}

unsafe fn observed_schedule(captures: usize, time: i64) {
    // SAFETY: trampoline for lambda_9(captures, interval_time), pinned above.
    let original: unsafe extern "system" fn(usize, i64) =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    if SENDER.get().is_none() {
        // No probe: no demand-set reads, allocations or per-interval logging.
        unsafe { original(captures, time) };
        return;
    }
    let state = read::<u64>((captures as u64).wrapping_add(native::STATE_CAPTURE));
    let pending = || state.and_then(|state| read::<i32>(state.wrapping_add(native::PENDING)));
    let enabled = COUNT.fetch_add(1, Ordering::Relaxed) < 4096;
    if enabled {
        observe(|| {
            RECORD.with(|record| {
                *record.borrow_mut() = Some(Record {
                    time,
                    before: pending(),
                    after: None,
                    comparisons: 0,
                    changes: Vec::new(),
                    truncated: false,
                })
            })
        });
    }
    // SAFETY: original arguments forwarded unchanged.
    unsafe { original(captures, time) };
    if enabled {
        observe(|| {
            RECORD.with(|slot| {
                if let Some(mut record) = slot.borrow_mut().take() {
                    record.after = pending();
                    if let Some(sender) = SENDER.get()
                        && sender.try_send(record).is_err()
                    {
                        DROPPED.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })
        });
    }
}

unsafe extern "system" fn town(captures: usize, entity: i32) {
    // SAFETY: native town closure takes exactly this captures/entity pair.
    let original: unsafe extern "system" fn(usize, i32) =
        unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    let previous = TOWN.with(|town| town.replace(entity));
    // SAFETY: original arguments forwarded unchanged.
    unsafe { original(captures, entity) };
    TOWN.with(|town| town.set(previous));
}

unsafe extern "system" fn equal(left: usize, right: usize) -> bool {
    // SAFETY: bool flat_hash_set<int> equality takes two const references.
    let original: unsafe extern "system" fn(usize, usize) -> bool =
        unsafe { std::mem::transmute(ORIGINAL[2].load(Ordering::Acquire)) };
    // SAFETY: original arguments forwarded unchanged; comparison is read-only.
    let result = unsafe { original(left, right) };
    observe(|| {
        RECORD.with(|slot| {
            if let Some(record) = slot.borrow_mut().as_mut() {
                record.comparisons += 1;
                if !result {
                    if record.changes.len() < 64 {
                        record.changes.push(Change {
                            town: TOWN.with(Cell::get),
                            left: set(left as u64),
                            right: set(right as u64),
                        });
                    } else {
                        record.truncated = true;
                    }
                }
            }
        })
    });
    result
}

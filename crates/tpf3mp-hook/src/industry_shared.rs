//! Exact-build sharing of IndustryChainSpawnState across the
//! engine's simulation buffers. The native state owns its normal containers;
//! the hook retains one native shared_ptr and only redirects a stack copy of
//! the scheduler's captures. World load starts a fresh logical history.
#![allow(unsafe_code)]
use super::schedule::native;
use crate::{image::Readable, log};
use std::sync::{
    Mutex,
    atomic::{AtomicI32, AtomicU64, Ordering},
};
static EPOCH: AtomicU64 = AtomicU64::new(0);
static STATE: Mutex<LogicalState> = Mutex::new(LogicalState {
    epoch: 0,
    native: None,
});
struct LogicalState {
    epoch: u64,
    native: Option<OwnedShared>,
}
struct OwnedShared {
    data: usize,
    control: usize,
    destroy: usize,
    delete: usize,
}

pub(super) fn reset_world() {
    EPOCH.fetch_add(1, Ordering::AcqRel);
}
fn read<T: Copy>(at: usize) -> Option<T> {
    Readable::new().read(at as u64)
}

impl OwnedShared {
    fn retain(
        data: usize,
        control: usize,
        vtable: usize,
        destroy: usize,
        delete: usize,
    ) -> Option<Self> {
        if control == 0
            || !control.is_multiple_of(8)
            || data != control.checked_add(16)?
            || read::<usize>(control)? != vtable
            || read::<[usize; 2]>(vtable)? != [destroy, delete]
            || read::<i32>(control.checked_add(12)?)? <= 0
        {
            return None;
        }
        // SAFETY: exact native shared_ptr allocation, validated live while its
        // owning lambda is executing; MSVC uses interlocked i32 at +8/+12.
        let strong = unsafe { &*((control + 8) as *const AtomicI32) };
        strong
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (1..i32::MAX).contains(&n).then(|| n + 1)
            })
            .ok()?;
        Some(Self {
            data,
            control,
            destroy,
            delete,
        })
    }
}
impl Drop for OwnedShared {
    fn drop(&mut self) {
        // SAFETY: our retained strong reference keeps this control block live.
        let strong = unsafe { &*((self.control + 8) as *const AtomicI32) };
        if strong.fetch_sub(1, Ordering::AcqRel) == 1 {
            // SAFETY: exact-build vtable verified before retaining the object.
            let destroy: unsafe extern "system" fn(usize) =
                unsafe { std::mem::transmute(self.destroy) };
            unsafe { destroy(self.control) };
            // Destroying the payload leaves the implicit weak reference alive.
            let weak = unsafe { &*((self.control + 12) as *const AtomicI32) };
            if weak.fetch_sub(1, Ordering::AcqRel) == 1 {
                let delete: unsafe extern "system" fn(usize) =
                    unsafe { std::mem::transmute(self.delete) };
                unsafe { delete(self.control) };
            }
        }
    }
}

/// Calls the native/diagnostic wrapper once when the state validates. Outside
/// a room and on inactive buffers, retain the original captures. An invalid
/// state holds the room without running an unsafe or divergent callback. No queued callback keeps
/// the stack captures: the native function copies its resource/state/logger
/// references and global demand set into its queued spawn lambda.
pub(super) fn invoke(captures: usize, call: impl FnOnce(usize)) {
    if !crate::lua::in_room() {
        call(captures);
        return;
    }
    let Some(mut copy) = read::<[usize; 6]>(captures) else {
        super::FAULT.store(true, Ordering::Release);
        return;
    };
    let active = copy[1]
        .checked_add(0x18)
        .and_then(read::<usize>)
        .and_then(|engine| engine.checked_add(0x1f8))
        .and_then(read::<u8>);
    if active == Some(0) {
        call(captures);
        return;
    }
    if active != Some(1) {
        super::FAULT.store(true, Ordering::Release);
        return;
    }
    let mut logical = STATE.lock().unwrap_or_else(|poison| poison.into_inner());
    let epoch = EPOCH.load(Ordering::Acquire);
    if logical.epoch != epoch {
        logical.native = None;
        logical.epoch = epoch;
    }
    if logical.native.is_none() {
        // SAFETY: null module name asks for the loaded executable, no mutation.
        let base = unsafe {
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()) as usize
        };
        logical.native = OwnedShared::retain(
            copy[4],
            copy[5],
            base + native::STATE_VTABLE as usize,
            base + native::STATE_DESTROY as usize,
            base + native::STATE_DELETE as usize,
        );
        log::line(if logical.native.is_some() {
            "industry scheduler: sharing native demand history across world buffers"
        } else {
            "industry scheduler: shared state refused; native ownership layout did not validate"
        });
    }
    if let Some(shared) = logical.native.as_ref() {
        copy[4] = shared.data;
        copy[5] = shared.control;
        call(copy.as_ptr() as usize);
    } else {
        super::FAULT.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[repr(C)]
    struct Control {
        vtable: usize,
        strong: AtomicI32,
        weak: AtomicI32,
        data: [u64; 14],
    }
    static DESTROYED: AtomicU64 = AtomicU64::new(0);
    static DELETED: AtomicU64 = AtomicU64::new(0);
    unsafe extern "system" fn destroy(_: usize) {
        DESTROYED.fetch_add(1, Ordering::Relaxed);
    }
    unsafe extern "system" fn delete(_: usize) {
        DELETED.fetch_add(1, Ordering::Relaxed);
    }
    #[test]
    fn shared_history_lives_until_last_native_owner_and_releases_once() {
        let vtable = [destroy as *const () as usize, delete as *const () as usize];
        let control = Control {
            vtable: vtable.as_ptr() as usize,
            strong: AtomicI32::new(1),
            weak: AtomicI32::new(1),
            data: [0; 14],
        };
        let address = &control as *const _ as usize;
        let hold = OwnedShared::retain(address + 16, address, control.vtable, vtable[0], vtable[1])
            .unwrap();
        assert_eq!(control.strong.load(Ordering::Relaxed), 2);
        assert_eq!(control.strong.fetch_sub(1, Ordering::AcqRel), 2); // native lambda releases
        let destroyed = DESTROYED.load(Ordering::Relaxed);
        let deleted = DELETED.load(Ordering::Relaxed);
        drop(hold);
        assert_eq!(control.strong.load(Ordering::Relaxed), 0);
        assert_eq!(control.weak.load(Ordering::Relaxed), 0);
        assert_eq!(DESTROYED.load(Ordering::Relaxed), destroyed + 1);
        assert_eq!(DELETED.load(Ordering::Relaxed), deleted + 1);
        assert!(
            OwnedShared::retain(address + 16, address, control.vtable, vtable[0], vtable[1])
                .is_none()
        );
    }
    #[test]
    fn invalid_native_ownership_is_rejected_without_retaining() {
        let vtable = [destroy as *const () as usize, delete as *const () as usize];
        let control = Control {
            vtable: vtable.as_ptr() as usize,
            strong: AtomicI32::new(1),
            weak: AtomicI32::new(1),
            data: [0; 14],
        };
        let at = &control as *const _ as usize;
        for (data, owner, table, drop_fn) in [
            (at + 24, at, control.vtable, vtable[0]),
            (at + 16, at + 1, control.vtable, vtable[0]),
            (at + 16, at, control.vtable + 8, vtable[0]),
            (at + 16, at, control.vtable, vtable[1]),
        ] {
            assert!(OwnedShared::retain(data, owner, table, drop_fn, vtable[1]).is_none());
            assert_eq!(control.strong.load(Ordering::Relaxed), 1);
        }
        control.strong.store(i32::MAX, Ordering::Relaxed);
        assert!(OwnedShared::retain(at + 16, at, control.vtable, vtable[0], vtable[1]).is_none());
        assert_eq!(control.strong.load(Ordering::Relaxed), i32::MAX);
        control.strong.store(1, Ordering::Relaxed);
        control.weak.store(0, Ordering::Relaxed);
        assert!(OwnedShared::retain(at + 16, at, control.vtable, vtable[0], vtable[1]).is_none());
        assert_eq!(control.strong.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn buffer_local_pending_counts_change_spawn_times() {
        fn run(buffers: &[usize], shared: bool) -> Vec<usize> {
            let mut state = [(false, 0); 2];
            let mut spawns = Vec::new();
            for (tick, buffer) in buffers.iter().enumerate() {
                let (seen, pending) = &mut state[if shared { 0 } else { *buffer }];
                if *pending > 0 {
                    *pending -= 1;
                    spawns.push(tick);
                }
                if !*seen {
                    *seen = true;
                    *pending += 5;
                }
            }
            spawns
        }
        // The captured failure reads 3 from one buffer and 4 from the other
        // at the same interval; selecting the depleted buffer later skips a
        // spawn even while the other buffer still has pending work.
        let a = [0, 1, 0, 1, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0];
        let b = [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1];
        assert_ne!(run(&a, false), run(&b, false));
        assert_eq!(run(&a, true), vec![1, 2, 3, 4, 5]);
        assert_eq!(run(&a, true), run(&b, true));
    }
}

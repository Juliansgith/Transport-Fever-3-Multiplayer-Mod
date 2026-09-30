//! Reading the running game's memory safely: whether an address may be read
//! before the hook reads it on the game's thread (`crate::order`,
//! `crate::seeds`).
//!
//! Windows only for now, like the step gate. Elsewhere nothing is readable,
//! and the hook's native reads refuse (fail closed).

#![allow(unsafe_code)]

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
    };

    /// Whether `len` bytes at `address` are committed, readable memory.
    pub fn readable(address: usize, len: usize) -> bool {
        if len == 0 {
            return true;
        }
        let Some(end) = address.checked_add(len) else {
            return false;
        };
        let mut at = address;
        while at < end {
            // SAFETY: MEMORY_BASIC_INFORMATION is plain data, zero is a valid
            // value for it, and VirtualQuery only writes into it.
            let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
            let written = unsafe {
                VirtualQuery(
                    at as *const _,
                    &mut info,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if written == 0
                || info.State != MEM_COMMIT
                || info.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0
            {
                return false;
            }
            let region_end = info.BaseAddress as usize + info.RegionSize;
            if region_end <= at {
                return false;
            }
            at = region_end;
        }
        true
    }
}

#[cfg(windows)]
pub use windows::readable;

/// Whether `len` bytes at `address` may be read. Not known here, so `false`:
/// nothing native is read on this platform yet.
#[cfg(not(windows))]
pub fn readable(_address: usize, _len: usize) -> bool {
    false
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn this_code_is_readable_and_unmapped_memory_is_not() {
        let here = this_code_is_readable_and_unmapped_memory_is_not as *const () as usize;
        assert!(readable(here, 16));
        // The null page is never mapped.
        assert!(!readable(0x10, 8));
        assert!(!readable(usize::MAX - 4, 8));
        assert!(readable(0x10, 0), "zero bytes are always readable");
    }
}

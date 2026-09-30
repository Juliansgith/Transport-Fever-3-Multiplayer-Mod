//! Reading the running game's memory safely: whether an address may be read
//! before the hook reads it on the game's thread (`crate::order`,
//! `crate::seeds`).
//!
//! Windows only for now, like the step gate. Elsewhere nothing is readable,
//! and the hook's native reads refuse (fail closed).
//!
//! Each check asks the system (`VirtualQuery`, a system call of about a
//! microsecond). A hook that reads many words of one structure in one call
//! (the road fix walks eight words to find an edge's entries, for each edge
//! an append touched) takes a [`Probe`] for that call instead: the regions
//! it found committed and readable are remembered until the call returns,
//! so the second word of a region costs a comparison, not a system call.
//! A probe lives for one hook call on the game's thread and no longer: the
//! engine may free and decommit memory between calls.

#![allow(unsafe_code)]

/// The committed, readable region `[base, end)` that holds `address`, or
/// `None` when `address` is not readable.
#[cfg(windows)]
fn query(address: usize) -> Option<(usize, usize)> {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
    };
    // SAFETY: MEMORY_BASIC_INFORMATION is plain data, zero is a valid value
    // for it, and VirtualQuery only writes into it.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let written = unsafe {
        VirtualQuery(
            address as *const _,
            &mut info,
            std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if written == 0 || info.State != MEM_COMMIT || info.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0
    {
        return None;
    }
    let base = info.BaseAddress as usize;
    Some((base, base.saturating_add(info.RegionSize)))
}

/// Nothing is known readable on this platform.
#[cfg(not(windows))]
fn query(_address: usize) -> Option<(usize, usize)> {
    None
}

/// Regions already found readable, most recent first; at most `N`.
#[derive(Debug, Clone)]
pub struct RegionCache<const N: usize> {
    regions: [(usize, usize); N],
    len: usize,
}

impl<const N: usize> RegionCache<N> {
    pub const fn new() -> Self {
        Self {
            regions: [(0, 0); N],
            len: 0,
        }
    }

    /// Whether `len` bytes at `address` are readable, asking `query` only
    /// for the parts no remembered region covers. `query(at)` answers the
    /// readable region `[base, end)` holding `at`, or `None`.
    pub fn readable_with(
        &mut self,
        address: usize,
        len: usize,
        mut query: impl FnMut(usize) -> Option<(usize, usize)>,
    ) -> bool {
        if len == 0 {
            return true;
        }
        let Some(end) = address.checked_add(len) else {
            return false;
        };
        let mut at = address;
        while at < end {
            if let Some(&(_, region_end)) = self.regions[..self.len]
                .iter()
                .find(|(base, region_end)| *base <= at && at < *region_end)
            {
                at = region_end;
                continue;
            }
            let Some((base, region_end)) = query(at) else {
                return false;
            };
            if base > at || region_end <= at {
                return false;
            }
            self.remember(base, region_end);
            at = region_end;
        }
        true
    }

    fn remember(&mut self, base: usize, end: usize) {
        if N == 0 {
            return;
        }
        let keep = self.len.min(N - 1);
        self.regions.copy_within(0..keep, 1);
        self.regions[0] = (base, end);
        self.len = keep + 1;
    }

    /// How many regions are remembered.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> Default for RegionCache<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// The readable regions one hook call found, for that call only.
#[derive(Debug, Clone, Default)]
pub struct Probe {
    cache: RegionCache<4>,
}

impl Probe {
    pub const fn new() -> Self {
        Self {
            cache: RegionCache::new(),
        }
    }

    /// Whether `len` bytes at `address` are committed, readable memory.
    pub fn readable(&mut self, address: usize, len: usize) -> bool {
        if cfg!(not(windows)) {
            return false;
        }
        self.cache.readable_with(address, len, query)
    }

    /// A plain value of the game's memory, only if it is readable.
    pub fn read<T: Copy>(&mut self, address: u64) -> Option<T> {
        let address = usize::try_from(address).ok()?;
        if !self.readable(address, std::mem::size_of::<T>()) {
            return None;
        }
        // SAFETY: `size_of::<T>()` bytes at `address` are committed,
        // readable memory, checked just above in this hook call; the read is
        // unaligned and by value.
        Some(unsafe { std::ptr::read_unaligned(address as *const T) })
    }
}

/// Whether `len` bytes at `address` are committed, readable memory: a fresh
/// question to the system every time.
#[cfg(windows)]
pub fn readable(address: usize, len: usize) -> bool {
    Probe::new().readable(address, len)
}

/// Whether `len` bytes at `address` may be read. Not known here, so `false`:
/// nothing native is read on this platform yet.
#[cfg(not(windows))]
pub fn readable(_address: usize, _len: usize) -> bool {
    false
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn a_remembered_region_answers_without_asking_again() {
        let mut cache = RegionCache::<2>::new();
        let asked = std::cell::RefCell::new(Vec::new());
        let mut query = |at: usize| {
            asked.borrow_mut().push(at);
            // Readable: [0x1000, 0x3000) and [0x3000, 0x4000); not below.
            match at {
                0x1000..0x3000 => Some((0x1000, 0x3000)),
                0x3000..0x4000 => Some((0x3000, 0x4000)),
                _ => None,
            }
        };
        assert!(cache.readable_with(0x1010, 8, &mut query));
        assert!(cache.readable_with(0x2ff0, 0x10, &mut query));
        assert!(cache.readable_with(0x1000, 0x2000, &mut query));
        assert_eq!(*asked.borrow(), vec![0x1010], "one question for the region");
        // Across the region's end: the next region is asked for once.
        assert!(cache.readable_with(0x2ff8, 0x10, &mut query));
        assert!(cache.readable_with(0x3008, 8, &mut query));
        assert_eq!(*asked.borrow(), vec![0x1010, 0x3000]);
        assert_eq!(cache.len(), 2);
        // Past the readable memory: refused, whatever is remembered.
        assert!(!cache.readable_with(0x3ff8, 0x10, &mut query));
        assert!(!cache.readable_with(0x800, 8, &mut query));
        assert!(!cache.readable_with(usize::MAX - 4, 8, &mut query));
        assert!(cache.readable_with(0x10, 0, &mut query), "zero bytes");
    }

    #[test]
    fn the_oldest_region_is_forgotten_first() {
        let mut cache = RegionCache::<2>::new();
        let asked = std::cell::Cell::new(0);
        let mut query = |at: usize| {
            asked.set(asked.get() + 1);
            let base = at & !0xfff;
            Some((base, base + 0x1000))
        };
        for page in [0x1000, 0x2000, 0x3000] {
            assert!(cache.readable_with(page, 4, &mut query));
        }
        assert_eq!(cache.len(), 2);
        // 0x2000 and 0x3000 remembered; 0x1000 was forgotten.
        assert!(cache.readable_with(0x3004, 4, &mut query));
        assert!(cache.readable_with(0x2004, 4, &mut query));
        assert_eq!(asked.get(), 3);
        assert!(cache.readable_with(0x1004, 4, &mut query));
        assert_eq!(asked.get(), 4);
    }

    #[test]
    fn a_region_that_does_not_hold_the_address_is_refused() {
        let mut cache = RegionCache::<4>::new();
        assert!(!cache.readable_with(0x1000, 4, |_| Some((0x2000, 0x3000))));
        assert!(!cache.readable_with(0x1000, 4, |_| Some((0x800, 0x1000))));
        assert!(cache.is_empty());
    }
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
        let mut probe = Probe::new();
        let value = 0x1234_5678_u32;
        assert_eq!(
            probe.read::<u32>(&value as *const u32 as u64),
            Some(0x1234_5678)
        );
        assert_eq!(probe.read::<u32>(0x10), None);
    }
}

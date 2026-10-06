//! The whole executable mapped into a test, to run the game's own code.
//!
//! `bigmap::original` relocates single functions; the emitters' update
//! calls a dozen (its lambdas through `std::function` vtables, the
//! standard library's vectors and sort, `logf`), so this maps the image
//! as the loader would: every section at its RVA in one region, the base
//! relocations applied (so vtables and RIP-relative references work), the
//! C runtime's imports resolved (`api-ms-win-crt-*`, `vcruntime140`,
//! `msvcp140`, `kernel32`), every other import pointed at a trap. The
//! executable is only READ. A test then overwrites the functions it
//! stands in for (the thread pool, the ECS lookups, the thread-local
//! indices) with jumps to its own.
//!
//! Nothing of the image runs but what a test calls: no entry point, no
//! static initialisers, no TLS callbacks.

#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used)]

use std::ffi::CString;

use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc, VirtualFree,
};

use crate::bigmap::original::Exe;

extern "system" fn unresolved_import() {
    eprintln!("the game's code called an import the mapped image does not resolve");
    std::process::abort();
}

/// The executable, mapped.
pub struct Mapped {
    base: usize,
    size: usize,
}

// SAFETY: the mapping is plain memory; tests serialise their use of it.
unsafe impl Send for Mapped {}
// SAFETY: as above.
unsafe impl Sync for Mapped {}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// The C runtime's DLLs, whose functions the test may run for real.
fn resolvable(dll: &str) -> bool {
    let dll = dll.to_ascii_lowercase();
    dll.starts_with("api-ms-win-crt-")
        || dll.starts_with("vcruntime140")
        || dll.starts_with("msvcp140")
        || dll == "kernel32.dll"
        || dll == "ucrtbase.dll"
}

impl Mapped {
    /// Maps `exe`'s file (`image`, as read from disk).
    pub fn new(exe: &Exe) -> Self {
        let image = exe.file();
        let pe = exe.headers();
        let size = pe.size_of_image as usize;
        // SAFETY: a fresh region of the test's own.
        let base = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                size,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
        } as usize;
        assert_ne!(base, 0, "no region for the image");
        let map = Self { base, size };
        for section in &pe.sections {
            let raw = section.raw(image).unwrap();
            let len = raw
                .len()
                .min(section.virtual_size.max(section.size_of_raw_data) as usize);
            assert!(section.virtual_address as usize + len <= size);
            // SAFETY: inside the region.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    raw.as_ptr(),
                    (base + section.virtual_address as usize) as *mut u8,
                    len,
                );
            }
        }
        let mapped = map.slice_mut();
        let pe_at = u32_at(image, 0x3c) as usize;
        let optional = pe_at + 24;
        assert_eq!(u16_at(image, optional), 0x20b, "a PE32+ image");
        let dir = |i: usize| {
            let at = optional + 112 + i * 8;
            (u32_at(image, at) as usize, u32_at(image, at + 4) as usize)
        };
        // Base relocations: every DIR64 entry moves by the delta.
        let delta = (base as u64).wrapping_sub(pe.image_base);
        let (reloc, reloc_len) = dir(5);
        let mut at = reloc;
        while at < reloc + reloc_len {
            let page = u32_at(mapped, at) as usize;
            let block = u32_at(mapped, at + 4) as usize;
            assert!(block >= 8);
            for e in 0..(block - 8) / 2 {
                let entry = u16_at(mapped, at + 8 + e * 2);
                match entry >> 12 {
                    0 => {}
                    10 => {
                        let rva = page + usize::from(entry & 0xfff);
                        let v = u64_at(mapped, rva).wrapping_add(delta);
                        mapped[rva..rva + 8].copy_from_slice(&v.to_le_bytes());
                    }
                    kind => panic!("a relocation of type {kind}"),
                }
            }
            at += block;
        }
        // Imports.
        let (imports, _) = dir(1);
        let mut desc = imports;
        loop {
            let names = u32_at(mapped, desc) as usize;
            let name = u32_at(mapped, desc + 12) as usize;
            let slots = u32_at(mapped, desc + 16) as usize;
            if name == 0 {
                break;
            }
            let dll = cstr(mapped, name);
            let module = if resolvable(&dll) {
                let c = CString::new(dll.clone()).unwrap();
                // SAFETY: a system DLL of the C runtime, by name.
                unsafe { LoadLibraryA(c.as_ptr().cast()) }
            } else {
                std::ptr::null_mut()
            };
            let lookup = if names != 0 { names } else { slots };
            let mut i = 0;
            loop {
                let thunk = u64_at(mapped, lookup + i * 8);
                if thunk == 0 {
                    break;
                }
                let mut to = unresolved_import as *const () as usize;
                if !module.is_null() {
                    let found = if thunk >> 63 == 1 {
                        // SAFETY: an ordinal of a loaded module.
                        unsafe { GetProcAddress(module, (thunk & 0xffff) as usize as *const u8) }
                    } else {
                        let f =
                            CString::new(cstr(mapped, (thunk as usize & 0x7fff_ffff) + 2)).unwrap();
                        // SAFETY: a name of a loaded module.
                        unsafe { GetProcAddress(module, f.as_ptr().cast()) }
                    };
                    if let Some(f) = found {
                        to = f as usize;
                    }
                }
                let slot = slots + i * 8;
                mapped[slot..slot + 8].copy_from_slice(&(to as u64).to_le_bytes());
                i += 1;
            }
            desc += 20;
        }
        map
    }

    #[allow(clippy::mut_from_ref)]
    fn slice_mut(&self) -> &mut [u8] {
        // SAFETY: the region is the mapping's own.
        unsafe { std::slice::from_raw_parts_mut(self.base as *mut u8, self.size) }
    }

    /// The address of `rva` in the mapping.
    pub fn at(&self, rva: u64) -> usize {
        assert!((rva as usize) < self.size);
        self.base + rva as usize
    }

    /// Overwrites the function at `rva` with a jump to `to`.
    pub fn jump(&self, rva: u64, to: usize) {
        let mut code = vec![0xFF, 0x25, 0, 0, 0, 0];
        code.extend_from_slice(&(to as u64).to_le_bytes());
        let at = rva as usize;
        self.slice_mut()[at..at + code.len()].copy_from_slice(&code);
    }

    /// The pointer-sized value at `rva` (an import slot, a vtable entry).
    pub fn pointer(&self, rva: u64) -> usize {
        u64_at(self.slice_mut(), rva as usize) as usize
    }
}

fn cstr(b: &[u8], at: usize) -> String {
    let end = b[at..].iter().position(|&c| c == 0).unwrap();
    String::from_utf8_lossy(&b[at..at + end]).into_owned()
}

impl Drop for Mapped {
    fn drop(&mut self) {
        // SAFETY: the region VirtualAlloc gave; nothing runs in it now.
        unsafe { VirtualFree(self.base as *mut _, 0, MEM_RELEASE) };
    }
}

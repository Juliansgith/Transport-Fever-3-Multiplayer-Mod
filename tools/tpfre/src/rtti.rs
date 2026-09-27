//! MSVC x64 RTTI: type descriptors, complete object locators and vtables.
//!
//! * A type descriptor is `{ vfptr: u64, spare: u64 (0), name: ".?AV...@@" }`.
//! * A complete object locator (COL) is `{ signature: 1, offset, cdOffset,
//!   pTypeDescriptor, pClassDescriptor, pSelf }` with image-relative fields;
//!   `pSelf` equal to its own RVA makes it unmistakable.
//! * A vtable is preceded by an absolute pointer to its COL; its slots are
//!   absolute pointers into code, up to the first that is not.
//!
//! Slot `N` of the vtable of class `C` is named `C::vfN` (`C::vfN@0x10` for
//! the vtable of a secondary base at offset 0x10). A slot's function may be
//! shared by many classes (inherited or folded); the name records that.

use std::collections::{BTreeMap, HashMap};

use crate::pe::Pe;

#[derive(Debug, Clone)]
pub struct TypeDesc {
    pub rva: u32,
    pub mangled: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Vtable {
    pub rva: u32,
    pub col: u32,
    pub td: u32,
    pub class: String,
    pub offset: u32,
    pub slots: Vec<u32>,
}

#[derive(Debug, Default)]
pub struct Rtti {
    pub types: Vec<TypeDesc>,
    pub cols: usize,
    pub vtables: Vec<Vtable>,
}

impl Vtable {
    pub fn slot_name(&self, slot: usize) -> String {
        if self.offset == 0 {
            format!("{}::vf{}", self.class, slot)
        } else {
            format!("{}::vf{}@0x{:x}", self.class, slot, self.offset)
        }
    }
    pub fn label(&self) -> String {
        if self.offset == 0 {
            format!("{}::vftable", self.class)
        } else {
            format!("{}::vftable@0x{:x}", self.class, self.offset)
        }
    }
}

/// `.?AVFoo@ns@@` -> `ns::Foo`; templates and other shapes through the MSVC
/// demangler; the mangled name itself when neither works.
pub fn demangle_type(mangled: &str) -> String {
    let Some(body) = mangled
        .strip_prefix(".?AV")
        .or_else(|| mangled.strip_prefix(".?AU"))
    else {
        return mangled.to_owned();
    };
    if !body.contains('?')
        && !body.contains('$')
        && let Some(inner) = body.strip_suffix("@@")
    {
        let parts: Vec<&str> = inner.split('@').collect();
        if parts.iter().all(|p| !p.is_empty()) {
            return parts.into_iter().rev().collect::<Vec<_>>().join("::");
        }
    }
    let sym = format!("??_7{body}6B@");
    if let Ok(s) = msvc_demangler::demangle(&sym, msvc_demangler::DemangleFlags::llvm()) {
        let s = s.strip_prefix("const ").unwrap_or(&s);
        if let Some(c) = s.strip_suffix("::`vftable'") {
            return c.trim().to_owned();
        }
    }
    mangled.to_owned()
}

pub fn parse(pe: &Pe) -> Rtti {
    let data_secs: Vec<_> = pe
        .sections
        .iter()
        .filter(|s| !s.exec() && s.raw_size > 0)
        .collect();

    // Type descriptors, by name.
    let mut types: BTreeMap<u32, TypeDesc> = BTreeMap::new();
    for s in &data_secs {
        let blob = pe.section_bytes(s);
        for pos in memchr::memmem::find_iter(blob, b".?A") {
            if pos < 16 {
                continue;
            }
            let rva = s.rva + pos as u32 - 16;
            if !rva.is_multiple_of(8) || pe.u64_at(rva + 8) != Some(0) {
                continue;
            }
            let Some(name) = pe.cstr(rva + 16, 4096) else {
                continue;
            };
            if name.len() < 5 || name.bytes().any(|b| !(0x21..=0x7e).contains(&b)) {
                continue;
            }
            types.insert(
                rva,
                TypeDesc {
                    rva,
                    name: demangle_type(&name),
                    mangled: name,
                },
            );
        }
    }

    // Complete object locators: signature 1 and pSelf == own RVA.
    let mut cols: HashMap<u32, (u32, u32)> = HashMap::new(); // rva -> (offset, td)
    for s in &data_secs {
        let blob = pe.section_bytes(s);
        let first = (4 - (s.rva % 4)) % 4;
        let mut p = first as usize;
        while p + 24 <= blob.len() {
            if blob[p..p + 4] == [1, 0, 0, 0] {
                let rva = s.rva + p as u32;
                let rd = |k: usize| {
                    u32::from_le_bytes([
                        blob[p + k],
                        blob[p + k + 1],
                        blob[p + k + 2],
                        blob[p + k + 3],
                    ])
                };
                if rd(20) == rva && types.contains_key(&rd(12)) {
                    cols.insert(rva, (rd(4), rd(12)));
                }
            }
            p += 4;
        }
    }

    // Vtables: an absolute pointer to a COL, then code pointers.
    let mut vtables = Vec::new();
    for s in &data_secs {
        let blob = pe.section_bytes(s);
        let first = (8 - (s.rva % 8)) % 8;
        let mut p = first as usize;
        while p + 16 <= blob.len() {
            let mut q = [0u8; 8];
            q.copy_from_slice(&blob[p..p + 8]);
            let v = u64::from_le_bytes(q);
            p += 8;
            let Some(col) = pe.va_to_rva(v) else {
                continue;
            };
            let Some(&(offset, td)) = cols.get(&col) else {
                continue;
            };
            let vt = s.rva + p as u32;
            let mut slots = Vec::new();
            let mut at = vt;
            while slots.len() < 4096 {
                let Some(ptr) = pe.u64_at(at) else {
                    break;
                };
                let Some(target) = pe.va_to_rva(ptr) else {
                    break;
                };
                match pe.section_of(target) {
                    Some(sec) if sec.exec() && !sec.packed() => {}
                    _ => break,
                }
                slots.push(target);
                at += 8;
            }
            if slots.is_empty() {
                continue;
            }
            let class = types.get(&td).map(|t| t.name.clone()).unwrap_or_default();
            vtables.push(Vtable {
                rva: vt,
                col,
                td,
                class,
                offset,
                slots,
            });
        }
    }
    vtables.sort_by_key(|v| v.rva);
    Rtti {
        types: types.into_values().collect(),
        cols: cols.len(),
        vtables,
    }
}

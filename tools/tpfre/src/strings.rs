//! String extraction from the image's data sections.
//!
//! ASCII strings are NUL-terminated runs of at least four printable bytes
//! (plus tab, CR and LF) that start on a NUL boundary, the rule
//! `tools/re/tpfbin.py` uses for real literals. UTF-16LE strings are the same
//! shape in 16-bit units at even addresses. Strings that code references but
//! that do not start on a boundary (a literal merged into the tail of
//! another) are added afterwards by [`referenced_string`].

use crate::pe::Pe;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Str {
    pub rva: u32,
    /// 1 = ASCII, 2 = UTF-16LE.
    pub enc: u8,
    pub text: String,
    /// Starts on a NUL boundary (the rule for assert-string classification).
    pub boundary: bool,
}

pub const MIN_LEN: usize = 4;

fn printable(b: u8) -> bool {
    (0x20..=0x7e).contains(&b) || b == b'\t' || b == b'\n' || b == b'\r'
}

/// Every string in the image's non-executable readable sections.
pub fn extract(pe: &Pe) -> Vec<Str> {
    use rayon::prelude::*;
    let secs: Vec<_> = pe
        .sections
        .iter()
        .filter(|s| !s.exec() && s.readable() && s.raw_size > 0)
        .collect();
    let mut all: Vec<Str> = secs
        .par_iter()
        .flat_map_iter(|s| {
            let blob = pe.section_bytes(s);
            let mut v = ascii(blob, s.rva);
            v.extend(utf16(blob, s.rva));
            v
        })
        .collect();
    all.sort_by_key(|s| (s.rva, s.enc));
    all.dedup_by_key(|s| s.rva);
    all
}

fn ascii(blob: &[u8], base: u32) -> Vec<Str> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let n = blob.len();
    while i < n {
        if !printable(blob[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && printable(blob[i]) {
            i += 1;
        }
        if i < n && blob[i] == 0 && i - start >= MIN_LEN && (start == 0 || blob[start - 1] == 0) {
            // Only printable bytes: the text is valid ASCII.
            let text = String::from_utf8_lossy(&blob[start..i]).into_owned();
            out.push(Str {
                rva: base + start as u32,
                enc: 1,
                text,
                boundary: true,
            });
        }
    }
    out
}

fn utf16(blob: &[u8], base: u32) -> Vec<Str> {
    let mut out = Vec::new();
    let units = blob.len() / 2;
    let unit = |k: usize| u16::from_le_bytes([blob[2 * k], blob[2 * k + 1]]);
    let ok = |u: u16| u < 0x80 && printable(u as u8);
    let mut k = 0usize;
    while k < units {
        if !ok(unit(k)) {
            k += 1;
            continue;
        }
        let start = k;
        while k < units && ok(unit(k)) {
            k += 1;
        }
        if k < units && unit(k) == 0 && k - start >= MIN_LEN && (start == 0 || unit(start - 1) == 0)
        {
            let text: String = (start..k).map(|j| unit(j) as u8 as char).collect();
            out.push(Str {
                rva: base + 2 * start as u32,
                enc: 2,
                text,
                boundary: true,
            });
        }
    }
    out
}

/// A string that starts exactly at `rva` (referenced by code), whether or
/// not it starts on a NUL boundary. ASCII first, then UTF-16LE.
pub fn referenced_string(pe: &Pe, rva: u32) -> Option<Str> {
    let s = pe.section_of(rva)?;
    if s.exec() {
        return None;
    }
    let b = pe.slice_from(rva)?;
    let b = &b[..b.len().min(4096)];
    let len = b.iter().take_while(|&&c| printable(c)).count();
    if len >= MIN_LEN && b.get(len) == Some(&0) {
        return Some(Str {
            rva,
            enc: 1,
            text: String::from_utf8_lossy(&b[..len]).into_owned(),
            boundary: false,
        });
    }
    if rva.is_multiple_of(2) {
        let mut text = String::new();
        let mut k = 0;
        while 2 * k + 1 < b.len() {
            let u = u16::from_le_bytes([b[2 * k], b[2 * k + 1]]);
            if u == 0 {
                break;
            }
            if u >= 0x80 || !printable(u as u8) {
                return None;
            }
            text.push(u as u8 as char);
            k += 1;
        }
        if text.len() >= MIN_LEN && 2 * k + 1 < b.len() {
            return Some(Str {
                rva,
                enc: 2,
                text,
                boundary: false,
            });
        }
    }
    None
}

/// Printable-only (0x20..=0x7e): the byte class `tpfbin.py` classifies.
pub fn pure_printable(text: &str) -> bool {
    text.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

//! A read-only PE32+ (x86-64) image: headers, sections, imports, exports,
//! base relocations and the exception directory.
//!
//! Only what the indexer needs is parsed, and every read is bounds-checked:
//! a malformed or truncated image is refused with the reason, never guessed
//! at. Two layouts are accepted:
//!
//! * the file on disk, where a section's bytes sit at `PointerToRawData`;
//! * a raw memory dump of the loaded image (`--image-base`), where every
//!   section sits at its RVA. SteamStub and other packers decrypt `.text`
//!   only in memory, so a dump is how their code is read.
//!
//! ELF and Mach-O are recognised and refused with a message; their function
//! starts would come from symbols and `.eh_frame` (a TODO, see README).

use anyhow::{Context, Result, bail};

pub const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
pub const IMAGE_SCN_MEM_READ: u32 = 0x4000_0000;
pub const IMAGE_SCN_MEM_WRITE: u32 = 0x8000_0000;

/// Entropy (bits per byte) above which an executable section is treated as
/// packed or encrypted.
pub const PACKED_ENTROPY: f64 = 7.2;

#[derive(Debug, Clone)]
pub struct Section {
    pub name: String,
    pub rva: u32,
    pub vsize: u32,
    /// Offset of the section's bytes in the input (file or dump).
    pub raw_off: usize,
    /// Bytes of the section present in the input (clipped to its length).
    pub raw_size: usize,
    pub characteristics: u32,
    pub entropy: f64,
}

impl Section {
    pub fn exec(&self) -> bool {
        self.characteristics & IMAGE_SCN_MEM_EXECUTE != 0
    }
    pub fn readable(&self) -> bool {
        self.characteristics & IMAGE_SCN_MEM_READ != 0
    }
    pub fn writable(&self) -> bool {
        self.characteristics & IMAGE_SCN_MEM_WRITE != 0
    }
    /// The RVA one past the last byte present in the input.
    pub fn raw_end(&self) -> u32 {
        self.rva.saturating_add(self.raw_size as u32)
    }
    pub fn contains(&self, rva: u32) -> bool {
        rva >= self.rva && rva < self.raw_end()
    }
    pub fn packed(&self) -> bool {
        self.exec() && self.entropy > PACKED_ENTROPY
    }
    pub fn perms(&self) -> String {
        format!(
            "{}{}{}",
            if self.readable() { 'r' } else { '-' },
            if self.writable() { 'w' } else { '-' },
            if self.exec() { 'x' } else { '-' }
        )
    }
}

#[derive(Debug, Clone)]
pub struct Import {
    /// RVA of the IAT slot the code calls through.
    pub slot: u32,
    pub dll: String,
    pub name: String,
    pub delay: bool,
}

#[derive(Debug, Clone)]
pub struct Export {
    pub rva: u32,
    pub name: String,
}

/// One `.pdata` RUNTIME_FUNCTION.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeFunction {
    pub begin: u32,
    pub end: u32,
    pub unwind: u32,
}

pub struct Pe<'a> {
    pub data: &'a [u8],
    /// True when `data` is a memory dump (sections at their RVAs).
    pub dump: bool,
    /// The base absolute pointers in the image are relative to: the header's
    /// `ImageBase` for a file, the load address for a dump.
    pub image_base: u64,
    pub header_image_base: u64,
    pub size_of_image: u32,
    pub entry: u32,
    pub timestamp: u32,
    pub sections: Vec<Section>,
    dirs: Vec<(u32, u32)>,
}

pub enum Format {
    Pe,
    Elf,
    MachO,
    Unknown,
}

pub fn detect_format(data: &[u8]) -> Format {
    if data.len() >= 0x40 && &data[..2] == b"MZ" {
        let off = u32::from_le_bytes([data[0x3c], data[0x3d], data[0x3e], data[0x3f]]) as usize;
        if data.get(off..off + 4) == Some(b"PE\0\0") {
            return Format::Pe;
        }
        return Format::Unknown;
    }
    if data.len() >= 4 && &data[..4] == b"\x7fELF" {
        return Format::Elf;
    }
    if data.len() >= 4 {
        let m = [data[0], data[1], data[2], data[3]];
        let macho: [[u8; 4]; 6] = [
            [0xfe, 0xed, 0xfa, 0xce],
            [0xce, 0xfa, 0xed, 0xfe],
            [0xfe, 0xed, 0xfa, 0xcf],
            [0xcf, 0xfa, 0xed, 0xfe],
            [0xca, 0xfe, 0xba, 0xbe],
            [0xbe, 0xba, 0xfe, 0xca],
        ];
        if macho.contains(&m) {
            return Format::MachO;
        }
    }
    Format::Unknown
}

pub fn shannon_entropy(blob: &[u8]) -> f64 {
    if blob.is_empty() {
        return 0.0;
    }
    let mut counts = [0u64; 256];
    for &b in blob {
        counts[b as usize] += 1;
    }
    let n = blob.len() as f64;
    counts
        .iter()
        .filter(|&&c| c != 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

fn rd_u16(d: &[u8], off: usize) -> Option<u16> {
    d.get(off..off + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}
fn rd_u32(d: &[u8], off: usize) -> Option<u32> {
    d.get(off..off + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
fn rd_u64(d: &[u8], off: usize) -> Option<u64> {
    d.get(off..off + 8).map(|b| {
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        u64::from_le_bytes(a)
    })
}

pub const DIR_EXPORT: usize = 0;
pub const DIR_IMPORT: usize = 1;
pub const DIR_EXCEPTION: usize = 3;
pub const DIR_BASERELOC: usize = 5;
pub const DIR_DELAY_IMPORT: usize = 13;

impl<'a> Pe<'a> {
    /// Parse `data`. `dump_base` is `Some(load address)` for a memory dump.
    pub fn parse(data: &'a [u8], dump_base: Option<u64>) -> Result<Self> {
        match detect_format(data) {
            Format::Pe => {}
            Format::Elf => bail!(
                "ELF images are not supported yet: only PE32+ x86-64 is indexed today \
                 (TODO: ELF x86-64 function starts from .symtab/.eh_frame)"
            ),
            Format::MachO => bail!(
                "Mach-O images are not supported: only PE32+ x86-64 is indexed today \
                 (TPF3's macOS build is arm64, which this decoder does not handle)"
            ),
            Format::Unknown => bail!("not a PE, ELF or Mach-O image"),
        }
        let pe_off = rd_u32(data, 0x3c).context("truncated DOS header")? as usize;
        let fh = pe_off + 4;
        let machine = rd_u16(data, fh).context("truncated file header")?;
        if machine != 0x8664 {
            bail!("machine 0x{machine:x} is not x86-64 (0x8664); only PE32+ x86-64 is indexed");
        }
        let nsections = rd_u16(data, fh + 2).context("truncated file header")? as usize;
        let timestamp = rd_u32(data, fh + 4).context("truncated file header")?;
        let opt_size = rd_u16(data, fh + 16).context("truncated file header")? as usize;
        let opt = fh + 20;
        let magic = rd_u16(data, opt).context("truncated optional header")?;
        if magic != 0x20b {
            bail!("optional header magic 0x{magic:x} is not PE32+ (0x20b)");
        }
        let entry = rd_u32(data, opt + 16).context("truncated optional header")?;
        let header_image_base = rd_u64(data, opt + 24).context("truncated optional header")?;
        let size_of_image = rd_u32(data, opt + 56).context("truncated optional header")?;
        let ndirs = rd_u32(data, opt + 108).context("truncated optional header")? as usize;
        let mut dirs = Vec::new();
        for i in 0..ndirs.min(16) {
            let at = opt + 112 + i * 8;
            if at + 8 > opt + opt_size {
                break;
            }
            let rva = rd_u32(data, at).context("truncated data directory")?;
            let size = rd_u32(data, at + 4).context("truncated data directory")?;
            dirs.push((rva, size));
        }
        let sec_table = opt + opt_size;
        let mut sections = Vec::with_capacity(nsections);
        for i in 0..nsections {
            let s = sec_table + i * 40;
            let raw_name = data.get(s..s + 8).context("truncated section table")?;
            let name_len = raw_name.iter().position(|&b| b == 0).unwrap_or(8);
            let name = String::from_utf8_lossy(&raw_name[..name_len]).into_owned();
            let vsize = rd_u32(data, s + 8).context("truncated section table")?;
            let rva = rd_u32(data, s + 12).context("truncated section table")?;
            let raw_size_hdr = rd_u32(data, s + 16).context("truncated section table")? as usize;
            let raw_ptr = rd_u32(data, s + 20).context("truncated section table")? as usize;
            let characteristics = rd_u32(data, s + 36).context("truncated section table")?;
            let (raw_off, raw_size) = if dump_base.is_some() {
                // In memory a section spans its virtual size from its RVA.
                let off = rva as usize;
                let size = (vsize as usize).min(data.len().saturating_sub(off));
                (off, size)
            } else {
                let size = raw_size_hdr.min(data.len().saturating_sub(raw_ptr));
                // Bytes past the virtual size are file alignment padding.
                let size = if vsize != 0 {
                    size.min(vsize as usize)
                } else {
                    size
                };
                (raw_ptr, size)
            };
            // A section whose bytes start past the end of the input has none.
            let raw_off = raw_off.min(data.len());
            sections.push(Section {
                name,
                rva,
                vsize,
                raw_off,
                raw_size,
                characteristics,
                entropy: 0.0,
            });
        }
        if sections.is_empty() {
            bail!("the image has no sections");
        }
        Ok(Pe {
            data,
            dump: dump_base.is_some(),
            image_base: dump_base.unwrap_or(header_image_base),
            header_image_base,
            size_of_image,
            entry,
            timestamp,
            sections,
            dirs,
        })
    }

    /// Entropy of every section, computed in parallel.
    pub fn compute_entropy(&mut self) {
        use rayon::prelude::*;
        let data = self.data;
        let ents: Vec<f64> = self
            .sections
            .par_iter()
            .map(|s| shannon_entropy(&data[s.raw_off..s.raw_off + s.raw_size]))
            .collect();
        for (s, e) in self.sections.iter_mut().zip(ents) {
            s.entropy = e;
        }
    }

    pub fn dir(&self, i: usize) -> Option<(u32, u32)> {
        self.dirs
            .get(i)
            .copied()
            .filter(|&(rva, size)| rva != 0 && size != 0)
    }

    pub fn section_of(&self, rva: u32) -> Option<&Section> {
        self.sections.iter().find(|s| s.contains(rva))
    }

    pub fn section_index(&self, rva: u32) -> Option<usize> {
        self.sections.iter().position(|s| s.contains(rva))
    }

    pub fn is_exec(&self, rva: u32) -> bool {
        self.section_of(rva).is_some_and(|s| s.exec())
    }

    /// The input bytes at `rva`, up to the end of its section's bytes.
    pub fn slice_from(&self, rva: u32) -> Option<&'a [u8]> {
        let s = self.section_of(rva)?;
        let off = s.raw_off + (rva - s.rva) as usize;
        self.data.get(off..s.raw_off + s.raw_size)
    }

    /// Exactly `len` bytes at `rva`, all inside one section.
    pub fn read(&self, rva: u32, len: usize) -> Option<&'a [u8]> {
        self.slice_from(rva).and_then(|b| b.get(..len))
    }

    pub fn section_bytes(&self, s: &Section) -> &'a [u8] {
        &self.data[s.raw_off..s.raw_off + s.raw_size]
    }

    pub fn u32_at(&self, rva: u32) -> Option<u32> {
        self.read(rva, 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u64_at(&self, rva: u32) -> Option<u64> {
        self.read(rva, 8).map(|b| {
            let mut a = [0u8; 8];
            a.copy_from_slice(b);
            u64::from_le_bytes(a)
        })
    }

    /// A NUL-terminated ASCII string at `rva` (at most `max` bytes).
    pub fn cstr(&self, rva: u32, max: usize) -> Option<String> {
        let b = self.slice_from(rva)?;
        let b = &b[..b.len().min(max)];
        let end = b.iter().position(|&c| c == 0)?;
        std::str::from_utf8(&b[..end]).ok().map(str::to_owned)
    }

    /// An absolute pointer (VA) converted to an RVA inside the image.
    pub fn va_to_rva(&self, va: u64) -> Option<u32> {
        let rva = va.checked_sub(self.image_base)?;
        (rva < self.size_of_image as u64).then_some(rva as u32)
    }

    /// `.pdata` entries, in file order.
    pub fn runtime_functions(&self) -> Vec<RuntimeFunction> {
        let Some((rva, size)) = self.dir(DIR_EXCEPTION) else {
            return Vec::new();
        };
        let Some(b) = self.read(rva, size as usize) else {
            return Vec::new();
        };
        b.as_chunks::<12>()
            .0
            .iter()
            .map(|e| RuntimeFunction {
                begin: u32::from_le_bytes([e[0], e[1], e[2], e[3]]),
                end: u32::from_le_bytes([e[4], e[5], e[6], e[7]]),
                unwind: u32::from_le_bytes([e[8], e[9], e[10], e[11]]),
            })
            .filter(|r| r.begin != 0 && r.end > r.begin)
            .collect()
    }

    /// Follow chained unwind info from a RUNTIME_FUNCTION to the function it
    /// continues, as `tools/re/tpfbin.py` does: `UNW_FLAG_CHAININFO`, or the
    /// indirection form (unwind RVA with bit 0 set). Returns the primary
    /// function's begin.
    pub fn primary_of(&self, rf: RuntimeFunction) -> u32 {
        let (mut cur_b, mut cur_u) = (rf.begin, rf.unwind);
        for _ in 0..64 {
            if cur_u & 1 != 0 {
                match (self.u32_at(cur_u - 1), self.u32_at(cur_u + 7)) {
                    (Some(b), Some(u)) => {
                        cur_b = b;
                        cur_u = u;
                        continue;
                    }
                    _ => break,
                }
            }
            let Some(hdr) = self.read(cur_u, 4) else {
                break;
            };
            let flags = hdr[0] >> 3;
            let count = hdr[2] as u32;
            if flags & 0x4 != 0 {
                let p = cur_u + 4 + ((count + 1) & !1) * 2;
                match (self.u32_at(p), self.u32_at(p + 8)) {
                    (Some(b), Some(u)) => {
                        cur_b = b;
                        cur_u = u;
                        continue;
                    }
                    _ => break,
                }
            }
            break;
        }
        cur_b
    }

    pub fn imports(&self) -> Vec<Import> {
        let mut out = Vec::new();
        if let Some((rva, _)) = self.dir(DIR_IMPORT) {
            let mut d = rva;
            for _ in 0..4096 {
                let (Some(oft), Some(name), Some(ft)) =
                    (self.u32_at(d), self.u32_at(d + 12), self.u32_at(d + 16))
                else {
                    break;
                };
                if oft == 0 && name == 0 && ft == 0 {
                    break;
                }
                let dll = self.cstr(name, 256).unwrap_or_else(|| "?".into());
                let int = if oft != 0 { oft } else { ft };
                self.thunks(&dll, int, ft, false, &mut out);
                d += 20;
            }
        }
        if let Some((rva, _)) = self.dir(DIR_DELAY_IMPORT) {
            let mut d = rva;
            for _ in 0..4096 {
                let (Some(attrs), Some(name), Some(iat), Some(int)) = (
                    self.u32_at(d),
                    self.u32_at(d + 4),
                    self.u32_at(d + 12),
                    self.u32_at(d + 16),
                ) else {
                    break;
                };
                if name == 0 && iat == 0 {
                    break;
                }
                // Old-style descriptors hold VAs rather than RVAs.
                let fix = |v: u32| -> u32 {
                    if attrs & 1 == 0 {
                        (v as u64).wrapping_sub(self.header_image_base) as u32
                    } else {
                        v
                    }
                };
                let dll = self.cstr(fix(name), 256).unwrap_or_else(|| "?".into());
                self.thunks(&dll, fix(int), fix(iat), true, &mut out);
                d += 32;
            }
        }
        out
    }

    fn thunks(&self, dll: &str, int: u32, iat: u32, delay: bool, out: &mut Vec<Import>) {
        for i in 0..65536u32 {
            let Some(t) = self.u64_at(int + i * 8) else {
                break;
            };
            if t == 0 {
                break;
            }
            let name = if t & (1 << 63) != 0 {
                format!("#{}", t & 0xffff)
            } else {
                self.cstr((t as u32).wrapping_add(2), 512)
                    .unwrap_or_else(|| format!("?{t:x}"))
            };
            out.push(Import {
                slot: iat + i * 8,
                dll: dll.to_owned(),
                name,
                delay,
            });
        }
    }

    pub fn exports(&self) -> Vec<Export> {
        let Some((rva, size)) = self.dir(DIR_EXPORT) else {
            return Vec::new();
        };
        let (Some(nfuncs), Some(nnames), Some(afuncs), Some(anames), Some(aords)) = (
            self.u32_at(rva + 20),
            self.u32_at(rva + 24),
            self.u32_at(rva + 28),
            self.u32_at(rva + 32),
            self.u32_at(rva + 36),
        ) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for i in 0..nnames.min(1 << 20) {
            let (Some(np), Some(ord)) = (
                self.u32_at(anames + i * 4),
                self.read(aords + i * 2, 2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]])),
            ) else {
                break;
            };
            if ord as u32 >= nfuncs {
                continue;
            }
            let Some(f) = self.u32_at(afuncs + ord as u32 * 4) else {
                continue;
            };
            if f >= rva && f < rva + size {
                continue; // a forwarder string, not code
            }
            if let Some(name) = self.cstr(np, 512) {
                out.push(Export { rva: f, name });
            }
        }
        out
    }

    /// Every DIR64 base relocation: the RVAs that hold absolute pointers.
    pub fn relocations(&self) -> Vec<u32> {
        let Some((rva, size)) = self.dir(DIR_BASERELOC) else {
            return Vec::new();
        };
        let Some(b) = self.read(rva, size as usize) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut p = 0usize;
        while p + 8 <= b.len() {
            let page = u32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]]);
            let bsize = u32::from_le_bytes([b[p + 4], b[p + 5], b[p + 6], b[p + 7]]) as usize;
            if bsize < 8 || p + bsize > b.len() {
                break;
            }
            for e in b[p + 8..p + bsize].as_chunks::<2>().0 {
                let v = u16::from_le_bytes([e[0], e[1]]);
                if v >> 12 == 10 {
                    out.push(page + (v & 0xfff) as u32);
                }
            }
            p += bsize;
        }
        out.sort_unstable();
        out
    }
}

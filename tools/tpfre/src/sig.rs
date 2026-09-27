//! Byte signatures and pattern search, in the form `tools/re/make_profile.py`
//! writes and `tpf3mp-hookcore` reads: uppercase hex bytes separated by
//! spaces, `??` for a wildcard.
//!
//! The rules are make_profile's, with iced-x86 in place of capstone:
//!
//! * every relative branch or call target, every RIP-relative displacement,
//!   and every immediate or absolute displacement whose value is an address
//!   inside the image (at its preferred base) becomes `??`;
//! * the signature starts as the instructions covering at least `steal`
//!   bytes (14, the detour engine's far jump) and grows one instruction at a
//!   time until it matches exactly once in the section holding the
//!   function; it never runs past the function or `max_length` bytes;
//! * the prologue is the exact bytes of those first instructions, which must
//!   all be straight-line code (iced `FlowControl::Next`, as the engine's
//!   `decode_prologue` requires). With `prologue: false` the signature starts
//!   at one instruction and no prologue is required.
//!
//! Anything that cannot meet the rules is refused with the reason.

use iced_x86::{Decoder, DecoderOptions, FlowControl, Instruction, OpKind, Register};

use crate::pe::{Pe, Section};

pub const FAR_STEAL: usize = 14;
pub const NEAR_STEAL: usize = 5;
pub const DEFAULT_MAX_LENGTH: usize = 128;

#[derive(Debug, Clone)]
pub struct Options {
    pub steal: usize,
    pub max_length: usize,
    /// Require a stealable prologue (make_profile's rule).
    pub prologue: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            steal: FAR_STEAL,
            max_length: DEFAULT_MAX_LENGTH,
            prologue: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    pub section: String,
    pub signature: String,
    pub prologue: Option<String>,
    pub sig_len: usize,
    pub prologue_len: usize,
}

struct Insn {
    raw: Vec<u8>,
    mask: Vec<bool>,
    text: String,
    straight: bool,
}

pub fn hex(raw: &[u8], mask: Option<&[bool]>) -> String {
    raw.iter()
        .enumerate()
        .map(|(i, b)| {
            if mask.is_some_and(|m| !m[i]) {
                "??".to_owned()
            } else {
                format!("{b:02X}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn section_of<'p>(pe: &'p Pe, rva: u32) -> Result<&'p Section, String> {
    pe.sections
        .iter()
        .find(|s| s.exec() && s.contains(rva))
        .ok_or_else(|| {
            format!("RVA 0x{rva:x} is not in the on-disk bytes of an executable section")
        })
}

fn decode_one(pe: &Pe, code: &[u8], rva: u32) -> Option<Insn> {
    let mut d = Decoder::with_ip(64, code, rva as u64, DecoderOptions::NONE);
    if !d.can_decode() {
        return None;
    }
    let mut instr = Instruction::default();
    d.decode_out(&mut instr);
    if instr.is_invalid() {
        return None;
    }
    let len = instr.len();
    let raw = code[..len].to_vec();
    let mut mask = vec![true; len];
    let co = d.get_constant_offsets(&instr);
    // Absolute addresses as the bytes hold them: at the preferred base on
    // disk, at the load address in a dump.
    let lo = pe.image_base;
    let hi = lo + pe.size_of_image as u64;
    let is_address = |v: u64| lo != 0 && v >= lo && v < hi;
    let mut wild = |off: usize, size: usize| {
        for m in mask.iter_mut().skip(off).take(size) {
            *m = false;
        }
    };
    for i in 0..instr.op_count() {
        match instr.op_kind(i) {
            OpKind::NearBranch16 | OpKind::NearBranch32 | OpKind::NearBranch64 => {
                if co.has_immediate() {
                    wild(co.immediate_offset(), co.immediate_size());
                }
            }
            OpKind::Immediate8
            | OpKind::Immediate16
            | OpKind::Immediate32
            | OpKind::Immediate64
            | OpKind::Immediate8to16
            | OpKind::Immediate8to32
            | OpKind::Immediate8to64
            | OpKind::Immediate32to64 => {
                if is_address(instr.immediate(i)) && co.has_immediate() {
                    wild(co.immediate_offset(), co.immediate_size());
                }
            }
            OpKind::Memory => {
                let rip = instr.memory_base() == Register::RIP;
                let absolute = instr.memory_base() == Register::None
                    && instr.memory_index() == Register::None
                    && is_address(instr.memory_displacement64());
                if (rip || absolute) && co.has_displacement() {
                    wild(co.displacement_offset(), co.displacement_size());
                }
            }
            _ => {}
        }
    }
    Some(Insn {
        raw,
        mask,
        text: format!("{:?}", instr.mnemonic()).to_lowercase(),
        straight: instr.flow_control() == FlowControl::Next,
    })
}

/// Every offset in `blob` where the masked pattern matches.
pub fn find_all(blob: &[u8], raw: &[u8], mask: &[bool]) -> Vec<usize> {
    // Anchor on the longest run of fixed bytes.
    let (mut best, mut best_len, mut cur, mut cur_len) = (0usize, 0usize, 0usize, 0usize);
    for (i, &keep) in mask.iter().enumerate() {
        if keep {
            if cur_len == 0 {
                cur = i;
            }
            cur_len += 1;
            if cur_len > best_len {
                best = cur;
                best_len = cur_len;
            }
        } else {
            cur_len = 0;
        }
    }
    let n = raw.len();
    let matches_at = |c: usize| {
        c + n <= blob.len()
            && raw
                .iter()
                .zip(mask)
                .enumerate()
                .all(|(k, (&b, &keep))| !keep || blob[c + k] == b)
    };
    if best_len == 0 {
        return (0..blob.len().saturating_sub(n) + 1)
            .filter(|&c| matches_at(c))
            .collect();
    }
    let needle = &raw[best..best + best_len];
    memchr::memmem::find_iter(blob, needle)
        .filter_map(|p| p.checked_sub(best))
        .filter(|&c| matches_at(c))
        .collect()
}

/// A unique signature for the function at `rva`. `size` bounds it to the
/// function's own bytes when known.
pub fn build(
    pe: &Pe,
    rva: u32,
    size: Option<u32>,
    name: &str,
    opts: &Options,
) -> Result<Built, String> {
    if opts.steal < NEAR_STEAL {
        return Err(format!(
            "--steal must be at least {NEAR_STEAL} (the near jump)"
        ));
    }
    let section = section_of(pe, rva)?;
    let blob = pe.section_bytes(section);
    let index = (rva - section.rva) as usize;
    let mut limit = blob.len() - index;
    if let Some(s) = size.filter(|&s| s > 0) {
        limit = limit.min(s as usize);
    }
    limit = limit.min(opts.max_length);
    let place = format!("{name} at RVA 0x{rva:x}");

    let mut insns: Vec<Insn> = Vec::new();
    let mut covered = 0usize;
    let decode_next = |insns: &mut Vec<Insn>, covered: &mut usize| -> Result<usize, String> {
        if *covered >= limit {
            return Err(format!(
                "{place}: ran out of bytes after {covered} (function end or --max-length)"
            ));
        }
        let code = &blob[index + *covered..index + limit];
        let found = decode_one(pe, code, rva + *covered as u32).ok_or_else(|| {
            format!(
                "{place}: no instruction decodes at +{covered} within the {limit}-byte limit \
                 (function end or --max-length)"
            )
        })?;
        *covered += found.raw.len();
        insns.push(found);
        Ok(insns.len() - 1)
    };

    let min_prologue = if opts.prologue { opts.steal } else { 1 };
    while covered < min_prologue {
        let i = decode_next(&mut insns, &mut covered)
            .map_err(|e| format!("{e}; the prologue needs {} bytes", opts.steal))?;
        if opts.prologue && !insns[i].straight {
            let before = covered - insns[i].raw.len();
            let hint = if opts.steal > NEAR_STEAL && before >= NEAR_STEAL {
                format!(
                    " (the first {before} bytes would do for a near detour: --steal {NEAR_STEAL})"
                )
            } else {
                String::new()
            };
            return Err(format!(
                "{place}: the prologue must cover {} bytes but `{}` at +{before} is not straight-line \
                 code, which the detour engine refuses to steal{hint}",
                opts.steal, insns[i].text
            ));
        }
    }
    let prologue_len = covered;
    let prologue = &blob[index..index + prologue_len];

    let mut raw: Vec<u8> = insns.iter().flat_map(|i| i.raw.clone()).collect();
    let mut mask: Vec<bool> = insns.iter().flat_map(|i| i.mask.clone()).collect();
    let mut candidates = find_all(blob, &raw, &mask);
    loop {
        if !candidates.contains(&index) {
            return Err(format!(
                "{place}: internal error, the signature does not match itself"
            ));
        }
        if candidates.len() == 1 {
            break;
        }
        let i = match decode_next(&mut insns, &mut covered) {
            Ok(i) => i,
            Err(e) => {
                let others: Vec<String> = candidates
                    .iter()
                    .filter(|&&c| c != index)
                    .take(8)
                    .map(|&c| format!("0x{:x}", section.rva as usize + c))
                    .collect();
                return Err(format!(
                    "{place}: the signature is still not unique ({} matches; also at RVA {}): {e}. \
                     Is the function an identical twin of another? Hook a caller or a different \
                     function instead.",
                    candidates.len(),
                    others.join(", ")
                ));
            }
        };
        let start = raw.len();
        raw.extend_from_slice(&insns[i].raw);
        mask.extend_from_slice(&insns[i].mask);
        let (r, m) = (&insns[i].raw, &insns[i].mask);
        candidates.retain(|&c| {
            r.iter()
                .zip(m)
                .enumerate()
                .all(|(k, (&b, &keep))| !keep || blob.get(c + start + k) == Some(&b))
        });
    }
    Ok(Built {
        section: section.name.clone(),
        signature: hex(&raw, Some(&mask)),
        prologue: opts.prologue.then(|| hex(prologue, None)),
        sig_len: raw.len(),
        prologue_len,
    })
}

/// Parse `48 8B ?? 05` (or `488B??05`) into bytes and a keep-mask.
pub fn parse_pattern(text: &str) -> Result<(Vec<u8>, Vec<bool>), String> {
    let tokens: Vec<String> = if text.contains(char::is_whitespace) {
        text.split_whitespace().map(str::to_owned).collect()
    } else {
        if !text.len().is_multiple_of(2) {
            return Err(format!("pattern {text:?} has an odd number of hex digits"));
        }
        text.as_bytes()
            .chunks(2)
            .map(|c| String::from_utf8_lossy(c).into_owned())
            .collect()
    };
    let mut raw = Vec::new();
    let mut mask = Vec::new();
    for t in tokens {
        if t == "?" || t == "??" {
            raw.push(0);
            mask.push(false);
        } else if t.len() == 2 {
            let b = u8::from_str_radix(&t, 16)
                .map_err(|_| format!("token {t:?} is not two hex digits or ??"))?;
            raw.push(b);
            mask.push(true);
        } else {
            return Err(format!("token {t:?} is not two hex digits or ??"));
        }
    }
    if raw.is_empty() {
        return Err("empty pattern".into());
    }
    if !mask.iter().any(|&k| k) {
        return Err("pattern is all wildcards, so it would match everywhere".into());
    }
    Ok((raw, mask))
}

/// Every RVA in the image's executable sections where the pattern matches.
pub fn search(pe: &Pe, raw: &[u8], mask: &[bool]) -> Vec<u32> {
    let mut out = Vec::new();
    for s in pe.sections.iter().filter(|s| s.exec()) {
        let blob = pe.section_bytes(s);
        out.extend(
            find_all(blob, raw, mask)
                .into_iter()
                .map(|c| s.rva + c as u32),
        );
    }
    out
}

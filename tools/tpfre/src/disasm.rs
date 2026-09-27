//! Function bounds and full linear disassembly.
//!
//! Function bounds come from `.pdata` first: every x64 function that touches
//! the stack has a RUNTIME_FUNCTION, and chained entries are collapsed onto
//! the function they continue (as `tools/re/tpfbin.py` does), keeping each
//! entry as a chunk. Leaf functions have no `.pdata` entry; they are found as
//! targets of direct calls, tail jumps, `lea` of code, vtable slots and
//! relocated data pointers, and bounded by following their control flow up
//! to the next known function.
//!
//! Each function is decoded linearly, chunk by chunk, with iced-x86. MSVC
//! puts switch jump tables in `.text` after a function's code, so a
//! base+index*4 (or *1) operand whose displacement lands later in the same
//! chunk marks a table; the sweep skips it instead of decoding data as code.

use iced_x86::{
    Decoder, DecoderOptions, FlowControl, Instruction, InstructionInfoFactory,
    InstructionInfoOptions, Mnemonic, OpAccess, OpKind, Register,
};
use std::collections::{BTreeMap, HashSet};

use crate::pe::Pe;

pub const CALL_DIRECT: u8 = 0;
pub const CALL_TAIL: u8 = 1;
pub const CALL_IMPORT: u8 = 2;
pub const CALL_IMPORT_JMP: u8 = 3;

pub fn call_kind_name(k: u8) -> &'static str {
    match k {
        CALL_DIRECT => "call",
        CALL_TAIL => "tail",
        CALL_IMPORT => "import",
        CALL_IMPORT_JMP => "import-jmp",
        _ => "?",
    }
}

pub const XREF_ADDR: u8 = 0;
pub const XREF_READ: u8 = 1;
pub const XREF_WRITE: u8 = 2;
pub const XREF_RW: u8 = 3;
pub const XREF_ICALL: u8 = 4;
pub const XREF_IJMP: u8 = 5;
pub const XREF_MEM: u8 = 6;

pub fn xref_kind_name(k: u8) -> &'static str {
    match k {
        XREF_ADDR => "addr",
        XREF_READ => "read",
        XREF_WRITE => "write",
        XREF_RW => "rw",
        XREF_ICALL => "icall",
        XREF_IJMP => "ijmp",
        XREF_MEM => "mem",
        _ => "?",
    }
}

#[derive(Debug, Clone)]
pub struct Func {
    pub start: u32,
    /// Sorted, non-overlapping [start, end) ranges; the first holds `start`.
    pub chunks: Vec<(u32, u32)>,
    pub kind: &'static str,
}

impl Func {
    pub fn contains(&self, a: u32) -> bool {
        self.chunks.iter().any(|&(s, e)| a >= s && a < e)
    }
    /// One past the highest byte of any chunk (the `.pdata` extent).
    pub fn extent_end(&self) -> u32 {
        self.chunks.iter().map(|c| c.1).max().unwrap_or(self.start)
    }
    /// Bytes of code in all chunks.
    pub fn size(&self) -> u32 {
        self.chunks.iter().map(|c| c.1 - c.0).sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Call {
    pub site: u32,
    pub target: u32,
    pub kind: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Xref {
    pub insn: u32,
    pub target: u32,
    pub kind: u8,
}

#[derive(Debug, Default, Clone)]
pub struct Facts {
    pub calls: Vec<Call>,
    pub xrefs: Vec<Xref>,
    pub ninsn: u32,
    pub invalid: u32,
    pub tables: u32,
}

/// Functions from `.pdata`, with chained entries collapsed onto the primary.
pub fn from_pdata(pe: &Pe) -> Vec<Func> {
    let mut by_primary: BTreeMap<u32, Vec<(u32, u32)>> = BTreeMap::new();
    for rf in pe.runtime_functions() {
        let primary = pe.primary_of(rf);
        by_primary
            .entry(primary)
            .or_default()
            .push((rf.begin, rf.end));
    }
    by_primary
        .into_iter()
        .map(|(start, mut chunks)| {
            chunks.sort_unstable();
            let mut merged: Vec<(u32, u32)> = Vec::with_capacity(chunks.len());
            for (s, e) in chunks {
                match merged.last_mut() {
                    Some(last) if s <= last.1 => last.1 = last.1.max(e),
                    _ => merged.push((s, e)),
                }
            }
            // The chunk holding the entry point comes first.
            if let Some(i) = merged.iter().position(|&(s, e)| start >= s && start < e) {
                let c = merged.remove(i);
                merged.insert(0, c);
            } else {
                // A primary whose own entry is missing: give it a point chunk
                // so it is still a function, bounded by what follows.
                merged.insert(0, (start, start));
            }
            Func {
                start,
                chunks: merged,
                kind: "pdata",
            }
        })
        .collect()
}

/// A sorted map of every chunk to its function, for "which function holds
/// this address".
#[derive(Default, Clone)]
pub struct ChunkMap {
    /// (start, end, function start), sorted by start.
    pub chunks: Vec<(u32, u32, u32)>,
}

impl ChunkMap {
    pub fn build(funcs: &[Func]) -> Self {
        let mut chunks: Vec<(u32, u32, u32)> = funcs
            .iter()
            .flat_map(|f| f.chunks.iter().map(move |&(s, e)| (s, e, f.start)))
            .filter(|c| c.1 > c.0)
            .collect();
        chunks.sort_unstable();
        ChunkMap { chunks }
    }
    pub fn containing(&self, a: u32) -> Option<u32> {
        let i = self.chunks.partition_point(|c| c.0 <= a);
        if i == 0 {
            return None;
        }
        let c = self.chunks[i - 1];
        (a < c.1).then_some(c.2)
    }
    /// The first chunk start strictly after `a`.
    pub fn next_start_after(&self, a: u32) -> Option<u32> {
        let i = self.chunks.partition_point(|c| c.0 <= a);
        self.chunks.get(i).map(|c| c.0)
    }
}

pub struct Ctx<'a> {
    pub pe: &'a Pe<'a>,
    pub iat: &'a HashSet<u32>,
}

/// Decode every chunk of `f` and collect its facts.
pub fn disassemble(ctx: &Ctx, f: &Func) -> Facts {
    let mut facts = Facts::default();
    let span = f.span();
    let mut info = InstructionInfoFactory::new();
    let mut items = Vec::new();
    for &(cs, ce) in &f.chunks {
        if ce <= cs {
            continue;
        }
        items.clear();
        decode_chunk(ctx.pe, span, cs, ce, &mut items);
        for item in &items {
            match item {
                Item::Insn(instr) => {
                    facts.ninsn += 1;
                    handle(ctx, f, instr, &mut facts, &mut info);
                }
                Item::Table { .. } => facts.tables += 1,
            }
        }
    }
    facts
}

/// One decoded element of a chunk: an instruction, or a skipped table.
#[derive(Debug, Clone)]
pub enum Item {
    Insn(Instruction),
    /// A jump table (scale 4: RVAs into the function) or an index table
    /// (scale 1), `len` bytes at `at`.
    Table {
        at: u32,
        len: u32,
        scale: u32,
    },
}

impl Func {
    /// [lowest chunk start, highest chunk end): where a jump table's
    /// entries may point.
    pub fn span(&self) -> (u32, u32) {
        let lo = self.chunks.iter().map(|c| c.0).min().unwrap_or(self.start);
        (lo, self.extent_end())
    }
}

/// Decode `[cs, ce)` linearly, skipping the jump and index tables MSVC puts
/// after a function's code. A table is recognised from the code that reads
/// it: `[base + index*4 + disp]` (or `*1`) whose displacement is an RVA later
/// in the same chunk. Everything decoded from the first table on is
/// discarded and decoded again with the tables skipped.
pub fn decode_chunk(pe: &Pe, span: (u32, u32), cs: u32, ce: u32, out: &mut Vec<Item>) {
    let Some(avail) = pe.slice_from(cs) else {
        return;
    };
    let bytes = &avail[..avail.len().min(ce.saturating_sub(cs) as usize)];
    let ce = cs + bytes.len() as u32;
    let mark = out.len();
    let mut tables: BTreeMap<u32, u32> = BTreeMap::new();
    let mut decoder = Decoder::with_ip(64, bytes, cs as u64, DecoderOptions::NONE);
    let mut instr = Instruction::default();
    while decoder.can_decode() {
        decoder.decode_out(&mut instr);
        note_table(&instr, cs, ce, &mut tables);
        out.push(Item::Insn(instr));
    }
    let Some((&t0, _)) = tables.iter().next() else {
        return;
    };
    let keep = out[mark..].partition_point(|it| match it {
        Item::Insn(i) => (i.ip() as u32) < t0,
        Item::Table { at, .. } => *at < t0,
    });
    out.truncate(mark + keep);
    let mut pos = t0;
    while pos < ce {
        if let Some(&scale) = tables.get(&pos) {
            let at = pos;
            if scale == 4 {
                while pos + 4 <= ce {
                    let v = pe.u32_at(pos).unwrap_or(0);
                    if v < span.0 || v >= span.1 {
                        break;
                    }
                    pos += 4;
                }
                if pos == at {
                    pos += 4.min(ce - pos);
                }
            } else {
                pos = tables
                    .range(pos + 1..)
                    .next()
                    .map(|(&k, _)| k)
                    .unwrap_or(ce);
            }
            out.push(Item::Table {
                at,
                len: pos - at,
                scale,
            });
            continue;
        }
        if decoder.set_position((pos - cs) as usize).is_err() {
            break;
        }
        decoder.set_ip(pos as u64);
        decoder.decode_out(&mut instr);
        note_table(&instr, cs, ce, &mut tables);
        pos += instr.len().max(1) as u32;
        out.push(Item::Insn(instr));
    }
}

fn note_table(instr: &Instruction, cs: u32, ce: u32, tables: &mut BTreeMap<u32, u32>) {
    if instr.is_invalid()
        || instr.memory_index() == Register::None
        || !matches!(instr.memory_index_scale(), 1 | 4)
        || matches!(instr.memory_base(), Register::None | Register::RIP)
        || !(0..instr.op_count()).any(|i| instr.op_kind(i) == OpKind::Memory)
    {
        return;
    }
    let d = instr.memory_displacement64();
    if d < (1 << 31) {
        let d = d as u32;
        if d > instr.ip() as u32 && d >= cs && d < ce {
            tables.entry(d).or_insert(instr.memory_index_scale());
        }
    }
}

fn handle(
    ctx: &Ctx,
    f: &Func,
    instr: &Instruction,
    facts: &mut Facts,
    info: &mut InstructionInfoFactory,
) {
    if instr.is_invalid() {
        facts.invalid += 1;
        return;
    }
    let ip = instr.ip() as u32;
    let image = ctx.pe.size_of_image as u64;
    if instr.is_call_near() {
        let t = instr.near_branch_target();
        if t < image {
            facts.calls.push(Call {
                site: ip,
                target: t as u32,
                kind: CALL_DIRECT,
            });
        }
    } else if instr.is_jmp_short_or_near() || instr.is_jcc_short_or_near() {
        let t = instr.near_branch_target();
        if t < image && !f.contains(t as u32) {
            facts.calls.push(Call {
                site: ip,
                target: t as u32,
                kind: CALL_TAIL,
            });
        }
    }
    if instr.is_ip_rel_memory_operand() {
        let t = instr.ip_rel_memory_address();
        if t < image {
            let t = t as u32;
            let icall = instr.is_call_near_indirect();
            let ijmp = instr.is_jmp_near_indirect();
            if (icall || ijmp) && ctx.iat.contains(&t) {
                facts.calls.push(Call {
                    site: ip,
                    target: t,
                    kind: if icall { CALL_IMPORT } else { CALL_IMPORT_JMP },
                });
            } else {
                let kind = if icall {
                    XREF_ICALL
                } else if ijmp {
                    XREF_IJMP
                } else if instr.mnemonic() == Mnemonic::Lea {
                    XREF_ADDR
                } else {
                    access_kind(instr, t, info)
                };
                facts.xrefs.push(Xref {
                    insn: ip,
                    target: t,
                    kind,
                });
            }
        }
    }
}

fn access_kind(instr: &Instruction, target: u32, info: &mut InstructionInfoFactory) -> u8 {
    let i = info.info_options(instr, InstructionInfoOptions::NO_REGISTER_USAGE);
    for m in i.used_memory() {
        if m.base() == Register::None && m.displacement() == target as u64 {
            return match m.access() {
                OpAccess::Read | OpAccess::CondRead => XREF_READ,
                OpAccess::Write | OpAccess::CondWrite => XREF_WRITE,
                OpAccess::ReadWrite | OpAccess::ReadCondWrite => XREF_RW,
                _ => XREF_MEM,
            };
        }
    }
    XREF_MEM
}

/// Bound a function that has no `.pdata` entry by following its control
/// flow from `start`, never past `limit`. `None` when `start` is not code.
pub fn explore(pe: &Pe, start: u32, limit: u32) -> Option<u32> {
    const MAX_BYTES: u32 = 0x10000;
    const MAX_INSNS: usize = 20000;
    let avail = pe.slice_from(start)?;
    let limit = limit.min(start.saturating_add(MAX_BYTES));
    let len = (avail.len() as u64).min((limit - start) as u64) as usize;
    if len == 0 || avail[0] == 0xCC || (avail.len() >= 2 && avail[0] == 0 && avail[1] == 0) {
        return None;
    }
    let bytes = &avail[..len];
    let mut decoder = Decoder::with_ip(64, bytes, start as u64, DecoderOptions::NONE);
    let mut instr = Instruction::default();
    let mut work = vec![start];
    let mut seen: HashSet<u32> = HashSet::new();
    let mut end = start;
    let mut count = 0usize;
    while let Some(pc) = work.pop() {
        let mut pc = pc;
        loop {
            if pc < start || pc >= start + len as u32 || !seen.insert(pc) {
                break;
            }
            count += 1;
            if count > MAX_INSNS || decoder.set_position((pc - start) as usize).is_err() {
                break;
            }
            decoder.set_ip(pc as u64);
            decoder.decode_out(&mut instr);
            if instr.is_invalid() {
                if pc == start {
                    return None;
                }
                break;
            }
            let next = pc + instr.len() as u32;
            end = end.max(next);
            match instr.flow_control() {
                FlowControl::Next
                | FlowControl::Call
                | FlowControl::IndirectCall
                | FlowControl::XbeginXabortXend => pc = next,
                FlowControl::ConditionalBranch => {
                    let t = instr.near_branch_target();
                    if t >= start as u64 && t < limit as u64 {
                        work.push(t as u32);
                    }
                    pc = next;
                }
                FlowControl::UnconditionalBranch => {
                    let t = instr.near_branch_target();
                    if t >= start as u64 && t < limit as u64 {
                        work.push(t as u32);
                    }
                    break;
                }
                _ => break, // return, indirect branch, int3, ud2
            }
        }
    }
    (end > start).then_some(end)
}

//! `tpfre index`: one parallel pass over a binary into one SQLite file.

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::db;
use crate::disasm::{self, CALL_DIRECT, CALL_TAIL, ChunkMap, Ctx, Facts, Func, XREF_ADDR};
use crate::naming;
use crate::pe::{Export, Import, Pe};
use crate::rtti::{self, Rtti};
use crate::strings::{self, Str};

pub struct Options {
    pub binary: PathBuf,
    pub out: PathBuf,
    /// Load address when `binary` is a raw memory dump of the image.
    pub dump_base: Option<u64>,
}

pub struct Report {
    pub stats: Vec<(String, String)>,
    pub warnings: Vec<String>,
    pub seconds: f64,
    pub db_bytes: u64,
}

/// Drop Windows' `\\?\` verbatim prefix from a canonical local path.
fn plain_path(p: PathBuf) -> PathBuf {
    let plain = p
        .to_str()
        .and_then(|s| s.strip_prefix(r"\\?\"))
        .filter(|rest| !rest.starts_with(r"UNC\"))
        .map(PathBuf::from);
    plain.unwrap_or(p)
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(data);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Everything the indexer learns, before it is written.
pub struct Analysis {
    pub funcs: Vec<Func>,
    pub facts: Vec<Facts>,
    pub strings: Vec<Str>,
    pub rtti: Rtti,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
    pub naming: naming::Naming,
    pub dataptrs: Vec<(u32, u32)>,
    /// Import thunks: (function, IAT slot) for a function that starts with
    /// `jmp [IAT]`.
    pub thunks: Vec<(u32, u32)>,
    pub warnings: Vec<String>,
    pub discovered_rounds: usize,
}

pub fn run(opts: &Options) -> Result<Report> {
    let t0 = Instant::now();
    let data = std::fs::read(&opts.binary)
        .with_context(|| format!("reading {}", opts.binary.display()))?;
    if let (Ok(a), Ok(b)) = (opts.binary.canonicalize(), opts.out.canonicalize())
        && a == b
    {
        bail!("refusing to write the database over the binary itself");
    }
    let (sha, pe) = rayon::join(
        || sha256_hex(&data),
        || -> Result<Pe> {
            let mut pe = Pe::parse(&data, opts.dump_base)?;
            pe.compute_entropy();
            Ok(pe)
        },
    );
    let pe = pe?;
    let an = analyse(&pe);
    let binary_path = plain_path(
        opts.binary
            .canonicalize()
            .unwrap_or_else(|_| opts.binary.clone()),
    );
    let mut stats = write_db(&opts.out, &pe, &an, &sha, &binary_path)?;
    let db_bytes = std::fs::metadata(&opts.out).map(|m| m.len()).unwrap_or(0);
    stats.insert(0, ("sha256".into(), sha));
    Ok(Report {
        stats,
        warnings: an.warnings,
        seconds: t0.elapsed().as_secs_f64(),
        db_bytes,
    })
}

fn packer_warnings(pe: &Pe) -> Vec<String> {
    let mut w = Vec::new();
    for s in &pe.sections {
        if s.name == ".bind" {
            w.push(format!(
                "SteamStub (Steam DRM) section .bind present (entropy {:.2}){}",
                s.entropy,
                if s.contains(pe.entry) {
                    "; the entry point is in it"
                } else {
                    ""
                }
            ));
        }
    }
    for s in &pe.sections {
        if s.packed() && s.name != ".bind" {
            w.push(format!(
                "executable section {} has entropy {:.2}: it looks packed or encrypted, so its code \
                 must come from a memory dump of the running game (tpfre index <dump> --image-base <load address>)",
                s.name, s.entropy
            ));
        }
    }
    let text_ok: Vec<String> = pe
        .sections
        .iter()
        .filter(|s| s.exec() && !s.packed())
        .map(|s| format!("{} (entropy {:.2})", s.name, s.entropy))
        .collect();
    if !w.is_empty() && !text_ok.is_empty() {
        w.push(format!(
            "code in {} is not encrypted on disk: static analysis of it is valid",
            text_ok.join(", ")
        ));
    }
    if pe.runtime_functions().is_empty() {
        w.push("no .pdata: function bounds come from call targets alone".into());
    }
    w
}

pub fn analyse(pe: &Pe) -> Analysis {
    let mut warnings = packer_warnings(pe);
    let imports = pe.imports();
    let exports = pe.exports();
    let relocs = pe.relocations();
    let iat: HashSet<u32> = imports.iter().map(|i| i.slot).collect();

    let (mut strs, (rtti, funcs, facts, rounds)) = rayon::join(
        || strings::extract(pe),
        || {
            let rtti = rtti::parse(pe);
            let (funcs, facts, rounds) = discover(pe, &iat, &rtti, &relocs, &exports);
            (rtti, funcs, facts, rounds)
        },
    );

    // Strings code points at that do not start on a NUL boundary.
    let mut targets: Vec<u32> = facts
        .iter()
        .flat_map(|f| f.xrefs.iter().map(|x| x.target))
        .collect();
    targets.par_sort_unstable();
    targets.dedup();
    let extra: Vec<Str> = targets
        .par_iter()
        .filter(|&&t| strs.binary_search_by_key(&t, |s| s.rva).is_err())
        .filter_map(|&t| strings::referenced_string(pe, t))
        .collect();
    strs.extend(extra);
    strs.sort_by_key(|s| s.rva);

    // Naming from assert strings.
    let refs: Vec<(u32, u32)> = funcs
        .iter()
        .zip(&facts)
        .flat_map(|(f, fx)| fx.xrefs.iter().map(move |x| (f.start, x.target)))
        .collect();
    let starts: Vec<u32> = funcs.iter().map(|f| f.start).collect();
    let naming = naming::build(&strs, &refs, &starts);

    // Absolute pointers in data to functions or strings.
    let start_set: HashSet<u32> = starts.iter().copied().collect();
    let is_target =
        |t: u32| start_set.contains(&t) || strs.binary_search_by_key(&t, |s| s.rva).is_ok();
    let slots: Vec<u32> = if relocs.is_empty() {
        warnings.push(
            "no base relocations: data pointers found by an aligned scan of data sections".into(),
        );
        pe.sections
            .iter()
            .filter(|s| !s.exec())
            .flat_map(|s| {
                let first = (8 - s.rva % 8) % 8;
                (s.rva + first..s.raw_end().saturating_sub(7)).step_by(8)
            })
            .collect()
    } else {
        relocs
            .iter()
            .copied()
            .filter(|&r| pe.section_of(r).is_some_and(|s| !s.exec()))
            .collect()
    };
    let dataptrs: Vec<(u32, u32)> = slots
        .par_iter()
        .filter_map(|&at| {
            let t = pe.va_to_rva(pe.u64_at(at)?)?;
            is_target(t).then_some((at, t))
        })
        .collect();

    // A function whose first instruction jumps through the IAT is that import.
    let thunks: Vec<(u32, u32)> = funcs
        .iter()
        .zip(&facts)
        .filter_map(|(f, fx)| {
            fx.calls
                .iter()
                .find(|c| c.site == f.start && c.kind == disasm::CALL_IMPORT_JMP)
                .map(|c| (f.start, c.target))
        })
        .collect();

    Analysis {
        funcs,
        facts,
        strings: strs,
        rtti,
        imports,
        exports,
        naming,
        dataptrs,
        thunks,
        warnings,
        discovered_rounds: rounds,
    }
}

/// `.pdata` functions, then functions without unwind info found from
/// references, round by round until no new one appears.
fn discover(
    pe: &Pe,
    iat: &HashSet<u32>,
    rtti: &Rtti,
    relocs: &[u32],
    exports: &[Export],
) -> (Vec<Func>, Vec<Facts>, usize) {
    let ctx = Ctx { pe, iat };
    let mut funcs = disasm::from_pdata(pe);
    let mut facts: Vec<Facts> = funcs
        .par_iter()
        .map(|f| disasm::disassemble(&ctx, f))
        .collect();

    // Seed kinds in priority order (a lower index wins on a tie).
    const KINDS: [&str; 6] = ["entry", "export", "call", "tail", "vtable", "pointer"];
    let kind_rank = |k: &str| KINDS.iter().position(|x| *x == k).unwrap_or(KINDS.len());
    let seeds_of = |fx: &[Facts]| -> Vec<(u32, &'static str)> {
        let mut v = Vec::new();
        for f in fx {
            for c in &f.calls {
                match c.kind {
                    CALL_DIRECT => v.push((c.target, "call")),
                    CALL_TAIL => v.push((c.target, "tail")),
                    _ => {}
                }
            }
            for x in &f.xrefs {
                if x.kind == XREF_ADDR {
                    v.push((x.target, "pointer"));
                }
            }
        }
        v
    };
    let mut seeds = seeds_of(&facts);
    seeds.push((pe.entry, "entry"));
    seeds.extend(exports.iter().map(|e| (e.rva, "export")));
    for vt in &rtti.vtables {
        seeds.extend(vt.slots.iter().map(|&s| (s, "vtable")));
    }
    for &r in relocs {
        if pe.section_of(r).is_some_and(|s| !s.exec())
            && let Some(t) = pe.u64_at(r).and_then(|v| pe.va_to_rva(v))
        {
            seeds.push((t, "pointer"));
        }
    }

    let mut rejected: HashSet<u32> = HashSet::new();
    let mut rounds = 0;
    let mut map = ChunkMap::build(&funcs);
    let mut starts: HashSet<u32> = funcs.iter().map(|f| f.start).collect();
    while rounds < 64 {
        seeds.retain(|&(t, _)| {
            !starts.contains(&t)
                && !rejected.contains(&t)
                && pe.section_of(t).is_some_and(|s| s.exec() && !s.packed())
                && map.containing(t).is_none()
        });
        seeds.sort_unstable_by_key(|&(t, k)| (t, kind_rank(k)));
        seeds.dedup_by_key(|s| s.0);
        if seeds.is_empty() {
            break;
        }
        rounds += 1;
        let limits: Vec<u32> = (0..seeds.len())
            .map(|i| {
                let t = seeds[i].0;
                let mut lim = pe.section_of(t).map(|s| s.raw_end()).unwrap_or(t);
                if let Some(n) = seeds.get(i + 1) {
                    lim = lim.min(n.0);
                }
                if let Some(n) = map.next_start_after(t) {
                    lim = lim.min(n);
                }
                lim
            })
            .collect();
        let found: Vec<Option<Func>> = seeds
            .par_iter()
            .zip(&limits)
            .map(|(&(t, kind), &lim)| {
                disasm::explore(pe, t, lim).map(|end| Func {
                    start: t,
                    chunks: vec![(t, end)],
                    kind,
                })
            })
            .collect();
        let mut new_funcs = Vec::new();
        for (s, f) in seeds.iter().zip(found) {
            match f {
                Some(f) => new_funcs.push(f),
                None => {
                    rejected.insert(s.0);
                }
            }
        }
        let new_facts: Vec<Facts> = new_funcs
            .par_iter()
            .map(|f| disasm::disassemble(&ctx, f))
            .collect();
        seeds = seeds_of(&new_facts);
        starts.extend(new_funcs.iter().map(|f| f.start));
        funcs.extend(new_funcs);
        facts.extend(new_facts);
        map = ChunkMap::build(&funcs);
    }
    let mut both: Vec<(Func, Facts)> = funcs.into_iter().zip(facts).collect();
    both.sort_by_key(|(f, _)| f.start);
    let (funcs, facts) = both.into_iter().unzip();
    (funcs, facts, rounds)
}

/// The best name for a function and where it came from, from its sources.
fn best_name(cands: &[(String, &str, String)]) -> Option<(String, String)> {
    // (name, source, confidence) -> rank; lower wins.
    let rank = |src: &str, conf: &str| -> u8 {
        match (src, conf) {
            ("funcsig" | "pretty", "exact") => 0,
            ("export", _) | ("import", "thunk") => 1,
            ("rtti", "unique") => 2,
            ("funcsig" | "pretty", _) => 3,
            ("rtti", _) => 4,
            _ => 5,
        }
    };
    let best = cands
        .iter()
        .min_by_key(|(n, s, c)| (rank(s, c), n.clone()))?;
    let r = rank(best.1, &best.2);
    let display = if r >= 3 {
        format!("~{}", best.0)
    } else {
        best.0.clone()
    };
    Some((display, format!("{}:{}", best.1, best.2)))
}

fn write_db(
    out: &Path,
    pe: &Pe,
    an: &Analysis,
    sha: &str,
    binary_path: &Path,
) -> Result<Vec<(String, String)>> {
    let tmp = out.with_extension("tpfdb.partial");
    if tmp.exists() {
        std::fs::remove_file(&tmp).with_context(|| format!("removing {}", tmp.display()))?;
    }
    let mut conn = Connection::open(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
    conn.execute_batch(
        "PRAGMA page_size=8192; PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; \
         PRAGMA locking_mode=EXCLUSIVE; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-524288;",
    )?;
    conn.execute_batch(db::SCHEMA)?;

    // Names, per address, from every source.
    let mut names: Vec<(u32, String, &str, String, String)> = Vec::new();
    for (&f, n) in &an.naming.names {
        names.push((
            f,
            n.name.clone(),
            n.source,
            n.confidence.to_owned(),
            n.signatures.join(" || "),
        ));
    }
    // Per slot function: how many slots hold it, whether all at the same
    // index (an inherited implementation) or not (identical code folded by
    // the linker, or unrelated uses), and the base-most vtable holding it
    // (fewest slots, then lowest address).
    struct SlotUse {
        n: usize,
        index: Option<usize>,
        same_index: bool,
        base: (usize, u32, String),
    }
    let mut slot_uses: HashMap<u32, SlotUse> = HashMap::new();
    for vt in &an.rtti.vtables {
        for (i, &s) in vt.slots.iter().enumerate() {
            let key = (vt.slots.len(), vt.rva, vt.slot_name(i));
            let u = slot_uses.entry(s).or_insert(SlotUse {
                n: 0,
                index: Some(i),
                same_index: true,
                base: key.clone(),
            });
            u.n += 1;
            u.same_index &= u.index == Some(i);
            if (key.0, key.1) < (u.base.0, u.base.1) {
                u.base = key;
            }
        }
    }
    for vt in &an.rtti.vtables {
        names.push((
            vt.rva,
            vt.label(),
            "rtti",
            "vtable".into(),
            format!("col 0x{:x} td 0x{:x}", vt.col, vt.td),
        ));
        for (i, &s) in vt.slots.iter().enumerate() {
            let conf = match slot_uses.get(&s) {
                Some(u) if u.n > 1 && u.same_index => format!("inherited({})", u.n),
                Some(u) if u.n > 1 => format!("folded({})", u.n),
                _ => "unique".to_owned(),
            };
            names.push((
                s,
                vt.slot_name(i),
                "rtti",
                conf,
                format!("vtable 0x{:x} slot {i}", vt.rva),
            ));
        }
    }
    for i in &an.imports {
        names.push((
            i.slot,
            format!("{}!{}", i.dll, i.name),
            "import",
            if i.delay { "delay" } else { "exact" }.into(),
            String::new(),
        ));
    }
    for &(f, slot) in &an.thunks {
        if let Some(i) = an.imports.iter().find(|i| i.slot == slot) {
            names.push((
                f,
                format!("{}!{}", i.dll, i.name),
                "import",
                "thunk".into(),
                format!("jmp [0x{slot:x}]"),
            ));
        }
    }
    for e in &an.exports {
        names.push((
            e.rva,
            e.name.clone(),
            "export",
            "exact".into(),
            String::new(),
        ));
    }

    let mut by_addr: BTreeMap<u32, Vec<(String, &str, String)>> = BTreeMap::new();
    for (a, n, s, c, _) in &names {
        // Functions show the template-aware label; `names` keeps the
        // Python pipeline's name.
        let shown = match (*s, an.naming.names.get(a)) {
            ("funcsig" | "pretty", Some(sn)) => sn.label.clone(),
            // A slot shared by several vtables names a function only through
            // the base-most one, added below; folded code is not named.
            ("rtti", _) if c != "unique" && c != "vtable" => continue,
            _ => n.clone(),
        };
        by_addr.entry(*a).or_default().push((shown, s, c.clone()));
    }
    for (&t, u) in &slot_uses {
        if u.n > 1 && u.same_index {
            by_addr.entry(t).or_default().push((
                u.base.2.clone(),
                "rtti",
                format!("inherited({})", u.n),
            ));
        }
    }

    let tx = conn.transaction()?;
    let stats = {
        let mut st = tx.prepare("INSERT INTO meta(key, value) VALUES (?1, ?2)")?;
        let mut meta = vec![
            ("schema_version".to_owned(), db::SCHEMA_VERSION.to_owned()),
            (
                "tool".into(),
                format!("tpfre {}", env!("CARGO_PKG_VERSION")),
            ),
            ("binary_path".into(), binary_path.display().to_string()),
            (
                "binary_name".into(),
                binary_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
            ("sha256".into(), sha.to_owned()),
            ("size".into(), pe.data.len().to_string()),
            ("pe_timestamp".into(), format!("0x{:08X}", pe.timestamp)),
            ("format".into(), "pe".into()),
            ("arch".into(), "x86_64".into()),
            ("image_base".into(), format!("0x{:x}", pe.image_base)),
            (
                "header_image_base".into(),
                format!("0x{:x}", pe.header_image_base),
            ),
            ("dump".into(), if pe.dump { "1" } else { "0" }.into()),
            ("entry".into(), format!("0x{:x}", pe.entry)),
            ("size_of_image".into(), format!("0x{:x}", pe.size_of_image)),
            ("warnings".into(), an.warnings.join("\n")),
        ];
        for (k, v) in &an.naming.stats {
            meta.push((format!("naming.{k}"), v.clone()));
        }
        let ncalls: usize = an.facts.iter().map(|f| f.calls.len()).sum();
        let nxrefs: usize = an.facts.iter().map(|f| f.xrefs.len()).sum();
        let ninsn: u64 = an.facts.iter().map(|f| f.ninsn as u64).sum();
        let ninvalid: u64 = an.facts.iter().map(|f| f.invalid as u64).sum();
        let ntables: u64 = an.facts.iter().map(|f| f.tables as u64).sum();
        let npdata = an.funcs.iter().filter(|f| f.kind == "pdata").count();
        let rtti_classes = an
            .rtti
            .types
            .iter()
            .filter(|t| t.mangled.starts_with(".?AV") || t.mangled.starts_with(".?AU"))
            .count();
        let counts = vec![
            ("functions", an.funcs.len().to_string()),
            ("functions_pdata", npdata.to_string()),
            (
                "functions_discovered",
                (an.funcs.len() - npdata).to_string(),
            ),
            ("discovery_rounds", an.discovered_rounds.to_string()),
            ("instructions", ninsn.to_string()),
            ("invalid_instructions", ninvalid.to_string()),
            ("jump_tables_skipped", ntables.to_string()),
            ("call_edges", ncalls.to_string()),
            ("xrefs", nxrefs.to_string()),
            ("strings", an.strings.len().to_string()),
            ("data_pointers", an.dataptrs.len().to_string()),
            ("imports", an.imports.len().to_string()),
            ("exports", an.exports.len().to_string()),
            ("rtti_type_descriptors", an.rtti.types.len().to_string()),
            ("rtti_class_type_descriptors", rtti_classes.to_string()),
            ("rtti_complete_object_locators", an.rtti.cols.to_string()),
            ("rtti_vtables", an.rtti.vtables.len().to_string()),
            (
                "rtti_vtable_slots",
                an.rtti
                    .vtables
                    .iter()
                    .map(|v| v.slots.len())
                    .sum::<usize>()
                    .to_string(),
            ),
        ];
        for (k, v) in &counts {
            meta.push((format!("count.{k}"), v.clone()));
        }
        for (k, v) in &meta {
            st.execute(params![k, v])?;
        }

        let mut st = tx.prepare(
            "INSERT INTO sections(idx, name, rva, vsize, raw_size, flags, entropy) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        )?;
        for (i, s) in pe.sections.iter().enumerate() {
            st.execute(params![
                i as i64,
                s.name,
                s.rva,
                s.vsize,
                s.raw_size as i64,
                s.characteristics,
                s.entropy
            ])?;
        }

        let mut st = tx.prepare(
            "INSERT INTO functions(rva, size, end, kind, ninsn, name, name_src) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        )?;
        for (f, fx) in an.funcs.iter().zip(&an.facts) {
            let (name, src) = by_addr
                .get(&f.start)
                .and_then(|c| best_name(c))
                .unwrap_or_default();
            st.execute(params![
                f.start,
                f.size(),
                f.extent_end(),
                f.kind,
                fx.ninsn,
                name,
                src
            ])?;
        }

        let mut chunks: Vec<(u32, u32, u32)> = an
            .funcs
            .iter()
            .flat_map(|f| f.chunks.iter().map(move |&(s, e)| (s, e, f.start)))
            .collect();
        chunks.sort_unstable();
        let mut st =
            tx.prepare("INSERT OR IGNORE INTO chunks(start, end, func) VALUES (?1,?2,?3)")?;
        for (s, e, f) in chunks {
            st.execute(params![s, e, f])?;
        }

        let mut calls: Vec<(u32, u32, u32, u8)> = an
            .funcs
            .iter()
            .zip(&an.facts)
            .flat_map(|(f, fx)| {
                fx.calls
                    .iter()
                    .map(move |c| (c.site, f.start, c.target, c.kind))
            })
            .collect();
        calls.par_sort_unstable();
        let mut st = tx.prepare(
            "INSERT OR IGNORE INTO calls(site, caller, callee, kind) VALUES (?1,?2,?3,?4)",
        )?;
        for (s, a, b, k) in calls {
            st.execute(params![s, a, b, k])?;
        }

        let mut xrefs: Vec<(u32, u32, u32, u8)> = an
            .funcs
            .iter()
            .zip(&an.facts)
            .flat_map(|(f, fx)| {
                fx.xrefs
                    .iter()
                    .map(move |x| (x.insn, f.start, x.target, x.kind))
            })
            .collect();
        xrefs.par_sort_unstable();
        let mut st = tx.prepare(
            "INSERT OR IGNORE INTO xrefs(insn, func, target, kind) VALUES (?1,?2,?3,?4)",
        )?;
        for (i, f, t, k) in xrefs {
            st.execute(params![i, f, t, k])?;
        }

        let mut st = tx.prepare("INSERT OR IGNORE INTO dataptrs(at, target) VALUES (?1,?2)")?;
        let mut dp = an.dataptrs.clone();
        dp.sort_unstable();
        for (a, t) in dp {
            st.execute(params![a, t])?;
        }

        let mut st = tx
            .prepare("INSERT OR IGNORE INTO strings(rva, enc, class, text) VALUES (?1,?2,?3,?4)")?;
        for s in &an.strings {
            let class = match an.naming.classes.get(&s.rva) {
                Some(naming::Class::FuncSig) => "funcsig",
                Some(naming::Class::Pretty) => "pretty",
                Some(naming::Class::File) => "file",
                None => "",
            };
            st.execute(params![s.rva, s.enc, class, s.text])?;
        }

        let mut st = tx.prepare(
            "INSERT INTO names(addr, name, source, confidence, detail) VALUES (?1,?2,?3,?4,?5)",
        )?;
        for (a, n, s, c, d) in &names {
            st.execute(params![a, n, s, c, d])?;
        }

        let mut st = tx.prepare("INSERT INTO func_src(func, file, kind) VALUES (?1,?2,?3)")?;
        for (f, s) in &an.naming.src {
            st.execute(params![f, s.file, s.kind])?;
        }

        let mut st = tx.prepare(
            "INSERT OR IGNORE INTO imports(slot, dll, name, delay) VALUES (?1,?2,?3,?4)",
        )?;
        for i in &an.imports {
            st.execute(params![i.slot, i.dll, i.name, i.delay])?;
        }

        let mut st = tx.prepare("INSERT INTO types(td, mangled, name) VALUES (?1,?2,?3)")?;
        for t in &an.rtti.types {
            st.execute(params![t.rva, t.mangled, t.name])?;
        }

        let mut st = tx.prepare(
            "INSERT OR IGNORE INTO vtables(rva, class, col, td, offset, nslots) VALUES (?1,?2,?3,?4,?5,?6)",
        )?;
        let mut sl =
            tx.prepare("INSERT OR IGNORE INTO vslots(vtable, slot, target) VALUES (?1,?2,?3)")?;
        for vt in &an.rtti.vtables {
            st.execute(params![
                vt.rva,
                vt.class,
                vt.col,
                vt.td,
                vt.offset,
                vt.slots.len() as i64
            ])?;
            for (i, &s) in vt.slots.iter().enumerate() {
                sl.execute(params![vt.rva, i as i64, s])?;
            }
        }
        let mut stats: Vec<(String, String)> = Vec::new();
        for (k, v) in counts {
            stats.push((k.to_owned(), v));
        }
        for (k, v) in &an.naming.stats {
            stats.push((format!("naming.{k}"), v.clone()));
        }
        stats
    };
    tx.commit()?;
    conn.execute_batch(db::INDEXES)?;
    conn.execute_batch("ANALYZE;")?;
    drop(conn);
    std::fs::rename(&tmp, out)
        .with_context(|| format!("moving {} to {}", tmp.display(), out.display()))?;
    Ok(stats)
}

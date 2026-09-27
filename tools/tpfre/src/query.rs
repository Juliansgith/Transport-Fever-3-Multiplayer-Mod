//! `tpfre q`: small questions about an indexed binary, answered with short,
//! stable lines (one fact per line, RVAs as `0x` hex, names inline) or, with
//! `--json`, an array of the same facts as objects.
//!
//! Commands that need the binary's bytes (`dis`, `sig`, `bytes`) read the
//! file the database was made from, or `--bin`, and refuse it unless its
//! SHA-256 is the one recorded at indexing time. Any command given `--bin`
//! checks it the same way.

use anyhow::{Context, Result, bail};
use iced_x86::{
    Formatter, FormatterOutput, FormatterTextKind, Instruction, IntelFormatter, SymbolResolver,
    SymbolResult,
};
use regex::{Regex, RegexBuilder};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::db;
use crate::disasm::{self, Item, call_kind_name, xref_kind_name};
use crate::index::sha256_hex;
use crate::pe::Pe;
use crate::sig;

/// Exit status: something was found and printed.
pub const OK: i32 = 0;
/// Nothing matched, or a request was refused.
pub const NONE: i32 = 1;
/// The argument named several things where one was needed.
pub const AMBIGUOUS: i32 = 2;

pub struct Query<'w> {
    conn: Connection,
    db_path: PathBuf,
    pub image_base: u64,
    pub size_of_image: u32,
    dump: bool,
    bin: Option<PathBuf>,
    json: bool,
    w: &'w mut dyn Write,
    items: Vec<Value>,
    labels: RefCell<HashMap<u32, String>>,
}

/// size, end, kind, instructions, best name, where the name came from.
type FuncRow = (u32, u32, String, u32, String, String);

pub enum Resolved {
    One(u32),
    Many(Vec<(u32, String)>),
    None,
}

fn parse_hex(s: &str) -> Option<u64> {
    let t = s.trim();
    let h = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .or_else(|| t.strip_prefix("sub_"))?;
    u64::from_str_radix(h, 16).ok()
}

/// A string for one line: escaped, and cut at `max` characters.
pub fn quote(text: &str, max: usize) -> String {
    let mut out = String::from("\"");
    for (n, c) in text.chars().enumerate() {
        if n >= max {
            out.push_str("...");
            break;
        }
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn hx(v: u32) -> String {
    format!("0x{v:x}")
}

fn regex_ci(pattern: &str, case: bool) -> Result<Regex> {
    RegexBuilder::new(pattern)
        .case_insensitive(!case)
        .build()
        .with_context(|| format!("bad regex {pattern:?}"))
}

struct Syms(HashMap<u64, String>);

impl SymbolResolver for Syms {
    fn symbol(
        &mut self,
        _instruction: &Instruction,
        _operand: u32,
        _instruction_operand: Option<u32>,
        address: u64,
        _address_size: u32,
    ) -> Option<SymbolResult<'_>> {
        self.0
            .get(&address)
            .map(|s| SymbolResult::with_str(address, s.as_str()))
    }
}

struct Text(String);
impl FormatterOutput for Text {
    fn write(&mut self, text: &str, _kind: FormatterTextKind) {
        self.0.push_str(text);
    }
}

impl<'w> Query<'w> {
    pub fn open(
        db_path: &Path,
        bin: Option<PathBuf>,
        json: bool,
        w: &'w mut dyn Write,
    ) -> Result<Self> {
        let conn = db::open_ro(db_path)?;
        let image_base =
            parse_hex(&db::meta_req(&conn, "image_base")?).context("bad image_base")?;
        let size_of_image =
            parse_hex(&db::meta_req(&conn, "size_of_image")?).context("bad size_of_image")? as u32;
        let dump = db::meta(&conn, "dump")?.as_deref() == Some("1");
        let q = Query {
            conn,
            db_path: db_path.to_owned(),
            image_base,
            size_of_image,
            dump,
            bin,
            json,
            w,
            items: Vec::new(),
            labels: RefCell::new(HashMap::new()),
        };
        if q.bin.is_some() {
            q.binary()?; // fail closed on a mismatched --bin, whatever the command
        }
        Ok(q)
    }

    /// Print one fact: the line, or (with --json) the object.
    fn emit(&mut self, text: impl AsRef<str>, v: Value) -> Result<()> {
        if self.json {
            self.items.push(v);
        } else {
            writeln!(self.w, "{}", text.as_ref())?;
        }
        Ok(())
    }

    /// Flush the JSON array (a no-op for text).
    pub fn finish(&mut self) -> Result<()> {
        if self.json {
            let items = std::mem::take(&mut self.items);
            writeln!(
                self.w,
                "{}",
                serde_json::to_string_pretty(&Value::Array(items))?
            )?;
        }
        Ok(())
    }

    /// The binary's bytes, refused unless their SHA-256 is the recorded one.
    pub fn binary(&self) -> Result<Vec<u8>> {
        let path = match &self.bin {
            Some(p) => p.clone(),
            None => PathBuf::from(db::meta_req(&self.conn, "binary_path")?),
        };
        let want = db::meta_req(&self.conn, "sha256")?;
        let data = std::fs::read(&path).with_context(|| {
            format!(
                "reading {} (the binary {} was made from; pass --bin <path> if it moved)",
                path.display(),
                self.db_path.display()
            )
        })?;
        let got = sha256_hex(&data);
        if got != want {
            bail!(
                "refusing: {} has SHA-256 {got}, but {} was indexed from a binary with SHA-256 {want} \
                 (size {}, PE timestamp {}). Re-index this binary, or pass --bin with the right one.",
                path.display(),
                self.db_path.display(),
                db::meta(&self.conn, "size")?.unwrap_or_default(),
                db::meta(&self.conn, "pe_timestamp")?.unwrap_or_default()
            );
        }
        Ok(data)
    }

    fn pe<'d>(&self, data: &'d [u8]) -> Result<Pe<'d>> {
        Pe::parse(data, self.dump.then_some(self.image_base))
    }

    // ---------------------------------------------------------------- lookup

    fn func_row(&self, rva: u32) -> Result<Option<FuncRow>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT size, end, kind, ninsn, name, name_src FROM functions WHERE rva=?1",
            )?
            .query_row([rva], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .optional()?)
    }

    /// The function whose chunk holds `rva`.
    pub fn containing(&self, rva: u32) -> Result<Option<u32>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT end, func FROM chunks WHERE start <= ?1 ORDER BY start DESC LIMIT 1",
            )?
            .query_row([rva], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))
            .optional()?
            .and_then(|(end, f)| (rva < end).then_some(f)))
    }

    fn is_function(&self, rva: u32) -> Result<bool> {
        Ok(self.func_row(rva)?.is_some())
    }

    /// A short name for any address: a function, import, vtable, string, or
    /// `function+0xoff`; empty when nothing is known.
    pub fn label(&self, rva: u32) -> String {
        if let Some(l) = self.labels.borrow().get(&rva) {
            return l.clone();
        }
        let l = self.label_uncached(rva).unwrap_or_default();
        self.labels.borrow_mut().insert(rva, l.clone());
        l
    }

    fn label_uncached(&self, rva: u32) -> Result<String> {
        if let Some(row) = self.func_row(rva)? {
            return Ok(if row.4.is_empty() {
                format!("sub_{rva:x}")
            } else {
                row.4
            });
        }
        let named: Option<String> = self
            .conn
            .prepare_cached(
                "SELECT name FROM names WHERE addr=?1 ORDER BY CASE source WHEN 'import' THEN 0 \
                 WHEN 'export' THEN 1 WHEN 'funcsig' THEN 2 ELSE 3 END LIMIT 1",
            )?
            .query_row([rva], |r| r.get(0))
            .optional()?;
        if let Some(n) = named {
            return Ok(n);
        }
        let s: Option<(u8, String)> = self
            .conn
            .prepare_cached("SELECT enc, text FROM strings WHERE rva=?1")?
            .query_row([rva], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        if let Some((enc, text)) = s {
            return Ok(format!(
                "{}{}",
                if enc == 2 { "u" } else { "" },
                quote(&text, 60)
            ));
        }
        let td: Option<String> = self
            .conn
            .prepare_cached("SELECT name FROM types WHERE td=?1")?
            .query_row([rva], |r| r.get(0))
            .optional()?;
        if let Some(t) = td {
            return Ok(format!("{t}::`RTTI Type Descriptor'"));
        }
        if let Some(f) = self.containing(rva)? {
            return Ok(format!("{}+0x{:x}", self.label(f), rva - f));
        }
        Ok(String::new())
    }

    /// `0x1234 Name` (or just `0x1234`).
    pub fn at(&self, rva: u32) -> String {
        let l = self.label(rva);
        if l.is_empty() {
            hx(rva)
        } else {
            format!("{} {l}", hx(rva))
        }
    }

    fn addr_json(&self, rva: u32) -> Value {
        json!({"rva": hx(rva), "name": self.label(rva)})
    }

    /// An argument to one address: `0x..` (an RVA, or a VA inside the image),
    /// `sub_..`, an exact name from any source, or a case-insensitive regex
    /// over names.
    pub fn resolve(&self, arg: &str) -> Result<Resolved> {
        if let Some(v) = parse_hex(arg) {
            let rva = if v >= self.image_base && v < self.image_base + self.size_of_image as u64 {
                v - self.image_base
            } else {
                v
            };
            if rva >= self.size_of_image as u64 {
                bail!(
                    "{arg} is outside the image (size 0x{:x})",
                    self.size_of_image
                );
            }
            return Ok(Resolved::One(rva as u32));
        }
        let mut exact: Vec<u32> = self
            .conn
            .prepare_cached("SELECT addr FROM names WHERE name=?1 UNION SELECT rva FROM functions WHERE name=?1")?
            .query_map([arg], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        exact.sort_unstable();
        exact.dedup();
        match exact.len() {
            1 => return Ok(Resolved::One(exact[0])),
            n if n > 1 => {
                return Ok(Resolved::Many(
                    exact.into_iter().map(|a| (a, self.label(a))).collect(),
                ));
            }
            _ => {}
        }
        // A name that is not a valid regex (`operator()`) matches literally.
        let re = regex_ci(arg, false).or_else(|_| regex_ci(&regex::escape(arg), false))?;
        let mut hits: Vec<u32> = Vec::new();
        let mut st = self.conn.prepare_cached(
            "SELECT addr, name FROM names UNION ALL SELECT rva, name FROM functions WHERE name != ''",
        )?;
        let rows = st.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (a, n) = row?;
            if re.is_match(&n) {
                hits.push(a);
            }
        }
        hits.sort_unstable();
        hits.dedup();
        Ok(match hits.len() {
            0 => Resolved::None,
            1 => Resolved::One(hits[0]),
            _ => Resolved::Many(hits.into_iter().map(|a| (a, self.label(a))).collect()),
        })
    }

    /// Resolve to exactly one address, or print why not and return the exit
    /// status.
    fn one(&mut self, arg: &str) -> Result<std::result::Result<u32, i32>> {
        match self.resolve(arg)? {
            Resolved::One(a) => Ok(Ok(a)),
            Resolved::None => {
                self.emit(
                    format!("none {arg}: no address or name matches"),
                    json!({"fact": "none", "query": arg}),
                )?;
                Ok(Err(NONE))
            }
            Resolved::Many(v) => {
                self.emit(
                    format!("ambiguous {arg}: {} matches; pass one RVA", v.len()),
                    json!({"fact": "ambiguous", "query": arg, "matches": v.len()}),
                )?;
                for (a, n) in v.iter().take(50) {
                    self.emit(
                        format!("match {} {n}", hx(*a)),
                        json!({"fact": "match", "rva": hx(*a), "name": n}),
                    )?;
                }
                if v.len() > 50 {
                    self.emit(
                        format!("more {}", v.len() - 50),
                        json!({"fact": "more", "count": v.len() - 50}),
                    )?;
                }
                Ok(Err(AMBIGUOUS))
            }
        }
    }

    /// Resolve to one function start: an address inside a function stands
    /// for that function.
    fn one_func(&mut self, arg: &str) -> Result<std::result::Result<u32, i32>> {
        let a = match self.one(arg)? {
            Ok(a) => a,
            Err(code) => return Ok(Err(code)),
        };
        if self.is_function(a)? {
            return Ok(Ok(a));
        }
        match self.containing(a)? {
            Some(f) => Ok(Ok(f)),
            None => {
                self.emit(
                    format!(
                        "none {} is not inside a known function (try whois)",
                        self.at(a)
                    ),
                    json!({"fact": "none", "rva": hx(a), "reason": "not in a function"}),
                )?;
                Ok(Err(NONE))
            }
        }
    }

    // -------------------------------------------------------------- commands

    pub fn info(&mut self) -> Result<i32> {
        let rows: Vec<(String, String)> = self
            .conn
            .prepare("SELECT key, value FROM meta ORDER BY key")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut warnings = String::new();
        for (k, v) in rows {
            if k == "warnings" {
                warnings = v;
                continue;
            }
            self.emit(
                format!("{k} {v}"),
                json!({"fact": "meta", "key": k, "value": v}),
            )?;
        }
        let secs: Vec<(String, u32, u32, u32, u32, f64)> = self
            .conn
            .prepare(
                "SELECT name, rva, vsize, raw_size, flags, entropy FROM sections ORDER BY idx",
            )?
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        for (name, rva, vsize, raw, flags, ent) in secs {
            let perms = format!(
                "{}{}{}",
                if flags & crate::pe::IMAGE_SCN_MEM_READ != 0 {
                    'r'
                } else {
                    '-'
                },
                if flags & crate::pe::IMAGE_SCN_MEM_WRITE != 0 {
                    'w'
                } else {
                    '-'
                },
                if flags & crate::pe::IMAGE_SCN_MEM_EXECUTE != 0 {
                    'x'
                } else {
                    '-'
                }
            );
            self.emit(
                format!(
                    "section {name} {} vsize=0x{vsize:x} raw=0x{raw:x} {perms} entropy={ent:.2}",
                    hx(rva)
                ),
                json!({"fact": "section", "name": name, "rva": hx(rva), "vsize": vsize,
                       "raw_size": raw, "perms": perms, "entropy": ent}),
            )?;
        }
        for w in warnings.lines().filter(|l| !l.is_empty()) {
            self.emit(
                format!("warning {w}"),
                json!({"fact": "warning", "text": w}),
            )?;
        }
        Ok(OK)
    }

    pub fn names(&mut self, pattern: &str, limit: usize) -> Result<i32> {
        let re = regex_ci(pattern, false)?;
        let mut rows: Vec<(u32, String, String, String)> = self
            .conn
            .prepare("SELECT addr, name, source, confidence FROM names ORDER BY addr")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        // Display labels that differ from every stored name (template-aware
        // funcsig labels) are searchable too, as source "label".
        let known: HashSet<(u32, String)> = rows.iter().map(|r| (r.0, r.1.clone())).collect();
        let labels: Vec<(u32, String, String)> = self
            .conn
            .prepare("SELECT rva, name, name_src FROM functions WHERE name != ''")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (a, n, src) in labels {
            let bare = n.trim_start_matches('~').to_owned();
            if !known.contains(&(a, bare.clone())) {
                rows.push((a, bare, "label".into(), src));
            }
        }
        rows.sort_by(|x, y| x.0.cmp(&y.0).then_with(|| x.1.cmp(&y.1)));
        let hits: Vec<_> = rows.into_iter().filter(|r| re.is_match(&r.1)).collect();
        for (a, n, s, c) in hits.iter().take(limit) {
            self.emit(
                format!("{} {n} {s} {c}", hx(*a)),
                json!({"fact": "name", "rva": hx(*a), "name": n, "source": s, "confidence": c}),
            )?;
        }
        self.more(hits.len(), limit)?;
        Ok(if hits.is_empty() { NONE } else { OK })
    }

    fn more(&mut self, total: usize, shown: usize) -> Result<()> {
        if total > shown {
            self.emit(
                format!("more {} (raise --limit)", total - shown),
                json!({"fact": "more", "count": total - shown}),
            )?;
        }
        Ok(())
    }

    pub fn func(&mut self, arg: &str, limit: usize, lines: usize) -> Result<i32> {
        let addrs: Vec<u32> = match self.resolve(arg)? {
            Resolved::One(a) => vec![a],
            Resolved::Many(v) => v.into_iter().map(|x| x.0).collect(),
            Resolved::None => {
                self.emit(
                    format!("none {arg}: no address or name matches"),
                    json!({"fact": "none", "query": arg}),
                )?;
                return Ok(NONE);
            }
        };
        let mut funcs: Vec<u32> = Vec::new();
        let mut others: Vec<u32> = Vec::new();
        for a in addrs {
            if self.is_function(a)? {
                funcs.push(a);
            } else if let Some(f) = self.containing(a)? {
                funcs.push(f);
            } else {
                others.push(a);
            }
        }
        funcs.sort_unstable();
        funcs.dedup();
        if funcs.is_empty() {
            for a in others.iter().take(limit) {
                self.emit(
                    format!("not-a-function {} (try whois)", self.at(*a)),
                    json!({"fact": "not-a-function", "rva": hx(*a), "name": self.label(*a)}),
                )?;
            }
            self.more(others.len(), limit)?;
        } else if !others.is_empty() {
            self.emit(
                format!(
                    "also {} matches that are not functions (try names)",
                    others.len()
                ),
                json!({"fact": "also", "not_functions": others.len()}),
            )?;
        }
        for &f in funcs.iter().take(limit) {
            self.func_block(f, lines)?;
        }
        self.more(funcs.len(), limit)?;
        Ok(if funcs.is_empty() { NONE } else { OK })
    }

    fn func_block(&mut self, f: u32, lines: usize) -> Result<()> {
        let Some((size, end, kind, ninsn, _name, name_src)) = self.func_row(f)? else {
            return Ok(());
        };
        let fj = hx(f);
        self.emit(
            format!("func {}", self.at(f)),
            json!({"fact": "func", "func": fj, "name": self.label(f), "name_src": name_src}),
        )?;
        self.emit(
            format!("  bounds {}-{} size={size} kind={kind} insns={ninsn}", hx(f), hx(end)),
            json!({"fact": "bounds", "func": fj, "start": hx(f), "end": hx(end), "size": size, "kind": kind, "insns": ninsn}),
        )?;
        let chunks: Vec<(u32, u32)> = self
            .conn
            .prepare_cached("SELECT start, end FROM chunks WHERE func=?1 ORDER BY start")?
            .query_map([f], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        if chunks.len() > 1 {
            for (s, e) in chunks {
                self.emit(
                    format!("  chunk {}-{}", hx(s), hx(e)),
                    json!({"fact": "chunk", "func": fj, "start": hx(s), "end": hx(e)}),
                )?;
            }
        }
        let names: Vec<(String, String, String, String)> = self
            .conn
            .prepare_cached("SELECT name, source, confidence, detail FROM names WHERE addr=?1")?
            .query_map([f], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (n, s, c, d) in names.iter().take(lines) {
            let tail = if d.is_empty() {
                String::new()
            } else {
                format!(" | {}", cut(d, 300))
            };
            self.emit(
                format!("  name {n} {s} {c}{tail}"),
                json!({"fact": "name", "func": fj, "name": n, "source": s, "confidence": c, "detail": d}),
            )?;
        }
        if names.len() > lines {
            self.emit(
                format!("  more names {}", names.len() - lines),
                json!({"fact": "more", "what": "names", "count": names.len() - lines}),
            )?;
        }
        if let Some((file, k)) = self.src_of(f)? {
            self.emit(
                format!("  src {file} {k}"),
                json!({"fact": "src", "func": fj, "file": file, "kind": k}),
            )?;
        }
        let (ncallers, nsites): (u32, u32) = self
            .conn
            .prepare_cached("SELECT count(DISTINCT caller), count(*) FROM calls WHERE callee=?1")?
            .query_row([f], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let nptr: u32 = self
            .conn
            .prepare_cached(
                "SELECT (SELECT count(*) FROM xrefs WHERE target=?1) + (SELECT count(*) FROM dataptrs WHERE target=?1) \
                 + (SELECT count(*) FROM vslots WHERE target=?1)",
            )?
            .query_row([f], |r| r.get(0))?;
        self.emit(
            format!("  callers {ncallers} sites={nsites} address-refs={nptr}"),
            json!({"fact": "callers", "func": fj, "callers": ncallers, "sites": nsites, "address_refs": nptr}),
        )?;
        let callees: Vec<(u32, u8, u32, u32)> = self
            .conn
            .prepare_cached(
                "SELECT callee, kind, count(*), min(site) FROM calls WHERE caller=?1 GROUP BY callee, kind ORDER BY min(site)",
            )?
            .query_map([f], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (c, k, n, site) in callees.iter().take(lines) {
            let word = match *k {
                disasm::CALL_TAIL => "tail",
                disasm::CALL_IMPORT | disasm::CALL_IMPORT_JMP => "import",
                _ => "callee",
            };
            let times = if *n > 1 {
                format!(" x{n}")
            } else {
                String::new()
            };
            self.emit(
                format!("  {word} {} @{}{times}", self.at(*c), hx(*site)),
                json!({"fact": word, "func": fj, "target": hx(*c), "name": self.label(*c), "site": hx(*site), "count": n}),
            )?;
        }
        if callees.len() > lines {
            self.emit(
                format!("  more callees {}", callees.len() - lines),
                json!({"fact": "more", "what": "callees", "count": callees.len() - lines}),
            )?;
        }
        let strs = self.strings_of(f)?;
        let mut seen = HashSet::new();
        let strs: Vec<_> = strs.into_iter().filter(|s| seen.insert(s.1)).collect();
        for (insn, s, enc, text) in strs.iter().take(lines) {
            self.emit(
                format!("  string {} {}{} @{}", hx(*s), if *enc == 2 { "u" } else { "" }, quote(text, 160), hx(*insn)),
                json!({"fact": "string", "func": fj, "rva": hx(*s), "text": text, "insn": hx(*insn)}),
            )?;
        }
        if strs.len() > lines {
            self.emit(
                format!("  more strings {}", strs.len() - lines),
                json!({"fact": "more", "what": "strings", "count": strs.len() - lines}),
            )?;
        }
        let vs: Vec<(u32, u32, String, u32)> = self
            .conn
            .prepare_cached(
                "SELECT v.vtable, v.slot, t.class, t.offset FROM vslots v JOIN vtables t ON t.rva=v.vtable WHERE v.target=?1 ORDER BY v.vtable",
            )?
            .query_map([f], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (vt, slot, class, off) in vs.iter().take(lines) {
            let n = slot_name(class, *slot, *off);
            self.emit(
                format!("  vslot {n} vtable={}", hx(*vt)),
                json!({"fact": "vslot", "func": fj, "name": n, "vtable": hx(*vt), "slot": slot}),
            )?;
        }
        if vs.len() > lines {
            self.emit(
                format!("  more vslots {}", vs.len() - lines),
                json!({"fact": "more", "what": "vslots", "count": vs.len() - lines}),
            )?;
        }
        Ok(())
    }

    fn src_of(&self, f: u32) -> Result<Option<(String, String)>> {
        Ok(self
            .conn
            .prepare_cached("SELECT file, kind FROM func_src WHERE func=?1")?
            .query_row([f], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?)
    }

    fn strings_of(&self, f: u32) -> Result<Vec<(u32, u32, u8, String)>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT x.insn, s.rva, s.enc, s.text FROM xrefs x JOIN strings s ON s.rva=x.target WHERE x.func=?1 ORDER BY x.insn",
            )?
            .query_map([f], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn fnstr(&mut self, arg: &str) -> Result<i32> {
        let f = match self.one_func(arg)? {
            Ok(f) => f,
            Err(c) => return Ok(c),
        };
        let strs = self.strings_of(f)?;
        for (insn, s, enc, text) in &strs {
            self.emit(
                format!("{} {} {}{}", hx(*insn), hx(*s), if *enc == 2 { "u" } else { "" }, quote(text, 300)),
                json!({"fact": "string", "func": hx(f), "insn": hx(*insn), "rva": hx(*s), "text": text}),
            )?;
        }
        Ok(if strs.is_empty() { NONE } else { OK })
    }

    pub fn strings(&mut self, pattern: &str, case: bool, limit: usize) -> Result<i32> {
        let re = regex_ci(pattern, case)?;
        let rows: Vec<(u32, u8, String)> = self
            .conn
            .prepare("SELECT rva, enc, text FROM strings ORDER BY rva")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let hits: Vec<_> = rows.into_iter().filter(|r| re.is_match(&r.2)).collect();
        for (rva, enc, text) in hits.iter().take(limit) {
            let funcs: Vec<u32> = self
                .conn
                .prepare_cached("SELECT DISTINCT func FROM xrefs WHERE target=?1 ORDER BY func")?
                .query_map([*rva], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let ndata: u32 = self
                .conn
                .prepare_cached("SELECT count(*) FROM dataptrs WHERE target=?1")?
                .query_row([*rva], |r| r.get(0))?;
            let mut refs: Vec<String> = funcs.iter().take(8).map(|&f| self.at(f)).collect();
            if funcs.len() > 8 {
                refs.push(format!("+{} more", funcs.len() - 8));
            }
            if ndata > 0 {
                refs.push(format!("data x{ndata}"));
            }
            let refs_text = if refs.is_empty() {
                "(none)".to_owned()
            } else {
                refs.join(", ")
            };
            self.emit(
                format!("{} {}{} <- {refs_text}", hx(*rva), if *enc == 2 { "u" } else { "" }, quote(text, 200)),
                json!({"fact": "string", "rva": hx(*rva), "text": text, "utf16": *enc == 2,
                       "funcs": funcs.iter().map(|&f| self.addr_json(f)).collect::<Vec<_>>(), "data_refs": ndata}),
            )?;
        }
        self.more(hits.len(), limit)?;
        Ok(if hits.is_empty() { NONE } else { OK })
    }

    /// Direct-call neighbours, breadth first. `up`: callers, else callees.
    pub fn graph(&mut self, arg: &str, up: bool, depth: usize, limit: usize) -> Result<i32> {
        let start = match self.one(arg)? {
            Ok(a) => a,
            Err(c) => return Ok(c),
        };
        let start = if !self.is_function(start)? {
            self.containing(start)?.unwrap_or(start)
        } else {
            start
        };
        let sql = if up {
            "SELECT caller, callee, kind, count(*), min(site) FROM calls WHERE callee=?1 GROUP BY caller, kind ORDER BY caller"
        } else {
            "SELECT caller, callee, kind, count(*), min(site) FROM calls WHERE caller=?1 GROUP BY callee, kind ORDER BY min(site)"
        };
        let mut seen: HashSet<u32> = HashSet::from([start]);
        let mut frontier = vec![start];
        let mut printed = 0usize;
        // Edges found but not printed at the node where --limit stopped the
        // walk; the walk goes no further.
        let mut left = 0usize;
        'walk: for d in 1..=depth.max(1) {
            let mut next = Vec::new();
            for &node in &frontier {
                let edges: Vec<(u32, u32, u8, u32, u32)> = self
                    .conn
                    .prepare_cached(sql)?
                    .query_map([node], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                    })?
                    .collect::<rusqlite::Result<_>>()?;
                let count = edges.len();
                for (i, (a, b, k, n, site)) in edges.into_iter().enumerate() {
                    if printed >= limit {
                        left = count - i;
                        break 'walk;
                    }
                    let other = if up { a } else { b };
                    let importish = matches!(k, disasm::CALL_IMPORT | disasm::CALL_IMPORT_JMP);
                    if !importish && seen.insert(other) {
                        next.push(other);
                    }
                    {
                        printed += 1;
                        let times = if n > 1 {
                            format!(" x{n}")
                        } else {
                            String::new()
                        };
                        let tag = if k == disasm::CALL_DIRECT {
                            String::new()
                        } else {
                            format!(" [{}]", call_kind_name(k))
                        };
                        self.emit(
                            format!("{d} {} -> {} @{}{times}{tag}", self.at(a), self.at(b), hx(site)),
                            json!({"fact": "edge", "depth": d, "caller": self.addr_json(a), "callee": self.addr_json(b),
                                   "site": hx(site), "count": n, "kind": call_kind_name(k)}),
                        )?;
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        if left > 0 {
            self.emit(
                format!("more stopped at --limit {limit} with {left}+ edges left (raise --limit or lower --depth)"),
                json!({"fact": "more", "limit": limit, "at_least": left}),
            )?;
        }
        Ok(if printed == 0 { NONE } else { OK })
    }

    fn neighbours(&self, node: u32, up: bool) -> Result<Vec<(u32, u32)>> {
        let sql = if up {
            "SELECT caller, min(site) FROM calls WHERE callee=?1 AND kind IN (0,1) GROUP BY caller"
        } else {
            "SELECT callee, min(site) FROM calls WHERE caller=?1 AND kind IN (0,1) GROUP BY callee"
        };
        Ok(self
            .conn
            .prepare_cached(sql)?
            .query_map([node], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Shortest direct-call path, by bidirectional breadth-first search.
    pub fn path(&mut self, from: &str, to: &str, max_depth: usize) -> Result<i32> {
        let a = match self.one_func(from)? {
            Ok(a) => a,
            Err(c) => return Ok(c),
        };
        let b = match self.one_func(to)? {
            Ok(b) => b,
            Err(c) => return Ok(c),
        };
        const BUDGET: usize = 400_000;
        // node -> (previous node, site) going forward from a / backward from b
        let mut fwd: HashMap<u32, (u32, u32)> = HashMap::from([(a, (a, 0))]);
        let mut bwd: HashMap<u32, (u32, u32)> = HashMap::from([(b, (b, 0))]);
        let mut ff = vec![a];
        let mut bf = vec![b];
        let mut meet = (a == b).then_some(a);
        let mut depth = 0;
        while meet.is_none() && depth < max_depth && !ff.is_empty() && !bf.is_empty() {
            depth += 1;
            let forward = ff.len() <= bf.len();
            let (frontier, mine, other) = if forward {
                (&mut ff, &mut fwd, &bwd)
            } else {
                (&mut bf, &mut bwd, &fwd)
            };
            let mut next = Vec::new();
            'outer: for &node in frontier.iter() {
                for (n, site) in self.neighbours(node, !forward)? {
                    if mine.contains_key(&n) {
                        continue;
                    }
                    mine.insert(n, (node, site));
                    if other.contains_key(&n) {
                        meet = Some(n);
                        break 'outer;
                    }
                    next.push(n);
                }
            }
            *frontier = next;
            if fwd.len() + bwd.len() > BUDGET {
                self.emit(
                    "none search budget exhausted",
                    json!({"fact": "none", "reason": "budget"}),
                )?;
                return Ok(NONE);
            }
        }
        let Some(m) = meet else {
            self.emit(
                format!(
                    "none no direct-call path from {} to {} within {max_depth} calls",
                    self.at(a),
                    self.at(b)
                ),
                json!({"fact": "none", "from": hx(a), "to": hx(b), "max_depth": max_depth}),
            )?;
            return Ok(NONE);
        };
        let mut hops: Vec<(u32, u32, u32)> = Vec::new(); // caller, callee, site
        let mut n = m;
        while n != a {
            let (p, site) = fwd[&n];
            hops.push((p, n, site));
            n = p;
        }
        hops.reverse();
        let mut n = m;
        while n != b {
            let (nx, site) = bwd[&n];
            hops.push((n, nx, site));
            n = nx;
        }
        self.emit(
            format!(
                "path {} calls from {} to {}",
                hops.len(),
                self.at(a),
                self.at(b)
            ),
            json!({"fact": "path", "length": hops.len(), "from": hx(a), "to": hx(b)}),
        )?;
        for (i, (x, y, site)) in hops.iter().enumerate() {
            self.emit(
                format!("{} {} -> {} @{}", i + 1, self.at(*x), self.at(*y), hx(*site)),
                json!({"fact": "hop", "step": i + 1, "caller": self.addr_json(*x), "callee": self.addr_json(*y), "site": hx(*site)}),
            )?;
        }
        Ok(OK)
    }

    pub fn xrefs(&mut self, arg: &str, limit: usize) -> Result<i32> {
        let t = match self.one(arg)? {
            Ok(a) => a,
            Err(c) => return Ok(c),
        };
        let calls: Vec<(u32, u32, u8)> = self
            .conn
            .prepare("SELECT site, caller, kind FROM calls WHERE callee=?1 ORDER BY site")?
            .query_map([t], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let refs: Vec<(u32, u32, u8)> = self
            .conn
            .prepare("SELECT insn, func, kind FROM xrefs WHERE target=?1 ORDER BY insn")?
            .query_map([t], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let data: Vec<u32> = self
            .conn
            .prepare("SELECT at FROM dataptrs WHERE target=?1 ORDER BY at")?
            .query_map([t], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let vs: Vec<(u32, u32, String, u32)> = self
            .conn
            .prepare("SELECT v.vtable, v.slot, t.class, t.offset FROM vslots v JOIN vtables t ON t.rva=v.vtable WHERE v.target=?1 ORDER BY v.vtable")?
            .query_map([t], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        self.emit(
            format!("xrefs {} calls={} refs={} data={} vslots={}", self.at(t), calls.len(), refs.len(), data.len(), vs.len()),
            json!({"fact": "xrefs", "target": self.addr_json(t), "calls": calls.len(), "refs": refs.len(), "data": data.len(), "vslots": vs.len()}),
        )?;
        let mut shown = 0usize;
        let total = calls.len() + refs.len() + data.len() + vs.len();
        for (site, caller, k) in calls {
            if shown >= limit {
                break;
            }
            shown += 1;
            self.emit(
                format!("call {} in {} [{}]", hx(site), self.at(caller), call_kind_name(k)),
                json!({"fact": "call", "site": hx(site), "func": self.addr_json(caller), "kind": call_kind_name(k)}),
            )?;
        }
        for (insn, func, k) in refs {
            if shown >= limit {
                break;
            }
            shown += 1;
            self.emit(
                format!("ref {} {} in {}", xref_kind_name(k), hx(insn), self.at(func)),
                json!({"fact": "ref", "kind": xref_kind_name(k), "insn": hx(insn), "func": self.addr_json(func)}),
            )?;
        }
        for at in data {
            if shown >= limit {
                break;
            }
            shown += 1;
            // A {name, function} pair (a luaL_Reg-like table) shows its name.
            let prev = self.pointer_before(at)?;
            let tail = prev
                .map(|p| format!(" after {}", self.at(p)))
                .unwrap_or_default();
            self.emit(
                format!("data {}{tail}", self.at(at)),
                json!({"fact": "data", "at": hx(at), "previous": prev.map(|p| self.addr_json(p))}),
            )?;
        }
        for (vt, slot, class, off) in vs {
            if shown >= limit {
                break;
            }
            shown += 1;
            let n = slot_name(&class, slot, off);
            self.emit(
                format!("vslot {n} vtable={}", hx(vt)),
                json!({"fact": "vslot", "name": n, "vtable": hx(vt), "slot": slot}),
            )?;
        }
        self.more(total, shown)?;
        Ok(if total == 0 { NONE } else { OK })
    }

    fn pointer_before(&self, at: u32) -> Result<Option<u32>> {
        Ok(self
            .conn
            .prepare_cached("SELECT target FROM dataptrs WHERE at=?1")?
            .query_row([at.wrapping_sub(8)], |r| r.get(0))
            .optional()?)
    }

    fn vtable_block(
        &mut self,
        rva: u32,
        class: &str,
        col: u32,
        off: u32,
        nslots: u32,
        lines: usize,
    ) -> Result<()> {
        let label = if off == 0 {
            format!("{class}::vftable")
        } else {
            format!("{class}::vftable@0x{off:x}")
        };
        self.emit(
            format!("vtable {} {label} offset=0x{off:x} slots={nslots} col={}", hx(rva), hx(col)),
            json!({"fact": "vtable", "rva": hx(rva), "class": class, "offset": off, "slots": nslots, "col": hx(col)}),
        )?;
        let slots: Vec<(u32, u32)> = self
            .conn
            .prepare_cached("SELECT slot, target FROM vslots WHERE vtable=?1 ORDER BY slot")?
            .query_map([rva], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (slot, t) in slots.iter().take(lines) {
            let shared: u32 = self
                .conn
                .prepare_cached("SELECT count(*) FROM vslots WHERE target=?1")?
                .query_row([*t], |r| r.get(0))?;
            let tail = if shared > 1 {
                format!(" shared={shared}")
            } else {
                String::new()
            };
            self.emit(
                format!("  slot {slot} {}{tail}", self.at(*t)),
                json!({"fact": "slot", "vtable": hx(rva), "slot": slot, "target": self.addr_json(*t), "shared": shared}),
            )?;
        }
        if slots.len() > lines {
            self.emit(
                format!("  more slots {}", slots.len() - lines),
                json!({"fact": "more", "what": "slots", "count": slots.len() - lines}),
            )?;
        }
        Ok(())
    }

    pub fn class(&mut self, pattern: &str, limit: usize, lines: usize) -> Result<i32> {
        let re = regex_ci(pattern, false)?;
        let rows: Vec<(u32, String, u32, u32, u32)> = self
            .conn
            .prepare("SELECT rva, class, col, offset, nslots FROM vtables ORDER BY class, offset")?
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let hits: Vec<_> = rows.into_iter().filter(|r| re.is_match(&r.1)).collect();
        for (rva, class, col, off, n) in hits.iter().take(limit) {
            self.vtable_block(*rva, class, *col, *off, *n, lines)?;
        }
        self.more(hits.len(), limit)?;
        Ok(if hits.is_empty() { NONE } else { OK })
    }

    pub fn vtable(&mut self, arg: &str, lines: usize) -> Result<i32> {
        let a = match self.one(arg)? {
            Ok(a) => a,
            Err(c) => return Ok(c),
        };
        let row: Option<(u32, String, u32, u32, u32)> = self
            .conn
            .prepare("SELECT rva, class, col, offset, nslots FROM vtables WHERE rva <= ?1 + 8 ORDER BY rva DESC LIMIT 1")?
            .query_row([a], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .optional()?;
        match row {
            Some((rva, class, col, off, n)) if a + 8 >= rva && a < rva + 8 * n => {
                self.vtable_block(rva, &class, col, off, n, lines)?;
                Ok(OK)
            }
            _ => {
                self.emit(
                    format!("none {} is not in a vtable", self.at(a)),
                    json!({"fact": "none", "rva": hx(a)}),
                )?;
                Ok(NONE)
            }
        }
    }

    pub fn file(&mut self, substr: &str, limit: usize) -> Result<i32> {
        let rows: Vec<(u32, String, String)> = self
            .conn
            .prepare("SELECT func, file, kind FROM func_src WHERE file LIKE '%' || ?1 || '%' ORDER BY file, func")?
            .query_map([substr], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut files: Vec<(String, usize, usize)> = Vec::new();
        for (_, file, kind) in &rows {
            match files.last_mut() {
                Some(l) if &l.0 == file => {
                    l.1 += 1;
                    l.2 += (kind == "direct") as usize;
                }
                _ => files.push((file.clone(), 1, (kind == "direct") as usize)),
            }
        }
        for (file, n, direct) in &files {
            self.emit(
                format!("file {file} functions={n} direct={direct}"),
                json!({"fact": "file", "file": file, "functions": n, "direct": direct}),
            )?;
        }
        for (f, file, kind) in rows.iter().take(limit) {
            self.emit(
                format!("{} {file} {kind}", self.at(*f)),
                json!({"fact": "func", "func": self.addr_json(*f), "file": file, "kind": kind}),
            )?;
        }
        self.more(rows.len(), limit)?;
        Ok(if rows.is_empty() { NONE } else { OK })
    }

    pub fn whois(&mut self, arg: &str) -> Result<i32> {
        let a = match self.one(arg)? {
            Ok(a) => a,
            Err(c) => return Ok(c),
        };
        self.emit(
            format!("whois {}", hx(a)),
            json!({"fact": "whois", "rva": hx(a)}),
        )?;
        let sec: Option<String> = self
            .conn
            .prepare("SELECT name FROM sections WHERE rva <= ?1 AND ?1 < rva + max(vsize, raw_size) ORDER BY rva DESC LIMIT 1")?
            .query_row([a], |r| r.get(0))
            .optional()?;
        if let Some(s) = &sec {
            self.emit(
                format!("  section {s}"),
                json!({"fact": "section", "name": s}),
            )?;
        }
        let func = if self.is_function(a)? {
            Some(a)
        } else {
            self.containing(a)?
        };
        if let Some(f) = func {
            let (size, _, kind, _, _, name_src) = self.func_row(f)?.unwrap_or_default();
            self.emit(
                format!("  func {} +0x{:x} size={size} kind={kind} best={name_src}", self.at(f), a - f),
                json!({"fact": "func", "func": self.addr_json(f), "offset": a - f, "size": size, "kind": kind, "best": name_src}),
            )?;
        }
        let mut found = func.is_some();
        let mut addrs = vec![a];
        if let Some(f) = func.filter(|&f| f != a) {
            addrs.push(f);
        }
        for &x in &addrs {
            let names: Vec<(String, String, String, String)> = self
                .conn
                .prepare_cached("SELECT name, source, confidence, detail FROM names WHERE addr=?1 ORDER BY source")?
                .query_map([x], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let whose = if x == a {
                String::new()
            } else {
                format!(" (of func {})", hx(x))
            };
            for (n, s, c, d) in names.iter().take(24) {
                found = true;
                let tail = if d.is_empty() {
                    String::new()
                } else {
                    format!(" | {}", cut(d, 300))
                };
                self.emit(
                    format!("  {s} {n} {c}{whose}{tail}"),
                    json!({"fact": "name", "addr": hx(x), "source": s, "name": n, "confidence": c, "detail": d}),
                )?;
            }
            if names.len() > 24 {
                self.emit(
                    format!("  more names {}", names.len() - 24),
                    json!({"fact": "more", "what": "names", "count": names.len() - 24}),
                )?;
            }
        }
        if let Some(f) = func
            && let Some((file, k)) = self.src_of(f)?
        {
            found = true;
            self.emit(
                format!("  file {file} {k}"),
                json!({"fact": "file", "file": file, "kind": k}),
            )?;
        }
        let s: Option<(u8, String)> = self
            .conn
            .prepare("SELECT enc, text FROM strings WHERE rva=?1")?
            .query_row([a], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        if let Some((enc, text)) = s {
            found = true;
            self.emit(
                format!(
                    "  string {}{}",
                    if enc == 2 { "u" } else { "" },
                    quote(&text, 300)
                ),
                json!({"fact": "string", "text": text}),
            )?;
        }
        let ty: Option<(String, String)> = self
            .conn
            .prepare("SELECT name, mangled FROM types WHERE td=?1")?
            .query_row([a], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        if let Some((n, m)) = ty {
            found = true;
            self.emit(
                format!("  type {n} | {m}"),
                json!({"fact": "type", "name": n, "mangled": m}),
            )?;
        }
        let vs: Vec<(u32, u32, String, u32)> = self
            .conn
            .prepare("SELECT v.vtable, v.slot, t.class, t.offset FROM vslots v JOIN vtables t ON t.rva=v.vtable WHERE v.target=?1 ORDER BY v.vtable")?
            .query_map([a], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (vt, slot, class, off) in vs.iter().take(16) {
            found = true;
            let n = slot_name(class, *slot, *off);
            self.emit(
                format!("  vslot {n} vtable={}", hx(*vt)),
                json!({"fact": "vslot", "name": n, "vtable": hx(*vt)}),
            )?;
        }
        if vs.len() > 16 {
            self.emit(
                format!("  more vslots {}", vs.len() - 16),
                json!({"fact": "more", "what": "vslots", "count": vs.len() - 16}),
            )?;
        }
        let ptr: Option<u32> = self
            .conn
            .prepare("SELECT target FROM dataptrs WHERE at=?1")?
            .query_row([a], |r| r.get(0))
            .optional()?;
        if let Some(p) = ptr {
            found = true;
            self.emit(
                format!("  points-to {}", self.at(p)),
                json!({"fact": "points-to", "target": self.addr_json(p)}),
            )?;
        }
        if !found {
            self.emit("  nothing known", json!({"fact": "none"}))?;
        }
        Ok(if found { OK } else { NONE })
    }

    /// Check known RVAs, as `name_functions.py --validate` does. Spec lines
    /// (`#` comments): `<rva> <name-substring>` must be funcsig-named with
    /// the substring in its name or signature; `<rva> ~ <src-substring>`
    /// must NOT be named (the binary embeds no signature for it) and must be
    /// attributed to a source file containing the substring.
    pub fn validate(&mut self, spec: &Path) -> Result<i32> {
        let text =
            std::fs::read_to_string(spec).with_context(|| format!("reading {}", spec.display()))?;
        let mut ok = true;
        let mut checked = 0;
        for raw in text.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (rva_text, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let rest = rest.trim();
            let rva = u32::from_str_radix(rva_text.trim_start_matches("0x"), 16)
                .with_context(|| format!("bad RVA in spec line {line:?}"))?;
            checked += 1;
            let f = if self.is_function(rva)? {
                Some(rva)
            } else {
                self.containing(rva)?
            };
            let named: Option<(String, String, String)> = match f {
                Some(f) => self
                    .conn
                    .prepare_cached(
                        "SELECT name, confidence, detail FROM names WHERE addr=?1 AND source IN ('funcsig','pretty')",
                    )?
                    .query_row([f], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .optional()?,
                None => None,
            };
            let src = match f {
                Some(f) => self.src_of(f)?.map(|s| s.0).unwrap_or_default(),
                None => String::new(),
            };
            let (verdict, detail) = if let Some(want) = rest.strip_prefix('~') {
                let want = want.trim().to_lowercase();
                match &named {
                    Some((n, ..)) => (
                        false,
                        format!("UNEXPECTED NAME {n:?} (binary embeds no __FUNCSIG__ here)"),
                    ),
                    None if !want.is_empty() && !src.to_lowercase().contains(&want) => (
                        false,
                        format!("WRONG SOURCE got {src:?}; expected ~ {want:?}"),
                    ),
                    None => (
                        true,
                        format!(
                            "OK not funcsig-named; source={}",
                            if src.is_empty() { "-" } else { &src }
                        ),
                    ),
                }
            } else {
                match &named {
                    None => (
                        false,
                        format!(
                            "NOT NAMED (expected ~ {rest:?}; source={})",
                            if src.is_empty() { "-" } else { &src }
                        ),
                    ),
                    Some((n, conf, d)) => {
                        let sig = d.split(" || ").next().unwrap_or("");
                        let want = rest.to_lowercase();
                        if !want.is_empty()
                            && !n.to_lowercase().contains(&want)
                            && !sig.to_lowercase().contains(&want)
                        {
                            (
                                false,
                                format!("MISMATCH got {n:?} [{conf}]; expected ~ {rest:?}"),
                            )
                        } else {
                            (
                                true,
                                format!(
                                    "OK {n} [{conf}] src={}",
                                    if src.is_empty() { "-" } else { &src }
                                ),
                            )
                        }
                    }
                }
            };
            ok &= verdict;
            self.emit(
                format!("{} {detail}", hx(rva)),
                json!({"fact": "check", "rva": hx(rva), "ok": verdict, "detail": detail}),
            )?;
        }
        let summary = if ok { "ALL OK" } else { "MISMATCHES PRESENT" };
        self.emit(
            format!("validate {checked} checked: {summary}"),
            json!({"fact": "validate", "checked": checked, "ok": ok}),
        )?;
        Ok(if ok { OK } else { NONE })
    }

    // ------------------------------------------------------- binary commands

    pub fn dis(&mut self, arg: &str, max: usize, context: usize, show_bytes: bool) -> Result<i32> {
        let a = match self.one(arg)? {
            Ok(a) => a,
            Err(c) => return Ok(c),
        };
        let data = self.binary()?;
        let pe = self.pe(&data)?;
        let func = if self.is_function(a)? {
            Some(a)
        } else {
            self.containing(a)?
        };
        let mut items: Vec<Item> = Vec::new();
        match func {
            Some(f) => {
                let chunks: Vec<(u32, u32)> = self
                    .conn
                    .prepare("SELECT start, end FROM chunks WHERE func=?1 ORDER BY start")?
                    .query_map([f], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                let span = (
                    chunks.iter().map(|c| c.0).min().unwrap_or(f),
                    chunks.iter().map(|c| c.1).max().unwrap_or(f),
                );
                for (s, e) in &chunks {
                    disasm::decode_chunk(&pe, span, *s, *e, &mut items);
                }
                let (size, ..) = self.func_row(f)?.unwrap_or_default();
                self.emit(
                    format!("func {} size={size} chunks={}", self.at(f), chunks.len()),
                    json!({"fact": "func", "func": self.addr_json(f), "size": size}),
                )?;
            }
            None => {
                if !pe.is_exec(a) {
                    self.emit(
                        format!("none {} is not code", self.at(a)),
                        json!({"fact": "none", "rva": hx(a)}),
                    )?;
                    return Ok(NONE);
                }
                self.emit(
                    format!(
                        "raw {}: not in a known function; decoding {max} instructions",
                        hx(a)
                    ),
                    json!({"fact": "raw", "rva": hx(a)}),
                )?;
                let end = pe.section_of(a).map(|s| s.raw_end()).unwrap_or(a);
                disasm::decode_chunk(
                    &pe,
                    (a, end),
                    a,
                    end.min(a.saturating_add(16 * max as u32)),
                    &mut items,
                );
            }
        }
        let ip_of = |it: &Item| match it {
            Item::Insn(i) => i.ip() as u32,
            Item::Table { at, .. } => *at,
        };
        let (lo, hi) = if func == Some(a) || func.is_none() {
            (0, items.len().min(max))
        } else {
            let i = items.iter().rposition(|it| ip_of(it) <= a).unwrap_or(0);
            (
                i.saturating_sub(context),
                (i + context + 1).min(items.len()),
            )
        };
        let window = &items[lo..hi];
        // Names for every call, branch and RIP-relative target outside the
        // function itself.
        let mut syms: HashMap<u64, String> = HashMap::new();
        let mut comments: HashMap<u32, String> = HashMap::new();
        for it in window {
            let Item::Insn(i) = it else { continue };
            let mut targets = Vec::new();
            if i.is_call_near() || i.is_jmp_short_or_near() || i.is_jcc_short_or_near() {
                targets.push(i.near_branch_target());
            }
            if i.is_ip_rel_memory_operand() {
                targets.push(i.ip_rel_memory_address());
            }
            for t in targets {
                if t >= self.size_of_image as u64 {
                    continue;
                }
                let t32 = t as u32;
                if func.is_some() && self.containing(t32)? == func && !self.is_function(t32)? {
                    continue; // a branch inside this function: keep the number
                }
                let l = self.label(t32);
                if !l.is_empty() && !l.starts_with('"') && !l.starts_with("u\"") {
                    syms.insert(t, l);
                }
            }
            if i.is_ip_rel_memory_operand() {
                let t = i.ip_rel_memory_address();
                if t < self.size_of_image as u64 {
                    let t = t as u32;
                    let s: Option<(u8, String)> = self
                        .conn
                        .prepare_cached("SELECT enc, text FROM strings WHERE rva=?1")?
                        .query_row([t], |r| Ok((r.get(0)?, r.get(1)?)))
                        .optional()?;
                    if let Some((enc, text)) = s {
                        comments.insert(
                            i.ip() as u32,
                            format!("{}{}", if enc == 2 { "u" } else { "" }, quote(&text, 120)),
                        );
                    } else if let Some(p) = self
                        .conn
                        .prepare_cached("SELECT target FROM dataptrs WHERE at=?1")?
                        .query_row([t], |r| r.get::<_, u32>(0))
                        .optional()?
                    {
                        comments.insert(i.ip() as u32, format!("-> {}", self.at(p)));
                    }
                }
            }
        }
        let mut fmt = IntelFormatter::with_options(Some(Box::new(Syms(syms))), None);
        fmt.options_mut().set_hex_prefix("0x");
        fmt.options_mut().set_hex_suffix("");
        fmt.options_mut().set_uppercase_hex(false);
        fmt.options_mut().set_space_after_operand_separator(true);
        fmt.options_mut().set_first_operand_char_index(8);
        fmt.options_mut().set_branch_leading_zeros(false);
        fmt.options_mut().set_show_branch_size(false);
        for it in window {
            match it {
                Item::Insn(i) => {
                    let ip = i.ip() as u32;
                    let mut t = Text(String::new());
                    fmt.format(i, &mut t);
                    let bytes = if show_bytes {
                        let raw = pe.read(ip, i.len()).unwrap_or(&[]);
                        format!(
                            "{:<22}",
                            raw.iter().map(|b| format!("{b:02x}")).collect::<String>()
                        )
                    } else {
                        String::new()
                    };
                    let comment = comments
                        .get(&ip)
                        .map(|c| format!("  ; {c}"))
                        .unwrap_or_default();
                    let mark = if ip == a && func != Some(a) {
                        ">> "
                    } else {
                        ""
                    };
                    self.emit(
                        format!("{mark}{} {bytes} {}{comment}", hx(ip), t.0),
                        json!({"fact": "insn", "ip": hx(ip), "text": t.0, "comment": comments.get(&ip),
                               "len": i.len(), "marked": ip == a}),
                    )?;
                }
                Item::Table { at, len, scale } => {
                    let what = if *scale == 4 {
                        "jump table"
                    } else {
                        "index table"
                    };
                    self.emit(
                        format!("{} {what} {len} bytes", hx(*at)),
                        json!({"fact": "table", "at": hx(*at), "len": len, "scale": scale}),
                    )?;
                }
            }
        }
        if hi < items.len() && (func == Some(a) || func.is_none()) {
            self.emit(
                format!("more {} (raise --max)", items.len() - hi),
                json!({"fact": "more", "count": items.len() - hi}),
            )?;
        }
        Ok(OK)
    }

    pub fn sig(&mut self, arg: &str, opts: &sig::Options, toml: bool) -> Result<i32> {
        let a = match self.one(arg)? {
            Ok(a) => a,
            Err(c) => return Ok(c),
        };
        let data = self.binary()?;
        let pe = self.pe(&data)?;
        let chunk_end: Option<u32> = self
            .conn
            .prepare("SELECT end FROM chunks WHERE start <= ?1 ORDER BY start DESC LIMIT 1")?
            .query_row([a], |r| r.get(0))
            .optional()?
            .filter(|&e: &u32| a < e);
        let name = {
            let l = self.label(a);
            if l.is_empty() {
                format!("sub_{a:x}")
            } else {
                l
            }
        };
        match sig::build(&pe, a, chunk_end.map(|e| e - a), &name, opts) {
            Ok(b) => {
                // In a TOML block the header is a comment, so it pastes as is.
                let lead = if toml { "# " } else { "" };
                self.emit(
                    format!("{lead}sig {} {name} section={} sig_len={} prologue_len={} unique", hx(a), b.section, b.sig_len, b.prologue_len),
                    json!({"fact": "sig", "rva": hx(a), "name": name, "section": b.section, "signature": b.signature,
                           "prologue": b.prologue, "sig_len": b.sig_len, "prologue_len": b.prologue_len}),
                )?;
                if toml && !self.json {
                    writeln!(self.w, "[[target]]")?;
                    writeln!(
                        self.w,
                        "name = {}",
                        serde_json::to_string(&name.trim_start_matches('~'))?
                    )?;
                    writeln!(self.w, "signature = \"{}\"", b.signature)?;
                    writeln!(self.w, "offset = 0")?;
                    if let Some(p) = &b.prologue {
                        writeln!(self.w, "prologue = \"{p}\"")?;
                    }
                    writeln!(self.w, "required = true")?;
                } else if !self.json {
                    writeln!(self.w, "signature {}", b.signature)?;
                    if let Some(p) = &b.prologue {
                        writeln!(self.w, "prologue {p}")?;
                    }
                }
                Ok(OK)
            }
            Err(e) => {
                self.emit(
                    format!("refused {e}"),
                    json!({"fact": "refused", "rva": hx(a), "reason": e}),
                )?;
                Ok(NONE)
            }
        }
    }

    pub fn bytes(&mut self, pattern: &str, limit: usize) -> Result<i32> {
        let (raw, mask) = sig::parse_pattern(pattern).map_err(anyhow::Error::msg)?;
        let data = self.binary()?;
        let pe = self.pe(&data)?;
        let hits = sig::search(&pe, &raw, &mask);
        self.emit(
            format!("matches {}", hits.len()),
            json!({"fact": "matches", "count": hits.len()}),
        )?;
        for &h in hits.iter().take(limit) {
            let f = if self.is_function(h)? {
                Some(h)
            } else {
                self.containing(h)?
            };
            let where_ = match f {
                Some(f) => format!(" in {} +0x{:x}", self.at(f), h - f),
                None => String::new(),
            };
            self.emit(
                format!("{}{where_}", hx(h)),
                json!({"fact": "match", "rva": hx(h), "func": f.map(|f| self.addr_json(f)), "offset": f.map(|f| h - f)}),
            )?;
        }
        self.more(hits.len(), limit)?;
        Ok(if hits.is_empty() { NONE } else { OK })
    }
}

fn slot_name(class: &str, slot: u32, off: u32) -> String {
    if off == 0 {
        format!("{class}::vf{slot}")
    } else {
        format!("{class}::vf{slot}@0x{off:x}")
    }
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push_str("...");
        t
    }
}

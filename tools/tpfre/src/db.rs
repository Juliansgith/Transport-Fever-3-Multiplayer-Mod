//! The `.tpfdb` file: one SQLite database per indexed binary.
//!
//! Integer columns hold RVAs (relative to the image base recorded in
//! `meta`). The schema carries a version; a query refuses a database of any
//! other version rather than misread it.

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::path::Path;

/// Bump when a table or the meaning of a column changes.
pub const SCHEMA_VERSION: &str = "1";

pub const SCHEMA: &str = r#"
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
CREATE TABLE sections(idx INTEGER PRIMARY KEY, name TEXT, rva INTEGER, vsize INTEGER,
    raw_size INTEGER, flags INTEGER, entropy REAL);
-- One row per function. size: bytes in all chunks; end: one past the
-- highest chunk byte (the .pdata extent). kind: how it was found.
-- name/name_src: the best name and where it came from ('' when unnamed).
CREATE TABLE functions(rva INTEGER PRIMARY KEY, size INTEGER, end INTEGER, kind TEXT,
    ninsn INTEGER, name TEXT, name_src TEXT);
CREATE TABLE chunks(start INTEGER PRIMARY KEY, end INTEGER, func INTEGER);
-- Direct call edges. kind: 0 call, 1 tail jump, 2 import call, 3 import jmp;
-- for imports callee is the IAT slot.
CREATE TABLE calls(site INTEGER PRIMARY KEY, caller INTEGER, callee INTEGER, kind INTEGER);
-- RIP-relative operands. kind: 0 addr (lea), 1 read, 2 write, 3 rw,
-- 4 icall, 5 ijmp, 6 mem.
CREATE TABLE xrefs(insn INTEGER PRIMARY KEY, func INTEGER, target INTEGER, kind INTEGER);
-- Absolute pointers in data (base relocations) to functions or strings.
CREATE TABLE dataptrs(at INTEGER PRIMARY KEY, target INTEGER);
-- enc: 1 ASCII, 2 UTF-16LE. class: '' or funcsig/pretty/file.
CREATE TABLE strings(rva INTEGER PRIMARY KEY, enc INTEGER, class TEXT, text TEXT);
-- Every naming source kept apart: funcsig, pretty, rtti, import, export.
CREATE TABLE names(addr INTEGER, name TEXT, source TEXT, confidence TEXT, detail TEXT);
CREATE TABLE func_src(func INTEGER PRIMARY KEY, file TEXT, kind TEXT);
CREATE TABLE imports(slot INTEGER PRIMARY KEY, dll TEXT, name TEXT, delay INTEGER);
CREATE TABLE types(td INTEGER PRIMARY KEY, mangled TEXT, name TEXT);
CREATE TABLE vtables(rva INTEGER PRIMARY KEY, class TEXT, col INTEGER, td INTEGER,
    offset INTEGER, nslots INTEGER);
CREATE TABLE vslots(vtable INTEGER, slot INTEGER, target INTEGER,
    PRIMARY KEY(vtable, slot)) WITHOUT ROWID;
"#;

/// Created after the bulk insert, which is faster than maintaining them.
pub const INDEXES: &str = r#"
CREATE INDEX calls_callee ON calls(callee);
CREATE INDEX calls_caller ON calls(caller);
CREATE INDEX xrefs_target ON xrefs(target);
CREATE INDEX xrefs_func ON xrefs(func);
CREATE INDEX dataptrs_target ON dataptrs(target);
CREATE INDEX names_addr ON names(addr);
CREATE INDEX names_name ON names(name);
CREATE INDEX func_src_file ON func_src(file);
CREATE INDEX vslots_target ON vslots(target);
CREATE INDEX vtables_class ON vtables(class);
"#;

pub fn open_ro(path: &Path) -> Result<Connection> {
    if !path.is_file() {
        bail!("no such database: {}", path.display());
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("opening {}", path.display()))?;
    let version: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |r| r.get(0),
        )
        .optional()
        .with_context(|| format!("{} is not a tpfre database", path.display()))?;
    match version.as_deref() {
        Some(SCHEMA_VERSION) => Ok(conn),
        Some(v) => bail!(
            "{} has schema version {v}, this tpfre reads version {SCHEMA_VERSION}: re-index the binary",
            path.display()
        ),
        None => bail!(
            "{} is not a tpfre database (no schema version)",
            path.display()
        ),
    }
}

pub fn meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0))
        .optional()?)
}

pub fn meta_req(conn: &Connection, key: &str) -> Result<String> {
    meta(conn, key)?.with_context(|| format!("the database has no '{key}' in meta"))
}

//! `tpfre diff`: which named functions moved, resized, appeared or
//! disappeared between two builds, as `tools/re/diff_builds.py` reports.
//!
//! Functions are matched by their `__FUNCSIG__` name plus source file (stable
//! across builds), positionally within a group sharing both, never by
//! address. A build-wide constant shift is summarised, not listed.

use anyhow::Result;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::Path;

use crate::db;

struct Named {
    rva: u32,
    size: u32,
}

type Groups = BTreeMap<(String, String), Vec<Named>>;

fn load(path: &Path) -> Result<(Groups, String, usize)> {
    let conn = db::open_ro(path)?;
    let label = format!(
        "{} ({})",
        db::meta(&conn, "binary_name")?.unwrap_or_default(),
        db::meta(&conn, "sha256")?
            .unwrap_or_default()
            .chars()
            .take(12)
            .collect::<String>()
    );
    let mut st = conn.prepare(
        "SELECT n.addr, n.name, f.end - f.rva, coalesce(s.file, '') FROM names n \
         JOIN functions f ON f.rva = n.addr LEFT JOIN func_src s ON s.func = n.addr \
         WHERE n.source IN ('funcsig', 'pretty') ORDER BY n.addr",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, u32>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, u32>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut groups: Groups = BTreeMap::new();
    let mut n = 0;
    for row in rows {
        let (rva, name, size, file) = row?;
        n += 1;
        groups
            .entry((name, file))
            .or_default()
            .push(Named { rva, size });
    }
    Ok((groups, label, n))
}

pub fn run(
    old: &Path,
    new: &Path,
    show_moved: bool,
    limit: usize,
    json_out: bool,
    w: &mut dyn Write,
) -> Result<i32> {
    let (og, olabel, on) = load(old)?;
    let (ng, nlabel, nn) = load(new)?;
    let mut items: Vec<Value> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut emit = |text: String, v: Value| {
        if json_out {
            items.push(v);
        } else {
            lines.push(text);
        }
    };
    let mut moved = Vec::new();
    let mut resized = Vec::new();
    let mut appeared = Vec::new();
    let mut disappeared = Vec::new();
    let mut unchanged = 0usize;
    let keys: std::collections::BTreeSet<&(String, String)> = og.keys().chain(ng.keys()).collect();
    let empty: Vec<Named> = Vec::new();
    for key in keys {
        let o = og.get(key).unwrap_or(&empty);
        let n = ng.get(key).unwrap_or(&empty);
        for i in 0..o.len().min(n.len()) {
            if o[i].size != n[i].size {
                resized.push((key, &o[i], &n[i]));
            } else if o[i].rva != n[i].rva {
                moved.push((key, &o[i], &n[i]));
            } else {
                unchanged += 1;
            }
        }
        for x in o.iter().skip(n.len()) {
            disappeared.push((key, x));
        }
        for x in n.iter().skip(o.len()) {
            appeared.push((key, x));
        }
    }
    let mut deltas: HashMap<i64, usize> = HashMap::new();
    for (_, o, n) in &moved {
        *deltas.entry(n.rva as i64 - o.rva as i64).or_default() += 1;
    }
    let shift = deltas
        .iter()
        .max_by_key(|(d, c)| (**c, -**d))
        .map(|(d, c)| (*d, *c));
    let sd = |d: i64| {
        if d < 0 {
            format!("-0x{:x}", -d)
        } else {
            format!("+0x{d:x}")
        }
    };
    emit(
        format!("diff {olabel} -> {nlabel}"),
        json!({"fact": "diff", "old": olabel, "new": nlabel}),
    );
    emit(
        format!("named old={on} new={nn}"),
        json!({"fact": "named", "old": on, "new": nn}),
    );
    emit(
        format!(
            "summary unchanged={unchanged} moved={} resized={} appeared={} disappeared={}",
            moved.len(),
            resized.len(),
            appeared.len(),
            disappeared.len()
        ),
        json!({"fact": "summary", "unchanged": unchanged, "moved": moved.len(), "resized": resized.len(),
               "appeared": appeared.len(), "disappeared": disappeared.len()}),
    );
    if let Some((d, c)) = shift {
        emit(
            format!(
                "shift {} functions={c} (a uniform shift is an ordinary relayout)",
                sd(d)
            ),
            json!({"fact": "shift", "delta": d, "functions": c}),
        );
    }
    let fmt_key = |k: &(String, String)| {
        if k.1.is_empty() {
            k.0.clone()
        } else {
            format!("{} {}", k.0, k.1)
        }
    };
    for (k, o, n) in resized.iter().take(limit) {
        emit(
            format!(
                "resized {} 0x{:x}->0x{:x} size {}->{}",
                fmt_key(k),
                o.rva,
                n.rva,
                o.size,
                n.size
            ),
            json!({"fact": "resized", "name": k.0, "file": k.1, "old": format!("0x{:x}", o.rva),
                   "new": format!("0x{:x}", n.rva), "old_size": o.size, "new_size": n.size}),
        );
    }
    for (k, n) in appeared.iter().take(limit) {
        emit(
            format!("appeared {} 0x{:x} size={}", fmt_key(k), n.rva, n.size),
            json!({"fact": "appeared", "name": k.0, "file": k.1, "rva": format!("0x{:x}", n.rva), "size": n.size}),
        );
    }
    for (k, o) in disappeared.iter().take(limit) {
        emit(
            format!("disappeared {} 0x{:x} size={}", fmt_key(k), o.rva, o.size),
            json!({"fact": "disappeared", "name": k.0, "file": k.1, "rva": format!("0x{:x}", o.rva), "size": o.size}),
        );
    }
    if show_moved {
        for (k, o, n) in moved.iter().take(limit) {
            emit(
                format!(
                    "moved {} 0x{:x}->0x{:x} {}",
                    fmt_key(k),
                    o.rva,
                    n.rva,
                    sd(n.rva as i64 - o.rva as i64)
                ),
                json!({"fact": "moved", "name": k.0, "file": k.1, "old": format!("0x{:x}", o.rva), "new": format!("0x{:x}", n.rva)}),
            );
        }
    }
    let over = [
        resized.len(),
        appeared.len(),
        disappeared.len(),
        if show_moved { moved.len() } else { 0 },
    ]
    .iter()
    .any(|&n| n > limit);
    if over {
        emit(
            format!("more (lists are cut at --limit {limit})"),
            json!({"fact": "more", "limit": limit}),
        );
    }
    if json_out {
        writeln!(w, "{}", serde_json::to_string_pretty(&Value::Array(items))?)?;
    } else {
        for l in lines {
            writeln!(w, "{l}")?;
        }
    }
    Ok(0)
}

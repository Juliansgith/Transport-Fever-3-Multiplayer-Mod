//! `__FUNCSIG__` / `__FILE__` naming: a port of the validated
//! `tools/re/name_functions.py` (`build_symbol_map`) and the string rules of
//! `tools/re/tpfbin.py` (`classify_string`, `strip_source_prefix`).
//!
//! MSVC's assert macros bake the enclosing function's full signature and
//! source path into read-only data, and the only code that loads such a
//! literal is the function that asserts with it. The rules below are the
//! Python ones, kept identical so the two agree name for name; see the
//! README for the comparison on TPF2 build 35924.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::LazyLock;

use regex::bytes::Regex;

use crate::strings::{Str, pure_printable};

static MSVC_CC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?-u)__(?:cdecl|thiscall|stdcall|fastcall|vectorcall|clrcall)\b")
        .expect("static regex")
});
static SRC_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i-u)\.(?:cpp|cc|cxx|c|h|hh|hpp|hxx|inl|ipp)$").expect("static regex")
});
static PRETTY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?-u)^[A-Za-z_][\w :<>,\*&~\[\]]*\s[\w:<>~]+\([^;{}\n]*\)(?:\s*const)?(?:\s*noexcept)?$",
    )
    .expect("static regex")
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    FuncSig,
    Pretty,
    File,
}

/// `classify_string` from tpfbin.py.
pub fn classify(raw: &str) -> Option<Class> {
    let b = raw.as_bytes();
    if raw.contains('(') && MSVC_CC.is_match(b) {
        return Some(Class::FuncSig);
    }
    if SRC_SUFFIX.is_match(b) && (raw.contains('\\') || raw.contains('/')) {
        return Some(Class::File);
    }
    if raw.contains('(')
        && raw.contains("::")
        && (6..=1024).contains(&b.len())
        && PRETTY.is_match(b)
    {
        return Some(Class::Pretty);
    }
    None
}

const CC: [&str; 6] = [
    "__cdecl",
    "__thiscall",
    "__stdcall",
    "__fastcall",
    "__vectorcall",
    "__clrcall",
];

/// `short_name` from name_functions.py: the qualified name out of a full
/// signature.
pub fn short_name(signature: &str) -> String {
    let mut s = signature.trim();
    for cc in CC {
        let pat = format!("{cc} ");
        if let Some(idx) = s.find(&pat) {
            s = &s[idx + pat.len()..];
            break;
        }
    }
    let mut depth = 0i32;
    let mut cut = s.len();
    for (i, ch) in s.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth = (depth - 1).max(0),
            '(' if depth == 0 => {
                cut = i;
                break;
            }
            _ => {}
        }
    }
    let mut head = s[..cut].trim();
    if head.contains(' ') {
        head = head.split_whitespace().last().unwrap_or(head);
    }
    if head.is_empty() {
        signature.to_owned()
    } else {
        head.to_owned()
    }
}

/// A display label for a signature: [`short_name`]'s rule, but template
/// arguments and operator names are kept whole. Python's rule cuts
/// `void __cdecl ecs::Engine::Add<struct ecs::X>(...)` to `ecs::X>`; this
/// gives `ecs::Engine::Add<struct ecs::X>`. The Python name stays in the
/// `names` table; this one is what queries show.
pub fn label_name(signature: &str) -> String {
    let mut s = signature.trim();
    for cc in CC {
        let pat = format!("{cc} ");
        if let Some(idx) = s.find(&pat) {
            s = &s[idx + pat.len()..];
            break;
        }
    }
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut cut = s.len();
    let mut spaces: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if s[i..].starts_with("operator")
            && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_'))
            && !b
                .get(i + 8)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
        {
            // Skip the operator's own symbol so `<`, `>` or `()` in it do not
            // count as template brackets or the argument list.
            i += 8;
            while i < b.len() && b[i] == b' ' {
                i += 1;
            }
            if s[i..].starts_with("()") || s[i..].starts_with("[]") {
                i += 2;
            } else {
                while i < b.len() && b"<>=!+-*/%^&|~,".contains(&b[i]) {
                    i += 1;
                }
            }
            continue;
        }
        match b[i] {
            b'<' | b'`' => depth += 1,
            b'>' | b'\'' => depth = (depth - 1).max(0),
            b'(' if depth == 0 => {
                cut = i;
                break;
            }
            b' ' if depth == 0 => spaces.push(i),
            _ => {}
        }
        i += 1;
    }
    let trimmed = s[..cut].trim_end();
    let head = match spaces.iter().rev().find(|&&p| p + 1 < trimmed.len()) {
        Some(&p) => &trimmed[p + 1..],
        None => trimmed.trim_start(),
    };
    if head.is_empty() {
        short_name(signature)
    } else {
        head.to_owned()
    }
}

/// `strip_source_prefix` from tpfbin.py: each path cut after its last
/// build-tree root, lowercased; and the most common prefix.
pub fn strip_source_prefix(paths: &[&str]) -> (HashMap<String, String>, String) {
    let mut out = HashMap::new();
    let mut counts: Vec<(String, usize)> = Vec::new();
    for &orig in paths {
        let low = orig.replace('/', "\\").to_lowercase();
        let mut cut: Option<usize> = None;
        for root in ["\\src\\", "\\urban_games\\", "\\include\\", "\\ext\\"] {
            if let Some(i) = low.rfind(root) {
                cut = Some(i + root.len());
                break;
            }
        }
        let cut = cut.unwrap_or_else(|| {
            let lb = low.as_bytes();
            let mut m = 0;
            while m < lb.len() && (lb[m] == b'.' || lb[m] == b'\\') {
                m += 1;
            }
            if lb.len() >= 2 && lb[1] == b':' {
                m = if lb.len() >= 3 { 3 } else { 2 };
            }
            m
        });
        out.insert(orig.to_owned(), low[cut..].to_owned());
        let prefix = &low[..cut];
        match counts.iter_mut().find(|(p, _)| p == prefix) {
            Some((_, n)) => *n += 1,
            None => counts.push((prefix.to_owned(), 1)),
        }
    }
    // Counter.most_common(1): highest count, first seen on a tie.
    let mut best: Option<&(String, usize)> = None;
    for c in &counts {
        if best.is_none_or(|b| c.1 > b.1) {
            best = Some(c);
        }
    }
    (out, best.map(|b| b.0.clone()).unwrap_or_default())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigName {
    /// The Python pipeline's name (`short_name`), kept identical.
    pub name: String,
    /// The display label (`label_name`).
    pub label: String,
    /// "funcsig" or "pretty".
    pub source: &'static str,
    /// "exact" or "ambiguous".
    pub confidence: &'static str,
    /// The chosen signature first, then the others sorted.
    pub signatures: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrcFile {
    pub file: String,
    /// "direct", "ambiguous" or "inferred".
    pub kind: &'static str,
}

#[derive(Debug, Default)]
pub struct Naming {
    pub names: BTreeMap<u32, SigName>,
    pub src: BTreeMap<u32, SrcFile>,
    /// Assert-string RVA -> class, for the string table.
    pub classes: HashMap<u32, Class>,
    pub stats: Vec<(&'static str, String)>,
}

/// `build_symbol_map` from name_functions.py.
///
/// `strings` are all extracted strings; `refs` are (function, target) for
/// every code reference; `starts` are the function starts (sorted) used for
/// translation-unit inference.
pub fn build(strings: &[Str], refs: &[(u32, u32)], starts: &[u32]) -> Naming {
    let mut funcsig: HashMap<u32, &str> = HashMap::new();
    let mut pretty: HashMap<u32, &str> = HashMap::new();
    let mut files: Vec<(u32, &str)> = Vec::new();
    let mut classes = HashMap::new();
    for s in strings {
        if s.enc != 1 || !s.boundary || !pure_printable(&s.text) {
            continue;
        }
        match classify(&s.text) {
            Some(Class::FuncSig) => {
                funcsig.insert(s.rva, &s.text);
                classes.insert(s.rva, Class::FuncSig);
            }
            Some(Class::Pretty) => {
                pretty.insert(s.rva, &s.text);
                classes.insert(s.rva, Class::Pretty);
            }
            Some(Class::File) => {
                files.push((s.rva, &s.text));
                classes.insert(s.rva, Class::File);
            }
            None => {}
        }
    }
    let file_paths: Vec<&str> = files.iter().map(|f| f.1).collect();
    let (file_short, prefix) = strip_source_prefix(&file_paths);
    let files: HashMap<u32, &str> = files.into_iter().collect();

    let mut sig_of: BTreeMap<u32, BTreeSet<&str>> = BTreeMap::new();
    let mut pretty_of: BTreeMap<u32, BTreeSet<&str>> = BTreeMap::new();
    let mut files_of: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    let mut funcs_of_sig: HashMap<&str, BTreeSet<u32>> = HashMap::new();
    let mut resolved = 0usize;
    for &(f, t) in refs {
        if let Some(&sig) = funcsig.get(&t) {
            sig_of.entry(f).or_default().insert(sig);
            funcs_of_sig.entry(sig).or_default().insert(f);
        } else if let Some(&p) = pretty.get(&t) {
            pretty_of.entry(f).or_default().insert(p);
        } else if let Some(&path) = files.get(&t) {
            let short = file_short
                .get(path)
                .cloned()
                .unwrap_or_else(|| path.to_owned());
            files_of.entry(f).or_default().insert(short);
        } else {
            continue;
        }
        resolved += 1;
    }

    let mut names: BTreeMap<u32, SigName> = BTreeMap::new();
    for (&f, sigs) in &sig_of {
        let (chosen, confidence) = if sigs.len() == 1 {
            (*sigs.iter().next().expect("one signature"), "exact")
        } else {
            let own: Vec<&str> = sigs
                .iter()
                .copied()
                .filter(|s| {
                    funcs_of_sig
                        .get(s)
                        .is_some_and(|fs| fs.len() == 1 && fs.contains(&f))
                })
                .collect();
            if own.len() == 1 {
                (own[0], "exact")
            } else {
                (*sigs.iter().next().expect("signatures"), "ambiguous")
            }
        };
        let mut signatures = vec![chosen.to_owned()];
        signatures.extend(sigs.iter().filter(|&&s| s != chosen).map(|s| s.to_string()));
        names.insert(
            f,
            SigName {
                name: short_name(chosen),
                label: label_name(chosen),
                source: "funcsig",
                confidence,
                signatures,
            },
        );
    }
    for (&f, ps) in &pretty_of {
        if names.contains_key(&f) {
            continue;
        }
        let first = *ps.iter().next().expect("pretty signatures");
        names.insert(
            f,
            SigName {
                name: short_name(first),
                label: label_name(first),
                source: "pretty",
                confidence: if ps.len() == 1 { "exact" } else { "ambiguous" },
                signatures: ps.iter().map(|s| s.to_string()).collect(),
            },
        );
    }

    let mut src: BTreeMap<u32, SrcFile> = BTreeMap::new();
    for (&f, fs) in &files_of {
        let first = fs.iter().next().expect("files").clone();
        src.insert(
            f,
            SrcFile {
                file: first,
                kind: if fs.len() == 1 { "direct" } else { "ambiguous" },
            },
        );
    }

    // Translation-unit range inference (src_ranges.py): the linker keeps a
    // .cpp contiguous, so the first and last directly attributed function of
    // a file bracket it.
    let anchors: Vec<(u32, &String)> = files_of
        .iter()
        .filter(|(_, fs)| fs.len() == 1)
        .map(|(&f, fs)| (f, fs.iter().next().expect("one file")))
        .collect();
    let mut file_only = 0usize;
    let mut i = 0;
    while i < anchors.len() {
        let (lo, fname) = anchors[i];
        let mut j = i;
        let mut hi = lo;
        while j + 1 < anchors.len() && anchors[j + 1].1 == fname {
            j += 1;
            hi = anchors[j].0;
        }
        if lo != hi {
            let a = starts.partition_point(|&s| s < lo);
            let b = starts.partition_point(|&s| s <= hi);
            for &rva in &starts[a..b] {
                if let std::collections::btree_map::Entry::Vacant(e) = src.entry(rva) {
                    e.insert(SrcFile {
                        file: fname.clone(),
                        kind: "inferred",
                    });
                    if !names.contains_key(&rva) {
                        file_only += 1;
                    }
                }
            }
        }
        i = j + 1;
    }

    let count = |src: &str, conf: &str| {
        names
            .values()
            .filter(|n| n.source == src && n.confidence == conf)
            .count()
    };
    let distinct: BTreeSet<&str> = src.values().map(|s| s.file.as_str()).collect();
    let stats = vec![
        ("assert_funcsig_strings", funcsig.len().to_string()),
        ("assert_pretty_strings", pretty.len().to_string()),
        ("assert_file_strings", files.len().to_string()),
        ("references_resolved", resolved.to_string()),
        (
            "functions_named_funcsig",
            count("funcsig", "exact").to_string(),
        ),
        (
            "functions_named_pretty",
            count("pretty", "exact").to_string(),
        ),
        (
            "functions_named_ambiguous",
            (count("funcsig", "ambiguous") + count("pretty", "ambiguous")).to_string(),
        ),
        (
            "functions_file_direct",
            src.values()
                .filter(|s| s.kind == "direct")
                .count()
                .to_string(),
        ),
        (
            "functions_file_ambiguous",
            src.values()
                .filter(|s| s.kind == "ambiguous")
                .count()
                .to_string(),
        ),
        (
            "functions_file_inferred",
            src.values()
                .filter(|s| s.kind == "inferred")
                .count()
                .to_string(),
        ),
        ("functions_file_only", file_only.to_string()),
        ("distinct_source_files", distinct.len().to_string()),
        ("source_prefix", prefix),
    ];
    Naming {
        names,
        src,
        classes,
        stats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_match_the_python_rules() {
        assert_eq!(
            short_name("struct Command __cdecl make_cmd::BuildProposal(int, float)"),
            "make_cmd::BuildProposal"
        );
        assert_eq!(short_name("void GameSim::Step(long, int)"), "GameSim::Step");
        assert_eq!(
            short_name("class std::vector<int,class std::allocator<int> > __cdecl a::b<c>(void)"),
            "a::b<c>"
        );
        assert_eq!(
            short_name("void __cdecl `anonymous namespace'::f(void)"),
            "namespace'::f"
        );
        assert_eq!(
            label_name("void __cdecl `anonymous namespace'::f(void)"),
            "`anonymous namespace'::f"
        );
        assert_eq!(short_name("   "), "   ");
    }

    #[test]
    fn labels_keep_templates_and_operators_whole() {
        let sig = "void __cdecl ecs::Engine::AddComponent<struct ecs::component::GameSpeed>(const class ecs::Entity &)";
        assert_eq!(short_name(sig), "ecs::component::GameSpeed>");
        assert_eq!(
            label_name(sig),
            "ecs::Engine::AddComponent<struct ecs::component::GameSpeed>"
        );
        assert_eq!(
            label_name("void __cdecl CommandList::Add::<lambda_71>::operator ()(int) const"),
            "CommandList::Add::<lambda_71>::operator ()"
        );
        assert_eq!(
            label_name("bool __cdecl A::operator<(const A &)"),
            "A::operator<"
        );
        assert_eq!(
            label_name("__cdecl A::operator bool(void)"),
            "A::operator bool"
        );
        assert_eq!(
            label_name("void __cdecl GameSim::Step(__int64,int)"),
            "GameSim::Step"
        );
        assert_eq!(label_name("void GameSim::Step(long, int)"), "GameSim::Step");
    }

    #[test]
    fn classification_matches_the_python_rules() {
        assert_eq!(classify("void __cdecl f(int)"), Some(Class::FuncSig));
        assert_eq!(classify("void __cdecl f"), None);
        assert_eq!(classify("c:\\src\\game\\a.cpp"), Some(Class::File));
        assert_eq!(classify("a.cpp"), None);
        assert_eq!(classify("void GameSim::Step(long)"), Some(Class::Pretty));
        assert_eq!(classify("hello world"), None);
    }

    #[test]
    fn source_prefix_is_cut_at_the_last_root() {
        let (m, p) = strip_source_prefix(&[
            "C:\\runner\\src\\game\\a.cpp",
            "C:\\runner\\src\\game\\b.cpp",
            "d:\\other\\x.h",
        ]);
        assert_eq!(m["C:\\runner\\src\\game\\a.cpp"], "game\\a.cpp");
        assert_eq!(m["d:\\other\\x.h"], "other\\x.h");
        assert_eq!(p, "c:\\runner\\src\\");
    }
}

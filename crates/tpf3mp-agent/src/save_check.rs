//! The check a received world's script data must pass before the game loads
//! it.
//!
//! TPF2 saves carried their scripts' state in a Lua sidecar whose `data()`
//! returns one table, and the game runs that file when it loads the save. A
//! world from another player could therefore run any code in the game. The
//! sidecar must be pure data, which [`check_lua_data`] admits without
//! running anything: TPF2MP's rule (`companion/tpf2mp/save_metadata.py`),
//! ported. Whether TPF3 saves carry such a file, and where the agent applies
//! this before handing a world to the hook, is for release day
//! (`docs/DAY_ONE.md`, section 7).

use std::path::Path;

use thiserror::Error;
use tpf3mp_bridge::mods::OWN_MOD;
use tpf3mp_modscan::save::SaveMod;

/// What the owner is told when the save they pick for a room does not run
/// TPF3-MP's mod: its game script orders the room's game, so a world
/// without it loads and then holds paused for good (seen live).
pub const SAVE_WITHOUT_OWN_MOD: &str = "This save doesn't have the TPF3-MP mod enabled: load it once, turn TPF3-MP on in its mods, save it, then pick it again";

/// What a player is told when the room's world, as it arrives, does not run
/// TPF3-MP's mod.
pub const WORLD_WITHOUT_OWN_MOD: &str = "The room's world doesn't have the TPF3-MP mod enabled, so it cannot play here: its owner should load the save once, turn TPF3-MP on in its mods, save it, and start a new room from it";

/// Whether a save lists TPF3-MP's own mod among its mods (`OWN_MOD`).
pub fn lists_own_mod(mods: &[SaveMod]) -> bool {
    mods.iter().any(|listed| listed.id == OWN_MOD)
}

/// Whether the save at `path` runs TPF3-MP's own mod: `Ok(false)` only when
/// its mods read and the mod is not among them; why not when they do not
/// read.
pub fn runs_own_mod(path: &Path) -> Result<bool, String> {
    tpf3mp_modscan::save::mods(path).map(|mods| lists_own_mod(&mods))
}

/// Largest script data admitted.
pub const MAX_LUA_DATA: usize = 32 << 20;
/// Deepest nesting of tables admitted.
const MAX_DEPTH: usize = 128;
const KEYWORDS: [&str; 22] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local",
    "nil", "not", "or", "repeat", "return", "then", "true", "until", "while", "goto",
];

/// Why script data is not admitted.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LuaDataError {
    #[error("the script data is over {MAX_LUA_DATA} bytes")]
    TooLarge,
    #[error("the script data is not UTF-8 text")]
    NotText,
    #[error("line {line}: {reason}")]
    Syntax { line: usize, reason: &'static str },
}

/// Admits `bytes` if they are exactly `function data() return <table> end`,
/// the table holding nothing but literal keys and values: strings, numbers,
/// booleans, `nil` and tables, nested at most 128 deep. Nothing is ever
/// evaluated, and anything else, such as a call, an operator or code after
/// the function, is refused.
pub fn check_lua_data(bytes: &[u8]) -> Result<(), LuaDataError> {
    if bytes.len() > MAX_LUA_DATA {
        return Err(LuaDataError::TooLarge);
    }
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| LuaDataError::NotText)?;
    Parser::new(text)?.parse()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token<'a> {
    String,
    Number,
    Name(&'a str),
    Punct(u8),
    End,
}

struct Parser<'a> {
    text: &'a str,
    position: usize,
    /// Where the current token starts, for error lines.
    start: usize,
    token: Token<'a>,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Result<Self, LuaDataError> {
        let mut parser = Self {
            text,
            position: 0,
            start: 0,
            token: Token::End,
        };
        parser.advance()?;
        Ok(parser)
    }

    fn bytes(&self) -> &'a [u8] {
        self.text.as_bytes()
    }

    fn error<T>(&self, reason: &'static str) -> Result<T, LuaDataError> {
        let line = self.bytes()[..self.start]
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count()
            + 1;
        Err(LuaDataError::Syntax { line, reason })
    }

    /// Moves to the next token, past whitespace and comments.
    fn advance(&mut self) -> Result<(), LuaDataError> {
        let bytes = self.bytes();
        loop {
            self.start = self.position;
            let Some(&byte) = bytes.get(self.position) else {
                self.token = Token::End;
                return Ok(());
            };
            match byte {
                b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => {
                    self.position += 1;
                    continue;
                }
                b'-' if bytes.get(self.position + 1) == Some(&b'-') => {
                    while bytes
                        .get(self.position)
                        .is_some_and(|&byte| byte != b'\r' && byte != b'\n')
                    {
                        self.position += 1;
                    }
                    continue;
                }
                b'"' | b'\'' => {
                    self.string(byte)?;
                    self.token = Token::String;
                    return Ok(());
                }
                b'{' | b'}' | b'[' | b']' | b'(' | b')' | b',' | b';' | b'=' => {
                    self.position += 1;
                    self.token = Token::Punct(byte);
                    return Ok(());
                }
                _ => {}
            }
            if self.number() {
                self.token = Token::Number;
                return Ok(());
            }
            if byte.is_ascii_alphabetic() || byte == b'_' {
                let length = bytes[self.position..]
                    .iter()
                    .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
                    .count();
                self.token = Token::Name(&self.text[self.position..self.position + length]);
                self.position += length;
                return Ok(());
            }
            return self.error("invalid or incomplete native Lua data");
        }
    }

    /// A quoted string on one line; a backslash escapes the next character,
    /// or a line break.
    fn string(&mut self, quote: u8) -> Result<(), LuaDataError> {
        let bytes = self.bytes();
        let mut at = self.position + 1;
        loop {
            match bytes.get(at) {
                Some(&byte) if byte == quote => {
                    self.position = at + 1;
                    return Ok(());
                }
                Some(b'\\') => match bytes.get(at + 1) {
                    Some(b'\r') if bytes.get(at + 2) == Some(&b'\n') => at += 3,
                    Some(_) => at += 2,
                    None => return self.error("invalid or incomplete native Lua data"),
                },
                Some(b'\r' | b'\n') | None => {
                    return self.error("invalid or incomplete native Lua data");
                }
                Some(_) => at += 1,
            }
        }
    }

    /// A number: `inf`, `nan`, hexadecimal or decimal, perhaps negative.
    fn number(&mut self) -> bool {
        let bytes = self.bytes();
        let digits = |from: usize, hex: bool| {
            bytes[from.min(bytes.len())..]
                .iter()
                .take_while(|byte| {
                    if hex {
                        byte.is_ascii_hexdigit()
                    } else {
                        byte.is_ascii_digit()
                    }
                })
                .count()
        };
        let mut at = self.position;
        if bytes.get(at) == Some(&b'-') {
            at += 1;
        }
        for word in [b"inf".as_slice(), b"nan"] {
            let rest = &bytes[at.min(bytes.len())..];
            let bounded = !rest
                .get(word.len())
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
            if rest.starts_with(word) && bounded {
                self.position = at + word.len();
                return true;
            }
        }
        if bytes.get(at) == Some(&b'0') && matches!(bytes.get(at + 1), Some(b'x' | b'X')) {
            let hex = digits(at + 2, true);
            if hex > 0 {
                self.position = at + 2 + hex;
                return true;
            }
        }
        let whole = digits(at, false);
        let mut end = if whole > 0 {
            let mut end = at + whole;
            if bytes.get(end) == Some(&b'.') {
                end += 1 + digits(end + 1, false);
            }
            end
        } else if bytes.get(at) == Some(&b'.') && digits(at + 1, false) > 0 {
            at + 1 + digits(at + 1, false)
        } else {
            return false;
        };
        if matches!(bytes.get(end), Some(b'e' | b'E')) {
            let mut exponent = end + 1;
            if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
                exponent += 1;
            }
            let length = digits(exponent, false);
            if length > 0 {
                end = exponent + length;
            }
        }
        self.position = end;
        true
    }

    fn expect_word(&mut self, word: &str) -> Result<(), LuaDataError> {
        if self.token != Token::Name(word) {
            return self.error("expected the saved data function");
        }
        self.advance()
    }

    fn expect_punct(&mut self, punct: u8, reason: &'static str) -> Result<(), LuaDataError> {
        if self.token != Token::Punct(punct) {
            return self.error(reason);
        }
        self.advance()
    }

    fn parse(mut self) -> Result<(), LuaDataError> {
        self.expect_word("function")?;
        self.expect_word("data")?;
        self.expect_punct(b'(', "expected the saved data function")?;
        self.expect_punct(b')', "expected the saved data function")?;
        self.expect_word("return")?;
        self.table(0)?;
        self.expect_word("end")?;
        if self.token == Token::Punct(b';') {
            self.advance()?;
        }
        if self.token != Token::End {
            return self.error("unexpected content after the saved data function");
        }
        Ok(())
    }

    fn table(&mut self, depth: usize) -> Result<(), LuaDataError> {
        if depth > MAX_DEPTH {
            return self.error("native Lua data exceeds the nesting limit");
        }
        self.expect_punct(b'{', "expected a table")?;
        while self.token != Token::Punct(b'}') {
            match self.token {
                Token::Punct(b'[') => {
                    self.advance()?;
                    if !matches!(
                        self.token,
                        Token::String | Token::Number | Token::Name("true" | "false")
                    ) {
                        return self.error("expected a literal table key");
                    }
                    self.advance()?;
                    self.expect_punct(b']', "expected ]")?;
                    self.expect_punct(b'=', "expected =")?;
                    self.literal(depth)?;
                }
                Token::Name(name) if !KEYWORDS.contains(&name) => {
                    self.advance()?;
                    self.expect_punct(b'=', "expected =")?;
                    self.literal(depth)?;
                }
                _ => self.literal(depth)?,
            }
            match self.token {
                Token::Punct(b',' | b';') => self.advance()?,
                Token::Punct(b'}') => {}
                _ => return self.error("expected a table separator or closing brace"),
            }
        }
        self.advance()
    }

    fn literal(&mut self, depth: usize) -> Result<(), LuaDataError> {
        match self.token {
            Token::Punct(b'{') => self.table(depth + 1),
            Token::String | Token::Number | Token::Name("true" | "false" | "nil") => self.advance(),
            _ => self.error("expected a serialized value (expressions are not accepted)"),
        }
    }
}

/// Saves for tests: the start of a save as the game writes one, listing
/// mods, as `tpf3mp_modscan::save` reads it.
#[cfg(test)]
pub(crate) mod saves {
    use std::path::Path;

    fn string(out: &mut Vec<u8>, text: &str) {
        out.extend_from_slice(&u32::try_from(text.len()).unwrap().to_le_bytes());
        out.extend_from_slice(text.as_bytes());
    }

    /// Writes a save at `path` listing the mods `ids`.
    pub(crate) fn write(path: &Path, ids: &[&str]) {
        let mut out = b"tf**\x5c\x02\x00\x00".to_vec();
        string(&mut out, "company");
        out.extend_from_slice(&[4, 0, 0, 0, 1, 1, 0, 0, 0, 3, 0, 0, 0]);
        out.extend_from_slice(&u32::try_from(ids.len()).unwrap().to_le_bytes());
        for id in ids {
            string(&mut out, id);
            string(&mut out, "StagingArea");
            string(&mut out, &format!("StagingArea,{id}"));
            string(&mut out, id);
            string(&mut out, "");
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        out.extend_from_slice(&[1, 0x80, 2, 0, 0, 0x68, 1, 0, 0]);
        std::fs::write(path, zstd::encode_all(&out[..], 3).unwrap()).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use proptest::{collection::vec, prelude::*, sample::Index};

    use super::*;

    #[test]
    fn a_save_runs_tpf3mps_mod_only_when_it_lists_it() {
        let dir = tempfile::tempdir().unwrap();
        // The live case: a save with the game's DLC and not TPF3-MP.
        let without = dir.path().join("without.sav");
        saves::write(
            &without,
            &["urbangames_deluxe_upgrade_pack", "urbangames_preorder_pack"],
        );
        assert_eq!(runs_own_mod(&without), Ok(false));
        let with = dir.path().join("with.sav");
        saves::write(&with, &["urbangames_deluxe_upgrade_pack", "tpf3mp_1"]);
        assert_eq!(runs_own_mod(&with), Ok(true));
        // Unreadable is not "without": the caller decides.
        let junk = dir.path().join("junk.sav");
        std::fs::write(&junk, b"not a save").unwrap();
        assert!(runs_own_mod(&junk).is_err());
        assert!(runs_own_mod(&dir.path().join("missing.sav")).is_err());
    }

    fn check(text: &str) -> Result<(), LuaDataError> {
        check_lua_data(text.as_bytes())
    }

    fn reason(text: &str) -> &'static str {
        match check(text) {
            Err(LuaDataError::Syntax { reason, .. }) => reason,
            other => panic!("{text:?} gave {other:?}"),
        }
    }

    // The cases below are TPF2MP's own tests of the rule (tests/test_save_metadata.py).

    #[test]
    fn data_literals_and_escaped_names_are_admitted() {
        check(
            "function data()\nreturn { [\"tpf2_mp.lua\"] = { enabled=true, count=-2, time=1.2e-5,\n \
             [1] = { \"quoted\\\"text\", false, nil, }, empty={}, }, }\nend\n-- retained comment\n",
        )
        .unwrap();
    }

    #[test]
    fn a_crashed_saves_trailing_fragment_is_refused() {
        assert_eq!(
            reason("function data() return { state={} } end\n = 0,\n recipeDigest=\"41b68bfc\","),
            "unexpected content after the saved data function"
        );
    }

    #[test]
    fn infinities_and_escaped_line_breaks_are_admitted() {
        check(
            "function data() return { min=inf, max=-inf, value=nan, \
             error=\"first\\\nsecond\\\r\nthird\" } end",
        )
        .unwrap();
    }

    #[test]
    fn empty_truncated_or_invalid_data_is_refused() {
        for text in [
            "",
            "function data() return {",
            "function data() return {}",
            "function data() return { value= } end",
            "function data() return { end=1 } end",
        ] {
            assert!(check(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn code_is_refused_and_never_run() {
        for text in [
            "function data() return { x=os.execute(\"bad\") } end",
            "function data() return {} end os.execute(\"bad\")",
            "function data() return { x=1+1 } end",
            "function data() return { x=\"a\"..\"b\" } end",
            "function data() return { f=function() end } end",
            "function data() return setmetatable({}, {}) end",
        ] {
            assert!(check(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn nesting_is_bounded() {
        let deep = |levels: usize| {
            format!(
                "function data() return {}{} end",
                "{".repeat(levels),
                "}".repeat(levels)
            )
        };
        check(&deep(100)).unwrap();
        assert_eq!(
            reason(&deep(130)),
            "native Lua data exceeds the nesting limit"
        );
    }

    #[test]
    fn non_text_and_oversized_data_are_refused() {
        assert_eq!(check_lua_data(b"\xff"), Err(LuaDataError::NotText));
        let mut large = b"function data() return { x=\"".to_vec();
        large.resize(MAX_LUA_DATA + 1, b'a');
        assert_eq!(check_lua_data(&large), Err(LuaDataError::TooLarge));
        check_lua_data(b"\xef\xbb\xbffunction data() return {} end").unwrap();
    }

    #[test]
    fn errors_name_their_line() {
        assert_eq!(
            check("function data() return {\n  a = 1,\n  b = os.time(),\n} end"),
            Err(LuaDataError::Syntax {
                line: 3,
                reason: "expected a serialized value (expressions are not accepted)"
            })
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(4096))]

        #[test]
        fn any_bytes_are_checked_without_panicking(bytes in vec(any::<u8>(), 0..400)) {
            let _ = check_lua_data(&bytes);
        }

        #[test]
        fn corrupted_data_is_checked_without_panicking(
            edits in vec((any::<Index>(), any::<u8>()), 1..8),
        ) {
            let mut bytes = b"function data() return { a = { 1, -2.5e3, \"x\\\"y\", true, nil }, \
                              [\"k\"] = 0x1F, b = { c = { d = inf } } } end"
                .to_vec();
            for (at, value) in edits {
                let index = at.index(bytes.len());
                bytes[index] = value;
            }
            let _ = check_lua_data(&bytes);
        }
    }
}

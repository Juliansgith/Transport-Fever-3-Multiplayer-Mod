//! Diagnostics: a client's own log lines, which it sends to the server it
//! plays on, so the server's operator can see what went wrong for a player
//! from the support ID alone, without asking for files ("Diagnostics" in
//! PROTOCOL.md). Both sides pass every line through [`redact`] first.
//!
//! TPF2MP's relay kept its players' diagnostics the same way; its
//! redaction missed paths outside `C:\Users` (Steam's `userdata\<account>`
//! among them) and paths escaped inside JSON. [`redact`] shortens every
//! absolute path to its last part, wherever it stands in a line.

use serde::{Deserialize, Serialize};

use crate::{BoundedVec, Text};

/// Events one request carries at most: the largest batch fits a control
/// frame.
pub const MAX_DIAGNOSTIC_EVENTS: usize = 32;

/// One line, redacted, of at most 1024 bytes.
pub type DiagnosticText = Text<1024>;
/// Where a line was logged, such as `tpf3mp_agent::bridge`.
pub type DiagnosticTarget = Text<48>;
/// The lines one request carries.
pub type DiagnosticBatch = BoundedVec<DiagnosticEvent, MAX_DIAGNOSTIC_EVENTS>;

/// A line of a client's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticEvent {
    /// When it was logged: milliseconds since the Unix epoch, by the
    /// client's clock.
    pub at_ms: u64,
    pub level: DiagnosticLevel,
    pub target: DiagnosticTarget,
    pub text: DiagnosticText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticLevel {
    Info,
    Warn,
    Error,
}

impl DiagnosticLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// Takes out of a log line what could identify a player or give access to
/// something:
///
/// - absolute paths, Windows' (`C:\…`, `\\host\…`, escaped `C:\\…` too) and
///   Unix's (`/…/…`, `~/…`), become `<path>/` and their last part, so a
///   user name or a Steam account in them is gone; a last part of digits
///   alone, such as an account's folder, goes too;
/// - IPv4 and IPv6 addresses become `<ip>`;
/// - invites become `<invite>`;
/// - the value after a key naming a secret (`token=`, `password:`, a
///   bearer token, …) becomes `<redacted>`;
/// - e-mail addresses become `<email>`, and 64-bit Steam IDs `<steam id>`.
///
/// Control characters count as spaces. Words are split on spaces; a path
/// with spaces in it goes on through the words that continue it.
pub fn redact(line: &str) -> String {
    let line: String = line
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let words: Vec<&str> = line.split(' ').collect();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    let mut index = 0;
    let mut hide_next = false;
    while index < words.len() {
        let word = words[index];
        let lower = word.to_ascii_lowercase();
        if hide_next && !word.is_empty() {
            // `Authorization: Bearer <token>` keeps the scheme's name.
            if lower == "bearer" || lower == "basic" {
                out.push(word.to_owned());
            } else {
                out.push("<redacted>".to_owned());
                hide_next = false;
            }
            index += 1;
            continue;
        }
        if let Some(start) = path_start(word) {
            let mut path = word[start..].to_owned();
            let separator = if is_windows_path(&path) { '\\' } else { '/' };
            while let Some(next) = words.get(index + 1) {
                let after = words.get(index + 2).copied().unwrap_or("");
                let goes_on = continues_path(next, separator)
                    || (!next.is_empty() && continues_path(after, separator));
                if !goes_on {
                    break;
                }
                index += 1;
                path.push(' ');
                path.push_str(next);
            }
            let (path, trailing) = split_trailing(&path);
            let last = path
                .rsplit(['\\', '/'])
                .find(|part| !part.is_empty())
                .filter(|part| !part.bytes().all(|b| b.is_ascii_digit()))
                .map_or(String::new(), |part| format!("/{part}"));
            out.push(format!(
                "{}<path>{last}{trailing}",
                redact_word(&word[..start])
            ));
            index += 1;
            continue;
        }
        if lower == "bearer" || lower == "basic" {
            hide_next = true;
            out.push(word.to_owned());
        } else if let Some(value) = secret_value(word) {
            if value == word.len() {
                // `password: hunter2` hides the next word.
                hide_next = true;
                out.push(word.to_owned());
            } else {
                out.push(format!("{}<redacted>", &word[..value]));
            }
        } else {
            out.push(redact_word(word));
        }
        index += 1;
    }
    out.join(" ")
}

/// Where an absolute path starts in `word`, if one does.
fn path_start(word: &str) -> Option<usize> {
    let bytes = word.as_bytes();
    for (index, window) in bytes.windows(3).enumerate() {
        let boundary = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        // C:\ or C:/, and C:\\ escaped.
        if boundary
            && window[0].is_ascii_alphabetic()
            && window[1] == b':'
            && (window[2] == b'\\' || window[2] == b'/')
        {
            return Some(index);
        }
    }
    for (index, pair) in bytes.windows(2).enumerate() {
        let boundary = index == 0 || matches!(bytes[index - 1], b'"' | b'\'' | b'=' | b'(' | b'[');
        if !boundary {
            continue;
        }
        let rest = &word[index..];
        // \\host\share, ~/…, and a Unix path of two parts or more.
        if (pair == b"\\\\" && rest.len() > 2)
            || pair == b"~/"
            || (pair[0] == b'/' && pair[1] != b'/' && rest[1..].contains('/'))
        {
            return Some(index);
        }
    }
    None
}

fn is_windows_path(path: &str) -> bool {
    path.starts_with("\\\\") || path.as_bytes().get(1) == Some(&b':')
}

/// Whether a word goes on with a path, as `(x86)\Steam` does with
/// `C:\Program Files`: it holds the path's separator, and starts no path of
/// its own.
fn continues_path(word: &str, separator: char) -> bool {
    word.contains(separator) && path_start(word) != Some(0)
}

/// Splits the punctuation a sentence puts after a path from the path.
fn split_trailing(path: &str) -> (&str, &str) {
    let end = path
        .trim_end_matches(['"', '\'', ')', ']', ',', '.', ';', ':', '!', '?'])
        .len();
    path.split_at(end)
}

/// Where the value starts in `key=value`, `key:` or `"key":"value"`, when
/// the key names a secret.
fn secret_value(word: &str) -> Option<usize> {
    const SECRETS: [&str; 8] = [
        "token",
        "password",
        "passwd",
        "secret",
        "authorization",
        "cookie",
        "credential",
        "api_key",
    ];
    let split = word.find(['=', ':'])?;
    let key = word[..split]
        .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .to_ascii_lowercase();
    let secret = key == "key"
        || key == "apikey"
        || key.ends_with("_key")
        || SECRETS.iter().any(|secret| key.contains(secret));
    if key.is_empty() || !secret {
        return None;
    }
    // Past the separator and any quote that opens the value.
    let value = &word[split + 1..];
    Some(split + 1 + value.len() - value.trim_start_matches(['"', '\'']).len())
}

/// Redacts addresses, invites, e-mail addresses and Steam IDs in one word.
fn redact_word(word: &str) -> String {
    if let Some(start) = word.find("TPF3MP1.") {
        let end = word[start..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '~')))
            .map_or(word.len(), |end| start + end);
        return format!(
            "{}<invite>{}",
            redact_word(&word[..start]),
            redact_word(&word[end..])
        );
    }
    if let Some(email) = email_span(word) {
        return format!(
            "{}<email>{}",
            redact_word(&word[..email.0]),
            redact_word(&word[email.1..])
        );
    }
    let bytes = word.as_bytes();
    let mut out = String::with_capacity(word.len());
    let mut index = 0;
    while index < word.len() {
        let starts_word = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        if starts_word && (bytes[index].is_ascii_hexdigit() || bytes[index] == b':') {
            // IPv4, or a Steam ID: digits and dots.
            let v4 = run(word, index, |c| c.is_ascii_digit() || c == '.');
            let v4 = v4.trim_end_matches('.');
            if is_ipv4(v4) {
                out.push_str("<ip>");
                index += v4.len();
                continue;
            }
            if is_steam_id(v4) {
                out.push_str("<steam id>");
                index += v4.len();
                continue;
            }
            // IPv6: hex digits and colons, and an IPv4 part at the end.
            let v6 = run(word, index, |c| {
                c.is_ascii_hexdigit() || c == ':' || c == '.'
            });
            let v6_len = v6.len();
            let v6 = v6.trim_end_matches(['.', ':']);
            if is_ipv6(v6) {
                out.push_str("<ip>");
                index += v6.len();
                continue;
            }
            // Nothing inside the run starts a word: keep it whole.
            out.push_str(&word[index..index + v6_len.max(1)]);
            index += v6_len.max(1);
            continue;
        }
        let next = word[index..]
            .char_indices()
            .nth(1)
            .map_or(word.len(), |(offset, _)| index + offset);
        out.push_str(&word[index..next]);
        index = next;
    }
    out
}

/// The longest run of `word` from `start` of characters `keep` takes.
fn run(word: &str, start: usize, keep: impl Fn(char) -> bool) -> &str {
    let end = word[start..]
        .find(|c: char| !keep(c))
        .map_or(word.len(), |end| start + end);
    &word[start..end]
}

/// Where an e-mail address is in `word`.
fn email_span(word: &str) -> Option<(usize, usize)> {
    let at = word.find('@')?;
    let local = |c: char| c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '+');
    let start = word[..at]
        .rfind(|c: char| !local(c))
        .map_or(0, |start| start + 1);
    let domain = run(word, at + 1, |c| {
        c.is_alphanumeric() || matches!(c, '.' | '-')
    });
    let domain = domain.trim_end_matches(['.', '-']);
    (start < at && domain.contains('.') && !domain.starts_with('.'))
        .then_some((start, at + 1 + domain.len()))
}

fn is_ipv4(run: &str) -> bool {
    let parts: Vec<&str> = run.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 3
                && part.bytes().all(|b| b.is_ascii_digit())
                && part.parse::<u16>().is_ok_and(|n| n <= 255)
        })
}

fn is_ipv6(run: &str) -> bool {
    let groups: Vec<&str> = run.split(':').collect();
    groups.len() >= 3
        && (run.contains("::") || groups.len() >= 5)
        && groups.iter().all(|group| {
            group.len() <= 4 && group.bytes().all(|b| b.is_ascii_hexdigit()) || is_ipv4(group)
        })
        && groups.iter().any(|group| !group.is_empty())
}

fn is_steam_id(run: &str) -> bool {
    run.len() == 17 && run.starts_with("7656119") && run.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CONTROL_MAX_FRAME, ClientMessage, Request, encode_frame};

    fn hides(line: &str, secret: &str) {
        let redacted = redact(line);
        assert!(!redacted.contains(secret), "{secret} left in: {redacted}");
    }

    #[test]
    fn paths_keep_only_their_last_part() {
        assert_eq!(
            redact(r"cannot open C:\Users\Alice\AppData\Local\TPF3-MP\identity.key: denied"),
            "cannot open <path>/identity.key: denied"
        );
        assert_eq!(
            redact("reading /home/alice/.local/share/TPF3-MP/launcher.json"),
            "reading <path>/launcher.json"
        );
        assert_eq!(
            redact("saved ~/Documents/world.sav."),
            "saved <path>/world.sav."
        );
        assert_eq!(redact(r"on \\nas\games\tpf3"), "on <path>/tpf3");
        assert_eq!(
            redact(r"copied C:\a\x.sav to C:\b\y.sav"),
            "copied <path>/x.sav to <path>/y.sav"
        );
    }

    #[test]
    fn what_tpf2mps_redaction_missed_is_hidden() {
        // A Steam path outside C:\Users, with spaces, names the account.
        let line = r"no C:\Program Files (x86)\Steam\userdata\63389028\3493540\local\save\My World.sav here";
        hides(line, "63389028");
        assert_eq!(redact(line), "no <path>/My World.sav here");
        // A path escaped inside JSON.
        hides(
            r#"{"path":"C:\\Users\\Alice\\Documents\\x.sav","ok":false}"#,
            "Alice",
        );
        // An account's folder at the end of a path.
        let line = "dir=/home/alice/.steam/steam/userdata/12345/";
        hides(line, "alice");
        hides(line, "12345");
        // A line with a newline in it.
        hides("failed:\nC:\\Users\\Alice\\x", "Alice");
    }

    #[test]
    fn addresses_invites_and_secrets_are_hidden() {
        assert_eq!(redact("from 203.0.113.9:29470"), "from <ip>:29470");
        assert_eq!(redact("peer [2001:db8::1]:29470"), "peer [<ip>]:29470");
        assert_eq!(redact("at fe80::1ff:fe23:4567:890a"), "at <ip>");
        assert_eq!(redact("addr=10.0.0.2"), "addr=<ip>");
        hides(
            "join tpf3mp.example.org:29470 TPF3MP1.zIi0KG_HnPRrqMW78mpwL5c",
            "zIi0KG",
        );
        assert_eq!(redact("token=abc123 ok"), "token=<redacted> ok");
        assert_eq!(redact("password: hunter2"), "password: <redacted>");
        assert_eq!(redact(r#""api_key":"sk-123","#), r#""api_key":"<redacted>"#);
        assert_eq!(
            redact("Authorization: Bearer eyJ.x.y"),
            "Authorization: Bearer <redacted>"
        );
        assert_eq!(
            redact("mail ann@example.org, thanks"),
            "mail <email>, thanks"
        );
        assert_eq!(redact("steam 76561198000000000"), "steam <steam id>");
    }

    #[test]
    fn ordinary_lines_stay_as_they_are() {
        for line in [
            "the room saves its world event=34 step=127",
            "connected in 42 ms at 10:30:15, session s-8c21f0a9d3e4b5c6d7e8f90a1b2c3d4e",
            "downloaded 1.5 MB of 12.0 MB (12%) and/or waited",
            "player p-3f2a91c0d4e5b6a7 left: the game session ended",
            "keyboard key pressed, monkey=5",
            "version 0.1.0 on windows x86_64",
            "",
        ] {
            assert_eq!(redact(line), line);
        }
    }

    #[test]
    fn the_largest_batch_fits_a_control_frame() {
        let event = DiagnosticEvent {
            at_ms: u64::MAX,
            level: DiagnosticLevel::Error,
            target: Text::new("x".repeat(48)).unwrap(),
            text: Text::new("y".repeat(1024)).unwrap(),
        };
        let batch = DiagnosticBatch::new(vec![event; MAX_DIAGNOSTIC_EVENTS]).unwrap();
        let message = ClientMessage::Request {
            id: u32::MAX,
            request: Request::Diagnostics(batch),
        };
        assert!(encode_frame(&message, CONTROL_MAX_FRAME).is_ok());
    }
}

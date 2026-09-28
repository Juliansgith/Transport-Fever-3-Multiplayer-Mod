//! The latest release's notes, for the panel under "How to play", as the
//! page's release panel shows them: fetched once from GitHub's API when
//! the launcher opens, read as plain text. Nothing read here is trusted
//! for installing; updates go through their signed manifest (`update.rs`).

use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use serde::Deserialize;

use crate::update::REPOSITORY;

/// Most of a release's notes kept, in bytes.
const MAX_NOTES: usize = 16 * 1024;

/// A piece of the notes: a heading, a list item or a paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading(String),
    Item(String),
    Paragraph(String),
}

/// What the panel shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notes {
    Checking,
    Unavailable,
    Release { version: String, blocks: Vec<Block> },
}

/// The notes, fetched on a thread of their own.
#[derive(Debug, Clone)]
pub struct ReleaseNotes {
    notes: Arc<Mutex<Notes>>,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
}

impl ReleaseNotes {
    /// Starts fetching the latest published release's notes.
    pub fn fetch() -> Self {
        let notes = Arc::new(Mutex::new(Notes::Checking));
        let shared = Arc::clone(&notes);
        let _ = std::thread::Builder::new()
            .name("release-notes".into())
            .spawn(move || {
                let found = latest().unwrap_or(Notes::Unavailable);
                *shared.lock().unwrap_or_else(PoisonError::into_inner) = found;
            });
        Self { notes }
    }

    /// Notes that are already known: for tests and screenshots.
    pub fn showing(notes: Notes) -> Self {
        Self {
            notes: Arc::new(Mutex::new(notes)),
        }
    }

    pub fn get(&self) -> Notes {
        self.notes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn latest() -> Option<Notes> {
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(15)))
            .build(),
    );
    let mut response = agent
        .get(&format!(
            "https://api.github.com/repos/{REPOSITORY}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .header(
            "User-Agent",
            concat!("tpf3mp-launcher/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .ok()?;
    let body = response
        .body_mut()
        .with_config()
        .limit(1 << 20)
        .read_to_vec()
        .ok()?;
    parse(&body)
}

/// The notes of a release as GitHub's API gives it.
fn parse(json: &[u8]) -> Option<Notes> {
    let release: ApiRelease = serde_json::from_slice(json).ok()?;
    let version = release.tag_name.strip_prefix('v')?.to_owned();
    let mut notes = release.body.unwrap_or_default();
    if notes.len() > MAX_NOTES {
        let mut end = MAX_NOTES;
        while !notes.is_char_boundary(end) {
            end -= 1;
        }
        notes.truncate(end);
    }
    Some(Notes::Release {
        version,
        blocks: blocks(&notes),
    })
}

/// Markdown as blocks of plain text: headings, list items and paragraphs.
/// Links keep their text; emphasis and code marks go.
pub fn blocks(markdown: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let flush = |paragraph: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            let text = inline(&paragraph.join(" "));
            if !text.is_empty() {
                out.push(Block::Paragraph(text));
            }
            paragraph.clear();
        }
    };
    for raw in markdown.lines() {
        let line = raw.trim();
        let rule =
            line.len() >= 3 && (line.chars().all(|c| c == '-') || line.chars().all(|c| c == '*'));
        if line.is_empty() || rule {
            flush(&mut paragraph, &mut out);
        } else if let Some(heading) = heading(line) {
            flush(&mut paragraph, &mut out);
            out.push(Block::Heading(inline(heading)));
        } else if let Some(item) = item(line) {
            flush(&mut paragraph, &mut out);
            out.push(Block::Item(inline(item)));
        } else {
            paragraph.push(line);
        }
    }
    flush(&mut paragraph, &mut out);
    out.retain(|block| match block {
        Block::Heading(text) | Block::Item(text) | Block::Paragraph(text) => !text.is_empty(),
    });
    out
}

fn heading(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    (1..=6)
        .contains(&hashes)
        .then(|| line[hashes..].strip_prefix(' '))?
}

fn item(line: &str) -> Option<&str> {
    for mark in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(mark) {
            return Some(rest);
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = &line[digits..];
        for mark in [". ", ") "] {
            if let Some(rest) = rest.strip_prefix(mark) {
                return Some(rest);
            }
        }
    }
    None
}

/// A line without its Markdown marks: `[text](url)` as its text, and no
/// `**`, `__`, backticks or single `*`/`_` around words.
fn inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](").map(|at| open + at) else {
            break;
        };
        let Some(end) = rest[close..].find(')').map(|at| close + at) else {
            break;
        };
        let before = &rest[..open];
        out.push_str(before.strip_suffix('!').unwrap_or(before));
        out.push_str(&rest[open + 1..close]);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    let out = out.replace("**", "").replace("__", "").replace('`', "");
    out.split(' ')
        .map(|word| {
            // `_word_` and `*word*`, and the punctuation after them.
            let body = word.trim_end_matches(['.', ',', ';', ':', '!', '?']);
            let after = &word[body.len()..];
            let inner = body.trim_matches(|c| c == '*' || c == '_');
            if !inner.is_empty() && inner.len() + 2 == body.len() {
                format!("{inner}{after}")
            } else {
                word.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_become_plain_blocks() {
        let markdown = "## New\n- Rooms remember **their rules**.\n- See [the guide](https://x).\n\nPlain `code` and _words_.\n---\n1. First";
        assert_eq!(
            blocks(markdown),
            [
                Block::Heading("New".into()),
                Block::Item("Rooms remember their rules.".into()),
                Block::Item("See the guide.".into()),
                Block::Paragraph("Plain code and words.".into()),
                Block::Item("First".into()),
            ]
        );
    }

    #[test]
    fn a_release_is_read_from_the_api() {
        let json = br###"{"tag_name":"v0.2.0","body":"## Fixes\n- Joining works."}"###;
        assert_eq!(
            parse(json),
            Some(Notes::Release {
                version: "0.2.0".into(),
                blocks: vec![
                    Block::Heading("Fixes".into()),
                    Block::Item("Joining works.".into())
                ],
            })
        );
        assert_eq!(parse(br#"{"tag_name":"latest"}"#), None);
        assert_eq!(parse(b"not json"), None);
    }
}

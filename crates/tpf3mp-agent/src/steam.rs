//! Finding a game Steam installed: its folder and its build.
//!
//! Steam keeps, in each library, `steamapps/appmanifest_<app>.acf`, which
//! names the app's folder under `steamapps/common` and the build installed,
//! and in its own folder `steamapps/libraryfolders.vdf`, which lists every
//! library. Both are in Valve's text KeyValues format, read here.
//!
//! Everything here only reads, and gives up quietly: a player without
//! Steam, or with the game elsewhere, simply has no game found.

use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

/// Transport Fever 3's Steam app ID.
pub const TRANSPORT_FEVER_3: u32 = 3_493_540;
/// Largest Steam file read.
const MAX_FILE: u64 = 1 << 20;
/// Deepest nesting read.
const MAX_DEPTH: usize = 32;

/// A game Steam installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub app: u32,
    /// The game's name, as Steam shows it.
    pub name: String,
    /// The game's folder.
    pub dir: PathBuf,
    /// Steam's build ID of the installed game: the same for every platform
    /// once a player has the latest update.
    pub build: String,
}

/// `app`, if Steam installed it on this machine.
pub fn find(app: u32) -> Option<Installed> {
    find_in(&steam_roots(), app)
}

/// `app`, in the libraries of the Steam installations at `roots`.
pub fn find_in(roots: &[PathBuf], app: u32) -> Option<Installed> {
    for root in roots {
        for library in libraries(root) {
            if let Some(installed) = installed_in(&library, app) {
                return Some(installed);
            }
        }
    }
    None
}

fn installed_in(library: &Path, app: u32) -> Option<Installed> {
    let steamapps = library.join("steamapps");
    let text = read_small(&steamapps.join(format!("appmanifest_{app}.acf")))?;
    let manifest = parse(&text).ok()?;
    let state = manifest.map("AppState")?;
    let folder = state.text("installdir")?;
    // One plain folder name: nothing that leaves `common`.
    let mut parts = Path::new(folder).components();
    let (Some(Component::Normal(_)), None) = (parts.next(), parts.next()) else {
        return None;
    };
    let dir = steamapps.join("common").join(folder);
    dir.is_dir().then(|| Installed {
        app,
        name: state.text("name").unwrap_or(folder).to_owned(),
        dir,
        build: state.text("buildid").unwrap_or("unknown").to_owned(),
    })
}

/// The libraries of the Steam installation at `root`: its own folder, and
/// every one `libraryfolders.vdf` lists.
fn libraries(root: &Path) -> Vec<PathBuf> {
    let mut found = vec![root.to_owned()];
    let listed = read_small(&root.join("steamapps").join("libraryfolders.vdf"))
        .and_then(|text| parse(&text).ok());
    if let Some(folders) = listed
        .as_ref()
        .and_then(|listed| listed.map("libraryfolders"))
    {
        for value in folders.0.values() {
            // Current Steam: a block with a "path"; older Steam: the path.
            let path = match value {
                Value::Map(folder) => folder.text("path"),
                Value::Text(path) => Some(path.as_str()),
            };
            if let Some(path) = path.map(PathBuf::from)
                && path.is_absolute()
                && !found.contains(&path)
            {
                found.push(path);
            }
        }
    }
    found
}

/// Where Steam may be installed on this system, the likeliest first.
pub fn steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(windows)]
    {
        if let Some(path) = windows_steam_path() {
            roots.push(path);
        }
        for base in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(base) = std::env::var_os(base) {
                roots.push(PathBuf::from(base).join("Steam"));
            }
        }
    }
    if let Some(home) = dirs::home_dir() {
        if cfg!(target_os = "macos") {
            roots.push(home.join("Library/Application Support/Steam"));
        } else if cfg!(target_os = "linux") {
            for path in [
                ".steam/steam",
                ".local/share/Steam",
                ".var/app/com.valvesoftware.Steam/.local/share/Steam",
                "snap/steam/common/.local/share/Steam",
            ] {
                roots.push(home.join(path));
            }
        }
    }
    let mut unique = Vec::new();
    for root in roots {
        let root = fs::canonicalize(&root).unwrap_or(root);
        if root.is_dir() && !unique.contains(&root) {
            unique.push(root);
        }
    }
    unique
}

/// Steam's folder as the registry names it, for installations outside
/// Program Files.
#[cfg(windows)]
fn windows_steam_path() -> Option<PathBuf> {
    use std::os::windows::process::CommandExt;
    // Without a console window flashing up from the launcher's window.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().find(|line| line.contains("SteamPath"))?;
    let (_, path) = line.split_once("REG_SZ")?;
    let path = path.trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn read_small(path: &Path) -> Option<String> {
    (fs::metadata(path).ok()?.len() <= MAX_FILE)
        .then(|| fs::read_to_string(path).ok())
        .flatten()
}

/// A KeyValues block: its keys, compared without regard to case as Steam
/// does, and their values.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Block(BTreeMap<String, Value>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Map(Block),
}

impl Block {
    fn get(&self, key: &str) -> Option<&Value> {
        self.0
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Text(text) => Some(text),
            Value::Map(_) => None,
        }
    }

    pub fn map(&self, key: &str) -> Option<&Block> {
        match self.get(key)? {
            Value::Map(block) => Some(block),
            Value::Text(_) => None,
        }
    }
}

/// Reads Valve's text KeyValues: `"key" "value"` pairs and `"key" { … }`
/// blocks, with `//` comments and `[$PLATFORM]` conditions, which are
/// skipped.
pub fn parse(text: &str) -> Result<Block, String> {
    let mut tokens = Tokens::new(text);
    let block = parse_block(&mut tokens, 0)?;
    match tokens.next()? {
        None => Ok(block),
        Some(_) => Err("a '}' closes nothing".into()),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Text(String),
    Open,
    Close,
}

fn parse_block(tokens: &mut Tokens<'_>, depth: usize) -> Result<Block, String> {
    if depth > MAX_DEPTH {
        return Err("nested too deeply".into());
    }
    let mut block = Block::default();
    loop {
        let key = match tokens.peek()? {
            None | Some(Token::Close) => return Ok(block),
            Some(Token::Open) => return Err("a block without a key".into()),
            Some(Token::Text(_)) => match tokens.next()? {
                Some(Token::Text(key)) => key,
                _ => unreachable!("peeked a key"),
            },
        };
        let value = match tokens.next()? {
            Some(Token::Text(value)) => Value::Text(value),
            Some(Token::Open) => {
                let inner = parse_block(tokens, depth + 1)?;
                match tokens.next()? {
                    Some(Token::Close) => Value::Map(inner),
                    _ => return Err(format!("the block \"{key}\" is not closed")),
                }
            }
            Some(Token::Close) | None => return Err(format!("\"{key}\" has no value")),
        };
        block.0.insert(key, value);
    }
}

struct Tokens<'a> {
    rest: &'a str,
    peeked: Option<Option<Token>>,
}

impl<'a> Tokens<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            rest: text.trim_start_matches('\u{feff}'),
            peeked: None,
        }
    }

    fn peek(&mut self) -> Result<Option<&Token>, String> {
        if self.peeked.is_none() {
            self.peeked = Some(self.read()?);
        }
        Ok(self.peeked.as_ref().and_then(Option::as_ref))
    }

    fn next(&mut self) -> Result<Option<Token>, String> {
        match self.peeked.take() {
            Some(token) => Ok(token),
            None => self.read(),
        }
    }

    fn read(&mut self) -> Result<Option<Token>, String> {
        loop {
            self.rest = self.rest.trim_start();
            if let Some(comment) = self.rest.strip_prefix("//") {
                self.rest = comment.split_once('\n').map_or("", |(_, rest)| rest);
                continue;
            }
            if self.rest.starts_with('[') {
                // A platform condition such as [$WIN32]: not a token.
                let end = self.rest.find(']').ok_or("an unclosed '['")?;
                self.rest = &self.rest[end + 1..];
                continue;
            }
            break;
        }
        let mut chars = self.rest.chars();
        let Some(first) = chars.next() else {
            return Ok(None);
        };
        match first {
            '{' => {
                self.rest = &self.rest[1..];
                Ok(Some(Token::Open))
            }
            '}' => {
                self.rest = &self.rest[1..];
                Ok(Some(Token::Close))
            }
            '"' => {
                let mut text = String::new();
                let mut escaped = false;
                for (at, c) in self.rest.char_indices().skip(1) {
                    if escaped {
                        text.push(match c {
                            'n' => '\n',
                            't' => '\t',
                            other => other,
                        });
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '"' {
                        self.rest = &self.rest[at + 1..];
                        return Ok(Some(Token::Text(text)));
                    } else {
                        text.push(c);
                    }
                }
                Err("an unclosed '\"'".into())
            }
            _ => {
                let end = self
                    .rest
                    .find(|c: char| c.is_whitespace() || matches!(c, '{' | '}' | '"'))
                    .unwrap_or(self.rest.len());
                let token = self.rest[..end].to_owned();
                self.rest = &self.rest[end..];
                Ok(Some(Token::Text(token)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIBRARIES: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"LIBRARY_ONE"
		"label"		""
		"apps"
		{
			"228980"		"584971454"
		}
	}
	"1"
	{
		"path"		"LIBRARY_TWO"
		"apps"
		{
			"3493540"		"40740960308"
		}
	}
}
"#;

    fn manifest(installdir: &str) -> String {
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"3493540\"\n\t\"name\"\t\t\"Transport Fever 3\"\n\t\"installdir\"\t\t\"{installdir}\"\n\t\"buildid\"\t\t\"20412345\"\n\t\"UserConfig\"\n\t{{\n\t\t\"language\"\t\t\"english\"\n\t}}\n}}\n"
        )
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tpf3mp-steam-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A Steam installation at `root` whose second library holds the game.
    fn steam(root: &Path, installdir: &str) -> PathBuf {
        let one = root.join("steam");
        let two = root.join("games");
        fs::create_dir_all(one.join("steamapps")).unwrap();
        fs::create_dir_all(
            two.join("steamapps")
                .join("common")
                .join("Transport Fever 3"),
        )
        .unwrap();
        let listing = LIBRARIES
            .replace(
                "LIBRARY_ONE",
                &one.display().to_string().replace('\\', "\\\\"),
            )
            .replace(
                "LIBRARY_TWO",
                &two.display().to_string().replace('\\', "\\\\"),
            );
        fs::write(one.join("steamapps").join("libraryfolders.vdf"), listing).unwrap();
        fs::write(
            two.join("steamapps").join("appmanifest_3493540.acf"),
            manifest(installdir),
        )
        .unwrap();
        one
    }

    #[test]
    fn keyvalues_read_as_steam_writes_them() {
        let block = parse(&manifest("Transport Fever 3")).unwrap();
        let state = block.map("appstate").unwrap();
        assert_eq!(state.text("installdir"), Some("Transport Fever 3"));
        assert_eq!(state.text("BuildID"), Some("20412345"));
        assert_eq!(
            state.map("UserConfig").unwrap().text("language"),
            Some("english")
        );
        let odd = parse("// a comment\nkey value [$WIN32]\n\"quoted \\\"one\\\"\" { inner \"x\" }")
            .unwrap();
        assert_eq!(odd.text("key"), Some("value"));
        assert_eq!(odd.map("quoted \"one\"").unwrap().text("inner"), Some("x"));
        assert!(parse("\"a\" { \"b\" \"c\"").is_err());
        assert!(parse("\"a\"").is_err());
        assert!(parse("}").is_err());
        let deep = "\"a\" {".repeat(64);
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn the_game_is_found_in_any_library() {
        let root = temp("found");
        let steam_root = steam(&root, "Transport Fever 3");
        let installed = find_in(&[steam_root], TRANSPORT_FEVER_3).unwrap();
        assert_eq!(installed.name, "Transport Fever 3");
        assert_eq!(installed.build, "20412345");
        assert_eq!(
            installed.dir,
            root.join("games")
                .join("steamapps")
                .join("common")
                .join("Transport Fever 3")
        );
        assert_eq!(find_in(&[root.join("steam")], 1_066_780), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_folder_name_that_leaves_the_library_is_ignored() {
        let root = temp("escape");
        let steam_root = steam(&root, "../../elsewhere");
        assert_eq!(find_in(&[steam_root], TRANSPORT_FEVER_3), None);
        fs::remove_dir_all(&root).unwrap();
    }

    /// This machine's Steam, if it has Transport Fever 2: a check by hand.
    #[test]
    #[ignore = "reads this machine's Steam libraries"]
    fn this_machines_transport_fever_2() {
        println!("{:?}", find(1_066_780));
    }
}

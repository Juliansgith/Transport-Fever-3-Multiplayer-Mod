//! Which build of TPF3-MP this is: what a launcher logs when it starts, shows
//! in its window, says when a server speaks another protocol, and tells
//! another launcher it meets ([`crate::launcher::instance`]). The commit and
//! build time come from the build script (`tpf3mp-buildinfo`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The version in `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The git commit built, short, or `unknown`.
pub const COMMIT: &str = env!("TPF3MP_COMMIT");
/// When it was built, in UTC.
pub const BUILT: &str = env!("TPF3MP_BUILT");

/// What decides whether two launchers are the same build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    pub version: String,
    /// The server protocol it speaks (`tpf3mp_proto::PROTOCOL_VERSION`).
    pub protocol: u32,
    /// The game link's protocol (`tpf3mp_bridge::BRIDGE_VERSION`).
    pub bridge: u32,
    pub commit: String,
}

impl Build {
    /// This program's build.
    pub fn this() -> Self {
        Self {
            version: VERSION.to_owned(),
            protocol: tpf3mp_proto::PROTOCOL_VERSION,
            bridge: tpf3mp_bridge::BRIDGE_VERSION,
            commit: COMMIT.to_owned(),
        }
    }

    /// `TPF3-MP 0.1.0 (protocol 13, commit 1316710abc)`.
    pub fn describe(&self) -> String {
        format!(
            "TPF3-MP {} (protocol {}, commit {})",
            self.version, self.protocol, self.commit
        )
    }
}

/// This program's file, as far as the system says.
pub fn exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// How a path shows in a message: in full, or that it is not known.
pub fn shown(path: Option<&Path>) -> String {
    path.map_or_else(
        || "(unknown file)".to_owned(),
        |path| path.display().to_string(),
    )
}

/// The line a launcher logs first: the file that runs and its build, so a
/// log always says which launcher wrote it.
pub fn startup_line(exe: Option<&Path>, build: &Build, built: &str) -> String {
    format!(
        "the launcher starts: {} is TPF3-MP {}, protocol {}, bridge {}, commit {}, built {}",
        shown(exe),
        build.version,
        build.protocol,
        build.bridge,
        build.commit,
        built
    )
}

/// What a player is told when the server speaks protocol `server` and this
/// launcher, the file `exe`, speaks `client`: which side is old, and what to
/// do about it. The advice comes before the file, so a short window (the
/// game's) still shows it.
pub fn protocol_mismatch(client: u32, server: u32, exe: Option<&Path>) -> String {
    let file = shown(exe);
    if server > client {
        format!(
            "This launcher is too old for the server (it speaks protocol {client}, the server \
             {server}). Close it and start the newest TPF3-MP, or reinstall. You started {file}."
        )
    } else {
        format!(
            "This launcher is newer than the server (it speaks protocol {client}, the server \
             {server}): the server needs updating. You started {file}."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build() -> Build {
        Build {
            version: "0.1.0".into(),
            protocol: 13,
            bridge: 19,
            commit: "1316710abc".into(),
        }
    }

    #[test]
    fn the_startup_line_names_the_file_and_the_build() {
        let exe = Path::new(r"C:\Users\p\AppData\Local\Programs\TPF3-MP\TPF3-MP.exe");
        let line = startup_line(Some(exe), &build(), "2026-09-30 21:00 UTC");
        assert_eq!(
            line,
            format!(
                "the launcher starts: {} is TPF3-MP 0.1.0, protocol 13, bridge 19, commit 1316710abc, built 2026-09-30 21:00 UTC",
                exe.display()
            )
        );
        assert!(startup_line(None, &build(), "x").contains("(unknown file)"));
    }

    #[test]
    fn this_build_is_this_programs() {
        let this = Build::this();
        assert_eq!(this.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(this.protocol, tpf3mp_proto::PROTOCOL_VERSION);
        assert_eq!(this.bridge, tpf3mp_bridge::BRIDGE_VERSION);
        assert!(
            COMMIT == "unknown" || COMMIT.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "{COMMIT}"
        );
        assert_eq!(
            build().describe(),
            "TPF3-MP 0.1.0 (protocol 13, commit 1316710abc)"
        );
    }

    #[test]
    fn an_old_launcher_is_told_to_start_the_newest_naming_its_file() {
        let exe = Path::new(r"C:\Games\old\TPF3-MP.exe");
        let message = protocol_mismatch(12, 13, Some(exe));
        assert_eq!(
            message,
            format!(
                "This launcher is too old for the server (it speaks protocol 12, the server 13). \
                 Close it and start the newest TPF3-MP, or reinstall. You started {}.",
                exe.display()
            )
        );
    }

    #[test]
    fn a_newer_launcher_says_the_server_needs_updating() {
        let exe = Path::new("/opt/tpf3mp/tpf3mp-launcher");
        let message = protocol_mismatch(14, 13, Some(exe));
        assert!(
            message.starts_with(
                "This launcher is newer than the server (it speaks protocol 14, the server 13): \
                 the server needs updating."
            ),
            "{message}"
        );
        assert!(message.contains("/opt/tpf3mp/tpf3mp-launcher"));
        assert!(!message.contains("too old"));
    }
}

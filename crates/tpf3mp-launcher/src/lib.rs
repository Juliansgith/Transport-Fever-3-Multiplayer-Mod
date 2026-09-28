//! The TPF3-MP launcher: the players' window, for Windows, Linux and macOS.
//!
//! The window is a web view (Tauri) showing the launcher's page (`ui/`, a
//! port of tearded's TPF2 Multiplayer Launcher; D16) over the launcher
//! backend of `tpf3mp-agent`, which runs in the same process ([`shell`]).
//! It logs to daily files ([`logs`]) and keeps itself up to date from the
//! project's signed releases ([`update`]). Where no window can be opened,
//! the launcher falls back to the agent's page in the browser.

pub mod installed;
pub mod logs;
pub mod probe;
pub mod releases;
pub mod shell;
pub mod update;

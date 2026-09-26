//! The TPF3-MP launcher as a native window for Windows, Linux and macOS.
//!
//! The window is drawn with egui ([`app`]) over the launcher backend of
//! `tpf3mp-agent`, which runs in the same process ([`backend`]). It logs
//! to daily files ([`logs`]) and keeps itself up to date from the
//! project's signed releases ([`update`]). Where no window can be opened,
//! the launcher falls back to the same launcher as a page in the browser.

pub mod app;
pub mod backend;
pub mod icon;
pub mod logs;
pub mod update;

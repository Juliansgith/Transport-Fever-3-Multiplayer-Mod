//! A launcher's setup from its command line, shared by `tpf3mp-agent
//! launcher` (the page in a browser) and `tpf3mp-launcher` (the native
//! window): the same options give the same player, server and game.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use clap::Args;
use tpf3mp_net::{CertificateDer, Identity, ServerTrust, tunnel::TunnelUrl};
use tpf3mp_proto::RoomSettings;

use super::{LauncherConfig, Remembered};
use crate::{TunnelChoice, Worlds, content};

/// Where the per-user files live: `TPF3-MP` in the user's local data
/// directory.
pub fn data_dir() -> Result<PathBuf> {
    Ok(dirs::data_local_dir()
        .context("this system has no per-user data directory")?
        .join("TPF3-MP"))
}

/// The identity key file: the one given, or the per-user default.
pub fn identity_path(given: Option<&Path>) -> Result<PathBuf> {
    match given {
        Some(path) => Ok(path.to_owned()),
        None => Ok(data_dir().context("pass --identity")?.join("identity.key")),
    }
}

/// The worlds kept for the game on `link`: in `dir`, or in a directory per
/// link in the per-user data directory.
pub fn open_worlds(dir: Option<&Path>, gib: u64, link: &str) -> Result<Worlds> {
    let dir = match dir {
        Some(dir) => dir.to_owned(),
        None => data_dir()
            .context("pass --worlds")?
            .join("worlds")
            .join(link),
    };
    Worlds::open(&dir, gib << 30)
        .with_context(|| format!("opening the worlds in {}", dir.display()))
}

/// How servers are trusted: exactly the certificate in `pin_cert`, or the
/// public certificate authorities.
pub fn trust(pin_cert: Option<&Path>) -> Result<ServerTrust> {
    match pin_cert {
        Some(path) => Ok(ServerTrust::Pinned(CertificateDer::from(
            std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
        ))),
        None => Ok(ServerTrust::WebPki),
    }
}

/// Opens `url` in the default browser, as far as the system allows.
/// Returns whether a browser could be started.
pub fn open_in_browser(url: &str) -> bool {
    open_with_system(url)
}

/// Opens a folder in the system's file manager. Returns whether one could
/// be started.
pub fn open_folder(dir: &Path) -> bool {
    open_with_system(&dir.to_string_lossy())
}

fn open_with_system(target: &str) -> bool {
    // Explorer opens folders and addresses alike, without the console
    // window `cmd /C start` would flash from a windowed program.
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(target)
        .spawn()
        .is_ok()
}

/// For networks that block UDP: QUIC through a WebSocket tunnel.
#[derive(Debug, Clone, Default, Args)]
pub struct TunnelArgs {
    /// The tunnel to take when UDP gets no answer, as a wss:// URL.
    /// Defaults to wss://<server host>/tpf3mp.
    #[arg(long)]
    pub tunnel: Option<String>,

    /// Connect through the tunnel only, never over UDP.
    #[arg(long, conflicts_with = "no_tunnel")]
    pub tunnel_only: bool,

    /// Never take a tunnel.
    #[arg(long, conflicts_with = "tunnel")]
    pub no_tunnel: bool,
}

impl TunnelArgs {
    pub fn choice(&self) -> Result<TunnelChoice> {
        if self.no_tunnel {
            return Ok(TunnelChoice::Off);
        }
        let url = self
            .tunnel
            .as_deref()
            .map(|url| {
                url.parse::<TunnelUrl>()
                    .with_context(|| format!("the tunnel URL {url}"))
            })
            .transpose()?;
        Ok(match (url, self.tunnel_only) {
            (url, true) => TunnelChoice::Only(url),
            (Some(url), false) => TunnelChoice::Url(url),
            (None, false) => TunnelChoice::Default,
        })
    }
}

/// What the player, the server and the game are.
#[derive(Debug, Clone, Args)]
pub struct LauncherArgs {
    /// Where the page is served, when there is one. Loopback only.
    #[arg(long, default_value = "127.0.0.1:47470")]
    pub listen: SocketAddr,

    /// The server offered first, as host:port. Without it, the server last
    /// connected to, then --default-server.
    #[arg(long)]
    pub server: Option<String>,

    /// The server offered when there is neither --server nor one from last
    /// time, as a package sets it.
    #[arg(long)]
    pub default_server: Option<String>,

    /// Trust exactly this DER certificate instead of public certificate
    /// authorities (for development servers).
    #[arg(long)]
    pub pin_cert: Option<PathBuf>,

    /// The name offered first. Without it, the name last used.
    #[arg(long)]
    pub name: Option<String>,

    /// Identity key file. Created on first use.
    #[arg(long)]
    pub identity: Option<PathBuf>,

    #[command(flatten)]
    pub tunnel: TunnelArgs,

    /// The shared-memory link the game's hook opens.
    #[arg(long, default_value = tpf3mp_bridge::DEFAULT_LINK)]
    pub game_link: String,

    /// The game's build. Every player in a room must run the same.
    #[arg(long, default_value = "tpf3")]
    pub game_build: String,

    /// A file listing the game's active mods in load order, one per line:
    /// the mod's name, then its version.
    #[arg(long)]
    pub mods: Option<PathBuf>,

    /// Where worlds are kept. Defaults to the per-user data directory.
    #[arg(long)]
    pub worlds: Option<PathBuf>,

    /// Space worlds may take, in GiB.
    #[arg(long, default_value_t = 8)]
    pub worlds_gib: u64,
}

impl LauncherArgs {
    /// The launcher these options describe: the player's identity (created
    /// on first use), with the server and name remembered from last time
    /// where none are given, and the game's content and worlds.
    pub fn config(&self) -> Result<LauncherConfig> {
        let identity_file = identity_path(self.identity.as_deref())?;
        let identity = Arc::new(Identity::load_or_create(&identity_file)?);
        // Next to the identity: the same player's last server and name.
        let remember = identity_file.with_file_name("launcher.json");
        let remembered = Remembered::load(&remember);
        Ok(LauncherConfig {
            listen: self.listen,
            server: self
                .server
                .clone()
                .or(remembered.server)
                .or_else(|| self.default_server.clone()),
            tunnel: self.tunnel.choice()?,
            remember: Some(remember),
            trust: trust(self.pin_cert.as_deref())?,
            identity,
            name: self
                .name
                .clone()
                .or(remembered.name)
                .unwrap_or_else(|| "player".to_owned()),
            content: content::manifest(&self.game_build, self.mods.as_deref())?,
            link: self.game_link.clone(),
            worlds: open_worlds(self.worlds.as_deref(), self.worlds_gib, &self.game_link)?,
            room_settings: RoomSettings::DEFAULT,
        })
    }
}

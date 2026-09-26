use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use clap::{Args as ClapArgs, Parser, Subcommand};
use tpf3mp_agent::{
    Client, ClientError, ClientEvent, ConnectOptions, Events, Worlds,
    bridge::{self, Bridge, BridgeOptions, Rejoin},
    connect, content,
    launcher::{self, Launcher},
};
use tpf3mp_net::Identity;
use tpf3mp_proto::{
    ContentDiff, ContentManifest, CreateRoom, Invite, JoinRoom, RequestError, RoomPhase,
    RoomSettings, RoomView, Text,
};
use tracing_subscriber::EnvFilter;

/// The TPF3-MP agent, which connects the game to a TPF3-MP server.
#[derive(Debug, Parser)]
#[command(version)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Connect to a server, complete the handshake and report the session.
    Connect(Server),
    /// Create a room, print its invite, and follow it until Ctrl-C.
    Host {
        #[command(flatten)]
        server: Server,
        #[command(flatten)]
        game: Game,
        /// Room name shown to players.
        #[arg(long, default_value = "TPF3-MP room")]
        room_name: String,
        /// Require this password in addition to the invite.
        #[arg(long)]
        password: Option<String>,
        #[arg(long, default_value_t = 8)]
        max_players: u8,
        /// The rules, and with them the economy, the room is played by: one
        /// the server offers (`connect` lists them). Without it, the
        /// server's default, normally `native`: the game's own.
        #[arg(long)]
        rules: Option<String>,
        /// Start the game once this many players are in the room and ready.
        #[arg(long)]
        start_with: Option<usize>,
    },
    /// Open the launcher: a page in your browser from which you connect,
    /// create or join rooms, get ready, chat and play.
    // A flag given twice counts once, the later winning, so a player may
    // add flags to a package's script.
    #[command(args_override_self = true)]
    Launcher(WebLauncherArgs),
    /// Join a room with an invite and follow it until Ctrl-C.
    Join {
        #[command(flatten)]
        server: Server,
        #[command(flatten)]
        game: Game,
        invite: String,
        #[arg(long)]
        password: Option<String>,
    },
}

#[derive(Debug, ClapArgs)]
struct Game {
    /// Play the room through the game: create the shared-memory link of
    /// this name, which the game's hook opens, and bridge the two. Without
    /// a name, the link the hook opens by default.
    #[arg(long, num_args = 0..=1, default_missing_value = tpf3mp_bridge::DEFAULT_LINK)]
    game_link: Option<String>,

    /// The game's build. Every player in a room must run the same.
    #[arg(long, default_value = "tpf3")]
    game_build: String,

    /// A file listing the game's active mods in load order, one per line:
    /// the mod's name, then its version. Every player in a room must run
    /// the same; the room says which differ.
    #[arg(long)]
    mods: Option<PathBuf>,

    /// Where worlds are kept: saves the room agreed on, and worlds received
    /// to join running games. Defaults to a directory per game link in the
    /// per-user data directory; two agents cannot share one.
    #[arg(long)]
    worlds: Option<PathBuf>,

    /// Space worlds may take, in GiB.
    #[arg(long, default_value_t = 8)]
    worlds_gib: u64,
}

impl Game {
    /// What this player's game runs.
    fn manifest(&self) -> Result<ContentManifest> {
        Ok(content::manifest(&self.game_build, self.mods.as_deref())?)
    }

    fn open_worlds(&self, link: &str) -> Result<Worlds> {
        launcher::setup::open_worlds(self.worlds.as_deref(), self.worlds_gib, link)
    }
}

/// The launcher as a page in the browser.
#[derive(Debug, ClapArgs)]
struct WebLauncherArgs {
    #[command(flatten)]
    launcher: launcher::setup::LauncherArgs,

    /// Only print the page's address; do not open a browser.
    #[arg(long)]
    no_open: bool,
}

#[derive(Debug, ClapArgs)]
struct Server {
    /// Server address as host:port.
    server: String,

    /// Trust exactly this DER certificate instead of public certificate
    /// authorities (for development servers started with --dev-self-signed).
    #[arg(long)]
    pin_cert: Option<PathBuf>,

    /// Player name shown to others.
    #[arg(long, default_value = "player")]
    name: String,

    /// Identity key file. Created on first use; keep it private, it is what
    /// makes you the same player next time.
    #[arg(long)]
    identity: Option<PathBuf>,

    #[command(flatten)]
    tunnel: launcher::setup::TunnelArgs,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .init();
    // On the heap: the commands' futures hold whole connection handshakes,
    // more than the main thread's stack has room for in a debug build.
    Box::pin(run(Args::parse().command)).await
}

async fn run(command: Command) -> Result<()> {
    match command {
        Command::Connect(server) => {
            let (client, _events) = open(&server).await?;
            let welcome = client.welcome();
            println!(
                "connected as {}: server {}, session {}, round trip {} ms",
                client.player(),
                welcome.server_version,
                welcome.session_id,
                client.rtt().as_millis()
            );
            for rules in &welcome.rules {
                println!("rules offered: {} ({})", rules.name, rules.description);
            }
            client.close().await;
        }
        Command::Host {
            server,
            game,
            room_name,
            password,
            max_players,
            rules,
            start_with,
        } => {
            let options = options(&server).await?;
            let (client, mut events) = connect(options.clone()).await?;
            let content = game.manifest()?;
            client.declare_content(content.clone()).await?;
            let password = password.map(Text::new).transpose().context("password")?;
            let (invite, room) = client
                .create_room(CreateRoom {
                    name: Text::new(room_name).context("room name")?,
                    max_players,
                    password: password.clone(),
                    settings: RoomSettings::DEFAULT,
                    rules: rules.map(Text::new).transpose().context("rules")?,
                })
                .await?;
            println!("invite: {invite}");
            print_room(&room);
            get_ready(&client).await?;
            if let Some(players) = start_with {
                start_when_ready(&client, &mut events, players).await?;
            }
            let rejoin = Rejoin {
                options,
                invite,
                password,
                content: Some(content),
                give_up_after: REJOIN_PATIENCE,
            };
            play(client, events, &game, rejoin).await?;
        }
        Command::Launcher(args) => launch(args).await?,
        Command::Join {
            server,
            game,
            invite,
            password,
        } => {
            let invite: Invite = invite.parse()?;
            let options = options(&server).await?;
            let (client, mut events) = connect(options.clone()).await?;
            let password = password.map(Text::new).transpose().context("password")?;
            let content = game.manifest()?;
            client.declare_content(content.clone()).await?;
            let joined = client
                .join_room(JoinRoom {
                    invite: invite.clone(),
                    password: password.clone(),
                    resume: None,
                })
                .await;
            let room = match joined {
                Err(ClientError::Refused(RequestError::ContentMismatch)) => {
                    match refused_diff(&mut events).await {
                        Some(diff) => anyhow::bail!("your game differs from the room's: {diff}"),
                        None => anyhow::bail!("{}", RequestError::ContentMismatch),
                    }
                }
                joined => joined?,
            };
            print_room(&room);
            // A running game was joined as it is; a lobby wants readiness.
            if room.phase == RoomPhase::Lobby {
                get_ready(&client).await?;
            }
            let rejoin = Rejoin {
                options,
                invite,
                password,
                content: Some(content),
                give_up_after: REJOIN_PATIENCE,
            };
            play(client, events, &game, rejoin).await?;
        }
    }
    Ok(())
}

/// Serves the launcher until Ctrl-C.
async fn launch(args: WebLauncherArgs) -> Result<()> {
    let listen = args.launcher.listen;
    let launcher = Launcher::start(args.launcher.config()?)
        .await
        .with_context(|| format!("serving the launcher on {listen}"))?;
    let url = launcher.url().context("the launcher serves no page")?;
    println!("TPF3-MP launcher: {url}");
    println!("Keep this window open while you play. Ctrl-C stops the launcher.");
    if !args.no_open && !launcher::setup::open_in_browser(url) {
        println!("Open the address above in your browser.");
    }
    tokio::select! {
        () = launcher.wait() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    Ok(())
}

/// How long the agent keeps trying to rejoin a room after losing the
/// server, for example while it restarts.
const REJOIN_PATIENCE: Duration = Duration::from_secs(300);

/// Says this player is ready. What the game runs was declared on
/// connecting; the room says if it differs.
async fn get_ready(client: &Client) -> Result<()> {
    client.set_ready(true).await?;
    Ok(())
}

/// How the game differs from a room that refused it, if the room says so
/// within a second.
async fn refused_diff(events: &mut Events) -> Option<ContentDiff> {
    tokio::time::timeout(Duration::from_secs(1), async {
        while let Some(event) = events.recv().await {
            if let ClientEvent::ContentDiff(diff) = event {
                return diff;
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

/// Waits until `players` members are in the room and ready, then starts.
async fn start_when_ready(client: &Client, events: &mut Events, players: usize) -> Result<()> {
    println!("starting once {players} players are ready");
    loop {
        match events.recv().await {
            Some(ClientEvent::RoomUpdate(room)) => {
                print_room(&room);
                if room.members.len() >= players && room.members.iter().all(|m| m.ready) {
                    client.start_game().await?;
                    println!("game started");
                    return Ok(());
                }
            }
            Some(ClientEvent::ContentDiff(Some(diff))) => {
                println!("a player's game differs from yours: {diff}");
            }
            Some(ClientEvent::Closed(reason)) => anyhow::bail!("disconnected: {reason}"),
            Some(_) => {}
            None => anyhow::bail!("the connection ended"),
        }
    }
}

/// Follows the room, through the game when a link name is given. Through
/// the game, a lost server is rejoined and the room resumed.
async fn play(client: Client, events: Events, game: &Game, rejoin: Rejoin) -> Result<()> {
    let Some(name) = &game.game_link else {
        follow(client, events).await;
        return Ok(());
    };
    let link = tpf3mp_ipc::Link::create(&tpf3mp_ipc::Config::new(name), tpf3mp_ipc::Role::Agent)
        .with_context(|| format!("creating the game link {name}"))?;
    println!("waiting for the game on link {name}");
    let options = BridgeOptions {
        worlds: Some(game.open_worlds(name)?),
        ..BridgeOptions::default()
    };
    let mut bridge = Bridge::new(link, options);
    tokio::select! {
        ended = bridge::play(&mut bridge, client, events, &rejoin) => match ended {
            Ok(end) => println!("the session ended: {end:?}"),
            Err(fault) => eprintln!("the bridge to the game failed: {fault}"),
        },
        _ = tokio::signal::ctrl_c() => bridge.end("the agent was stopped"),
    }
    Ok(())
}

async fn open(server: &Server) -> Result<(Client, Events)> {
    Ok(connect(options(server).await?).await?)
}

async fn options(server: &Server) -> Result<ConnectOptions> {
    let (host, _port) = server
        .server
        .rsplit_once(':')
        .context("the server address must be host:port")?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let address = tpf3mp_agent::resolve(&server.server)
        .await
        .with_context(|| format!("resolving {}", server.server))?;
    let trust = launcher::setup::trust(server.pin_cert.as_deref())?;
    let identity = Arc::new(Identity::load_or_create(&launcher::setup::identity_path(
        server.identity.as_deref(),
    )?)?);
    let name = Text::new(server.name.clone()).context("player name")?;
    let mut options = ConnectOptions::new(address, host, trust, identity, name);
    options.route = server.tunnel.choice()?.route(host)?;
    Ok(options)
}

fn print_room(room: &RoomView) {
    println!(
        "room {} ({:?}), {} of {} players:",
        room.name,
        room.phase,
        room.members.len(),
        room.max_players
    );
    for member in &room.members {
        println!(
            "  {} {} {:?}{}{}",
            member.player,
            member.name,
            member.platform.os,
            if member.ready { " ready" } else { "" },
            if member.connected { "" } else { " (away)" },
        );
    }
}

async fn follow(client: Client, mut events: Events) {
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(ClientEvent::RoomUpdate(room)) => print_room(&room),
                Some(ClientEvent::Closed(reason)) => {
                    println!("disconnected: {reason}");
                    return;
                }
                Some(other) => println!("{other:?}"),
                None => return,
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    client.close().await;
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn the_command_line_is_consistent() {
        Args::command().debug_assert();
    }
}

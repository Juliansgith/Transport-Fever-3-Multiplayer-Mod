//! What a launcher front end shows ([`State`]) and what it asks for
//! ([`Action`]): the web page as JSON, the native window as Rust values.

use serde::{Deserialize, Serialize};
use tpf3mp_proto::{
    Arch, ContentDiff, FixedBytes, ModRef, Os, Platform, PlayerId, RoomPhase, RulesOffer,
};

use crate::bridge::{Status, WorldStatus};

/// What the launcher itself knows, next to the session's [`Status`].
#[derive(Debug, Default)]
pub(crate) struct View {
    pub(crate) server: Option<String>,
    pub(crate) name: String,
    pub(crate) player: Option<PlayerId>,
    pub(crate) connecting: bool,
    pub(crate) connected: bool,
    /// The connection runs through a tunnel, not over UDP.
    pub(crate) tunneled: bool,
    pub(crate) server_version: Option<String>,
    /// The server's name for the connection outside a room, which its log
    /// uses.
    pub(crate) session: Option<String>,
    /// The rules the server offers new rooms, the default first.
    pub(crate) rules: Vec<RulesOffer>,
    pub(crate) in_room: bool,
    pub(crate) invite: Option<String>,
    pub(crate) error: Option<String>,
}

/// Something the player asks for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    Connect {
        server: String,
        name: String,
    },
    Disconnect,
    Create {
        room: String,
        max_players: u8,
        password: Option<String>,
        /// One of the server's rules; the default without.
        #[serde(default)]
        rules: Option<String>,
    },
    Join {
        invite: String,
        password: Option<String>,
    },
    Ready {
        ready: bool,
    },
    Start,
    Speed {
        percent: u16,
    },
    Kick {
        player: String,
    },
    Chat {
        text: String,
    },
    Leave,
}

/// Everything a launcher front end shows: the web page reads it as JSON,
/// the native window as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct State {
    pub name: String,
    /// This player's short ID, as others see it.
    pub player: Option<String>,
    pub server: Option<String>,
    pub server_version: Option<String>,
    /// What the player quotes to the server's operator: the connection's
    /// name in the server's log.
    pub support_id: Option<String>,
    /// The rules the server offers new rooms, the default first.
    pub rules: Vec<RulesChoice>,
    pub connection: Connection,
    /// The connection runs through a tunnel, not over UDP.
    pub tunneled: bool,
    /// What went wrong last, until something succeeds.
    pub error: Option<String>,
    pub room: Option<Room>,
    /// How this player's game differs from the room's, while it does.
    pub content_diff: Option<Differences>,
    pub game: Game,
    pub chat: Vec<ChatLine>,
    /// What the player should know, oldest first.
    pub notices: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    #[default]
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RulesChoice {
    pub name: String,
    pub description: String,
}

/// How this player's game differs from the room's, ready to show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Differences {
    /// All of it in a sentence.
    pub summary: String,
    /// The room's build and this player's, when they differ.
    pub game: Option<(String, String)>,
    /// Mods the room runs and this player does not, as "name version".
    pub missing: Vec<String>,
    /// How many more there are than `missing` names.
    pub missing_more: u32,
    /// Mods this player runs and the room does not.
    pub extra: Vec<String>,
    pub extra_more: u32,
    /// Mod, the room's version, this player's.
    pub changed: Vec<(String, String, String)>,
    pub changed_more: u32,
    /// The same mods, loaded in another order.
    pub reordered: bool,
    /// The mods beyond the listed ones differ.
    pub unlisted: bool,
}

impl Differences {
    fn of(diff: &ContentDiff) -> Self {
        let named = |mods: &[ModRef]| -> Vec<String> {
            mods.iter()
                .map(|listed| format!("{} {}", listed.id, listed.version))
                .collect()
        };
        let more = |total: u32, listed: usize| {
            total.saturating_sub(u32::try_from(listed).unwrap_or(u32::MAX))
        };
        Self {
            summary: diff.to_string(),
            game: diff
                .game
                .as_ref()
                .map(|builds| (builds.room.to_string(), builds.yours.to_string())),
            missing: named(&diff.missing),
            missing_more: more(diff.missing_total, diff.missing.len()),
            extra: named(&diff.extra),
            extra_more: more(diff.extra_total, diff.extra.len()),
            changed: diff
                .changed
                .iter()
                .map(|change| {
                    (
                        change.id.to_string(),
                        change.room.to_string(),
                        change.yours.to_string(),
                    )
                })
                .collect(),
            changed_more: more(diff.changed_total, diff.changed.len()),
            reordered: diff.reordered,
            unlisted: diff.unlisted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Room {
    pub name: String,
    pub rules: String,
    pub phase: Phase,
    /// What to send friends: the server and the room's invite.
    pub invite: Option<String>,
    pub you_own: bool,
    pub max_players: u8,
    pub has_password: bool,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Lobby,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Member {
    /// The player's full key, which [`Action::Kick`] takes.
    pub id: String,
    pub name: String,
    pub platform: String,
    pub ready: bool,
    pub connected: bool,
    pub owner: bool,
    pub you: bool,
    /// Whether this member's game matches the owner's.
    pub content: MemberContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberContent {
    Same,
    Differs,
    /// The member or the owner has not declared theirs.
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Game {
    /// The game's build, once its hook attached.
    pub attached: Option<String>,
    pub world: World,
    /// While fetching: bytes received of `total`.
    pub bytes: u64,
    pub total: u64,
    /// The last step the game ran.
    pub step: Option<u64>,
    /// The room's speed in percent; 0 is paused.
    pub speed: u16,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum World {
    #[default]
    None,
    Fetching,
    Loading,
    Playing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatLine {
    pub from: String,
    pub text: String,
    pub you: bool,
}

/// What the launcher shows now.
pub(crate) fn snapshot(view: &View, status: &Status) -> State {
    let you = view.player;
    let connection = if view.connected {
        Connection::Connected
    } else if view.connecting {
        Connection::Connecting
    } else {
        Connection::Disconnected
    };
    let room = status.room.as_ref().filter(|_| view.in_room).map(|room| {
        let owners = room
            .members
            .iter()
            .find(|member| member.player == room.owner)
            .and_then(|owner| owner.content);
        Room {
            name: room.name.as_str().to_owned(),
            rules: room.rules.as_str().to_owned(),
            phase: match room.phase {
                RoomPhase::Lobby => Phase::Lobby,
                RoomPhase::Running => Phase::Running,
            },
            invite: view.invite.clone(),
            you_own: Some(room.owner) == you,
            max_players: room.max_players,
            has_password: room.has_password,
            members: room
                .members
                .iter()
                .map(|member| Member {
                    id: player_hex(&member.player),
                    name: member.name.as_str().to_owned(),
                    platform: platform_name(member.platform),
                    ready: member.ready,
                    connected: member.connected,
                    owner: member.player == room.owner,
                    you: Some(member.player) == you,
                    content: match (owners, member.content) {
                        (Some(owners), Some(theirs)) if owners == theirs => MemberContent::Same,
                        (Some(_), Some(_)) => MemberContent::Differs,
                        _ => MemberContent::Unknown,
                    },
                })
                .collect(),
        }
    });
    let (world, bytes, total) = match status.world {
        WorldStatus::None => (World::None, 0, 0),
        WorldStatus::Fetching { bytes, total } => (World::Fetching, bytes, total),
        WorldStatus::Loading => (World::Loading, 0, 0),
        WorldStatus::Playing => (World::Playing, 0, 0),
    };
    let name_of = |player: &PlayerId| {
        status
            .room
            .as_ref()
            .and_then(|room| room.members.iter().find(|member| member.player == *player))
            .map_or_else(
                || player.to_string(),
                |member| member.name.as_str().to_owned(),
            )
    };
    State {
        name: view.name.clone(),
        player: you.map(|player| player.to_string()),
        server: view.server.clone(),
        server_version: view.server_version.clone(),
        support_id: status
            .session
            .map(|session| session.to_string())
            .or_else(|| view.session.clone())
            .filter(|_| view.connected || view.in_room),
        rules: view
            .rules
            .iter()
            .map(|offer| RulesChoice {
                name: offer.name.as_str().to_owned(),
                description: offer.description.as_str().to_owned(),
            })
            .collect(),
        connection,
        tunneled: view.connected && view.tunneled,
        error: view.error.clone(),
        room,
        content_diff: status.content_diff.as_ref().map(Differences::of),
        game: Game {
            attached: status.game.clone(),
            world,
            bytes,
            total,
            step: status.step,
            speed: status.speed.0,
        },
        chat: status
            .chat
            .iter()
            .map(|(from, text)| ChatLine {
                from: name_of(from),
                text: text.as_str().to_owned(),
                you: Some(*from) == you,
            })
            .collect(),
        notices: status.notices.iter().cloned().collect(),
    }
}

/// The state the page shows, as JSON.
pub(crate) fn render(view: &View, status: &Status) -> String {
    serde_json::to_string(&snapshot(view, status)).unwrap_or_else(|_| "{}".to_owned())
}

/// A player's full key as 64 hex digits, as the page names players.
pub(crate) fn player_hex(player: &PlayerId) -> String {
    player
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A player named by [`player_hex`].
pub(crate) fn parse_player(text: &str) -> Option<PlayerId> {
    let text = text.trim().trim_start_matches("p-");
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(PlayerId(FixedBytes(bytes)))
}

fn platform_name(platform: Platform) -> String {
    let os = match platform.os {
        Os::Windows => "Windows",
        Os::Linux => "Linux",
        Os::MacOs => "macOS",
        Os::Other => "other",
    };
    let arch = match platform.arch {
        Arch::X86_64 => "x86-64",
        Arch::Aarch64 => "arm64",
        Arch::Other => "other",
    };
    format!("{os} {arch}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_parse_from_the_pages_json() {
        let action: Action =
            serde_json::from_str(r#"{"action":"join","invite":"TPF3MP1.x","password":null}"#)
                .unwrap();
        assert_eq!(
            action,
            Action::Join {
                invite: "TPF3MP1.x".into(),
                password: None
            }
        );
        let action: Action = serde_json::from_str(
            r#"{"action":"create","room":"R","max_players":4,"password":null,"rules":"native"}"#,
        )
        .unwrap();
        assert_eq!(
            action,
            Action::Create {
                room: "R".into(),
                max_players: 4,
                password: None,
                rules: Some("native".into()),
            }
        );
        let action: Action = serde_json::from_str(r#"{"action":"start"}"#).unwrap();
        assert_eq!(action, Action::Start);
        assert!(serde_json::from_str::<Action>(r#"{"action":"format_disk"}"#).is_err());
    }

    #[test]
    fn players_round_trip_through_their_page_names() {
        let player = PlayerId(FixedBytes([0xab; 32]));
        let hex = player_hex(&player);
        assert_eq!(hex.len(), 64);
        assert_eq!(parse_player(&hex), Some(player));
        assert_eq!(parse_player(&format!("p-{hex}")), Some(player));
        assert_eq!(parse_player("abc"), None);
        assert_eq!(parse_player(&"zz".repeat(32)), None);
    }

    #[test]
    fn the_state_says_which_mods_differ() {
        let mods = |list: &[&str]| {
            tpf3mp_proto::ContentManifest::new(
                tpf3mp_proto::Text::new("35924").unwrap(),
                list.iter()
                    .map(|id| ModRef {
                        id: tpf3mp_proto::Text::new(*id).unwrap(),
                        version: tpf3mp_proto::Text::new("1").unwrap(),
                    })
                    .collect(),
            )
        };
        let status = Status {
            content_diff: mods(&["trains", "stations"]).compare(&mods(&["trains", "trees"])),
            ..Status::default()
        };
        let json: serde_json::Value =
            serde_json::from_str(&render(&View::default(), &status)).unwrap();
        let diff = &json["content_diff"];
        assert_eq!(diff["missing"], serde_json::json!(["stations 1"]));
        assert_eq!(diff["extra"], serde_json::json!(["trees 1"]));
        assert_eq!(
            diff["summary"],
            "you lack stations 1; the room lacks trees 1"
        );
        let json: serde_json::Value =
            serde_json::from_str(&render(&View::default(), &Status::default())).unwrap();
        assert!(json["content_diff"].is_null());
    }

    #[test]
    fn the_state_names_the_connection_and_the_room() {
        let view = View {
            name: "Ann".into(),
            connected: true,
            ..View::default()
        };
        let json: serde_json::Value =
            serde_json::from_str(&render(&view, &Status::default())).unwrap();
        assert_eq!(json["connection"], "connected");
        assert_eq!(json["tunneled"], false);
        assert_eq!(json["name"], "Ann");
        assert!(json["room"].is_null());
        assert_eq!(json["game"]["world"], "none");
    }

    #[test]
    fn the_support_id_names_the_current_connection() {
        let mut view = View {
            connected: true,
            session: Some("s-11".into()),
            ..View::default()
        };
        let support_id = |view: &View, status: &Status| {
            let json: serde_json::Value = serde_json::from_str(&render(view, status)).unwrap();
            json["support_id"].as_str().map(str::to_owned)
        };
        assert_eq!(
            support_id(&view, &Status::default()).as_deref(),
            Some("s-11")
        );
        // In a room, the room's connection, which a rejoin renews.
        let status = Status {
            session: Some(tpf3mp_proto::SessionId([0x22; 16])),
            ..Status::default()
        };
        assert_eq!(
            support_id(&view, &status).as_deref(),
            Some("s-22222222222222222222222222222222")
        );
        // Disconnected, there is none to quote.
        view.connected = false;
        assert_eq!(support_id(&view, &Status::default()), None);
    }
}

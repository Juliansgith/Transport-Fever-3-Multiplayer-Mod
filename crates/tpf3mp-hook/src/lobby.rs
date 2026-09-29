//! The lobby as the in-game menu sees it: the state it shows and the actions
//! it sends, and the forms they take on the way to and from Lua.
//!
//! The menu's Lua asks the hook for the state and hands it actions over the
//! request channel in [`crate::menu`]. The state crosses as a Lua table
//! literal ([`LobbyState::to_lua`]), which the Lua side evaluates with `load`
//! in an empty environment; an action crosses as one small JSON object
//! ([`parse_action`]), which the Lua side builds by hand. Both are plain text
//! a person can read in a log.
//!
//! Until the agent carries the room (the next step, `docs/LOBBY.md`), the hook
//! answers on its own: [`LobbyState::apply_local`] echoes each action into the
//! state, so the whole window can be used and seen to work from the menu.

use serde::Deserialize;

/// Whether the launcher's connection to the server is up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Connection {
    #[default]
    Disconnected,
    Connecting,
    Connected,
}

impl Connection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
        }
    }
}

/// One player in the room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub id: u32,
    pub name: String,
    pub ready: bool,
    pub owner: bool,
    pub you: bool,
    pub connected: bool,
    /// Whether the player's world matches: `same`, `differs` or `unknown`.
    pub content: String,
}

/// The room the player is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Room {
    pub name: String,
    pub invite: String,
    /// `lobby` or `playing`.
    pub phase: String,
    pub you_own: bool,
    pub max_players: u32,
    pub has_password: bool,
    pub members: Vec<Member>,
}

/// One line of the room's chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLine {
    pub from: String,
    pub text: String,
    pub you: bool,
}

/// Everything the lobby window shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LobbyState {
    pub connection: Connection,
    pub server: String,
    pub name: String,
    /// The last thing that went wrong, for the window to show.
    pub error: Option<String>,
    /// The last thing worth telling the player.
    pub notice: Option<String>,
    pub room: Option<Room>,
    pub chat: Vec<ChatLine>,
    /// True while the hook answers on its own, without an agent.
    pub preview: bool,
}

/// What the player asked for in the window. The JSON form is the tag
/// `action` plus the fields, e.g. `{"action":"connect","server":"…","name":"…"}`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum LobbyAction {
    Connect {
        server: String,
        name: String,
    },
    Disconnect,
    Create {
        room: String,
        #[serde(default = "default_max_players")]
        max_players: u32,
        #[serde(default)]
        password: String,
    },
    Join {
        invite: String,
        #[serde(default)]
        password: String,
    },
    Ready {
        ready: bool,
    },
    Start,
    Kick {
        player: u32,
    },
    Chat {
        text: String,
    },
    Leave,
}

fn default_max_players() -> u32 {
    8
}

/// Parses one action from the window's JSON.
pub fn parse_action(json: &str) -> Result<LobbyAction, String> {
    serde_json::from_str(json).map_err(|error| format!("not an action: {error}"))
}

/// A Lua string literal for `text`, safe for any bytes.
fn lua_str(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for byte in text.bytes() {
        match byte {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0..=0x1f | 0x7f => out.push_str(&format!("\\{byte}")),
            other => out.push(other as char),
        }
    }
    out.push('"');
    out
}

fn lua_opt(text: Option<&str>) -> String {
    text.map(lua_str).unwrap_or_else(|| "nil".to_owned())
}

impl LobbyState {
    /// The empty lobby: not connected, no room, nothing said. Usable in a
    /// `static`.
    pub const fn new() -> Self {
        Self {
            connection: Connection::Disconnected,
            server: String::new(),
            name: String::new(),
            error: None,
            notice: None,
            room: None,
            chat: Vec::new(),
            preview: false,
        }
    }

    /// The state as a Lua table literal: `{ connection = "…", … }`.
    pub fn to_lua(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push_str("{ connection = ");
        out.push_str(lua_str(self.connection.as_str()).as_str());
        out.push_str(", server = ");
        out.push_str(&lua_str(&self.server));
        out.push_str(", name = ");
        out.push_str(&lua_str(&self.name));
        out.push_str(", error = ");
        out.push_str(&lua_opt(self.error.as_deref()));
        out.push_str(", notice = ");
        out.push_str(&lua_opt(self.notice.as_deref()));
        out.push_str(", preview = ");
        out.push_str(if self.preview { "true" } else { "false" });
        out.push_str(", chat = {");
        for line in &self.chat {
            out.push_str(&format!(
                " {{ from = {}, text = {}, you = {} }},",
                lua_str(&line.from),
                lua_str(&line.text),
                line.you
            ));
        }
        out.push_str(" }");
        match &self.room {
            None => out.push_str(", room = nil"),
            Some(room) => {
                out.push_str(&format!(
                    ", room = {{ name = {}, invite = {}, phase = {}, you_own = {}, max_players = {}, has_password = {}, members = {{",
                    lua_str(&room.name),
                    lua_str(&room.invite),
                    lua_str(&room.phase),
                    room.you_own,
                    room.max_players,
                    room.has_password
                ));
                for member in &room.members {
                    out.push_str(&format!(
                        " {{ id = {}, name = {}, ready = {}, owner = {}, you = {}, connected = {}, content = {} }},",
                        member.id,
                        lua_str(&member.name),
                        member.ready,
                        member.owner,
                        member.you,
                        member.connected,
                        lua_str(&member.content)
                    ));
                }
                out.push_str(" } }");
            }
        }
        out.push_str(" }");
        out
    }

    /// Echoes an action into the state, standing in for the agent until it
    /// carries the room. Marks the state as a preview.
    pub fn apply_local(&mut self, action: &LobbyAction) {
        self.preview = true;
        self.error = None;
        match action {
            LobbyAction::Connect { server, name } => {
                if server.trim().is_empty() || name.trim().is_empty() {
                    self.error = Some("give a server and a name".to_owned());
                    return;
                }
                self.connection = Connection::Connected;
                self.server = server.trim().to_owned();
                self.name = name.trim().to_owned();
                self.notice = Some(format!(
                    "connected to {} (preview: no agent yet)",
                    self.server
                ));
            }
            LobbyAction::Disconnect => {
                self.connection = Connection::Disconnected;
                self.room = None;
                self.chat.clear();
                self.notice = Some("disconnected".to_owned());
            }
            LobbyAction::Create {
                room,
                max_players,
                password,
            } => {
                if self.connection != Connection::Connected {
                    self.error = Some("connect first".to_owned());
                    return;
                }
                self.room = Some(Room {
                    name: if room.trim().is_empty() {
                        format!("{}'s room", self.name)
                    } else {
                        room.trim().to_owned()
                    },
                    invite: "PREVIEW-2026".to_owned(),
                    phase: "lobby".to_owned(),
                    you_own: true,
                    max_players: (*max_players).clamp(2, 32),
                    has_password: !password.is_empty(),
                    members: vec![Member {
                        id: 1,
                        name: self.name.clone(),
                        ready: false,
                        owner: true,
                        you: true,
                        connected: true,
                        content: "same".to_owned(),
                    }],
                });
                self.chat.clear();
                self.notice = Some("room created (preview)".to_owned());
            }
            LobbyAction::Join { invite, .. } => {
                if self.connection != Connection::Connected {
                    self.error = Some("connect first".to_owned());
                    return;
                }
                if invite.trim().is_empty() {
                    self.error = Some("give an invite code".to_owned());
                    return;
                }
                self.room = Some(Room {
                    name: format!("room {}", invite.trim()),
                    invite: invite.trim().to_owned(),
                    phase: "lobby".to_owned(),
                    you_own: false,
                    max_players: 8,
                    has_password: false,
                    members: vec![
                        Member {
                            id: 1,
                            name: "Host".to_owned(),
                            ready: true,
                            owner: true,
                            you: false,
                            connected: true,
                            content: "same".to_owned(),
                        },
                        Member {
                            id: 2,
                            name: self.name.clone(),
                            ready: false,
                            owner: false,
                            you: true,
                            connected: true,
                            content: "same".to_owned(),
                        },
                    ],
                });
                self.chat.clear();
                self.notice = Some("joined (preview)".to_owned());
            }
            LobbyAction::Ready { ready } => {
                if let Some(room) = &mut self.room {
                    for member in &mut room.members {
                        if member.you {
                            member.ready = *ready;
                        }
                    }
                }
            }
            LobbyAction::Start => {
                self.notice = Some("start: the agent loads the room's world here".to_owned());
            }
            LobbyAction::Kick { player } => {
                if let Some(room) = &mut self.room {
                    room.members
                        .retain(|member| member.id != *player || member.you);
                }
            }
            LobbyAction::Chat { text } => {
                if text.trim().is_empty() {
                    return;
                }
                self.chat.push(ChatLine {
                    from: self.name.clone(),
                    text: text.trim().to_owned(),
                    you: true,
                });
                if self.chat.len() > 50 {
                    self.chat.remove(0);
                }
            }
            LobbyAction::Leave => {
                self.room = None;
                self.chat.clear();
                self.notice = Some("left the room".to_owned());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_parse_from_the_windows_json() {
        assert_eq!(
            parse_action(r#"{"action":"connect","server":"play.example:4433","name":"Ada"}"#),
            Ok(LobbyAction::Connect {
                server: "play.example:4433".into(),
                name: "Ada".into()
            })
        );
        assert_eq!(
            parse_action(r#"{"action":"create","room":"Alps"}"#),
            Ok(LobbyAction::Create {
                room: "Alps".into(),
                max_players: 8,
                password: String::new()
            })
        );
        assert_eq!(
            parse_action(r#"{"action":"leave"}"#),
            Ok(LobbyAction::Leave)
        );
        assert!(parse_action("{\"action\":\"fly\"}").is_err());
    }

    #[test]
    fn the_lua_literal_quotes_every_string() {
        let state = LobbyState {
            name: "A\"b\\c\nd".into(),
            ..LobbyState::default()
        };
        let lua = state.to_lua();
        assert!(lua.contains(r#"name = "A\"b\\c\nd""#), "{lua}");
        assert!(lua.starts_with("{ connection = \"disconnected\""));
        assert!(lua.contains("room = nil"));
        assert!(lua.ends_with(" }"));
    }

    #[test]
    fn the_local_echo_walks_a_room_through_its_life() {
        let mut state = LobbyState::default();
        state.apply_local(&LobbyAction::Create {
            room: "x".into(),
            max_players: 4,
            password: String::new(),
        });
        assert_eq!(state.error.as_deref(), Some("connect first"));

        state.apply_local(&LobbyAction::Connect {
            server: "s".into(),
            name: "Ada".into(),
        });
        assert_eq!(state.connection, Connection::Connected);
        state.apply_local(&LobbyAction::Create {
            room: "Alps".into(),
            max_players: 4,
            password: "pw".into(),
        });
        let room = state.room.as_ref().expect("a room");
        assert!(room.you_own && room.has_password && room.members[0].you);
        assert!(state.to_lua().contains("invite = \"PREVIEW-2026\""));

        state.apply_local(&LobbyAction::Ready { ready: true });
        assert!(state.room.as_ref().expect("room").members[0].ready);
        state.apply_local(&LobbyAction::Chat {
            text: " hi ".into(),
        });
        assert_eq!(state.chat[0].text, "hi");
        state.apply_local(&LobbyAction::Leave);
        assert!(state.room.is_none() && state.chat.is_empty());
        assert!(state.preview);
    }
}

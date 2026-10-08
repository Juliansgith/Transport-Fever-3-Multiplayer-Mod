//! The servers a release vouches for, and the launcher's look at each
//! (docs/DECISIONS.md, D12's PROPOSED amendment of 2026-10-06: several
//! operated servers, the room list from all, rooms hosted on the closest).
//!
//! A release names its servers when it is built: the default one
//! (`TPF3MP_DEFAULT_SERVER`, named by `TPF3MP_SERVER_NAME`) first, then
//! the others (`TPF3MP_SERVERS`, `NAME=host:port` separated by commas).
//! Players never type one, and an invite may only name a server on that
//! list. With one server, or `--server`, or a server of the player's own
//! in Settings, the launcher plays on that one alone, as before.
//!
//! While the launcher is connected on the list, it keeps a quiet
//! connection, a *lookout*, to every other listed server: it asks each for
//! its rooms as it asks its own, and its round trip is the server's ping.
//! A lookout declares nothing, joins nothing and sends no diagnostics.

use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use tokio::{
    sync::{oneshot, watch},
    task::JoinHandle,
};
use tpf3mp_net::{Identity, ServerTrust};
use tpf3mp_proto::{RoomPage, Text};
use tracing::{debug, info};

use super::api::{PublicRoom, RoomList, ServerRow};
use crate::{ClientEvent, ConnectOptions, Requests, TunnelChoice, connect};

/// Longest name a listed server may have, such as `EU`, as players see it.
pub const MAX_NAME: usize = 24;
/// Most servers a release may list.
pub const MAX_SERVERS: usize = 8;
/// Pings this close to the lowest count as equally close: the server
/// played on then stays, else the one listed first wins. Without a margin,
/// two servers in one data centre would swap at random.
pub const TIE_MARGIN: Duration = Duration::from_millis(10);
/// How long a lookout's connection may take before its server counts as
/// unreachable for now.
pub const PROBE_WAIT: Duration = Duration::from_secs(5);
/// How long a lookout whose server did not answer waits before trying
/// again.
const RETRY_AFTER: Duration = Duration::from_secs(30);
/// How often a lookout reads its connection's round trip again.
const PING_EVERY: Duration = Duration::from_secs(2);
/// How long a lookout's server has to send its room list.
pub const LIST_WAIT: Duration = Duration::from_secs(3);

/// One server a release vouches for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedServer {
    /// What players see of it, such as `EU`.
    pub name: String,
    /// Its `host:port`.
    pub address: String,
}

/// Whether `name` may name a listed server: 1 to [`MAX_NAME`] letters,
/// digits, spaces, dots, dashes and underscores, as the release workflow
/// checks `TPF3MP_SERVER_NAME`.
fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_'))
}

/// The servers `text` lists, as `TPF3MP_SERVERS` and `--more-servers` give
/// them: `NAME=host:port` entries separated by commas, such as
/// `EU=eu.example.org:29470,US=us.example.org:29470`. Empty lists none.
/// Fails closed: one entry that does not read refuses the whole list, and
/// so does a name or an address listed twice.
pub fn parse_list(text: &str) -> Result<Vec<ListedServer>, String> {
    let mut listed: Vec<ListedServer> = Vec::new();
    for entry in text.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let (name, address) = entry
            .split_once('=')
            .ok_or_else(|| format!("the server {entry} is not NAME=host:port"))?;
        let name = name.trim();
        if !name_ok(name) {
            return Err(format!(
                "the server name {name:?} must be 1 to {MAX_NAME} letters, digits, spaces, dots, dashes or underscores"
            ));
        }
        let address = super::server_address(address)?;
        if listed
            .iter()
            .any(|other| other.name.eq_ignore_ascii_case(name))
        {
            return Err(format!("the server name {name} is listed twice"));
        }
        if listed
            .iter()
            .any(|other| super::same_server(&other.address, &address))
        {
            return Err(format!("the server {address} is listed twice"));
        }
        listed.push(ListedServer {
            name: name.to_owned(),
            address,
        });
    }
    if listed.len() > MAX_SERVERS {
        return Err(format!("a release lists at most {MAX_SERVERS} servers"));
    }
    Ok(listed)
}

/// A release's servers: its `default` one first, called `default_name`
/// (else its host), then `more`. A bad default, duplicate name or address,
/// or more than [`MAX_SERVERS`] in total refuses the release list. Without
/// a default, none: the player types a server, as a developer's build
/// without one did.
pub fn release_list(
    default: Option<&str>,
    default_name: Option<&str>,
    more: &[ListedServer],
) -> Result<Vec<ListedServer>, String> {
    let Some(default) = default.map(str::trim).filter(|d| !d.is_empty()) else {
        if more.is_empty() {
            return Ok(Vec::new());
        }
        return Err("the release's other servers need a default server".into());
    };
    let address = super::server_address(default)?;
    let name = match default_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) if name_ok(name) => name.to_owned(),
        Some(name) => {
            return Err(format!(
                "the default server name {name:?} must be 1 to {MAX_NAME} letters, digits, spaces, dots, dashes or underscores"
            ));
        }
        None => address
            .rsplit_once(':')
            .map_or(address.as_str(), |(host, _)| host)
            .to_owned(),
    };
    let mut listed = vec![ListedServer { name, address }];
    for server in more {
        if listed
            .iter()
            .any(|before| before.name.eq_ignore_ascii_case(&server.name))
        {
            return Err(format!("the server name {} is listed twice", server.name));
        }
        if listed
            .iter()
            .any(|before| super::same_server(&before.address, &server.address))
        {
            return Err(format!("the server {} is listed twice", server.address));
        }
        listed.push(server.clone());
    }
    if listed.len() > MAX_SERVERS {
        return Err(format!("a release lists at most {MAX_SERVERS} servers"));
    }
    Ok(listed)
}

/// The listed server at `address`, if it is one.
pub fn find<'a>(servers: &'a [ListedServer], address: &str) -> Option<&'a ListedServer> {
    servers
        .iter()
        .find(|server| super::same_server(&server.address, address))
}

/// The listed server called `name`, ignoring ASCII case.
pub fn find_name<'a>(servers: &'a [ListedServer], name: &str) -> Option<&'a ListedServer> {
    servers
        .iter()
        .find(|server| server.name.eq_ignore_ascii_case(name))
}

/// A room card's selected server, or the unique server the last public room
/// page showed for `invite`. A typed invite with no unique public listing
/// stays unresolved for the launcher's existing invite path.
pub fn room_target(
    servers: &[ListedServer],
    room_servers: &[(String, String)],
    invite: &str,
    selected: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(name) = selected {
        return find_name(servers, name)
            .map(|server| Some(server.address.clone()))
            .ok_or_else(|| not_listed(name));
    }
    let mut targets = Vec::new();
    for (_, address) in room_servers.iter().filter(|(listed, _)| listed == invite) {
        if !targets
            .iter()
            .any(|known: &String| super::same_server(known, address))
        {
            targets.push(address.clone());
        }
    }
    match targets.as_slice() {
        [] => Ok(None),
        [server] => Ok(Some(server.clone())),
        _ => Err("that invite is listed on more than one server; choose its room card".into()),
    }
}

/// Where Connect goes on a launcher playing on its release's servers, for
/// what the player gave it (`typed`, and the invite in it): `Ok(Some)` a
/// listed server the text names, `Ok(None)` the closest. A server that is
/// not on the list is refused, as an invite naming it: no message sends a
/// player to a server the release does not vouch for (D12).
pub fn connect_target(
    servers: &[ListedServer],
    typed: &str,
    invite_server: Option<&str>,
    invite: bool,
) -> Result<Option<String>, String> {
    let named = match invite_server {
        Some(server) => Some(server),
        // A bare server, without an invite, as a playtest's auto room
        // passes it; anything else that is no invite is refused.
        None if !invite && !typed.trim().is_empty() => Some(typed.trim()),
        None => None,
    };
    match named {
        None => Ok(None),
        Some(named) => match find(servers, named) {
            Some(listed) => Ok(Some(listed.address.clone())),
            None if invite => Err(not_listed(named)),
            None => Err("that is not an invite".into()),
        },
    }
}

/// Why an invite naming `other` is refused by a launcher on its release's
/// servers.
pub fn not_listed(other: &str) -> String {
    format!(
        "that invite is for {other}, a server TPF3-MP does not vouch for: invites only join rooms on its own servers"
    )
}

/// Which of `pings` (in list order; `None` unreachable) is the closest
/// server: the lowest ping, where pings within [`TIE_MARGIN`] of it count
/// as equal, the `current` server among those staying, else the one listed
/// first. `None` when none answers.
pub fn fastest(pings: &[Option<Duration>], current: Option<usize>) -> Option<usize> {
    let best = pings.iter().flatten().min()?;
    let near = |at: usize| {
        pings
            .get(at)
            .copied()
            .flatten()
            .is_some_and(|ping| ping <= *best + TIE_MARGIN)
    };
    match current {
        Some(current) if near(current) => Some(current),
        _ => (0..pings.len()).find(|&at| near(at)),
    }
}

/// A ping as players read it, in whole milliseconds, at least 1.
pub fn millis(ping: Duration) -> u32 {
    u32::try_from(ping.as_millis().max(1)).unwrap_or(u32::MAX)
}

/// One server's page of rooms, to merge.
#[derive(Clone, Copy)]
pub struct Page<'a> {
    pub server: &'a ListedServer,
    pub ping: Option<Duration>,
    /// The server-local page this entry came from; several consecutive
    /// pages are merged to build a global page.
    pub page_index: u16,
    pub page: &'a RoomPage,
}

/// The rooms of every server's page, in one list: each room labelled with
/// its server's name and ping, rooms in their lobby first, then the fuller,
/// then by name, as a server sorts its own. It takes the requested global
/// slice after sorting all fetched pages, and callers provide each local
/// page from zero through `page`. `more` means the global slice has another
/// room behind it or a server's requested local page has another page.
/// Also returns each displayed room's server, by invite, so a join goes
/// where the room is.
pub fn merge(page: u16, pages: &[Page<'_>]) -> (RoomList, Vec<(String, String)>) {
    let mut rooms: Vec<(usize, PublicRoom)> = Vec::new();
    for (order, listed) in pages.iter().enumerate() {
        for room in RoomList::of(listed.page).rooms {
            rooms.push((
                order,
                PublicRoom {
                    server: Some(listed.server.name.clone()),
                    ping_ms: listed.ping.map(millis),
                    ..room
                },
            ));
        }
    }
    rooms.sort_by(|(a_order, a), (b_order, b)| {
        a.running
            .cmp(&b.running)
            .then(b.players.cmp(&a.players))
            .then_with(|| a.name.cmp(&b.name))
            .then(a_order.cmp(b_order))
    });
    let start = usize::from(page).saturating_mul(tpf3mp_proto::ROOMS_PER_PAGE);
    let end = start.saturating_add(tpf3mp_proto::ROOMS_PER_PAGE);
    let more = rooms.len() > end
        || pages
            .iter()
            .any(|listed| listed.page_index == page && listed.page.more);
    let selected: Vec<_> = rooms
        .into_iter()
        .skip(start)
        .take(tpf3mp_proto::ROOMS_PER_PAGE)
        .collect();
    let places = selected
        .iter()
        .map(|(order, room)| (room.invite.clone(), pages[*order].server.address.clone()))
        .collect();
    let list = RoomList {
        page,
        more,
        rooms: selected.into_iter().map(|(_, room)| room).collect(),
    };
    (list, places)
}

/// What a lookout knows of its server.
#[derive(Debug, Clone)]
pub enum Seen {
    /// Connecting, for the first time or again.
    Connecting,
    /// Connected: requests go this way, and the latest round trip.
    Up { requests: Requests, ping: Duration },
    /// It did not answer, or the connection went; tried again later.
    Down,
}

/// Waits up to `timeout` for every lookout to stop connecting. Lookouts run
/// concurrently, so this is one bound for the whole trusted server list,
/// rather than one bound per region.
async fn wait_until_settled(mut lookouts: Vec<watch::Receiver<Seen>>, timeout: Duration) -> bool {
    tokio::time::timeout(timeout, async move {
        for mut watched in lookouts.drain(..) {
            if matches!(*watched.borrow(), Seen::Connecting)
                && watched
                    .wait_for(|seen| !matches!(seen, Seen::Connecting))
                    .await
                    .is_err()
            {
                return false;
            }
        }
        true
    })
    .await
    .unwrap_or(false)
}

/// How lookouts connect: as the player, quietly.
#[derive(Clone)]
pub struct Quiet {
    pub trust: ServerTrust,
    pub identity: Arc<Identity>,
    pub tunnel: TunnelChoice,
    pub name: Text<32>,
}

impl Quiet {
    async fn options(&self, server: &str) -> Result<ConnectOptions, String> {
        super::base_options(
            &self.trust,
            &self.identity,
            &self.tunnel,
            server,
            self.name.clone(),
        )
        .await
    }
}

/// A quiet connection to one listed server, kept open and opened again
/// when it goes, for as long as it is held.
struct Lookout {
    server: ListedServer,
    seen: watch::Receiver<Seen>,
    task: JoinHandle<()>,
    stop: Option<oneshot::Sender<()>>,
}

impl Drop for Lookout {
    fn drop(&mut self) {
        // The connection goes with the task.
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.task.abort();
    }
}

impl Lookout {
    fn start(server: ListedServer, quiet: Quiet) -> Self {
        let (tell, seen) = watch::channel(Seen::Connecting);
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(look(server.clone(), quiet, tell, stopped));
        Self {
            server,
            seen,
            task,
            stop: Some(stop),
        }
    }
}

async fn look(
    server: ListedServer,
    quiet: Quiet,
    tell: watch::Sender<Seen>,
    mut stop: oneshot::Receiver<()>,
) {
    loop {
        tell.send_replace(Seen::Connecting);
        let opened = tokio::select! {
            _ = &mut stop => return,
            opened = async {
                match quiet.options(&server.address).await {
                    Ok(options) => match tokio::time::timeout(PROBE_WAIT, connect(options)).await {
                        Ok(Ok(opened)) => Ok(opened),
                        Ok(Err(error)) => Err(error.for_player()),
                        Err(_) => Err("no answer in time".to_owned()),
                    },
                    Err(error) => Err(error),
                }
            } => opened,
        };
        match opened {
            Ok((client, mut events)) => {
                info!(
                    server = %server.name,
                    ping_ms = millis(client.rtt()),
                    "a listed server answers"
                );
                tell.send_replace(Seen::Up {
                    requests: client.requests(),
                    ping: client.rtt(),
                });
                let mut every = tokio::time::interval(PING_EVERY);
                let mut stopping = false;
                loop {
                    tokio::select! {
                        _ = &mut stop => {
                            stopping = true;
                            break;
                        }
                        event = events.recv() => match event {
                            None | Some(ClientEvent::Closed(_)) => break,
                            Some(_) => {}
                        },
                        _ = every.tick() => {
                            let now = client.rtt();
                            tell.send_modify(|seen| {
                                if let Seen::Up { ping, .. } = seen {
                                    *ping = now;
                                }
                            });
                        }
                    }
                }
                if stopping {
                    drop(events);
                    client.close().await;
                    return;
                }
                info!(server = %server.name, "a listed server's lookout closed");
            }
            Err(error) => debug!(server = %server.name, %error, "a listed server does not answer"),
        }
        tell.send_replace(Seen::Down);
        tokio::select! {
            _ = &mut stop => return,
            _ = tokio::time::sleep(RETRY_AFTER) => {}
        }
    }
}

/// The lookouts on every listed server but the one played on.
#[derive(Default)]
pub(crate) struct Lookouts {
    watching: Mutex<Vec<Lookout>>,
}

impl Lookouts {
    fn watching(&self) -> std::sync::MutexGuard<'_, Vec<Lookout>> {
        self.watching.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Watches every one of `servers` but `home`: keeps the lookouts there
    /// already, starts the missing ones as `quiet`, and stops the one on
    /// `home`, where the launcher now plays itself.
    pub(crate) fn watch(&self, servers: &[ListedServer], home: Option<&str>, quiet: &Quiet) {
        let mut watching = self.watching();
        watching.retain(|lookout| {
            !home.is_some_and(|home| super::same_server(home, &lookout.server.address))
                && find(servers, &lookout.server.address).is_some()
        });
        for server in servers {
            let here = home.is_some_and(|home| super::same_server(home, &server.address));
            let watched = watching
                .iter()
                .any(|lookout| super::same_server(&lookout.server.address, &server.address));
            if !here && !watched {
                watching.push(Lookout::start(server.clone(), quiet.clone()));
            }
        }
    }

    /// Closes every lookout and waits for its server session to be released.
    pub(crate) async fn stop(&self) {
        let mut lookouts = {
            let mut watching = self.watching();
            std::mem::take(&mut *watching)
        };
        for lookout in &mut lookouts {
            if let Some(stop) = lookout.stop.take() {
                let _ = stop.send(());
            }
        }
        for lookout in &mut lookouts {
            let _ = (&mut lookout.task).await;
        }
    }

    /// Releases `address`'s quiet connection so it can be promoted to the
    /// primary connection without briefly counting twice against the
    /// server's per-address session limit.
    pub(crate) async fn stop_at(&self, address: &str) -> bool {
        let removed = {
            let mut watching = self.watching();
            watching
                .iter()
                .position(|lookout| super::same_server(&lookout.server.address, address))
                .map(|at| watching.remove(at))
        };
        if let Some(mut lookout) = removed {
            if let Some(stop) = lookout.stop.take() {
                let _ = stop.send(());
            }
            // Wait until the quiet Client has closed its connection and the
            // server can release its session before primary promotion.
            let _ = (&mut lookout.task).await;
            true
        } else {
            false
        }
    }

    /// What each lookout knows now, by server address.
    fn seen(&self) -> Vec<(ListedServer, watch::Receiver<Seen>)> {
        self.watching()
            .iter()
            .map(|lookout| (lookout.server.clone(), lookout.seen.clone()))
            .collect()
    }

    /// Waits up to [`PROBE_WAIT`] for every non-home listed server to answer,
    /// then returns one stable set of request handles. A room list must not
    /// silently omit a region that is connecting or down.
    pub(crate) async fn all_up(
        &self,
        servers: &[ListedServer],
        home: &str,
    ) -> Option<Vec<(ListedServer, Requests, Duration)>> {
        let seen = self.seen();
        let mut lookouts = Vec::new();
        for server in servers {
            if super::same_server(home, &server.address) {
                continue;
            }
            let (_, watched) = seen
                .iter()
                .find(|(listed, _)| super::same_server(&listed.address, &server.address))?;
            lookouts.push((server.clone(), watched.clone()));
        }
        let waiting = lookouts
            .iter()
            .map(|(_, watched)| watched.clone())
            .collect();
        if !wait_until_settled(waiting, PROBE_WAIT).await {
            return None;
        }
        lookouts
            .iter()
            .map(|(server, watched)| match watched.borrow().clone() {
                Seen::Up { requests, ping } => Some((server.clone(), requests, ping)),
                Seen::Connecting | Seen::Down => None,
            })
            .collect()
    }

    /// What the lookout on `address` knows now; `None` without one.
    pub(crate) fn of(&self, address: &str) -> Option<Seen> {
        self.watching()
            .iter()
            .find(|lookout| super::same_server(&lookout.server.address, address))
            .map(|lookout| lookout.seen.borrow().clone())
    }

    /// The listed servers' pings, in list order, once every lookout still
    /// connecting has answered or [`PROBE_WAIT`] passed: `home`'s as given,
    /// a server without an open lookout `None`.
    pub(crate) async fn pings(
        &self,
        servers: &[ListedServer],
        home: Option<(&str, Duration)>,
    ) -> Vec<Option<Duration>> {
        let seen = self.seen();
        let mut pings = Vec::with_capacity(servers.len());
        for server in servers {
            if let Some((address, ping)) = home
                && super::same_server(address, &server.address)
            {
                pings.push(Some(ping));
                continue;
            }
            let Some((_, mut watched)) = seen
                .iter()
                .find(|(listed, _)| super::same_server(&listed.address, &server.address))
                .cloned()
            else {
                pings.push(None);
                continue;
            };
            let settled = tokio::time::timeout(
                PROBE_WAIT,
                watched.wait_for(|seen| !matches!(seen, Seen::Connecting)),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .map(|seen| seen.clone());
            pings.push(match settled {
                Some(Seen::Up { ping, .. }) => Some(ping),
                _ => None,
            });
        }
        pings
    }

    /// The servers whose lookout is connected now, with its requests and
    /// ping, in list order.
    pub(crate) fn up(&self, servers: &[ListedServer]) -> Vec<(ListedServer, Requests, Duration)> {
        let seen = self.seen();
        servers
            .iter()
            .filter_map(|server| {
                let (_, watched) = seen
                    .iter()
                    .find(|(listed, _)| super::same_server(&listed.address, &server.address))?;
                match &*watched.borrow() {
                    Seen::Up { requests, ping } => Some((server.clone(), requests.clone(), *ping)),
                    _ => None,
                }
            })
            .collect()
    }

    /// Each listed server as the player sees it: the one played on with
    /// `home_ping`, the others as their lookouts know them.
    pub(crate) fn rows(
        &self,
        servers: &[ListedServer],
        home: Option<&str>,
        home_ping: Option<Duration>,
    ) -> Vec<ServerRow> {
        servers
            .iter()
            .map(|server| {
                let here = home.is_some_and(|home| super::same_server(home, &server.address));
                let (reachable, ping) = if here {
                    (true, home_ping)
                } else {
                    match self.of(&server.address) {
                        Some(Seen::Up { ping, .. }) => (true, Some(ping)),
                        Some(Seen::Connecting) | None => (false, None),
                        Some(Seen::Down) => (false, None),
                    }
                };
                ServerRow {
                    name: server.name.clone(),
                    ping_ms: ping.map(millis),
                    here,
                    reachable,
                }
            })
            .collect()
    }
}

/// Asks `requests` for page `page` of its server's rooms, within
/// [`LIST_WAIT`].
pub(crate) async fn list(requests: &Requests, page: u16) -> Option<RoomPage> {
    match tokio::time::timeout(
        LIST_WAIT,
        requests.request(tpf3mp_proto::Request::ListRooms { page }),
    )
    .await
    {
        Ok(Ok(tpf3mp_proto::Response::Rooms(page))) => Some(page),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use tpf3mp_proto::{BoundedVec, Invite, ListedRoom, RoomListing, RoomPhase};

    use super::*;

    fn eu_us() -> Vec<ListedServer> {
        parse_list("EU=eu.example.org:29470,US=us.example.org:29470").unwrap()
    }

    #[tokio::test]
    async fn waits_for_connecting_lookouts_as_one_bounded_list() {
        let (first, first_seen) = watch::channel(Seen::Connecting);
        let (second, second_seen) = watch::channel(Seen::Connecting);
        let waiting = tokio::spawn(wait_until_settled(
            vec![first_seen, second_seen],
            Duration::from_secs(1),
        ));

        tokio::time::sleep(Duration::from_millis(10)).await;
        first.send_replace(Seen::Down);
        second.send_replace(Seen::Down);
        assert!(waiting.await.unwrap(), "all trusted lookouts settled");

        let (_still_connecting, pending) = watch::channel(Seen::Connecting);
        assert!(
            !wait_until_settled(vec![pending], Duration::from_millis(10)).await,
            "an unsettled lookout makes the list incomplete"
        );
    }

    #[test]
    fn a_list_reads_as_names_and_addresses_or_not_at_all() {
        assert_eq!(
            eu_us(),
            [
                ListedServer {
                    name: "EU".into(),
                    address: "eu.example.org:29470".into(),
                },
                ListedServer {
                    name: "US".into(),
                    address: "us.example.org:29470".into(),
                },
            ]
        );
        assert_eq!(
            parse_list(" EU = eu.example.org:29470 , ,").unwrap().len(),
            1,
            "spaces and empty entries are nothing"
        );
        assert!(parse_list("").unwrap().is_empty());
        for bad in [
            "eu.example.org:29470",
            "EU=eu.example.org",
            "EU=eu.example.org:0",
            "EU=eu.example.org:99999",
            "EU=eu.example.org:65536",
            "=eu.example.org:29470",
            "E,U=eu.example.org:29470",
            "Europe/West=eu.example.org:29470",
            "A name far too long for a server=eu.example.org:29470",
            "EU=eu.example.org:29470,eu=other.example.org:29470",
            "EU=eu.example.org:29470,US=EU.example.org:29470",
        ] {
            assert!(parse_list(bad).is_err(), "{bad} is refused");
        }
        assert!(
            parse_list("EU=[::1]:29470").is_ok(),
            "bracketed IPv6 is valid"
        );
        let nine = (0..9)
            .map(|n| format!("S{n}=s{n}.example.org:29470"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_list(&nine).is_err(), "at most {MAX_SERVERS}");
    }

    #[test]
    fn the_default_leads_the_release_list() {
        let more = parse_list("US=us.example.org:29470").unwrap();
        let listed = release_list(Some("relay.example.org:29470"), Some("EU"), &more).unwrap();
        let names: Vec<&str> = listed.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["EU", "US"], "the default leads the additional list");
        assert_eq!(listed[0].address, "relay.example.org:29470");
        let unnamed = release_list(Some("relay.example.org:29470"), None, &[]).unwrap();
        assert_eq!(unnamed[0].name, "relay.example.org", "named by its host");
        assert!(release_list(None, Some("EU"), &[]).unwrap().is_empty());
        assert!(release_list(None, Some("EU"), &eu_us()).is_err());
        assert!(
            release_list(Some("relay.example.org:29470"), Some("EU"), &eu_us()).is_err(),
            "a repeated server name is refused instead of silently discarded"
        );
        let duplicate_default =
            parse_list("US=relay.example.org:29470").expect("the extra entry is valid alone");
        assert!(
            release_list(
                Some("relay.example.org:29470"),
                Some("EU"),
                &duplicate_default
            )
            .is_err(),
            "the default address cannot be repeated in the additional list"
        );
        let eight_more = parse_list(
            "S1=a.example.org:29470,S2=b.example.org:29470,S3=c.example.org:29470,S4=d.example.org:29470,S5=e.example.org:29470,S6=f.example.org:29470,S7=g.example.org:29470,S8=h.example.org:29470",
        )
        .unwrap();
        assert!(
            release_list(Some("relay.example.org:29470"), Some("EU"), &eight_more).is_err(),
            "the release's total server count includes the default"
        );
        assert!(
            release_list(Some("relay.example.org:99999"), None, &[]).is_err(),
            "the default uses the runtime host-and-port validator"
        );
        assert!(
            release_list(Some("relay.example.org:29470"), Some("bad/name"), &[]).is_err(),
            "the release validator applies the server-name rules to the default too"
        );
    }

    #[test]
    fn invites_and_connect_go_only_to_listed_servers() {
        let listed = eu_us();
        assert_eq!(
            connect_target(&listed, "", None, false),
            Ok(None),
            "the closest"
        );
        assert_eq!(
            connect_target(&listed, "K7QM2X", None, true),
            Ok(None),
            "a bare invite: the closest, then wherever the room is"
        );
        assert_eq!(
            connect_target(
                &listed,
                "US.example.org:29470 K7QM2X",
                Some("US.example.org:29470"),
                true
            ),
            Ok(Some("us.example.org:29470".into())),
            "an invite naming a listed server goes there"
        );
        assert_eq!(
            connect_target(&listed, "eu.example.org:29470", None, false),
            Ok(Some("eu.example.org:29470".into())),
            "a playtest's auto room names its server"
        );
        let refused = connect_target(
            &listed,
            "evil.example.org:29470 K7QM2X",
            Some("evil.example.org:29470"),
            true,
        );
        assert!(
            refused
                .as_ref()
                .is_err_and(|why| why.contains("does not vouch for")),
            "{refused:?}"
        );
        assert_eq!(
            connect_target(&listed, "evil.example.org:29470", None, false),
            Err("that is not an invite".into())
        );
    }

    #[test]
    fn the_closest_server_wins_ties_keep_the_current_and_the_unreachable_lose() {
        let ms = |ms| Some(Duration::from_millis(ms));
        assert_eq!(fastest(&[ms(110), ms(24)], None), Some(1));
        assert_eq!(
            fastest(&[ms(24), ms(110)], Some(1)),
            Some(0),
            "far enough to move"
        );
        assert_eq!(
            fastest(&[ms(20), ms(25)], Some(1)),
            Some(1),
            "within the margin: stay"
        );
        assert_eq!(
            fastest(&[ms(25), ms(20)], None),
            Some(0),
            "within the margin: the first"
        );
        assert_eq!(
            fastest(&[None, ms(200)], Some(0)),
            Some(1),
            "the unreachable never"
        );
        assert_eq!(fastest(&[None, None], Some(0)), None);
        assert_eq!(fastest(&[], None), None);
        assert_eq!(
            fastest(&[ms(30)], Some(4)),
            Some(0),
            "a current not on the list"
        );
    }

    fn room(name: &str, code: &str, players: u8, phase: RoomPhase) -> ListedRoom {
        ListedRoom {
            invite: code.parse::<Invite>().unwrap(),
            name: Text::new(name).unwrap(),
            rules: Text::new("native").unwrap(),
            players,
            max_players: 4,
            has_password: false,
            phase,
            listing: RoomListing {
                map: Text::new("dry").unwrap(),
                year: 1850,
                companies: 1,
            },
            competitive: false,
        }
    }

    fn page(rooms: Vec<ListedRoom>, more: bool) -> RoomPage {
        page_at(0, rooms, more)
    }

    fn page_at(page: u16, rooms: Vec<ListedRoom>, more: bool) -> RoomPage {
        RoomPage {
            page,
            rooms: BoundedVec::new(rooms).unwrap(),
            more,
        }
    }

    #[test]
    fn merged_rooms_say_their_server_and_ping_and_where_to_join() {
        let listed = eu_us();
        let eu = page(
            vec![
                room("Busy", "K7QM2X", 3, RoomPhase::Running),
                room("Quiet", "P4ZR9T", 1, RoomPhase::Lobby),
            ],
            false,
        );
        let us = page(
            vec![room("Full lobby", "H3WN7K", 3, RoomPhase::Lobby)],
            true,
        );
        let (list, places) = merge(
            0,
            &[
                Page {
                    server: &listed[0],
                    ping: Some(Duration::from_micros(23_400)),
                    page_index: 0,
                    page: &eu,
                },
                Page {
                    server: &listed[1],
                    ping: None,
                    page_index: 0,
                    page: &us,
                },
            ],
        );
        let shown: Vec<(&str, Option<&str>, Option<u32>)> = list
            .rooms
            .iter()
            .map(|room| (room.name.as_str(), room.server.as_deref(), room.ping_ms))
            .collect();
        assert_eq!(
            shown,
            [
                ("Full lobby", Some("US"), None),
                ("Quiet", Some("EU"), Some(23)),
                ("Busy", Some("EU"), Some(23)),
            ],
            "lobbies first, the fuller first, each with its server"
        );
        assert!(list.more, "one server has more");
        assert!(places.contains(&("H3WN7K".into(), "us.example.org:29470".into())));
        assert!(places.contains(&("K7QM2X".into(), "eu.example.org:29470".into())));
    }

    #[test]
    fn global_pages_include_overflow_from_each_region() {
        let listed = eu_us();
        let rooms = |region: &str, prefix: char| {
            let alphabet = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
            (0..11)
                .map(|index| {
                    let code = format!("{prefix}2A{}BC", alphabet[index] as char);
                    room(&format!("{region} {index:02}"), &code, 1, RoomPhase::Lobby)
                })
                .collect::<Vec<_>>()
        };
        let eu_first = page_at(0, rooms("EU", 'E'), false);
        let us_first = page_at(0, rooms("US", 'U'), false);
        let eu_second = page_at(1, Vec::new(), false);
        let us_second = page_at(1, Vec::new(), false);
        let sources = [
            Page {
                server: &listed[0],
                ping: Some(Duration::from_millis(24)),
                page_index: 0,
                page: &eu_first,
            },
            Page {
                server: &listed[0],
                ping: Some(Duration::from_millis(24)),
                page_index: 1,
                page: &eu_second,
            },
            Page {
                server: &listed[1],
                ping: Some(Duration::from_millis(30)),
                page_index: 0,
                page: &us_first,
            },
            Page {
                server: &listed[1],
                ping: Some(Duration::from_millis(30)),
                page_index: 1,
                page: &us_second,
            },
        ];

        let first_sources: Vec<_> = sources
            .iter()
            .filter(|source| source.page_index == 0)
            .copied()
            .collect();
        let (first, _) = merge(0, &first_sources);
        assert_eq!(first.rooms.len(), tpf3mp_proto::ROOMS_PER_PAGE);
        assert!(
            first.more,
            "two regions have 22 rooms, so page 0 has a next page"
        );

        let (second, places) = merge(1, &sources);
        assert_eq!(
            second.rooms.len(),
            2,
            "the two overflow rooms appear on page 1"
        );
        assert!(!second.more, "all 22 rooms have been shown");
        assert_eq!(places.len(), 2);
    }

    #[test]
    fn duplicate_public_codes_require_a_card_target() {
        let listed = eu_us();
        let routes = vec![
            ("K7QM2X".to_owned(), listed[0].address.clone()),
            ("K7QM2X".to_owned(), listed[1].address.clone()),
        ];
        assert_eq!(
            room_target(&listed, &routes, "K7QM2X", Some("US")),
            Ok(Some("us.example.org:29470".into())),
            "the clicked room's trusted server wins"
        );
        assert!(
            room_target(&listed, &routes, "K7QM2X", None)
                .unwrap_err()
                .contains("more than one server"),
            "a bare collision is refused instead of selecting the first route"
        );
        assert!(room_target(&listed, &routes, "K7QM2X", Some("ASIA")).is_err());
    }

    #[test]
    fn a_ping_reads_in_whole_milliseconds() {
        assert_eq!(millis(Duration::from_micros(300)), 1);
        assert_eq!(millis(Duration::from_millis(110)), 110);
    }
}

# Decision log

Each entry records a decision, why it was made, and what it rejected. A later
entry may supersede an earlier one; entries are never rewritten.

## D1 (2026-09-18): Rust for all project code

The server, agent, protocol, canonical rules and in-game hook are written in
Rust.

- **Performance.** On par with C++, with no garbage collector. That matters
  for the hook, which runs on the game's own threads.
- **Memory safety.** The server faces the internet, and the hook must not
  crash the game.
- **One language across both ends.** Client and server share the protocol and
  rules crates, so the two sides cannot drift apart.
- **Platforms.** One codebase builds for Windows x64, Linux x64 and macOS
  arm64. quinn (QUIC) runs on all three.

Rejected:

- **C++.** No memory safety, and the team prefers to avoid it.
- **C#.** A GC runtime inside the game process is a poor fit for detours on
  the simulation thread. Also, .NET's `System.Net.Quic` requires Windows 11 or
  Server 2022 (TPF3's minimum is Windows 10) and supports macOS only
  "partially, through a non-standard Homebrew package". See
  <https://learn.microsoft.com/en-us/dotnet/fundamentals/networking/quic/quic-overview>.
  The hook would still need a second, native language.
- **Python.** Both TPF2 projects use it. It has a performance ceiling for the
  server, and shipping it to players means PyInstaller executables.

## D2 (2026-09-18): canonical server authority, native worlds as replicas

The server advances a deterministic canonical state machine: companies,
money, ownership, identities, topology, lines, vehicles, economy, calendar.
Native TPF3 worlds apply the same ordered events at the same step, report
postconditions, and are rebased when they drift. See [ARCHITECTURE.md](ARCHITECTURE.md).

- Mixed platforms (Windows, Linux, macOS arm64) in one room are a hard
  requirement.
- Different binaries from different compilers, and arm64 FMA contraction,
  make bit-identical native simulation across platforms very unlikely.

This supersedes the first plan of 2026-09-18, "server-sequenced pure
lockstep", which relied on native determinism. That plan's turn-seal
sequencing is kept as the ordering and pacing layer.

Rejected:

- **Pure native lockstep.** It only works within one binary.
- **Trusting one designated replica's native economy.** That replica's machine
  becomes the economic truth. It is kept as an open co-op question, not as
  the foundation.

## D3 (2026-09-18): QUIC first, WebSocket over TLS as fallback

- QUIC gives TLS 1.3, independent streams (a snapshot transfer never delays a
  sealed turn), datagrams for advisory traffic, and connection migration.
- WebSocket over TLS on TCP 443 covers networks that block UDP.
- Both carry the same messages.

Rejected: TCP-only, which has head-of-line blocking between snapshots and
turns, and raw UDP with custom reliability and cryptography, which is what
`tpf2-multiplayer` had to build by hand.

Update (2026-09-19): the fallback carries QUIC itself, not the messages. A
tunnel is a WebSocket whose binary messages are QUIC datagrams, and the
server merges tunnels into its one QUIC endpoint. A second transport for the
same messages would have needed its own framing, multiplexing, flow control
and authentication, and every feature would have had to work on both. QUIC
inside TCP pays twice for congestion control and suffers TCP's head-of-line
blocking, which is acceptable for networks that leave no other way.

## D4 (2026-09-18): operated servers, trusted by clients

Servers are run by the project: first on the existing German server, then on
regional VPS nodes.

- Clients trust the server.
- Players authenticate to it with per-install keys and room invites.
- Community-run servers are out of scope. Supporting them later would require
  player-signed intents, so that a server cannot forge actions.

## D5 (2026-09-18): one team, both TPF2 codebases as input

Julian Cooper (TPF2MP, `tf2mp-relay`) and silver2127 (`tpf2-multiplayer`)
work on this repository together. Both TPF2 codebases are MIT licensed. Code
or test vectors taken from them are credited in the file that uses them.

## D6 (2026-09-19): the host chooses the room's rules, the game's own economy included

Each server offers a list of rules, and the host of a room picks one when
creating it. Each set of rules comes with its economy. The first on the list
is the default. The room keeps its rules for good: they are recorded in its
log, and a restarted server restores the room with the same rules or not at
all.

- **`native` is always offered, and is the default until others ship.** The
  server orders every player's commands and checks nothing about money. Each
  game runs its own economy as in single player. Checkpoints still compare
  every replica's world: one that drifts, for example on another platform,
  is re-sent the world the room agreed on.
- **Canonical rules are an option on the same list** (D2). The server
  validates intents and settles the economy itself, from TPF2MP's
  integer-arithmetic model. Those rooms get what D2 promises: money no
  client can forge, balance changes without client updates, and hidden
  information.
- The hook learns the room's rules when the game begins (`ToHook::Begin`),
  so with `native` it leaves the game's economy alone.

This amends D2. Canonical authority stays the design for rooms that choose
it, and stops being a requirement for every room. D2 rejected "trusting one
designated replica's native economy"; `native` rooms trust none: every replica
runs the economy, and the room's agreed world corrects any that disagrees.
Players who want the game as they know it can have it; players who want a
server that cannot be cheated choose the canonical rules.

Rejected:

- **One economy per server.** Groups on the same server want different
  games.
- **Changing rules mid-game.** The rules' state and the log would have to be
  converted. A new room is the way to switch.

## D7 (2026-09-26): a native launcher window that updates itself from signed releases

Players start TPF3-MP from a native window on Windows, Linux and macOS,
drawn with egui (`eframe`), in place of a page in their browser. The window
runs the agent's launcher backend in its own process; the page remains, as
`tpf3mp-agent launcher` and as the window's fallback on systems where no
window can open.

- **One program, one window.** No browser tab to keep open, no console on
  Windows, and closing the window during a game asks first.
- **Rust and one code base.** egui builds on all three platforms with the
  same crates as the rest; its UI is tested headless through AccessKit
  (`egui_kittest`), and screens can be rendered to images for review.
- **Updates are signed.** The launcher downloads the latest published
  release and installs it only if its manifest carries an Ed25519
  signature from the project's key, names a newer version, and describes
  the package byte for byte (SHA-256). The private key is a repository
  secret, the public half is built into every launcher. A launcher built
  without the key never updates. An update never interrupts a game: it
  installs when the player chooses or at the next start, and a failed
  install puts the old files back.

Rejected:

- **Tauri or another web view.** A web page in a native frame: still the
  browser engine, now shipped or required per platform (WebView2,
  WebKitGTK), and a second language for the UI.
- **Qt or GTK.** C++ or C libraries to build and ship on three platforms,
  against D1.
- **Unsigned updates over HTTPS.** Anyone who could publish a release, or
  replace an asset, would run code on every player's machine.

Update (2026-09-27), after a security review of the updater: signing
moved out of the build into `sign.yml`, which signs only a published
release, in a GitHub environment whose required reviewer approves each
signing. As a repository secret, the key reached every workflow on every
branch, so anyone who could push a branch could have signed an update and
skipped every check. Launchers trust a list of keys, so the key can be
changed. Installing is journalled and confirmed: an install that fails or
is cut short is undone, the old files stay until the new version opens its
window, a version that fails to three times is rolled back and not
installed again, and only files a journal names are deleted. One process
installs at a time. Releases are fetched from `releases/latest/download`,
over HTTPS only, rather than GitHub's rate-limited API.

## D8 (2026-09-26): player actions as a typed schema in millimetres, inside the opaque payload

A player action travels as `tpf3mp_proto::action::Action`: positions as
`i32` millimetres, resources by file name, and things that have no stable
position (companies, lines, vehicles, stations) by ids the server assigns.
See "The action schema" in [BUILDING.md](BUILDING.md).

- **Integers, not floats.** The same action is the same bytes on every
  platform, compares exactly and hashes the same; a millimetre is far below
  the tolerances replicas match geometry with.
- **Typed and bounded.** TPF2's text commands were parsed field by field and
  a mis-parse became a wrong build; here decoding checks every length and
  every index, so a replica only ever sees a well-formed action.
- **Its own version, inside the payload.** The network layer relays actions
  without reading them, so the schema can grow without a protocol change.

Rejected:

- **Floats in metres.** Not bit-identical once converted twice, and a
  canonical server cannot use them (D2).
- **Engine entity ids.** They differ between games and are recycled
  (BUILDING.md).
- **The TPF2 text format.** Unbounded, untyped, and hand-parsed on both
  sides.

## D9 (2026-09-27): the installer is scripts players can read

What puts TPF3-MP into the game (the mod, the hook, and on Windows the
proxy DLL in place of one of the game's own) ships in the package as
scripts: `INSTALL_TPF3MP.cmd`, which runs `tools\install.ps1`, on Windows,
and `install.sh` on Linux and macOS. They replace `tpf3mp-agent
install-hook`.

- **Players trust what they can read.** The installer changes the game's
  folder. TPF2MP's players did not trust a program doing that; its
  PowerShell installer, which anyone can open, is what they accepted. A
  script shows exactly what it changes, with nothing compiled in between.
- **The same rules as the code.** The scripts fail closed as the Rust
  installer did. They refuse while the game runs, in a folder that is not
  the game's, over another mod's proxy, when the game's own DLL is gone,
  and when their record names anything but TPF3-MP's files. A step that
  fails undoes the ones before it, and nothing is deleted: what is
  replaced goes to a backups folder. CI tests them on all three platforms,
  in the shells players have: Windows PowerShell 5.1, and the bash 3.2
  macOS ships.
- **An exception to D1, for this alone.** The launcher, the agent and the
  hook stay Rust.

Rejected:

- **A compiled installer** (the agent's `install-hook`, an MSI or a setup
  program): opaque to players, and flagged by SmartScreen and antivirus
  like any unsigned program.

Update (2026-09-27), with D11: the installers put in the mod alone. The
hook stays in the package, next to the launcher, and nothing goes into the
game's folder: no proxy, no hook, no record. The record is kept in
TPF3-MP's own data folder (`installed.json` on Windows, `installed.txt` on
Linux and macOS). The rest of this entry stands: the installers are still
readable scripts, fail closed, undo a failed step, delete nothing, and are
tested on all three platforms.

## D10 (2026-09-27): players' diagnostics go to the server by themselves

The launcher sends the lines of its log, redacted, to the server the player
plays on, which keeps them by session: the operator reads what went wrong
for a player from the support ID alone, as TPF2MP's relay let its operator
do. Asking players for files, which Collect logs still makes, comes late
and often not at all.

- **Over the game's connection.** A request on the control stream, from a
  client that has proven its identity, filed under its session: no second
  service, port or credential.
- **What TPF2MP's missed, fixed.** Its redaction missed paths outside
  `C:\Users`, Steam's `userdata\<account>` among them, and paths escaped
  in JSON; `tpf3mp_proto::redact` cuts every absolute path to its last
  part, on both sides. Its uploader lost the last lines before a session
  ended; closing now sends them, and lines left when a connection drops go
  with the next. Its only opt-out was giving up the relay; here a switch
  in the launcher stops them, and is remembered.
- **Never in a game's way.** Their own rate budget, a writer that does not
  make connections wait, a quota per session, and a total the oldest make
  room in.

Rejected:

- **Whole log files, or crash dumps.** Too large, and too much in them to
  redact reliably; Collect logs remains for those, on the player's say.
- **An HTTP upload to the server.** A second way in, needing its own
  authentication, rate limits and TLS, for what the game's connection
  already carries.

Update (2026-09-27): the launcher's window and page no longer offer
**Collect logs** or **Open logs folder**. With the log going to the server
by itself, players have nothing to send but their support ID. The game's
own log and crash dumps, which are never sent, come from `tpf3mp-agent
collect-logs` when an operator asks for them. The page's
`/api/collect-logs`, a way for the page to make the launcher write files,
is gone with the button.

## D11 (2026-09-27): the hook runs only in a game the launcher starts

TPF3-MP's code runs in Transport Fever 3 only when a player starts the game
from the TPF3-MP launcher, for a room they are in; that game runs it until
it closes. Started from Steam, the game is the plain game, with nothing of
TPF3-MP in it. TPF2MP worked this way, and its players expected it.

- **How it starts.** `tpf3mp-launch` starts the game with the hook in that
  one process. On Windows it starts the game suspended, has the game load
  the hook with `LoadLibraryW` on a thread the launcher creates in it,
  checks that the hook is there, and only then lets the game run; when any
  of that fails, the game is ended, not left running. This is how TPF2MP's
  injector started Transport Fever 2 (`--launch`). On Linux, the game gets
  `LD_PRELOAD` naming the hook, in its own environment only. macOS waits
  for the game: its hardened runtime refuses libraries it did not load
  itself.
- **The game is told which launcher started it.** The launcher passes the
  name of its link to the game (`TPF3MP_GAME_LINK`); a hook without it does
  nothing at all, so the hook is inert even if something else loads it.
  The game also gets `SteamAppId`, so that it does not restart through
  Steam without the hook. As TPF2MP's launcher did, it starts the game only
  while Steam runs, and one game at a time.
- **Nothing to undo.** Closing the game ends TPF3-MP's part in it. There is
  no file in the game's folder for Steam's file check to report, another
  mod to collide with, or a game update to break, and nothing to uninstall
  but the mod.
- **Fail closed, as before.** The hook still checks the game's build
  against its profiles and installs nothing when none matches (see
  [HOOKS.md](HOOKS.md)).

Rejected:

- **A proxy DLL in the game's folder** (TPF2's `alut.dll` trick,
  `tpf3mp-proxygen`): it loads the hook into every start of the game, from
  Steam too, until it is taken out; Steam's file check and game updates
  undo it, and it collides with other mods that proxy the same DLL. The
  generator is removed.
- **A Steam launch option with `LD_PRELOAD`** on Linux: the same, set by
  hand, and easy to forget when uninstalling.
- **Starting the game through Steam** (`steam://run`): the game would then
  start without the hook, since nothing of the launcher's reaches it.

## D12 (2026-09-27): the launcher plays on the project's server alone

A package's launcher plays on the one server it was built for, the
project's own (D4), set when the release is built
(`TPF3MP_DEFAULT_SERVER`). Players do not type a server and cannot choose
another, and an invite naming another server is refused, not followed.
TPF2MP's launcher offered its relay in a box players could change; this
one does not.

- **One place to meet.** Every player, and every invite, is on the same
  server: nobody mistypes an address or ends up alone on another.
- **Fail closed.** Clients trust the server they play on (D4): it sends
  the room's turns and worlds. Following any invite's server would let an
  invite send players to a server nobody vouches for.
- **Enforced in the launcher's backend**, so the window and the page
  behave alike. The server is shown, not asked for; Connect takes the
  player's name and, if they have one, an invite, to join in one step.
- **Development stays open.** `--server` on the command line fixes
  another server, for playtests against a local one; a build without a
  server, as a developer's own, offers the typed field as before. No
  release is drafted without `TPF3MP_DEFAULT_SERVER`.

This narrows D4 in the launcher: operated servers were the plan, and
now the players' launcher knows no other. More servers, such as regional
ones, come later through the launcher itself, never through what an
invite says.

Rejected:

- **A server field players can change** (as until now): an invite could
  name any server, and a typo ends in a lonely room.
- **Following an invite to its server** (as until now): the fail-closed
  reason above.

# Playing

How to play Transport Fever 3 together with TPF3-MP. The network side is
ready. The part that runs inside the game waits for the game's release:
this page says so where it applies.

## What you need

- Transport Fever 3, the same build and mods as everyone in your room, in
  the same order. The room compares everyone's before a game starts, and
  tells you exactly which mods to add, remove or update if yours differ.
- The TPF3-MP package for your system, from the project's releases:
  Windows x64, Linux x64 or macOS on Apple silicon. Players on different
  systems can share one room.
- The address of a TPF3-MP server, such as `tpf3mp.example.org:29470`,
  unless your package already offers one.

You do not need to forward any port or open anything on your router: your
launcher connects out to the server, and everything goes through it.

## Installing

1. Unpack the package anywhere you can write to, such as your Documents
   folder: the launcher updates the files in it (see "Updates").
2. **The mod.** Start Transport Fever 3 once, so Steam makes its folder for
   your mods, and close it again. Then:
   - **Windows:** double-click `INSTALL_TPF3MP.cmd` in the package.
   - **Linux and macOS:** run `./install.sh` from the package's folder.

   The installer is a script, not a program: open `tools\install.ps1`
   (Windows) or `install.sh` (Linux and macOS) to read exactly what it
   changes. It puts the TPF3-MP mod, `tpf3mp_1`, in Steam's folder for
   your Transport Fever 3 mods, `<Steam>/userdata/<account>/3493540/local/mods`,
   and notes its version in TPF3-MP's data folder, which the launcher
   shows. To put it in another mods folder, drop that folder onto
   `INSTALL_TPF3MP.cmd`, or run `./install.sh "<the mods folder>"`.

   Nothing goes into the game's own folder, and no launch option is set.
   The installer refuses, and changes nothing, while the game is running.
   A step that fails undoes the ones before it. Nothing is deleted: a
   TPF3-MP mod it replaces or takes out goes to the `backups` folder in
   TPF3-MP's data folder. `UNINSTALL_TPF3MP.cmd` or `./uninstall.sh` takes
   the mod out again.

   Run the installer again after an update of TPF3-MP. Until the game is
   out, packages carry no mod yet, and the installer says so.

The part of TPF3-MP that runs inside the game is not installed at all: the
launcher loads it into the game it starts for your room, into that game
alone, for as long as it runs (see "The launcher"). Started from Steam,
Transport Fever 3 is the plain game, as if TPF3-MP were not there.

## The launcher

Start the launcher from the package:

- **Windows:** `TPF3-MP.exe`. The first time, Windows may say it protected
  your PC from an unknown app: choose **More info**, then **Run anyway**.
- **macOS:** `TPF3-MP.app`. The first time, macOS refuses to open an app
  from an unidentified developer. On macOS 15 and later: try to open it
  once, then in **System Settings**, **Privacy & Security**, choose **Open
  Anyway**. On earlier versions: right-click it, choose **Open**, then
  **Open** again.
- **Linux:** `tpf3mp-launcher`. It needs a desktop with Vulkan or OpenGL
  drivers, as the game does.

It opens the TPF3-MP window. Keep it open while you play: closing it ends
your session, and during a game it asks first. On a system where the
window cannot open, the launcher opens the same launcher as a page in your
browser instead (`--browser` does so on purpose); that page works only on
your own machine, in the tab the launcher opened.

1. **Server.** Enter the server's address and the name others will see,
   then **Connect**; the launcher remembers both for next time. Got an
   invite? Paste the whole of it here instead, with your name: you are
   connected and in the room in one step.
2. **Rooms.** Either create a room, with an optional password, or paste an
   invite someone sent you and **Join room**. When the server offers more
   than one set of rules, the host picks one when creating the room:
   `native` is the game's own rules and economy, as in single player;
   others are run by the server, which checks everyone's money and
   actions. The room's title shows its rules, and they cannot change once
   the room exists.
3. **Invite.** In your room, **Copy invite** and send it to your friends,
   for example on Discord. It holds the server's address, as you typed it,
   and the room's code; if you typed `localhost` or a home network address,
   put the address your friends use in its place. Anyone with the invite
   (and the password, if you set one) can join; keep it within your
   group.
4. **Start the game.** In your room, press **Start Transport Fever 3** in
   the Game part, with Steam running. The launcher starts the game with
   TPF3-MP in it, for this room; once the game has loaded, the Game part
   says it is connected. Only a game started here joins the room: started
   from Steam, it is the plain game. Press it once; the launcher refuses to
   start a second game while the first still runs.
5. **Ready.** Everyone presses **Ready**. The room's owner then presses
   **Start game**. Everyone's game starts from the owner's world.

The window is laid out as the TPF2 multiplayer launcher is. On the left,
under the game's name, a checklist ticks these steps off as you go:
connect to a server, create or join a room, start the game from here,
everyone ready, play together. The step at hand is on the right. A narrow
window puts everything in one column.

The **Game** panel follows your game: started from here, downloading the
room's world, loading it, and playing. The bar along the bottom, **Your
game**, says where Steam has Transport Fever 3 and whether the TPF3-MP mod
is installed (see "Installing"). **Chat**
reaches everyone in the room. A message **From the server** is its
operator's, such as a restart coming: when the server comes back, the
launcher rejoins by itself. The **Session log** tells you what happened,
such as your world being replaced by the room's, or your connection coming
back.

## Updates

The launcher checks for a new version when it starts and every few hours,
and downloads it in the background. When it is ready, the window says so:
**Restart and update** installs it and restarts the launcher. During a game
it waits: the update installs the next time you start TPF3-MP.

The launcher installs only what the TPF3-MP project signed: a download
whose signature, version or contents do not check out is refused, and an
install that fails or is cut short puts the old files back, at once or at
the next start. The old version's files stay until the new version has
opened its window; if it fails to three times running, the launcher goes
back to the version before and does not install that one again. Updates
go into the package's folder, so unpack it where you can write, not into a
protected folder such as Program Files.

## While you play

- **Joining later.** You can join a game that is already running: the
  room sends you its world, and your game loads it and catches up.
- **Losing the connection.** If your connection or the server drops, the
  launcher rejoins the room by itself, and your game only pauses. If you
  were away too long to catch up, the room sends you its world again.
- **Leaving.** **Leave room** gives up your seat. The owner can also remove
  a player whose game froze; a removed player cannot come back to that
  room.
- **Your world disagrees.** Every few seconds everyone's game compares the
  world with the room's. If yours has drifted, the room sends you its world
  and your game reloads it. A notice says so.
- **Saving.** The room saves everyone's game together from time to time,
  which you notice as a short pause, like an autosave.

## Playtesting before the game is out

Until Transport Fever 3 is released, `tpf3mp-fakegame` in the package
stands in for it: a small toy game behind the same step gate, which builds
track, saves and loads worlds. Everything but TPF3 itself can be tried,
across PCs and systems:

1. Start TPF3-MP as above.
2. Start `tpf3mp-fakegame` from the same folder (from a terminal on Linux
   and macOS). The window's **Game** part says the game is connected.
3. Connect, create or join a room, get ready and start, as in a real game.
   The fake game plays by itself: watch the **Game** part count steps, and
   try chatting, leaving and rejoining, and joining a game already running.

Run one fake game next to each launcher. It stops when its room's game
ends.

Developers who want a whole room on one PC, several fake games each with
its own agent, use the multiplayer rig instead (`tpf3mp-rig`, see
"Development" in the [README](../README.md)).

## Your identity

The launcher creates a key for you on first use, in your user data folder
(`TPF3-MP/identity.key`). It is what makes you the same player next time,
so you can come back to your seat. There are no accounts or passwords. Keep
the file private, and copy it if you move to another computer.

Other players never see your IP address: everything goes through the
server.

## When something does not work

The top of the window shows your **support ID** (**Copy** copies it). It
names your connection in the server's log: send it to the server's operator
with your report, and they find exactly what happened to you. **Open logs
folder**, in the bar at the bottom, opens the launcher's own logs
(`TPF3-MP/logs` in your user data folder, one file a day, a week kept).

### Diagnostics

While you are connected, the launcher sends the lines of its log to the
server you play on, so its operator can see what went wrong for you from
your support ID, without asking you for files. Before a line leaves your
machine, paths are cut to their last part (so your user name and your
Steam account are not in them), and IP addresses, invites, keys and
passwords, e-mail addresses and Steam IDs are taken out; the server does
the same again. Your game's own log and crash dumps are not sent. The
server keeps the lines for a limited time, 30 days unless its operator
chose otherwise.

Untick **Send diagnostics**, under the bar at the bottom of the window (or
at the bottom of the browser page), to stop: the
launcher then sends nothing more, forgets the lines it had not sent yet,
and remembers your choice.

### Sending your logs

**Collect logs**, in the bar at the bottom of the window (and at the bottom
of the browser page),
puts everything a bug report needs into one zip,
`tpf3mp-logs-<time>.zip` in your Downloads folder (in `TPF3-MP` when there
is no Downloads folder), and shows it. Attach that zip to your report, with
your support ID. Without the launcher, `tpf3mp-agent collect-logs` writes
the same zip (`--out <folder>` for another place, `--since 2h` for a shorter
window, `--game-log <file>` to add a log kept elsewhere).

The zip holds:

- `tpf3mp/logs/`: the launcher's logs, which record its crashes too;
- `tpf3mp/hook.log`: the in-game hook's log;
- `game/…`: the game's own log (`stdout.txt`) and crash dumps. Until
  Transport Fever 3 is out, these are looked for in its Steam folder where
  Transport Fever 2 kept them in its own,
  `<Steam>/userdata/<account>/3493540/local/` (`stdout.txt` and
  `crash_dump/`); the manifest marks them "TPF2 location, confirm on TF3";
- `manifest.txt`: the versions of TPF3-MP, its protocol and its link to the
  game, your system, your support ID when connected, every file with its
  size, and which places were not found.

Only files changed in the last week are taken, newest first, up to 64 MB;
a long text log that does not fit whole keeps its end. What was left out
is listed in the manifest.

The zip never holds your identity key, invite keys, certificates or
tokens: only the logs folders above are read, and any file there whose name
looks like a key, certificate or token is withheld all the same. Your saved
worlds and remembered server are not included, and TPF3-MP's logs hold no
IP addresses. The game's own logs are the game's: look through the zip
before sharing it publicly if you want to be sure.

- **"the server is out of reach over UDP (...) and through wss://..."**:
  your network blocks both routes, or the server is down. Some school,
  office and hotel networks block the UDP the game uses; the launcher then
  tries a WebSocket connection on port 443 by itself. If the server's
  operator gave you a tunnel address, start the launcher with
  `--tunnel <address>`; if your network never passes UDP, add
  `--tunnel-only`.
- **"connected via tunnel"** next to the connection: your network blocks
  UDP, and the game plays through the tunnel. It works, but lost packets
  cost a little more delay.
- **A version mismatch**: your package and the server are different
  versions. The message says which side is older.
- **"Your game differs from the room's"**: the window lists what to change:
  the game build, the mods you lack, the mods the room does not run, and
  the mods you have in another version. Everyone needs the owner's build
  and mods in the same order. In the room, the **Game and mods** column
  shows whose game differs from the owner's; each player sees their own
  list.
- **"too many players are connected from this network"**: the server
  limits connections per network. Close another game, or ask the operator.
- **"the invite or password is not valid"**: the invite is from another
  server, the room closed, or the password is wrong.

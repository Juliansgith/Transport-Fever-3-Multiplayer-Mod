# Playing

How to play Transport Fever 3 together with TPF3-MP. The network side is
ready. Installing the part that runs inside the game waits for the game's
release: this page says so where it applies.

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
2. **Into the game: on release.** How the package's game library is put
   next to Transport Fever 3 depends on the released game, and this step is
   described here once it is known.

## The launcher

Start the launcher from the package:

- **Windows:** `TPF3-MP.exe`. The first time, Windows may say it protected
  your PC from an unknown app: choose **More info**, then **Run anyway**.
- **macOS:** `TPF3-MP.app`. The first time, macOS may refuse to open an app
  from an unidentified developer: right-click it, choose **Open**, then
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
4. **Ready.** Everyone presses **Ready**. The room's owner then presses
   **Start game**. Everyone's game starts from the owner's world.

The **Game** part of the window follows your game: downloading the room's
world, loading it, and playing. **Chat** reaches everyone in the room.
**Notices** tell you what happened, such as your world being replaced by
the room's, or your connection coming back.

## Updates

The launcher checks for a new version when it starts and every few hours,
and downloads it in the background. When it is ready, the window says so:
**Restart and update** installs it and restarts the launcher. During a game
it waits: the update installs the next time you start TPF3-MP.

The launcher installs only what the TPF3-MP project signed: a download
whose signature, version or contents do not check out is refused, and a
failed install puts the old files back. Updates go into the package's
folder, so unpack it where you can write, not into a protected folder such
as Program Files.

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
folder**, at the bottom, opens the launcher's own logs
(`TPF3-MP/logs` in your user data folder, one file a day, a week kept);
send the latest one too.

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

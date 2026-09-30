# The native hook

The native hook is the small library that runs *inside* the game process. It
captures and cancels player commands, gates the simulation step, controls speed
and save/load, and talks to the [agent](ARCHITECTURE.md#components) over
shared memory. This document specifies the parts built at milestone M0: the
build-signature engine, the per-build profile format, the detour engine, the
shared-memory ABI, and the release-day procedure. Locating and detouring the
actual TPF3 functions comes with the release-day profile (see
[DAY_ONE.md](DAY_ONE.md)); everything the hook needs to do it is here and
tested.

Crates:

| crate | contents |
|---|---|
| `tpf3mp-hookcore` | pattern scanning, per-build profiles + resolution, the x86-64 inline detour engine, a small read-only PE reader |
| `tpf3mp-ipc` | the shared-memory link (this document's ABI) |
| `tpf3mp-hook` | the `cdylib` loaded into the game: platform entry points, profile loading, agent connection |
| `tpf3mp-launch` | starts the game with the hook in that one process, and nowhere else |

## How the hook gets into the game

Only the TPF3-MP launcher puts the hook into the game, into the game it
starts for a room, and for as long as that game runs (D11 in
[DECISIONS.md](DECISIONS.md)). Nothing is installed into the game's folder
and no launch option is set: the game started from Steam is the plain game.

- **Windows.** `tpf3mp-launch` starts the game suspended, writes the hook's
  path into it, and runs `LoadLibraryW` on a thread it creates in the game.
  It checks the thread's result and the game's module list, and only then
  lets the game run. If the hook is not there, it ends the game rather than
  let it run without it. This is how TPF2MP's injector started Transport
  Fever 2 (`--launch`).
- **Linux.** The game starts with `LD_PRELOAD` naming the hook, in its own
  environment only, ahead of anything already preloaded (Steam's overlay).
- **macOS.** Not yet. The game's hardened runtime refuses libraries it did
  not load itself; how to get the hook in is a release-day question
  ([DAY_ONE.md](DAY_ONE.md)).

Before starting the game, the launcher checks four things, and refuses if
any fails:

- the player is in a room;
- the game and the hook are where they should be;
- Steam is running;
- no game it started is still running.

It gives the game `SteamAppId` and `SteamGameId` (3493540), so that the
game does not restart itself through Steam, which would start it without
the hook.

The hook runs only when the launcher started the game. The launcher names
two things in the game's environment:

- its link, `TPF3MP_GAME_LINK`;
- its own process, `TPF3MP_LAUNCHER_PID`.

Without the link the hook returns at once: it writes, hashes and opens
nothing. On Linux and macOS it also returns when its process's parent is
not that launcher. Programs the game starts, a browser opened from the
game for example, inherit `LD_PRELOAD` and the variables; the hook stays
out of them. On Windows nothing the game starts loads the hook.

## Design and the fail-closed rules

TPF3 will be patched often after launch, so the hook never pins raw addresses.
It carries one **profile** per game build. A profile binds a build identity
(executable SHA-256, and optionally file size and PE timestamp) to a set of
named **targets**, each located by a byte **signature** rather than an address.

Resolution is **fail-closed**. `tpf3mp_hookcore::profile::resolve` returns either
a complete, verified target table or a precise refusal, and it installs nothing
on the way to a refusal:

- **Unknown build** - the running executable's identity does not match the
  profile. The hook does not scan at all; multiplayer is disabled.
- **Missing** - a *required* target's signature is not found. Resolution fails
  as a whole, so required hooks are all-or-nothing; a partial install never
  happens.
- **Ambiguous** - a signature matches more than once. Refused, even for an
  optional target: a second, unexpected match is a corruption signal, not
  something to skip.
- **Prologue mismatch** - the signature matched but the exact bytes at the
  target are not the ones the profile expects. Refused.

An *optional* target that is simply absent is recorded and does not fail the
profile. Everything else fails closed. The hook logs the precise reason and
leaves the game untouched.

### Where the resolver scans (production vs. this repo's test)

`resolve` takes a byte slice plus the address its first byte corresponds to, so
it does not care whether those bytes come from a file or from memory.

- **Production**: the hook scans the running process's **mapped, unpacked module
  image** - the bytes the loader (and any DRM stub) produced in memory - passing
  the module's base address as the region base. This is the only correct source
  when a build's code section is packed or encrypted on disk.
- **This repo's static proof** (`tpf3mp-hookcore/tests/tpf2_static_proof.rs`)
  scans the executable **on disk**. That is a development convenience, valid
  only when the build's `.text` is readable on disk (see
  [the TPF2 verification](#what-was-verified-on-the-tpf2-binary)). The call is
  identical; only the byte source differs.

## Signatures

A signature is an IDA-style pattern: two hex digits per fixed byte and `??` (or
`?`) for a byte that may be anything, for example `48 8B ?? ?? E8`. Wildcards
exist so a signature skips the bytes that move between builds - RIP-relative
displacements, call targets, absolute addresses - and matches only the opcodes
and operands that identify the code. A signature must be **unique** across the
scanned region; the scanner reports zero, one, or many matches, and the resolver
treats "many" as a refusal.

Rules of thumb, applied to the TPF2 profile below:

- Prefer register/immediate operands; wildcard every relative or absolute
  displacement.
- Extend the pattern only as far as needed to make it unique. Two functions can
  share a prologue (the two TPF2 menu functions share a seven-`push` opening);
  run the signature to the first distinguishing bytes.
- Keep the **prologue** field free of wildcards: it is the exact code the detour
  engine relocates, and it is re-checked byte-for-byte after the scan.

## The profile format

A profile is TOML. `tpf3mp_hookcore::profile::Profile::from_toml` parses and
validates it.

```toml
name = "Transport Fever 2 Build 35924 (Windows x64)"
image_base = 0x140000000   # informational: what RVAs are relative to
region = ".text"           # informational: the section the resolver scans

[build]
sha256 = "782b904a8f7bbdac1f7a18528f1a5c778691e5aa3087c37c351bf6912585175c"
size = 72843280            # optional; checked when present
pe_timestamp = 0x675ABCC6  # optional; checked when present

[[target]]
name = "GameSim::Step"
signature = "40 53 41 56 48 83 EC 68 48 8B DA 4C 8B F1 48 81 FA E8 03 00 00"
offset = 0                 # bytes from the match to the target (default 0)
prologue = "40 53 41 56 48 83 EC 68 48 8B DA 4C 8B F1 48 81 FA E8 03 00 00"
required = true            # default true
```

- **`signature`** locates the target. **`offset`** (signed, default 0) is added
  to the match position to reach the target address, for the case where a
  signature must begin before or after the function it names.
- **`prologue`** is the exact, wildcard-free bytes expected at the target; the
  resolver verifies them and the detour engine relocates them.
- **`required`** (default `true`): a required target that does not resolve
  refuses the whole profile.

A resolved target's address is `region_base + match_index + offset`.

## The detour engine

Trampolines are allocated within 1 GiB of their target, so a relocated
RIP-relative operand keeps a 32-bit displacement to the data it addresses.
On Windows the free regions around the target are walked; on Unix, mmap
hints step away from it. A trampoline is written while read-write, then
turned read-execute; it is never writable and executable at once. On
Windows, the prologue read stops at the end of the target's memory region.
A relocation that still cannot reach fails the install cleanly
(`DetourError::Encode`); it never produces wrong code.

`tpf3mp_hookcore::detour::InlineDetour` is an x86-64 inline hook. Installing it
overwrites a function's first instructions with a jump to a replacement, after
copying those instructions into a **trampoline** that ends by jumping back into
the function; calling the trampoline therefore runs the original.

- **Relocation.** The stolen prologue is decoded and re-encoded at the
  trampoline's address with iced-x86's block encoder, so a RIP-relative operand
  keeps addressing the same absolute memory from its new home. A prologue that
  cannot be relocated - it branches, returns, or does not decode - is **refused**
  (`DetourError::UnsupportedPrologue`) rather than patched wrong. Only
  straight-line instructions are stolen.
- **Patch form.** A near replacement (within 2 GiB) is reached with a 5-byte
  `jmp rel32`; otherwise a 14-byte `jmp [rip+0]` absolute jump. The trampoline
  always returns with an absolute jump, so it works at any distance.
- **iced-x86, not `retour`.** `retour`'s stable line is 0.3.1 (0.4 is alpha) and
  it owns trampoline allocation and instruction relocation internally - exactly
  the part that must be inspectable and testable on a binary that shifts every
  patch. iced-x86 is a pure-Rust decoder/encoder with no build script; the
  engine drives decode/relocate directly and keeps the trampoline and patch
  bytes in this crate, where tests read them.
- **Architecture.** The engine is x86-64 only. On any other architecture it
  compiles to a stub that returns `DetourError::UnsupportedArchitecture`, so the
  workspace still builds and the caller fails closed (see
  [macOS arm64](#macos-arm64)).

### Thread-safety assumptions

Installing overwrites up to fourteen live code bytes with a non-atomic copy. The
caller must guarantee the target cannot execute during install or uninstall:
**install before the target's first run**, or **park every thread that could
reach it first**. The launcher loads the hook before the game's entry point
runs (into the suspended game on Windows, by `LD_PRELOAD` on Linux). The hook
installs from its bootstrap thread while the game starts, as TPF2MP's injector
let its worker do: the targets run only once a world is loaded, long after.
The engine does not stop threads itself. Detours are removed by
dropping the handle (or `detach`), under the same quiescence rule. The engine's
own tests only ever hook functions inside the test binary, never another
process.

## The `tpf3mp-ipc` ABI

The hook and the agent share one memory mapping: a fixed 64-byte header followed
by two single-producer/single-consumer ring buffers. This section is
byte-exact, because the agent is written separately.

### Object naming and security

The logical link name is mapped to a per-user OS object:

- **Windows**: `CreateFileMappingW`/`MapViewOfFile` in the per-session `Local\`
  namespace, object name `Local\tpf3mp.<hash>` where `<hash>` includes the user
  name. No explicit security descriptor is passed, so the mapping gets the
  process token's default DACL - access for the creating user and SYSTEM only.
- **Linux/macOS**: `shm_open`/`mmap` with mode `0600` (owner only). The name is
  `/tpf3mp.<hash>`, where `<hash>` includes the uid; it is kept within macOS's
  31-character `shm_open` limit.
- **Other users** cannot reach the link. **The same user's processes can.**
  Creation is not exclusive, because the agent re-creates the mapping after
  a restart while the game still holds it (see "Restart"). A process of the
  same user could therefore create the object first, or write into it. That
  process can already debug or inject into the game, so this opens no new
  boundary. Each side still treats the other's bytes as hostile:
  - `open` refuses sizes `create` would refuse;
  - no frame is read beyond its ring;
  - a producer refuses a consumer index claiming more than the ring holds;
  - `next_len` never exceeds `max_message`.

  A hostile peer can garble or stall the link, but cannot make this side
  read or write out of bounds (`tpf3mp-ipc/tests/poc_hostile_peer.rs`).
  Nor can it make the agent take in, and so delete, a file other than a
  save in the directory the agent named (`tests/hook_save_path.rs` in
  `tpf3mp-agent`). A game that stops reading makes the agent stop taking
  the room's turns once about 16 MiB wait for it, rather than hold them
  all.

### Header layout (little-endian)

Total mapping size is `64 + 2 * ring_capacity` bytes.

| offset | size | field | notes |
|---|---|---|---|
| 0  | 4 | `magic` | `T3MP` (bytes `54 33 4D 50`), written **last** as a readiness flag |
| 4  | 4 | `abi_version` | currently `1` |
| 8  | 4 | `header_size` | `64` |
| 12 | 4 | `ring_capacity` | bytes per ring; power of two, `<= 2^31` |
| 16 | 4 | `max_message` | largest payload per message |
| 20 | 4 | `session` | non-zero link generation; changes on re-create |
| 24 | 4 | `hook_pid` | 0 until the hook attaches |
| 28 | 4 | `agent_pid` | 0 until the agent attaches |
| 32 | 8 | `hook_heartbeat` | `u64`, bumped by the hook |
| 40 | 8 | `agent_heartbeat` | `u64`, bumped by the agent |
| 48 | 4 | `h2a_head` | hook->agent read index (consumer: agent) |
| 52 | 4 | `h2a_tail` | hook->agent write index (producer: hook) |
| 56 | 4 | `a2h_head` | agent->hook read index (consumer: hook) |
| 60 | 4 | `a2h_tail` | agent->hook write index (producer: agent) |

Then the data areas: hook->agent at `[64, 64 + ring_capacity)`, agent->hook at
`[64 + ring_capacity, 64 + 2 * ring_capacity)`.

### Rings

Each ring is a byte stream carrying length-prefixed messages: a little-endian
`u32` payload length followed by that many payload bytes. Both the length and
the payload may wrap around the end of the buffer.

- `head` and `tail` are **free-running** `u32` counters (they wrap at `2^32`,
  not at the capacity). Bytes in the ring = `tail - head` with wrapping
  subtraction; this is correct because capacity is a power of two `<= 2^31`. The
  index into the data area is `counter & (capacity - 1)`.
- **Producer**: writes the payload bytes, then stores `tail` with **Release**.
  It reads `head` with **Acquire** to compute free space; it only writes `tail`.
- **Consumer**: reads `tail` with **Acquire**, reads the bytes, then stores
  `head` with **Release**. It only writes `head`.
- A message larger than `max_message` is rejected by the producer; a full ring
  returns "full". Nothing is allocated on either side of a send or receive.

### Startup, heartbeat and restart

- **Startup.** One side (in TPF3-MP, the agent) is the owner: it creates the
  mapping, zeroes the header, writes the ABI version, ring sizes and a fresh
  non-zero `session`, sets its pid and heartbeat, and **publishes `magic` last**
  with a Release store. The other side opens the mapping and reads `magic` with
  an Acquire load; until it appears the open returns "not ready". The opener
  then checks `abi_version` and `header_size`, reads the ring sizes, and sets
  its own pid and heartbeat. The hook fails closed (runs solo) if no mapping is
  present.
- **Heartbeat.** Each side bumps its own counter and reads the peer's. A counter
  that stops advancing means the peer is gone.
- **Restart.** The owner re-creates the mapping with a new `session`. A peer
  that sees `session` change knows the rings were reset and drops anything in
  flight, then re-syncs from the new generation. The launcher does this for
  every room session (`begin_session`): leaving one room and creating or
  joining another re-creates the link while the game runs on. On Windows
  the new owner re-initialises the very mapping the game still holds; on
  Linux and macOS the old owner unlinks the name when it lets go, and the
  new link is another object, so a peer finds the next generation by
  opening the link again by name. What the hook does about a new
  generation is in "Following the launcher from room to room" below.

### Several games on one PC

The hook opens the link its launcher names, and keeps its log (`hook.log`)
and build profiles (`profiles/*.toml`) in the per-user `TPF3-MP` data
folder. It also carries the release's own profiles, built in from the
repository's `profiles/` folder (Transport Fever 3 Steam build 40408 on
Windows, so far); a profile in the data folder for the same build comes
first, so one can be tried there without a release. The game's environment says which, so several games on one PC each
reach their own agent:

| variable | effect |
|---|---|
| `TPF3MP_GAME_LINK` | the link name to open, the launcher's `--game-link` (`tpf3mp.default` unless given); without it the hook does nothing |
| `TPF3MP_LAUNCHER_PID` | the process that started the game; on Linux and macOS, the hook does nothing in a process whose parent is another |
| `TPF3MP_DATA_DIR` | the folder for the hook's log and profiles; unset or empty, the per-user one |

`tpf3mp-fakegame` reads `TPF3MP_GAME_LINK` too, when no link is given on its
command line. The multiplayer rig (`tpf3mp-rig`, in
[DEVELOPMENT.md](DEVELOPMENT.md)) sets all three for every game it starts, and starts a real
game with the hook in it as the launcher does.

## The bridge: what travels over the link

`tpf3mp-bridge` defines the messages, postcard-encoded, one per ring frame,
at most 60 KiB each. It has no async runtime or network code, so the hook can
link it. The agent's side is `tpf3mp_agent::bridge`.

- **From the agent (`ToHook`):**
  - `Hello`: always first.
  - `Begin`: a game starts, and saves go in this directory. It also names the
    room's rules: with `native`, the game's own economy runs untouched;
    with canonical rules, the Lua mod shows the server's values instead. And
    the local player, as the room's events name the actor: the hook knows
    the player's own commands by it when the room orders them (bridge
    version 5).
  - `Load { file, next_step }`: load a world, then run `next_step`.
    Without a file, the game loads the world the player chose to start
    from: the owner's, or everyone's on a server that keeps no snapshots.
    With one, a save the room agreed on: the owner's world at the start,
    or the room's latest for a player who joins a running game, could no
    longer resume, or is rebased after diverging. Everything sent before
    a load is void.
  - `Apply(event)`: apply this event before its step. A `Save` event is not
    applied: the session saves the world there (see below).
  - `Release { through }`: steps up to and including this one may run.
  - `Speed`: the room's speed, for display only: sent with the game's
    first turn, whatever the speed, and whenever it changes.
  - `Diverged`, `Refused`: tell the player.
  - `Chat { from, text }`: a member of the room said something. Sent only
    once the game has begun; talk in the lobby stays in the launcher.
  - `Room(RoomInfo)`: the room as it stands (its name, owner, and members
    with their names and whether they are connected), for the game's
    Multiplayer window: sent when the game begins and whenever the room
    changes (bridge version 6).
  - `End`: the session is over.
- **From the hook (`ToAgent`):**
  - `Hello`: always first, with the game build.
  - `Loaded { next_step }`: the ordered world is loaded.
  - `Command { payload }`: the player acted; the room orders it.
  - `Ran { step }`: the game ran this step.
  - `Checkpoint { step, lanes }`: digests at a checkpoint.
  - `Saved { event, lanes, file }`: the world as saved at a save event, and
    its digests there; no file if saving failed. The file must be in the
    directory `Begin` named; the agent takes in no other.
  - `Chat { text }`: the player says something to the room
    (`Session::chat`).
  - `Speed { speed }`: the player picked this speed in the game's speed
    row (`Session::request_speed`); the agent asks the room, which takes it
    from the owner only. Bridge version 4.
  - `Log`: a line for the agent's log.
- **The step gate.** The game asks the hook's `Gate` before every step. Until
  the step is released, the hook reads messages and applies each event the
  gate hands over, so an event for step `s` is applied after step `s - 1`
  and before step `s`, never mid-step.
- **Ordering.** The agent sends every event for step `s` after the release of
  step `s - 1` and before the release of step `s`. It only merges releases
  of consecutive steps with no event between them. The hook stops reading
  once its next step is released. The gate refuses anything that breaks
  this: an event for another step, an event after its step's release, or a
  release that goes back. The hook must then stop following and say so.
- **Pacing.** The agent releases steps on its jitter-buffered schedule
  (`Playout`). The game runs a released step at its own speed and waits at
  the gate for the next one. It reports each step it ran; the agent reports
  progress to the server from that, at most every 20 ms.
- **Liveness.** The hook must beat its heartbeat from a thread of its own,
  since the game thread blocks while loading. The agent gives up on a hook
  whose heartbeat stands still for 60 s, or 10 minutes while the world
  loads (`BridgeOptions::hook_timeout`, `load_timeout`), and ends the
  session with "the hook stopped responding". A game the launcher started
  is also watched as a process: once its hook attached, the launcher tells
  the bridge within half a second of the process exiting
  (`Control::GameClosed`), and the session ends the same way with
  "Transport Fever 3 closed", without waiting out those limits, so the
  player can start the game again at once. The heartbeat limits remain for
  games the launcher did not start and for a hook whose game still runs
  but hangs.

### The hook's session

`tpf3mp_bridge::Session` is the hook's whole side of the link, run on the
game thread. The game-specific part of the hook implements the `Game` trait
and calls the session from its detours:

- **Startup.** `Session::attach(DEFAULT_LINK, build, patience)`, then
  `wait_for_begin()`. The gate's first answer is `StepGate::Load`.
- **Loading.** Whenever `before_step` or `poll_step` answers
  `StepGate::Load(load)`, replace the world: with the save `load.file`, or
  without one with the world every player starts from. Then call
  `loaded(load.next_step)`. Call `heartbeat()` while loading.
- **Before each simulation step.**
  - `before_step(&mut game)` blocks until the room releases the step,
    calling `Game::apply` for each event on the way. A pause can hold it
    there for as long as the pause lasts.
  - A game whose simulation shares a thread with its rendering, which must
    never block, calls `poll_step` instead. It returns `Wait` until the
    step is released, and the detour skips the step for that frame.
- **After each step.** `after_step(&mut game)` reports it, and at
  checkpoint steps sends `Game::lanes()`.
- **Saving.** At a save event the session calls `Game::save(file)`, then
  `Game::lanes()`, and reports both. The save must hold everything needed
  to continue from that point, because it is what other players load. The
  agent cuts it into its chunk store and deletes the file.
- **When the player acts.** Capture the action before the game applies it
  locally and call `command(payload)`. For a build the payload is the
  action the Lua mod handed over as a table, converted by
  `tpf3mp_proto::lua` and encoded with `Action::to_payload` ("The action
  schema" in [BUILDING.md](BUILDING.md)). The action happens only when the
  room's event comes back through `Game::apply`, on every replica alike.
- **Notices.** `Game::notice` receives speed changes, refusals,
  divergences and the end of the session, for the game's UI.

The session gives up (`AgentGone`) only when the agent's heartbeat stands
still for its patience, never merely because a step is withheld.

#### Following the launcher from room to room

A game started from the launcher outlives the room it was started in: the
player may leave the room and create or join another while the game runs
on. Until the room's game begins (`Begin`), the session follows:

- `End` before `Begin` means the room session is over, not the game: the
  session stops reading that link and does not check that agent's
  heartbeat any more. A link whose `session` changes before `Begin` means
  the same, whether or not the `End` was read first.
- It then opens the link again by name, at most every 100 ms, until it
  finds a new `session`. There it exchanges hellos as `attach` does: its
  own `Hello` first (then a `Log` line saying it followed), and the new
  agent's `Hello` must be the first thing it reads; anything else is
  refused. Then it waits for that room's `Begin`. The agent needs nothing
  new for this: each room session is a fresh `Bridge`, which expects the
  hook's hello first, exactly once.
- Until the next room is created the game runs on its own, as before any
  room. `try_begin`, which the game's step polls, waits for as long as that
  takes; the blocking `wait_for_begin` gives up after the session's
  patience (`AgentGone`).

Everything else still fails closed. A second `Hello` on the same link
generation is refused, and once a game has begun a new generation is
never followed: every read then checks the link's `session`, and a change
is `SessionError::LinkReset`, on which the hook holds the world. The
bridge's messages did not change for this, so `BRIDGE_VERSION` stays.
(`session::tests` in `tpf3mp-bridge` play these through over a real link.)

`tpf3mp_testkit::fake_hook` implements `Game` for the toy game and runs it
through `Session`: the exact code the real hook will run, over the real
link. The `games_behind_the_bridge_and_gate_agree` scenario runs three of
them in one room end to end; others have a player join a running game,
rebase a replica that drifted, and hand a world on across a server restart.
`tpf3mp-fakegame` does the same as a separate process, for trying the stack
by hand (see [DEVELOPMENT.md](DEVELOPMENT.md)). On release day, what remains for TPF3 is:

- the build profile with its signatures;
- the detours that call the session;
- `Game` for the real world: applying an event means executing the player
  command it carries, the lanes are digests of the game state, and saving
  and loading use the game's own save format;
- checking that a save is complete and loads on every platform
  (DAY_ONE.md);
- checking a received save's script data before the game loads it
  (`tpf3mp_agent::save_check`, DAY_ONE.md section 7);
- on the server, a `Ruleset` that validates TPF3's command format and
  applies the canonical economy, with `save` and `restore` so its rooms'
  logs compact (`crates/tpf3mp-server/src/ruleset.rs`). It is added to
  the server's `RulesMenu` next to `native`, which stays offered.

### The step gate in the game

`tpf3mp-hook` detours the game's `GameSim::Step` (TF3 build 40408:
`0x159390`, from the built-in profile) and hands every call to
`step::StepDriver` (`crates/tpf3mp-hook/src/step.rs`, `install.rs`).

What one call of `GameSim::Step` does decides the design. In TPF2 and TF3
alike it is one batch of the game's own pacing: the main thread
(`CGame::Step`/`CGame::Sync`) calls it on its own schedule, 5 times a second
at 1x, and it runs as many simulation updates as its one call of the speed
getter answers: none while paused (the game's own paused path), one at 1x,
four at 4x. TF3 also adds a pending count, fed by the debug command
`makeGamePerformSimulationStepsCmd` and capped at 64 a call (the global at
`0x403b8e0`). The renderer interpolates from each batch, so **every call
must run**: a call skipped looks like a batch that ran without the world
moving on, the render clock goes back, and TF3 fails
`data.emitCount >= .0f && data.emitCount < 1.0f` in
`UI::particle_manager_util::UpdateParticleSystemInstance` (a negative
particle time step) within seconds of starting a game. The hook therefore
runs the game's step exactly once per call and chooses the answer to its
call of the speed getter, as TPF2MP's speed hook
(`tpf2-multiplayer/native/src/speedhook.cpp`) patched TPF2's step:

- **Before the room begins a game**, and **after it ends**, the game's own
  speed.
- **In the room's game**, the steps the room has released
  (`Session::poll_step`, never blocking the game's thread, then
  `Session::batch`, which reads on through the agent's one-step releases),
  at most 16 a call and never past a checkpoint step, whose lanes must be
  the world's right after it; the call then reports each (`after_step`).
  A room faster than the game's own pace catches up that way, up to 16x.
  When the room withholds the next step (paused, or a player behind), 0:
  the game's paused path, and the world stands still.
- **The room's world.** A `Load` without a file (the world every player
  starts from) takes the world the game has loaded. A `Load` with a save
  file, and a `Save` the room orders, go through the mod's GUI ("The
  room's world" below), every call answering 0 until they are done. Any
  error (the agent gone, a malformed message, a failed report, a world
  that did not load) **holds** the world for good: every call answers 0,
  rather than run apart from the room's (fail closed).
- **Only the step's own call is changed.** The profile's target
  `GameSim::Step/GetSpeed call` (`0x1593ee`) is the step's one call of
  `CGameTime::GetSpeed` (`0x2a95a0`); the hook redirects that call
  (`hookcore::detour::CallRedirect`, which checks the call targets the
  getter) to answer the chosen count. The getter's six other callers
  (`CGame::Sync`, the UI, the camera, the particles) read the game's own
  speed: telling them 1 when the speed row says otherwise also trips the
  particle assertion. The game's own speed and pause therefore change
  nothing in the room's game but the display, until the room follows them.
  Nothing may send the debug step command during a room: its pending count
  would add updates to a call.
- **The speed row asks the room.** The getter's detour only reads: the
  game's own speed, the speed row's value, every time the game asks
  (`CGame::Sync` and the game UI ask every frame, paused or not). When it
  changes in the room's game, the hook sends `ToAgent::Speed` and the agent
  asks the room (`Request::SetSpeed`): the owner's choice sets the room's
  speed for everyone, and anyone else's is refused, shown as a notice. The
  value found on entering the room's game is not sent, so joining never
  resets a room's speed. What the room says back (its speed, a refusal, the
  end) goes to the hook's log.

Measured in TF3 build 40408 on release day (the rig with one player,
`app.startGame()` from the console, the game's speed set with
`makeGameSetSpeedCmd` as the speed row does, `GameTime.updateCount` read
before and after): 4.97 updates a second at 1x, 20.34 at 4x, 10.02 at 2x,
none paused, 4.98 back at 1x, the room confirming each change within a
second, and no assertion in several minutes of play.

The hook installs the detours from its bootstrap thread while the game
starts, before any world is loaded, so no thread is inside the step when
it is patched. It resolves the profile in the game's own mapped image
(not the file on disk), attaches the `Session` to the launcher's link, and
calls the game's step through the detour's trampoline with all four
register arguments passed through unchanged. A panic in the detour sets it
to hold. Tests: `step::tests` drive the driver against a scripted room;
`install::tests` detours a stand-in step in the test binary and checks it
runs exactly once per call with the count chosen and its arguments
untouched; `session::tests` read releases ahead over a real link;
`detour::x86_64::tests` redirect one call in hand-written code.

### The Lua side

The mod runs in two kinds of Lua state: the game's GUI state, started by a
game bar plugin on the first frame of a game
(`mod/tpf3mp_1/content/gui/tpf3mp/`), and the engine states the game runs
game scripts in (`mod/tpf3mp_1/content/tpf3mp_sim/`, "Actions in the
game" below). The hook and the mod meet through one global table,
`tpf3mp_native`, which the hook gives every Lua state that calls `print`:
it detours Lua's `luaB_print` (profile target `luaB_print`) and, after the
game's print, adds the table to that state's globals, once, set raw past
any metatable a strict state gives them. The mod prints before it looks
for the table (`bridge.find`). Its contract is in
`mod/tpf3mp_1/content/scripts/tpf3mp/bridge.lua`; the hook's half is
`crates/tpf3mp-hook/src/lua.rs`:

- `tpf3mp_native.version`: 8. The mod refuses any other.
- `tpf3mp_native.command(action)`: an action table, in the game's units.
  The hook reads it into a `tpf3mp_proto::lua::LuaValue`, within
  `MAX_DEPTH` and `MAX_NODES` (a function, userdata or a table as a key is
  refused), converts it with `action_from_lua` and queues
  `Action::to_payload`; the step gate hands it to `Session::command`, in
  the room's game only. It returns `true` and a ticket, or `false` and why.
  `false` or an error means refused, and the mod does not apply the action
  locally either: every game applies it when the room orders it. The
  ticket comes back in `results()`.
- `tpf3mp_native.take()`: the actions the room ordered for this simulation
  update, as `action_to_lua` tables, or `nil` (below). A list's items are
  in its table's array part, so `next` walks them in order. The game
  copies a list it is handed (a stop's loading flags, a consist's groups)
  into its own vector in the order `next` gives, and what a game script's
  `update` returns reaches `postUpdate` as the game's own copy, whose
  lists `next` walks in hash order all the same (build 40408: a bus line's
  stops set to load grain, one cargo over from passengers). So `apply.lua`
  hands the game every such list afresh, filled in order (`seq`).
- `tpf3mp_native.log(line)`: a line for `hook.log`, marked `mod:`.
- `tpf3mp_native.poll()`: in the GUI, every frame: what the hook asks of
  it, once, `{ save = name }` or `{ load = name }`, or `nil` ("The room's
  world" below).
- `tpf3mp_native.saved(name, ok, why)`: the GUI's answer to a save.
- `tpf3mp_native.world()`: a world's GUI started.
- `tpf3mp_native.room()`: whether the room's game runs (the step gate's
  phase, held included), for the guard ("The player's commands" below).
- `tpf3mp_native.checkpoint()`: in a game script's update, whether it is
  the last of a batch that ends at a checkpoint step ("The world's lanes"
  below).
- `tpf3mp_native.lanes(t)`: the lanes read there, a table from lane
  numbers to strings. Returns `true`, or `false` and why.
- `tpf3mp_native.clicks()`: the player's builds queued in the room's game
  so far, or `nil` where the hook cannot take them to the room ("The build
  tools" below).
- `tpf3mp_native.replaying(on)`: the game script begins or ends applying
  the room's actions, whose builds the hook lets through.
- `tpf3mp_native.applied(index, ok, entity, why)`: in the game script's
  `postUpdate`, after the batch's action `index` (from 1): whether it went,
  the entity it made, if any, and why not.
- `tpf3mp_native.status()`: in the GUI: the room for the Multiplayer
  window, `{ room =, speed =, diverged =, players = { { name =,
  connected =, owner =, me = } } }`, or nil before the room's game. The
  hook keeps what the room tells it (`Room`, `Speed`, `Diverged`), and
  forgets the divergence when a world loads (bridge version 9).
- `tpf3mp_native.chat()`: in the GUI: what the room's members said since
  the last call, `{ { from =, text =, old = } }`, oldest first, 64 lines
  at most. A world's GUI starts with none of the chat so far, so after
  `world()` the next call first gives the last 50 lines taken before
  again, with `old` set.
- `tpf3mp_native.say(text)`: in the GUI: says `text` (280 bytes at most,
  trimmed) to the room for the player, `true` or `false` and why; the step
  driver sends it (`Session::chat`) in the room's game only.
- `tpf3mp_native.results()`: in the GUI: what became of the player's own
  actions since the last call, `{ { ticket =, ok =, entity =, why = } }`,
  oldest first. The step driver knows the player's own actions when the
  room orders them back: the room's event names the player (`Begin`'s
  `player`) and the client sequence number, which `Session::command`
  returned when the driver handed the action over, and which it keeps with
  the action's ticket. An action the room refuses (`Notice::Refused`), or
  one handed over when no room's game runs, answers `ok = false` with why. on whichever thread runs their state (the GUI's
the main thread, the game scripts' a pool of simulation threads) and share
nothing but the hook's queues. They reach Lua through Lua 5.2's C API as
the build profile names it: 18 functions besides `luaB_print`, found from
`luaB_print`'s own calls and their places in lapi.c, which the linker laid
out alphabetically (TPF2's were too; several prologues match TPF2's byte
for byte). They are only called, never detoured. Everything that crosses
into Lua is `C-unwind`: a Lua error, which TF3 raises as a C++ exception,
passes through as the game raised it.

Without the table, the mod logs "no hook in this game" and does nothing.
That is every game Steam started (D11).

### Actions in the game

A player's action happens in no game until the room orders it, and then in
every game in the same simulation update:

1. The mod hands the action to `tpf3mp_native.command`. The step gate
   sends it to the room.
2. The room orders it as an event for a step `s`. The session ends a
   batch before every step with events (`Session::batch`), so `s` is
   always the first update of a batch; the driver hands that batch its
   actions (`lua::begin_batch`), and runs it.
3. The mod's game script asks the hook in every `update`
   (`tpf3mp_native.take`); the first update of the batch gets the actions
   and returns them, and its `postUpdate` applies them through `api.cmd`
   (`mod/tpf3mp_1/content/scripts/tpf3mp/apply.lua`).
4. After the batch, the driver checks the actions were taken
   (`lua::end_batch`). If they were not, the world ran step `s` without
   them: none of those steps is reported and the world stands still
   (fail closed).

Measured on build 40408:

- A mod's game script (`*.gs.lua`, discovered by the game) has its
  `update` called once per simulation update: `GameTime.updateCount` goes
  up by one from call to call, and `dt` is 0.2. Its script file defines
  `data()` returning the functions, as the GUI's do.
- The game runs game scripts on a pool of Lua states ("Sim Pool" threads),
  so a script's locals are kept per state, not per game.
- The game's own scripts decide in `update` and change the world in
  `postUpdate`, which the game calls with what `update` returned, and not
  when that is nil: the company script reads its argument unchecked, and
  a lane read from a `postUpdate` after an update that returned nothing
  never reached the hook. The mod's game script does the same, so the
  world changes only in `postUpdate`, not while other scripts' updates may
  run beside it.
- In an engine state a command runs at once (the game's
  `api/tealdef/api/cmd.d.tl`). The game takes no callback in `update`
  ("Callbacks are currently disallowed"), but in `postUpdate` it calls one
  at once, with the command's result: the game's mission scripts read the
  line they made from it right after `sendCommand`
  (`mission_vehicle_util.tl`). `apply.lua` sends every command with one
  there: a command the game answers as failed fails the action in every
  game, and the entity a command made (`resultEntity`,
  `resultVehicleEntity`, else the first of its result entities) is what
  the action made. A state where the game refuses the callback sends
  without one, logs so once, and leaves finding what an action made to
  the registry. A refused command raises. In the GUI and console states
  commands run "in the next simulation step", which is a different update
  on each game: the reason the game script applies them, not the GUI.
- The console has a Lua state of its own. For tests, its
  `api.cmd.makeScriptingSendEventCmd("", "tpf3mp", "command", action)`
  reaches the game script's `handleEvent` in that game only, which hands
  the action to the room.

Seen in two hooked games in one room (the rig, the fixture save): a road
depot sent from the first game's console was handed to the room, and both
games applied it in the same update; the determinism probe's construction
lane changed between steps 500 and 600 in both, to the same value, and
matched at every sample after.

Seen on build 40408, two games through the deployed server: a loan
taken in the guest's finance window went to the room, and both games
booked it in the same update, the account and the loan script's list of
loans alike in both; the loan script's own callback, which books the
money, ran in the game script's `postUpdate`.

`apply.lua` applies, among others:

- `BuildConstruction`: a `SimpleProposal` with one `ConstructionEntity`
  (the file, the matrix from the transform, the parameters from their
  flattened paths, the name, the player) and, in its street proposal, the
  construction's connection, resolved as a road build's polyline, less
  the construction's own entrance (a new vertex at the end of a single
  link), sent with a `Context` naming the
  player, who pays, and gathering the town buildings and fields in its
  way, and `playerInitiated` true, as the player's own build. The game's
  verdict comes first (`makeProposalData`): a critical error refuses the
  build, with its reasons, in every game; warnings (town buildings to
  remove, reputation lost) are logged and built through, as the tool
  builds once the player clicks (`ignoreErrors` true: with it false the
  game dropped such a build without a word, seen on build 40408);
- `Loan`: the loan script's own event, `makeScriptingSendEventCmd("",
  "Loan", "Obtain", { next, offer })` or `"Repay", { nil, loan }`, with the
  tables the finance window sends.

Every other action is refused with a line in `hook.log`, the same on every
game, so the worlds stay alike. The native build tools come next.

### The room's world

A room plays its owner's world. On a server that keeps worlds, the room
has the owner's game save it before step 1, and every other player's game
loads that save; a player who joins later loads the room's latest one
("The first world" in [PROTOCOL.md](PROTOCOL.md)). Transport Fever 3 saves and loads only through its GUI's
script API (`app.saveGame`, `app.loadGame`) and only in its own save
folder, so the hook asks the mod's GUI for both
(`crates/tpf3mp-hook/src/worlds.rs`,
`mod/tpf3mp_1/content/gui/tpf3mp/tpf3mp.script.lua`), and the world
stands still meanwhile:

- **Saving.** The session answers `StepGate::Save(order)` until the save
  is reported (`Session::saved`). The driver asks the GUI to save under a
  name of this game's own, `tpf3mp_<pid>_<event>`, as two games on one PC
  share the folder. The GUI calls `app.saveGame(name, callback, false,
  true)` (the last argument leaves the player's own save name alone) and
  answers through `tpf3mp_native.saved` from the callback, once the file
  is written. The hook finds `<name>.sav`, removes the picture the game
  writes beside it, and moves the save to `order.file`, which the agent
  cuts into its store. A save the game refuses, or does not make within
  `SAVE_PATIENCE` (120 s), is reported failed, and the game goes on.
- **Loading.** A `Load` with a file (the room's save, fetched by the
  agent) is copied into the folder as `tpf3mp_room_<pid>.sav`, and the GUI
  asked to load it (`app.loadGame`, with a `SavegameId` in the game's save
  namespace). The GUI tells the hook each time a world's GUI starts
  (`tpf3mp_native.world`); the room's world is the first to start after
  the GUI took the request, never the one it was asked in, whose GUI may
  well report itself in between. Then `Session::loaded(next_step)`, and
  the room's steps run on. A world not up within `LOAD_PATIENCE` (600 s)
  is held.
- **The folder** is Steam's for the account playing,
  `<Steam>/userdata/<account>/3493540/local/save`, from the registry
  (Steam's `SteamPath` and `ActiveProcess\ActiveUser`), or else the one
  account with a save folder for the game. Without one, a load holds the
  world and a save is reported failed.

The GUI runs only in a world, so a game must be in one, any one, before it
can load the room's. Loading from the main menu (`CMenuUI::StartSavegame`,
in the profile) is next.

Tried on build 40408 through the deployed server, with two games on one PC
(the rig, the fixture save): the owner's game saved its world for the room
in 215 ms and reported it within a second. The guest fetched the save,
its GUI took the load in the world it was in, and the room's world was up
4 s later and played from step 1. A road depot sent from the owner's
console was applied by both games, and the determinism probe's 15 samples
from step 500 to 1900 matched in every lane it reads, the construction and
money lanes changing alike after the build
(`investigation/dayone-2026-09-29/6-determinism.md`; the edge lane is
still unread there).

### The player's commands

In the room's game a player's command runs in every game at the same
update, or in none. Transport Fever 3's GUI sends most of what a player
does from its own Lua state, through `api.cmd.sendCommand` (build 40408's
scripts): buying, selling, replacing and assigning vehicles and making and
editing lines (`gui/line_vehicle_mgmt/`, `gui/entity_window/`), a
construction's parameters and bridges and tunnels
(`makeWorldBuildProposalCmd`, from `gui/construction/construction.tl` and
the entity windows), loans and other game mechanics as script events
(`makeScriptingSendEventCmd`), and the speed row (`makeGameSetSpeedCmd`).
The street and track tools, and placing a construction, are native
builders that reach the command queue (`CommandList::Add`) without Lua.

So the stock windows do send through `api.cmd.sendCommand`, as the mod's
replays do (the question in PLAN.md, Part 2). What tells them apart is the
Lua state, not a caller's address: the player's commands come from the
GUI's state, the room's replays from the game script's states, whose
`api.cmd` the mod leaves alone.

The mod guards the build tools in its game script, whose `guiHandleEvent`
runs in the GUI's state. The street, track, station and depot, stop and
bulldozer tools tell game scripts of every proposal they make
(`builder.proposalCreate`), and they honour an error a script returns, as
the game's company script refuses constructions without a permit
(`game_mechanics/company/company.script.tl`). In the room's game the mod's
script returns "Not in multiplayer yet: building with this tool" for every
proposal of a tool the room does not carry yet, and the tool shows it in
red and builds nothing (seen on build 40408, with the street tool). A tool
the room carries builds through it instead ("The build tools" below). The
script subscribes to those events by name, as a save may carry an older
version's subscriptions.

The mod guards the GUI's commands
(`mod/tpf3mp_1/content/scripts/tpf3mp/guard.lua`). `api.cmd` is a plain
table whose factories and `sendCommand` (a callable table) can be replaced,
as the console's state showed, and none of the game's 6,041 scripts keeps a
reference of its own to either. Once linked, the GUI wraps every
`make*Cmd` factory, to note which one made each command, and
`sendCommand`. While the room's game runs (`tpf3mp_native.room()`):

- a command of a kind in `guard.PASS` is sent, its arguments untouched. So
  far that is the speed row's `makeGameSetSpeedCmd`, which the step gate
  reads as the player's request to the room;
- a command `guard.CARRY` makes an action of goes to the room instead
  (`tpf3mp_native.command`), which orders it for every game, this one
  included ("Actions in the game"). Its callback hears what became of it
  once this game has applied it (`results()`, `guard.deliver`), as the
  game's own command would answer: `(data, ok, {{ entity, 0 }})`, with the
  entity the action made. The game's windows chain on that: the store
  puts the vehicle it bought on a line by the entity its callback hears
  (`resultVehicleEntity`), the line manager opens the new line
  (`resultEntities[1][1]`); told nothing, or told before its world had the
  line, the line manager made a second line at the next stop clicked (seen
  on build 40408). So an answer waits until the GUI's world has the entity
  it names (`api.engine.entityExists`; the game script made it in the
  simulation), a few seconds at most, and answers keep their order; a
  command that should have made something and made nothing the game could
  name is answered as failed, which the windows handle. With both, a new
  line took its stops one by one as in single player (build 40408). So
  far:
  - loans, the finance window's `makeScriptingSendEventCmd("", "Loan",
    "Obtain" | "Repay", …)`, as a `Loan` action carrying the loans' terms,
    which every game's game script replays through the loan script's own
    event;
  - vehicles: buying (`makeVehicleBuyCmd`: the depot by its construction's
    file and position, the consist part by part, as the store configured
    it), selling, putting on a line, and the vehicle window's stop, start,
    to the depot (sold there or not), reverse and depart;
  - lines: creating, changing (the line whole, as the line manager built
    it: stops, terminals, loading rules), deleting, renaming and
    recolouring.

  Vehicles, lines and station groups have no place to name them by, so
  actions name them by canonical id (`tpf3mp/registry.lua`). Every game
  gives them the same ids without telling another: every game runs the
  same world, so after the same action the same things exist, and the
  mod's game script binds each new one to the next id of its kind, lowest
  entity first, after every action the room orders and at the room's
  first update, and retires the ids of those gone; an id never comes back.
  What an action made, as the game answered its command, is bound at
  once, whatever the game's lists say, and an id is retired only when its
  entity no longer exists, never because a list left it out.
  It keeps the registry in its state, which the game saves with the world,
  so a player who joins or reloads from the room's save has it as the
  others do. The GUI reads it from the script's state
  (`gameScriptSystem.getEntityForGameScript`, the `GAME_SCRIPT`
  component), as the loan window reads the loan script's. A command
  naming something the registry has no id for is refused, and says so;
- any other kind, and a command no wrapped factory made, is refused, as
  PLAN.md (Part 3) says of every action whose strict flag is off. It is
  not sent. Its callback, if it has one, is called on the next frame with
  `(command, false, {})`, as the game answers a command that failed. The
  game bar shows "Not in multiplayer yet: …" for a few seconds, and
  `hook.log` gets a line for the first refusal of each kind and every
  hundredth after.

Before the room begins, and after it ends, every command is sent as it
would be, and every tool builds. A kind the room comes to carry is
captured into an action instead of refused, and applied by every game
("Actions in the game").

### The build tools

The street, track and construction tools are native: a click queues a
`WorldBuildProposal` command, which the simulation applies at its next
step, in that game alone. Nothing a script does stops one on build 40408:
the street tool sends no `builder.proposalPrepareForApply` (a click is
`proposalCreate` twice, then the simulation's `onPreBuildProposal` and
`onPostBuildProposal`, then the GUI's `proposalApply`), an error raised in
`onPreBuildProposal` is logged and the build goes on, and emptying the
proposal's lists there crashes the game (found with
`tools/probe/tf3/tpf3mp_buildprobe_1`). So the hook stops the player's
builds natively (`crates/tpf3mp-hook/src/builds.rs`, profile targets
`CommandList::Add` and `WorldBuildProposal apply`):

- **At the click**, `CommandList::Add`, on the main thread: a command whose
  payload is a `WorldBuildProposal` (its variant index, at payload +
  0x9b8, is 52) with `playerInitiated` set (payload + 0x3d2) is counted, in
  the room's game only, and added as the game would.
- **At the apply**, the simulation's apply of a `WorldBuildProposal` (the
  command dispatcher's case 53, `0x9e1160`): a player-initiated build in
  the room's game answers false, as a build the game refused, and the game
  tells the tool so through its own path. The room's own builds go
  through: the mod's game script applies them with
  `tpf3mp_native.replaying(true)`. Towns' growth and the game's scripts,
  not player-initiated, go through as ever.

The mod's game script, in the GUI (`guiHandleEvent`), keeps the action each
proposal of a tool the room carries makes
(`mod/tpf3mp_1/content/scripts/tpf3mp/capture.lua`), marked with the clicks
counted when it saw it (`tpf3mp_native.clicks()`). Its `guiUpdate` hands
the room the one each click saw last: the proposal and the click both run
on the main thread, in order, so that is the proposal clicked. The room
orders it for every game, this one included, and each game's `postUpdate`
builds it, paid by the player (`Context.player`) and clearing town
buildings in its way (`gatherBuildings`), as the tool builds; without a
context the game builds for free.

Four tools build through the room so far:

- **The construction tool** (`constructionBuilder`): a proposal of one
  construction (a station, a depot, anything the tool places) becomes a
  `BuildConstruction`, with the file, the transform, the parameters
  flattened and the game's name for it. The streets in its proposal are
  what the tool built around the construction, and travel with it as its
  connection: a bus station placed by a road rebuilds the road through a
  new junction and adds an entrance edge from the junction to the
  station's own street node (seen on build 40408, the construction's
  frozen nodes and edges empty in the proposal). Built without them, the
  station stood beside the road, its entrance a dead end, and the line
  manager could not connect it ("Could Not Connect Stations"). The
  entrance edge in the tool's proposal is the construction's own, snapped
  onto the road: every station and depot has one frozen node and one
  frozen edge, its entrance, and a scripted build makes that edge again
  unsnapped, ending about 2 m short of the road (build 40408, read from
  the console). The game's refresh of the construction
  (`api.engine.util.proposal.refreshConstruction`) snaps it as the tool
  does: its proposal's entrance edge ends at the road's node. So the
  replay builds, in one action, the construction with the rest of the
  connection (the road rebuilt through the junction), then the refresh of
  the new construction, free (no context) and not as a click of the
  player's; the refresh finds the junction the tool chose. The town
  buildings in its way the replay clears again (`gatherBuildings`, and
  `gatherFields` for fields). Several
  constructions at once, or one replacing a construction that is not a town
  building (a module edit), is refused. Before sending, the replay asks the
  game's verdict (`makeProposalData`) and refuses what it calls critical,
  with its reasons.
- **The street and track tools** (`streetBuilder`, `trackBuilder`): the
  proposal becomes a `BuildRoad` or `BuildTrack`, as the tool made it, by
  positions (docs/BUILDING.md, "The action schema"): the nodes and edges it
  adds, each edge in its own kind (the street it joins is rebuilt through
  the new junction in that street's template), and the edges and nodes it
  removes. The replay builds it as the game's own scripted track builder
  does, `nodesToRemove` included. A build that moves or removes an edge
  with a stop or signal on it, or that places stops, signals or
  constructions, is refused.
- **The bulldozer** (`bulldozer`): its proposal removes one construction
  (with the construction's own entrance edge and node) or edges of one
  network (with the nodes they leave on their own). It becomes a
  `Bulldoze`: the construction by its file and position, or the edges by
  their ends. The replay removes them as the game makes such a removal
  itself, `createProposalRemove` for the construction and
  `makeSegmentsRemoveProposal` for the edges (on build 40408 the first
  gave exactly the bulldozer's proposal), and the player pays. Removing a
  stop or signal, or an edge with one on it, is refused.

A refusal shows its reason in the tool, and the log has each new reason
with the proposal's shape (`the room cannot carry this ... build`); every
build handed to the room is logged with its shape too. The stop tool, and
the upgrade, bus lane and tram track tools, stay refused until their builds
are captured. Where the profile lacks the
two targets, `clicks()` is nil and every tool stays refused.

Seen on build 40408, through the deployed server with two games on one PC:
a maintenance building placed with the construction tool in the guest's
game was stopped there, handed to the room, and built in both games in the
same update; both accounts paid its $180,336, and the room found no
divergence. The same for a street across open ground ($71,820), a street
onto an existing junction ($94,231), a street onto another's middle, a
track across open ground ($22,054), a track across a street, and a bus
depot snapped onto a town street, clearing three town buildings
($825,816): identical in both games, towns included.

### The world's lanes

A room finds a game that drifted from the others by comparing the world's
lanes at every checkpoint step (`checkpoint_interval`, 50 steps by
default): digests of parts of the world, which the session reports with
the step (`Session::after_step`, `Game::lanes`). The lanes are read by the
mod's game script, which sees the world between updates:

- The session ends every batch at a checkpoint step, so a checkpoint is
  always a batch's last update. The driver knows the step a batch starts
  at (`RoomGate::next_step`) and so whether it ends at one, and tells the
  hook's Lua side (`lua::begin_batch`), which counts the batch's updates by
  their `take()`.
- In that last update, `tpf3mp_native.checkpoint()` answers true; the game
  script's `update` returns that, and its `postUpdate` reads the lanes
  (`mod/tpf3mp_1/content/scripts/tpf3mp/lanes.lua`) and hands them over
  (`tpf3mp_native.lanes`).
- After the batch the driver takes them (`lua::end_batch`), makes a
  SHA-256 digest of each lane's text (`step::lane_digests`), and the
  session reports them for the checkpoint step. A batch that ended at a
  checkpoint without them holds the world: the room could not tell whether
  it is still its own (fail closed).

The lanes, numbered as the regression harness's model numbers its own
(`crates/tpf3mp-testkit/src/regress/model.rs`, `lane`), each a count and a
hash of sorted rows, so the order the engine lists things in does not
matter:

| lane | reads |
|---|---|
| 0 network | every street and track edge by its ends (0.1 m) and road template, from the street system's node map |
| 1 constructions | every construction by its file and position (0.1 m) |
| 2 lines | every line's number of stops |
| 3 vehicles | each vehicle's state, stop and place on its path: the path edge, the distance along it (1 cm) and the speed (1 cm/s), the simulation's own (`MOVE_PATH.dyn`) |
| 4 economy | the player's balance |
| 5 towns | each town's number of buildings |
| 6 people | the number of people |

Nothing is read by an entity id that two games agreeing on the world could
number differently, except where the save carries it (towns, the player).
A lane the engine cannot read is `err` on every game alike and says why in
`hook.log`, once per Lua state. On build 40408 `getEntitiesWithComponent`
refuses `BASE_EDGE`, `LINE` and `PLAYER` ("Cannot loop over this component
type"), hence the street and line systems.

Vehicles are compared by their place on their paths, not in the world. On
build 40408, with a bus running a line in two games in one room, the bus's
world position (`api.engine.util.vehicle.getPosition`) and its path state
as the frame began (`MOVE_PATH.dyn0`) differed between the games in the
same simulation update, by millimetres to metres: they follow each game's
own frames. Its path state (`MOVE_PATH.dyn`) was the same in every sample.
Read to 1 m, the world position flipped at a rounding edge now and then,
and the room resynced a game whose simulation had not diverged.

Seen on build 40408, two games through the deployed server: every lane
read, and the room found no divergence over several checkpoints.

## Release-day procedure: adding a target for a new build

The first TF3 build's targets are already located (RVAs, RTTI/source
names) in
[investigation/TPF3_RECON_2026-09-29.md](../investigation/TPF3_RECON_2026-09-29.md):
the command queue (`CommandList::Add`), the sim step (`GameSim::Step`,
`CGame::RunGameSimLoop`), `CGameTime`, the two-`GameState` swap, the
player/company commands, and a lockstep step-budget global. This procedure
turns each into a verified profile target; the recon page also lists the
reconciliations to settle in-game first (e.g. `CommandList` vs
`DeferredCommandBuffer`).

1. **Archive the build.** Record the executable SHA-256, file size and PE
   timestamp (`BuildIdentity::of_file`), plus the Steam build/manifest ids. Keep
   a private copy (see [DAY_ONE.md](DAY_ONE.md)).
2. **Find the function** with the RE pipeline, and note its RVA and the bytes at
   its start. `tools/tpfre` indexes the executable in seconds and answers
   `func`, `callers`, `xrefs`, `str`, `dis` and `whois` queries on it
   ([its README](../tools/tpfre/README.md)).
3. **Write a signature.** Take the opening bytes; replace every relative or
   absolute displacement with `??`; extend only until the pattern is unique
   across the scanned section. Record the exact, wildcard-free `prologue` (at
   least the number of bytes the detour must steal - 5 for a near hook, 14 for a
   far one, on an instruction boundary).
4. **Add a `[[target]]`** to the build's profile with `name`, `signature`,
   `offset`, `prologue` and `required`.

   `tools/re/make_profile.py` does steps 3 and 4 for x86-64 builds, from the
   binary and the symbol map `name_functions.py` wrote:

   ```
   python tools/re/make_profile.py TransportFever3.exe out/TransportFever3.symbols.json \
       GameSim::Step CGame::Step -o profile.toml
   ```

   It writes the `[build]` identity (SHA-256, size, and the PE timestamp on a
   PE) and one target per function, with `offset = 0`. Displacements it
   wildcards: branch and call targets (rel8 and rel32), RIP-relative operands,
   and immediates or absolute displacements that point into the image. The
   signature starts as the prologue's instructions and grows one instruction at
   a time until it matches once in the function's on-disk section, never past
   the function's end or `--max-length` (128) bytes. The prologue covers
   `--steal` bytes, 14 by default: a far jump, since how far the detour lands is
   only known at install. Like the engine, it refuses a prologue holding a
   branch, call, return or interrupt, and keeps RIP-relative data operands,
   which the engine relocates. Every refusal names the target and the reason: a
   name shared by several functions (pick one with `NAME@0xRVA`), a function
   byte-identical to another (never unique), a branch too early to steal around.
   `tpfre q <db> sig NAME --toml` applies the same rules to one function and
   prints its `[[target]]` block (identical to make_profile's on TPF2's
   targets), to try a target before writing the profile.
   `tools/re/test_make_profile.py` checks the tool on a synthetic PE and keeps
   `tpf3mp-hookcore/tests/data/make_profile_fixture.{pe,toml}` current, which
   `tests/make_profile_fixture.rs` resolves with hookcore itself.
5. **Verify.** Resolve the profile against the **in-memory module image** of the
   running build and confirm the target resolves uniquely to the expected
   address and that the prologue matches. Keep a static check against an
   archived copy where the code section is readable on disk.
6. **Never widen a signature to force a match** on a build you have not archived.
   An unknown build must stay unknown, so the hook fails closed.

## What was verified on the TPF2 binary

Against `TransportFever2.exe`, Steam build 35924 (SHA-256
`782b904a...585175c`, size 72,843,280, PE timestamp `0x675ABCC6`, image base
`0x140000000`), the profile in `tpf3mp-hookcore/tests/data/tpf2_build35924.toml`
resolves all five targets, each **matching exactly once** across `.text`:

| target | RVA |
|---|---|
| `GameSim::Step` | `0x15aa00` |
| `CGame::Step` | `0x118e90` |
| `CGameTime::GetSpeed` | `0x2877a0` |
| `UI::CMenuUI::StartSavegame` | `0x6785c0` |
| `UI::CMenuUI::CreatePage` | `0x663370` |

`CGameTime::GetSpeed` sits next to two near-identical siblings, so its signature
runs past the (wildcarded) call to the distinguishing `mov eax,[rax+4]`;
`StartSavegame` and `CreatePage` share a seven-`push` prologue, so each signature
runs to its distinct `lea`/frame bytes (and `CreatePage` to the `mov
[rsp+0x330],rbx` store that separates it from a twin at `0x215c480`). The test
also confirms the resolver refuses a modified copy (corrupting one target's
bytes yields a `Missing` refusal) and refuses a mismatched build identity.

**DRM note.** This build carries a SteamStub section (`.bind`, high entropy),
which can decrypt code at load time. For build 35924 the code section is
nonetheless **readable on disk**: all five prologues match the on-disk `.text`
exactly, consistent with the RE survey's ~88,000 assert-string references found
in the same on-disk section. On-disk verification is therefore valid *for this
build*. It is not guaranteed in general - a future build could encrypt `.text` -
which is why the production resolver scans the in-memory, unpacked module image,
and why on-disk scanning is documented as a development convenience only.

## What a shipped mod hooks on the same build

Build 35924 has a shipped lockstep mod hooking it, [TpF2 Multiplayer](https://github.com/silver2127/tpf2-multiplayer)
(0.6.1.12, 2026-09-20), so every target in the profile and everything in this
section runs in players' games rather than in a test. RVAs are from image base
`0x140000000`. Names are the ones the binary carries in its `__FUNCSIG__` assert
strings where it has one (`tools/re/name_functions.py` recovers those); the
rest are the mod's own names for functions it identified by decompiling or by
differential capture. Steal sizes are the bytes that mod's detour engine
overwrites; its engine refuses RIP-relative instructions in a prologue rather
than relocating them, so its steals are a conservative bound for one that does.

### The five profile targets, as the mod uses them

| target | RVA | how the mod uses it |
|---|---|---|
| `GameSim::Step` | `0x15aa00` | Not detoured whole. Two sites inside it are patched: the calls to `CGameTime::GetSpeed` at `0x15aa30` and `0x15aae4` (a fractional speed scales the batch interval) and the paused branch's `call 0xaea970` (GameTime advance) at `0x15aa4a`, so a paused game advances `GameTime+0x30` per simulation step and not per render batch. Both siblings of `GetSpeed` are real: the profile's extended signature is the right call. |
| `CGame::Step` | `0x118e90` | Detoured, 16-byte steal, for pacing (the leader is the clock; joiners pace to it). |
| `CGameTime::GetSpeed` | `0x2877a0` | Read through its call sites rather than hooked; `GameTime::get` at `0x2877c0` returns the counter at `+0x30`. |
| `UI::CMenuUI::StartSavegame` | `0x6785c0` | Detoured (the share observer: a host that loads another world pushes it), and called directly to load a shared save in-process: build a `SaveGameId` `{wstring path; string name; string namespace}`, get its `SavegameInfo` from the save manager (`0x2e6ca0`; the manager is `+200` on the app object from `0xbb23c0`), default-construct `LoadGameParams` (`0x553b70`, 0x138 bytes) and call from `CMenuUI`'s own per-frame update, vftable `0x301dc38` slot 33 (`0x672b10`), on the main thread, where the game starts its own queued loads. Guards on the menu object: `+0x4e8` non-zero while a game runs, `+0x1988` "initialization already active", `+0x19a0` a queued load. |
| `UI::CMenuUI::CreatePage` | `0x663370` | Detoured, 20-byte steal (the Multiplayer panel on the title menu). Two more menu entries go with it: the list-add at `0x22d99e0` (15) and the main-page builder at `0x667bc0` (14). |

### The command pipeline: two hooks, not one

TF3's command surface is now documented, not guessed: Urban Games'
reference lists **61 `api.cmd.make*Cmd` factories** with their argument
types, recorded in
[investigation/TF3_OFFICIAL_API_2026-09-29.md](../investigation/TF3_OFFICIAL_API_2026-09-29.md).
A TF3 profile's factory targets are found for that list, not ported name
for name from TPF2's. Two entries change the design directly:
`makeWorldBuildProposalCmd` takes a fifth `playerInitiated` argument (a
possible player-vs-replay signal, see below), and companies are commands
(`makeGameAddPlayerCmd`, `makeEntitySetPlayerCmd`), so ownership changes go
through this same pipeline rather than the native, assert-bypassed
`setPlayer` binding TPF2 patched.

Every player action becomes a `Command` built by a `make_cmd::*` factory and
handed to `CommandList::Add(list, OUT handle, cmd, ..., callback)`. The mod
hooks both, and the reason is worth carrying into a TPF3 profile:

- The **factory** hook sees *what* the command is, while its arguments are still
  the caller's typed structures (a proposal, a `component::Line`, a vehicle
  configuration), which is the only moment they are cheap to decode.
- The **`Add`** hook is the only place a command can be *cancelled*: it zeroes
  the result handle and returns without queueing. The factory cannot cancel;
  its caller still holds the command.
- The mod's own replays go through the same factories (the script's
  `api.cmd.*` path), so the **return address of the factory call** is the only
  thing that tells a player's command from the mod's replay of one. That
  caller-RVA filter is load-bearing, not tidiness: without it every replay is
  captured again. On TF3 it may not hold: the GUI is script, and if the
  stock tools build their commands through `api.cmd` as our replays do,
  both arrive from the same caller. Check this before porting the filter
  ([investigation/TF3_MODS_2026-09-27.md](../investigation/TF3_MODS_2026-09-27.md)).

| factory | RVA | steal | |
|---|---|---|---|
| `BuildProposal` | `0x9dc750` | 19 | roads, track, constructions, terrain, assets, the bulldozer: one command, told apart by the proposal's shape ([BUILDING.md](BUILDING.md)) |
| `CommandList::Add` | `0x9d2a00` | 18 | the cancel point |
| `BuyVehicle` | `0x9dca00` | 15 | its UI waits on the result entity |
| `SellVehicle` | `0x9de380` | 20 | |
| `ReplaceVehicle` | `0x9dddb0` | 15 | its UI waits on the result entity |
| `SendToDepot` | `0x9de6f0` | 20 | |
| `SetLine` | `0x9dea10` | 18 | |
| `CreateLine` | `0x9dcde0` | 19 | its UI asserts on an empty result |
| `UpdateLine` | `0x9df4e0` | 19 | |
| `DeleteLine` | `0x9dd190` | 20 | |
| `Reverse` | `0x9ddfe0` | 20 | a toggle: replaying an uncancelled one applies it twice |
| `SetColor` | `0x9de8a0` | 20 | `r9 -> CVec3f*` |
| `SetName` | `0x9deb70` | 15 | `r9 -> std::string*` (MSVC SSO) |
| `SetGameSpeed` | `0x9de9e0` | 21 | the clock buttons |
| `SetDate`, `SetCalendarSpeed` | `0x9de9b0`, `0x9de870` | 21 | the editor's date controls |

Three rules the cancel point taught, each after a crash or a wedged tool:

1. **A cancelled command's completion callback is a contract.** Commands whose
   UI waits on the result (the build tools, `BuyVehicle`, `ReplaceVehicle`)
   must have their callback fired with a zeroed result at `Add`, or the tool
   hangs for the rest of the session. The callback is a `std::function` whose
   impl the game builds on the stack (`{vftable, captured this}`; `_Do_call`
   is vftable slot 2) or on the heap (impl pointer at `r9+0x38`).
2. **Fire-and-forget commands must not have it fired.** `SetLine` and
   `Reverse` fired with the success byte still 0 make the UI take its failure
   branch ("unable to find a path to a stop"). Suppress without firing.
3. **A callback that asserts on an empty result is moved, not fired.**
   `CreateLine`'s callers (`UI::LineList` `0x610490`, `UI::LineManager`
   `0x6154a0`) assert `resultEntity != ecs::Entity()`. The mod moves the
   callback object into a stash and fires it later, from a later `Add` on the
   same thread, with a stand-in result naming the entity the replay created
   (a 16-byte entry `{int32 entity; double gen; int32}` whose generation must
   match the registry's, `[reg+0xb8]+id*12`).

And one that holds everywhere: **never cancel when the decode failed.** A
command the mod cannot ship in full runs natively and is read back afterwards;
cancelling it would lose the player's action.

### Layouts the capture depends on

Every vector is read at the game's own length (`{begin, end, cap}`), with a
sanity bound on the span and a readability check on every page it touches,
under SEH: a misread pointer fails the decode loudly, and a failed decode is
never cancelled.

- `component::Line`: `vector<Stop>` at `+0x00`, `int waitingTime` `+0x18`,
  `VehicleInfo` `+0x1c` (8 bytes: a `std::bitset<16>` of transport modes plus
  4). `VehicleInfo` is **engine-maintained**: the sim-side `UpdateLine`
  handler (`0x9d9fd0`) restores its own copy, and a command cannot set it.
- `Line::Stop`, 0xa8 bytes: `Entity stationGroup` `+0x00`, `int station`
  `+0x04`, `int terminal` `+0x08`, `vector<StationTerminal{int,int}>
  alternativeTerminals` `+0x10`, `int loadMode` `+0x28` (0..3), two `float`
  waits `+0x2c`/`+0x30`, `vector waypoints` `+0x38`.
- A proposal's edge record, 120 bytes: node ids `+0x00`/`+0x04`, tangents
  `+0x10`/`+0x1c` (3 floats each), `BaseEdge` type and type index
  `+0x28`/`+0x2c` (1 bridge, 2 tunnel), and an optional `PlayerOwned` as
  `{int32 player +0x70; uint8 present +0x74}`.
- `TransportNetwork` and the other components are reached through the engine's
  type index: `GetComponentDataIndex` (`0xd0920`) with the component's
  `RTTI_Type_Descriptor`, then `engine+0x88[typeIndex]`, entries of 0x48
  bytes, data at `+0x68` (indices below `0x40000000`) or paged at `+0x80`.

### The game has two engines

`CGame::RunGameSimLoop` (`0x1184d0`) keeps two `GameState` objects at
`CGame+0x168 -> { GameState*[2], ..., int current at +0x20 }` and copies one
into the other every frame with `GameState::Replicate` (`0x241630`);
`CGame+0x158` is whichever is current this frame, and that is what the UI's
`GameStateProvider` returns (`0x8badf0`: `mov rax,[rcx+8]; mov rax,[rax+0x158]`).
`GameState+0x28` is that state's `ecs::Engine`, and each engine owns its own
system objects. A command carries a specific engine pointer, so anything
computed on its behalf (the mod re-runs the line editor's platform assignment
at the replay) has to take the state whose `+0x28` is that engine, never
"this frame's".

### What lockstep needed beyond command capture

Identical commands at identical steps were not enough; the mod patches four
places where the engine's order depended on memory layout or on a seed:

| | RVA | steal | |
|---|---|---|---|
| train reservation order | `0xabe02d` | 16 | the engine shuffles the order trains claim track with a `minstd_rand` seeded from `GameTime+0x30` over node-list positions, which differ per machine; the detour orders by train name with a seeded jitter |
| free space on a road edge | `0x2117350`, `0x2117140` | 5 | the sum is taken in ascending order in `double`, so every peer gets the same float |
| road edge use entries | `0xa64473` | | kept in name order after `EdgeUseManager::Add` |
| ship and aircraft claim order | `0xa6c1e0`, `0xa2bc60` | 5 | measured only: the family node vector is engine-owned |

The world comparison that finds the remaining divergences hashes geometry and
state, never entity ids: ids, seeds and town growth differ legitimately
between machines that agree on the world.

### UI patches for companies mode

Small in-place patches, each verified against the exact bytes at the site
before it is applied: the line editor's station owner gate (`0x609631`, a
5-byte `cmp eax,[rbx+0x28]; je` with accept `0x609605` and reject `0x609636`),
three owner gates that hide other players' icons, the icon draw call
(`0x80b613`), the station label background (`0x80a0ee`), a foreign entity's
window opening read-only (`jne` at `0x8b3060`), the window bind (`0x8b2390`),
and the HUD station and depot icons (`0x5e38e1`, `0x5e45d0`, depot ctor
`0x5e2b70`). The `setPlayer` binding's ownership assert is bypassed at the
`je` `0x11677a1`. Each is a separate patch with its own byte check, so a build
change disables one feature rather than the mod.

### Detour rules that held up

- **Verify, then steal.** Every target's expected bytes are compared before
  the patch; a mismatch logs and leaves that feature off. Steals stop on an
  instruction boundary at or past 14 bytes and cover only plain,
  position-independent instructions; a `call`, a jump or a RIP-relative
  operand in the prologue is a refusal. Where only five bytes are safe to
  take, a page within ±2 GiB of the site is allocated for the detour and the
  five bytes become a `jmp rel32` into it.
- **Install before the target's first run.** The mod's proxy `alut.dll` loads
  every DLL from `DllMain`, before the game's entry point, so no thread can
  be inside a target when it is patched.
- **The cancel is gated on evidence.** Cancelling is only safe because
  something replays, so it is switched on by fresh evidence from the script
  half on disk (its per-tick status file). With the mod's Lua side absent, the
  hooks capture nothing and cancel nothing, and the base game is unchanged.

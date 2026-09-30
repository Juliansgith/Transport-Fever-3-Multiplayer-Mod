# Architecture review: the native multiplayer stack

A read-only review of the TPF3-MP repository, written against the code rather than the docs,
and checked against the game installed on the reviewer's PC.

- **Repository:** `E:\VS\Transport-Fever-3-Multiplayer-Mod`
- **Reviewed at:** `dev` = `9d20db2`, 2026-09-30
- **Game:** Steam build 40408, `TransportFever3.exe`, SHA-256
  `de1daad3a13f3b7e9f79903361bb43769cf4f15e59271a263aefe1f075f23ef2` — **byte-identical to
  the build the shipped profile pins**
- **Scope:** research only. No game was launched, nothing in the game folder was modified, no
  decision was written or changed.

Labels: **VERIFIED** (read in source or measured), **INFERRED**, **UNKNOWN** (needs binary
analysis or in-game measurement).

> This is a review of what the code *is*, not a proposal. It deliberately contradicts several
> starting assumptions; the evidence is cited so each can be checked.

---

## 1. Three starting assumptions that do not hold

### 1.1 The mapping layer is not RVA-based

The brief asks for a version-independent mapping layer and warns against hard-coded addresses.
The repository already has one, and it is signature-based:

- `crates/tpf3mp-hookcore/src/pattern.rs` — IDA-style patterns, `??`/`?` wildcards, an
  anchored `memchr` scan, and `find_unique`, which returns `NotFound` or `Ambiguous` rather
  than taking the first match.
- `crates/tpf3mp-hookcore/src/profile.rs` — `BuildIdentity { sha256, size, pe_timestamp }`,
  `TargetSpec { name, signature, offset, prologue, required }`, `Profile::from_toml`,
  `Profile::verify_identity`, `profile::resolve(image, region_base)`.
- `profiles/tf3_build40408_steam_windows.toml` — **28 targets**, compiled in with `include_str!`
  and overridable by any `*.toml` in `%LOCALAPPDATA%\TPF3-MP\profiles\`.

The RVAs that appear in the profile's comments and in `tf3_static_proof.rs` are
*documentation and assertions about a scan result*, not runtime input. Adding a second
addressing scheme would duplicate a solved problem.

Resolution is genuinely fail-closed. Every one of these refuses, and a refusal installs
nothing: build identity mismatch; a required signature missing; a signature matching more than
once (**even for an optional target** — a second match is treated as a corruption signal); the
target or its prologue falling outside the scanned slice; the prologue bytes differing.

### 1.2 The crates the brief proposes already exist, under other names

| brief | actual |
|---|---|
| `tpf3mp-mapping` | `tpf3mp-hookcore` (`pattern`, `profile`, `pe`) |
| `tpf3mp-game` | `tpf3mp-bridge::Game` + `tpf3mp-hook::{HookGame, worlds, builds}` |
| `tpf3mp-sync` | `tpf3mp-bridge::{Session, Gate}` + `tpf3mp-hook::StepDriver` |

The brief's §27 principle — multiplayer code on stable abstractions, the mapping layer holding
the unstable details — is already how the crates are split. Nothing here needs a rename.

### 1.3 The chain has one more link than the brief lists

There is a native Lua bridge. The hook registers a global `tpf3mp_native` table into the
game's own Lua states, and the mod calls it. This is not incidental: it is how the game is
driven, and it is load-bearing for correctness (§6).

---

## 2. The chain, end to end

VERIFIED throughout, with the transition points cited.

```
launcher (TPF3-MP.exe)
  └─ agent launcher backend                 agent/src/launcher/mod.rs:436
       └─ tpf3mp_launch::start()             launch/src/lib.rs:145
            ├─ CreateProcessW(CREATE_SUSPENDED)          launch/src/windows.rs:136
            ├─ env: SteamAppId/SteamGameId,
            │       TPF3MP_GAME_LINK, TPF3MP_LAUNCHER_PID launch/src/lib.rs:197
            ├─ VirtualAllocEx + WriteProcessMemory       windows.rs:169/184
            ├─ CreateRemoteThread(LoadLibraryW)          windows.rs:219
            ├─ verify the module is in the target's list windows.rs:262
            └─ ResumeThread                              windows.rs:107
                 └─ tpf3mp_hook.dll DllMain      hook/src/platform.rs:22
                      └─ CreateThread → bootstrap hook/src/lib.rs:52
                           ├─ launched_link()  (D11 gate)  hook/src/lib.rs:214
                           ├─ BuildIdentity::of_file(exe)   hookcore/profile.rs:39
                           ├─ select_profile()              hook/src/lib.rs:200
                           ├─ profile::resolve(.text)       hook/src/install.rs:269
                           ├─ lua_api() 18 Lua C functions  hook/src/install.rs:297
                           ├─ Session::attach(link)         bridge/src/session.rs:144
                           ├─ StepDriver::new(.., GuiWorlds)
                           ├─ detour CGameTime::GetSpeed    install.rs:326
                           ├─ CallRedirect step's GetSpeed  install.rs:337
                           ├─ detour luaB_print             install.rs:349
                           ├─ detour GameSim::Step          install.rs:353
                           ├─ builds::install(Add, apply)   install.rs:364
                           └─ StepDriver::on_step           hook/src/step.rs:426
                                ├─ lua::take_commands()
                                ├─ hand_over → ToAgent::Command
                                └─ lua::begin_batch → run_step → end_batch
                                     └─ agent/src/bridge.rs:475
                                          └─ GameMessage → server over QUIC
```

**A real DLL is injected** — `LoadLibraryW` on a thread the launcher creates in the suspended
game, not a manual map. The reasoning is in the file header (`windows.rs:1-7`): `kernel32.dll`
sits at the same address in every process of a session, so the launcher's own `LoadLibraryW`
is the game's.

**The launcher never searches for the game.** It keeps the PID *and* the process HANDLE from
`CreateProcessW` (`windows.rs:103-119`). No pid file, no window-title match, no
`ReadProcessMemory` anywhere in the workspace.

**Steam is only checked, never driven.** No Steam API, no `steam://run`, no `ShellExecute` in
`crates/`. Steam's path comes from parsing `.acf` and the registry (`agent/src/steam.rs:38,141`).
Starting through Steam is an explicitly rejected option (D11), and a proxy DLL is explicitly
rejected (D9 + D11). Install scripts place the mod only.

**Fail-closed at the boundary:** `Suspended` is dropped un-resumed on any failure and the game
is `TerminateProcess`d — a half-started game is never left behind (`windows.rs:41-62`).

**`tpf3mp-agent` is not in the game process** and has no `#[no_mangle]`. The only `no_mangle`
in the workspace is the hook's `DllMain`.

### The `luaB_print` trick

Worth understanding before changing anything (`install.rs:45-62`). The hook does **not** look up
a Lua state. The game calls `print` in each state it starts; the detour lets the game's own
`print` run first, then registers the table into *that* state. Consequences: the `lua_State*`
is never dereferenced (`lua.rs:52-53`); the table is set **raw**, so a strict state with an
`__index`/`__newindex` metatable cannot interfere; registration is idempotent; `checkstack(8)`
is sufficient (peak 4). The hook never calls Lua *code*, only the C API, so a Lua error can
only come from the API itself — and everything crossing in is `extern "C-unwind"`.

---

## 3. The hook, as built

**28 profile targets**: 6 game, 3 command, 19 Lua. 25 `required = true`, 3 optional.

| target | RVA | hook type | effect |
|---|---|---|---|
| `CGameTime::GetSpeed` | `0x2a95a0` | `InlineDetour` | read-only; records the speed row's value |
| `GameSim::Step/GetSpeed call` | `0x1593ee` | `CallRedirect` | one call of the step becomes N updates |
| `luaB_print` | `0x2fccd10` | `InlineDetour` | hands each Lua state to the registrar |
| `GameSim::Step` | `0x159390` | `InlineDetour` | the step gate |
| `CommandList::Add` | `0x9d29c0` | `InlineDetour` | counts the player's builds; always calls on |
| `WorldBuildProposal apply` | `0x9e1160` | `InlineDetour` | refuses a player-initiated build in the room's game |

18 Lua 5.2 C functions are called only. Four are resolved and **never read**:
`CGame::Step`, `UI::CMenuUI::StartSavegame`, `UI::CMenuUI::CreatePage` (all
`required = true`) and `CommandList::Add::lambda` (`required = false`).

Engine properties (`hookcore/src/detour/x86_64.rs`): decodes the prologue and **refuses
anything that branches**; re-encodes with iced-x86's `BlockEncoder` so RIP-relative operands
keep addressing the same memory; `jmp rel32` (5 bytes) within 2 GiB else a 14-byte
`jmp [rip+0]`; the trampoline is allocated within 1 GiB, written RW then sealed RX — never
writable and executable at once; `detach` exists and works.

---

## 4. The command pipeline: how a build crosses the room

This is the part the brief asked to understand most carefully, and it is split across a thread
boundary in a way worth stating plainly.

**The tools are native and nothing in Lua can stop them.** That is measured, not assumed —
`tools/probe/tf3/tpf3mp_buildprobe_1/` exists solely to establish it, and
`investigation/dayone-2026-09-29/` records the result. So the hook works at two native points:

1. **The click** — `add_detour` (`builds.rs:101`). Reads two bytes off the command
   (`i8` at `payload+0x9b8 == 52`, `u8` at `payload+0x3d2 == 1`) and increments a counter,
   then **always calls the original** — the command is still queued, so the tool still runs its
   own path and the game later tells the tool it failed.
2. **The apply** — `apply_detour` (`builds.rs:130`). In the room's game, and not replaying, a
   player-initiated `WorldBuildProposal` **returns `0` without calling the original**. That is
   the world's authoritative "this build was refused" answer.

**The payload itself is decoded in Lua, not natively.** `capture.lua` and `engine.lua` turn
the GUI's `builder.proposalCreate` event into an `Action`; the native side never reads a
proposal structure. The join between the two is a **monotonic counter**: the game script stores
`snapshots[clicks]` and `guiUpdate` walks it. The justification is ordering — the proposal and
the click both run on the main thread.

**Refusal is six-layered** and the layers agree on wording, which is the sort of detail that
usually goes wrong:

| layer | where | what the player sees |
|---|---|---|
| native apply | `builds.rs:136` | the game's own "build failed" path tells the tool |
| Lua, tool not carried | `tpf3mp_sim.script.lua:199` | `"Not in multiplayer yet: building with this tool"` |
| Lua, carried but un-capturable | `tpf3mp_sim.script.lua:197` | the same, plus a log line |
| Lua, GUI command not carried | `guard.lua:229` | a notice in the game bar |
| step gate, not in a room's game | `step.rs:505` | `results()` `ok=false` |
| the room refuses it | `step.rs:218` | the GUI callback hears `(data, false, {})` |

---

## 5. The recursion guard — the most important finding in this review

The brief asks for an explicit execution context so a remote action applied locally is not
re-captured. **There is no `ExecutionOrigin`, no allow-list, no entity registry, no thread
check, nothing thread-local, and no caller-RVA filter.** VERIFIED by exhaustive search of
`crates/tpf3mp-hook`.

There is one process-global flag (`builds.rs:56`):

```rust
/// The mod's game script is applying the room's actions.
static REPLAYING: AtomicBool = AtomicBool::new(false);
```

read at `builds.rs:132` and written from any Lua state, on whatever thread that state runs,
by `native_replaying` (`lua.rs:879-887`), bracketed in Lua around the action loop
(`tpf3mp_sim.script.lua:131,152`).

**This works today, and it is not a general answer.** It is a boolean with three properties
that matter:

1. **No nesting, no owner, no timeout.** The only thing that clears it is the Lua bracket.
   `registry.sync` is called **un-`pcall`ed** inside the bracket
   (`tpf3mp_sim.script.lua:124,139`). If it raises, `replaying(false)` never runs, `Link:replaying`
   is itself `pcall`-swallowed (`bridge.lua:182`), and **every later player build in that
   process applies natively** — a permanent, silent fail-open divergence. `end_batch` does not
   reset it either.
2. **Global, but the game runs game scripts on a pool of Lua states** ("Sim Pool" threads, per
   `docs/HOOKS.md`). Two concurrent replays, or a player's click landing inside another state's
   replay window, are not excluded by any mechanism.
3. **`add_detour` has no `!REPLAYING` guard**, unlike `apply_detour`. Whether the game script's
   at-once `api.cmd.sendCommand` passes through `CommandList::Add` at all is **UNKNOWN**; if it
   does, every room-ordered replay bumps the click counter and the mod consumes a count with no
   proposal beside it.

`docs/HOOKS.md:687-694` already records that TPF2's answer to this — a caller-RVA filter — "may
not hold" on TF3, because TF3's GUI is script and the stock tools may build through `api.cmd`
exactly as the replays do. The current design sidesteps the question by construction, which is
legitimate, but it is a **flag, not a principle**. `INFERRED`: a robust version needs either a
counter scoped to the apply call, or an entity-id allow-list from the room, plus a reset in the
`end_batch` failure path. This is a decision for the owner, not an implementation.

The bulldozer work merged after this review was written (`9d20db2`) routes a
`WorldBuildProposal` replay through exactly the path the guard governs, and carries the guard
through unchanged — so the finding is not stale, and it is not urgent today. It becomes urgent
the moment a replay can nest.

---

## 6. Lanes, divergence and determinism

**`Game::lanes()` is implemented** — as a mailbox, not a reader (`step.rs:208`). The hook reads
no native object for it; `lanes.lua` reads the world through the engine's own script API and
hands text up through `tpf3mp_native.lanes`. Seven lanes:

| lane | reads |
|---|---|
| 0 NETWORK | every street+track edge, ends to 0.1 m, ordered, plus the road template |
| 1 CONSTRUCTIONS | every construction, file name and origin to 0.1 m |
| 2 LINES | every line's stop count |
| 3 VEHICLES | each vehicle's state, stop, and place on its path — the path edge, the distance along it (1 cm) and the speed, taken from the simulation's own `MOVE_PATH.dyn`, not from the world position |
| 4 ECONOMY | each player's balance |
| 5 TOWNS | each town's building count |
| 6 PEOPLE | the person count |

The digest is a **hash of a hash**: two prime-modulus hashes in pure Lua produce a text
(`lanes.lua:50-66`), and Rust takes SHA-256 of that text (`step.rs:306-319`).

Fail-closed and correct here: if a checkpoint is due and no lanes arrive, the driver **holds the
world** and returns *before* `after_step` (`step.rs:464-474`), so the steps that ran are never
reported and the room's counter never advances.

**Determinism, measured** (`investigation/dayone-2026-09-29/6-determinism.md`): two **hooked**
games, one PC, through the deployed server. The two logs are **byte-for-byte identical**. 15
samples, steps 500–1900, no first differing step. A road depot sent from game 1's console
reached the room and both games applied it in the same update.

That is a real positive and a weak one. The caveats, from the data:

- **Lane `e` (road and track geometry) is `err` in all 15 samples of both files**, and
  `compare_runs.py` skips `err` samples. The thing a transport game is made of is **UNKNOWN**.
  The report says so itself.
- **`v` = 0 and `p` constant throughout** — zero vehicles, so the vehicle lane compares two
  empty sets. `n` and `t` are also constant.
- **Only `c` and `m` ever moved**, and both moved identically.
- **Cross-platform is untested** — the case D2 exists for.

`ARCHITECTURE.md:205` already calls relaxing drift control "an optimisation, never a correctness
requirement". The honest reading: this supports the architecture and licenses nothing yet.

---

## 7. The synchronization architecture

The brief asks to choose between lockstep and server-authoritative on technical grounds.
**D2 chose, and recorded why** (`docs/DECISIONS.md:32-53`): pure lockstep is rejected because
"it only works within one binary", and mixed-platform rooms are a hard requirement. The chosen
design is a hybrid neither option names:

> server-authoritative **ordering and pacing**; every client executes the same sealed steps;
> divergence detected per-lane at checkpoints and repaired by rebase from an agreed world.

Implemented in `Pacer::advance` (`server/src/pacing.rs:70`), `Game::seal` (`room.rs:2842`),
`Gate::may_run` (`bridge/src/gate.rs:131`), the four turn invariants
(`docs/PROTOCOL.md:161-174`) and `Playout` on the client. The server runs **no** simulation: the
only ruleset it ships is `AcceptAll`, whose `validate` returns `Ok(())` for everything
(`server/src/ruleset.rs:44-66`), and `tpf3mp-canon` is not a dependency of the server at all.

**The economy is replicated, not canonicalised.** Loans and money are the game's own; the room
replays the action and the game's own script books it (`apply.lua:724-734`, and
`docs/HOOKS.md` records the measured confirmation that both games booked it in the same
update). The balance appears in lane 4 purely as a divergence check. `tpf3mp-canon` holds the
TPF2 integer economy model and has no notion of `Action`. So the canonical ruleset is designed,
tested, and **not on the multiplayer path** — which is exactly what D6's `native` says, so this
is a decision working as designed rather than a gap, but it should be a conscious one.

**Save, load and join-in-progress are working on the game side.** The game is driven through its
own script API: `worlds.rs` finds the Steam save folder (registry `ActiveProcess\ActiveUser`,
else the single account with a folder; ambiguity is an error), saves under `tpf3mp_<pid>_<event>`,
waits for the GUI to report it written, moves it, and deletes the thumbnail. Loading copies to
`tpf3mp_room_<pid>`. The completion test is subtle and right (`lua.rs:228-235`): a load is done
only when a *new* world starts **after** the GUI took the request, because the world the request
was made in reports itself too.

**Snapshots are bytes, not state.** FastCDC 2020 + BLAKE3 + zstd, content-addressed, so a
transfer is an implicit delta. The known trap is recorded and unresolved
(`docs/SNAPSHOTS.md:515`): if TPF3 compresses its save stream, dedup buys nothing — and TPF2's
`.sav` is "one zstd stream over the whole serializer payload", the bad case. **UNKNOWN for TF3.**

---

## 8. Protocol, schema and the Lua surface

`PROTOCOL_VERSION = 6` (unchanged in this range). `BRIDGE_VERSION` 4 → **5**, one new field:
`ToHook::Begin.player`, so the hook can tell whose actions are its own. No new wire messages.

`ACTION_SCHEMA_VERSION` 2 → **5**. New variants `Loan(Box<LoanOp>)` and `VehicleOp`; `Tint`
replaces `Rgb`; `NodeRef`, `EdgeKind`, `ConsistPart`, `LoadMode`, `StopRules`, `LineData`,
`LineChange::Update`, `LoanTerms` are new. The positions stay `i32` millimetres and every
length is bounded at decode — D8's rule, held.

`tpf3mp_native` is at **version 8** with 13 functions. The ladder is worth reading
(`bridge.lua:58-66`): 1 raw bytes, 2 GUI handlers, 3 `take`, 4 `poll`/`saved`/`world`,
5 `room`/guard, 6 `checkpoint`/`lanes`, 7 `clicks`/`replaying`, 8 tickets and answers.

Two details that are easy to get wrong and were got right: sequences are pushed as
`createtable(array, records)` so `next` walks them in order (`lua.rs:541-558`) — from the hash
part the order is not the list's, which build 40408 demonstrates; and a dropped answer at
`MAX_ANSWERS` falls out as the **oldest** (`lua.rs:269-275`), so a ticket can be silently
forgotten.

---

## 9. Defects found

VERIFIED by reading. Ordered by how much they could cost.

| # | severity | finding |
|---|---|---|
| 1 | **high** | The recursion guard is one process-global `AtomicBool` with no nesting, no owner and no reset outside the Lua bracket. A raise in the un-`pcall`ed `registry.sync` leaves it set for the life of the process, and every later player build then applies natively — silent fail-open divergence. `builds.rs:56,132`, `lua.rs:879`, `tpf3mp_sim.script.lua:124,152` |
| 2 | **high** | The guard is global but the game runs game scripts on a **pool** of states. Concurrent replays, or a click inside another state's replay window, are not excluded. `docs/HOOKS.md` "The Lua side" |
| 3 | **medium** | Click↔proposal correlation is a **counter**, not an identity. A proposal the mod's GUI event did not see makes it hand over the wrong or no action. The design rests on main-thread ordering alone. `tpf3mp_sim.script.lua:197,220-237` |
| 4 | **medium** | `add_detour` has no `!REPLAYING` guard, unlike `apply_detour`. Whether the game script's at-once `api.cmd.sendCommand` traverses `CommandList::Add` is **UNKNOWN**; if it does, every room-ordered replay bumps the counter. `builds.rs:108-117` |
| 5 | **medium** | **No rollback on a partial install**, and the window is now **six** irreversible `mem::forget`ed detours deep. `builds::install` deliberately swallows failure, so a half-installed pair is possible. Behaviour is fail-closed there, but the doc comment "Any failure installs nothing" is true only of the *resolution* stage. `install.rs:236,345,364-371` |
| 6 | **medium** | The `DRIVER` mutex is still held **across the game's own step**, and the step now also takes `SHARED`. Lock order is `DRIVER → SHARED`; nothing takes them in reverse today, but a second thread entering the detour would block with no timeout. `install.rs:168,185` |
| 7 | **low/med** | Three `required = true` targets (`CGame::Step`, `StartSavegame`, `CreatePage`) have no consumer yet still veto the whole install. `CommandList::Add::lambda`'s comment is now stale — the design settled on a different entry. `profiles/…toml`, `install.rs:39-40` |
| 8 | **low/med** | A dropped answer at
`MAX_ANSWERS = 256` leaves a ticket un-answered and its GUI callback waiting; the frame-bounded hold covers the entity wait, not the missing-answer wait. `lua.rs:269`, `guard.lua:254` |
| 9 | **low** | The two receivers of the same bytes disagree: a `ConstructionBuild.connection` link with no `kind` is refused by the regression model but only errors in the Lua replay. `apply.lua:406` vs `model.rs:736` |
| 10 | **info** | Lane 0 sorts on a string built from Lua's `%g` float formatting. Cross-platform `%g` identity is an **UNKNOWN** the docs do not pin — contrast `geom.parameterAt`, which is explicitly restricted for determinism. `lanes.lua:68,96` |
| 11 | **info** | Mapping-layer hygiene: `region` and `image_base` in a profile are **dead data** (the installer hard-codes `.text`); `Section` carries no `Characteristics`, so there is no executable-section check; there is no `deny_unknown_fields`, so a misspelled key is silently ignored. `hookcore/src/pe.rs:26`, `install.rs:258` |

---

## 10. What the next milestone should be

Not a phase from the brief — the brief's ordering is now behind the repository's. In order:

1. **Make the recursion guard a principle rather than a flag** (defect 1). Reset it in the
   `end_batch` failure path, make it a counter, and settle whether `add_detour` needs it. This
   is the one item that can cause silent divergence, and it is a design decision, so it belongs
   to the owner rather than to a task.
2. **Complete the `e` lane's determinism measurement.** The geometry lane is `err` in every
   sample, and it is the lane that decides whether the drift-control machinery is needed at
   all. `tools/probe/compare_runs.py` skips it silently; the failure should at least be loud.
3. **Lanes for TF3's new systems** — warehouse stock and spoilage, vehicle wear, town
   happiness/pollution/noise/reputation, company rank/subsidies — PLAN Part 2, Dev B.
4. **Rollback on partial install** (defect 5), and a `Mapping` type in `hookcore` owning section
   selection and the executable check, so multiplayer code never sees an address.
5. **Patch duty** as a named owner. One build, one platform, and PLAN Part 1 already says to
   expect a day-one patch. `tools/tpfre` exists to make this ~3 s; the index of *this* binary
   needs re-creating (§11).

---

## 11. Verification on the reviewer's PC

**Game identity — PASS.** `TransportFever3.exe` is 69,711,288 bytes, SHA-256
`de1daad3…3ef2` — byte-identical to the profile's pinned build (Steam 40408, build id
25533170). `tf3_static_proof` would therefore run for real rather than skip, and the install
produced the `local/crash_dump` and `local/staging_area` paths the release-day notes expect.

**The `tpfre` index of this binary is missing and unrecoverable here.** VERIFIED: no `.tpfdb`
on `C:`, `D:`, `E:` or in git history; `F:` does not exist. The index recorded in
`investigation/dayone-2026-09-29/3-decode.md` was produced on another machine
(`C:\Users\Sepgi\…`, game on `F:`) and is gitignored. Re-creating it is a single `tpfre index`
once the kit builds. Note that `crates/tpf3mp-hookcore/tests/tpf2_static_proof.rs:22` hard-codes
an `E:\` path with no environment override and no skip-on-mismatch, so it will **panic** against
a local Transport Fever 2 that is not the pinned build 35924.

**Building is blocked on this machine, and it is the machine, not the repository.** VERIFIED:

- `cargo build` and `cargo test` fail on every Rust **build script** with
  `could not execute process … (never executed)` / `Zugriff verweigert (os error 5)`.
- The linked `build-script-build.exe` exists, is the right size, and cannot be opened for
  reading, copied, or have its ACL read. A garbage `.exe` written into the same directory is
  readable and, if valid, runs — so the block is content-based, not path-based. It reproduces
  with `CARGO_TARGET_DIR` moved elsewhere.
- Windows Defender is **off** (`RealTimeProtectionEnabled: False`; `WinDefend` and `WdNisSvc`
  stopped). There is no Smart App Control state key, no WDAC policy and no AppLocker policy.
- The blocker is **`Surfshark.AntivirusService`, running**: a filesystem minifilter denying read
  and execute on freshly-linked unsigned binaries.

No security setting was changed. To unblock locally, exclude the repository (or `target/`) in
Surfshark Antivirus, or pause it while building. The knock-on effect is that `cargo test` cannot
run on that PC at all, which also affects `fmt` and `clippy` checks that must compile build
scripts.

**Not attempted:** anything requiring the game to run. The game was not launched, and nothing in
its folder was modified.

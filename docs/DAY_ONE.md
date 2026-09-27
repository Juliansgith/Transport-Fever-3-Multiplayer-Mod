# Release-day investigation

Transport Fever 3 releases on 2026-09-29. Until then, nothing in this project
has been checked against the real game. This plan settles the **[needs game]**
questions in [ARCHITECTURE.md](ARCHITECTURE.md), in order of how much they can
change the design. Results go into `investigation/TPF3_RECON_<date>.md`, and
each finding carries one label:

- **CONFIRMED**: decompiled or named by the binary's own strings, and
  consistent with live behaviour.
- **MEASURED**: observed in the running game.
- **INFERRED**: placed by elimination only.

## 1. Archive every build

- Record for every build:
  - Steam build ID and depot manifest IDs;
  - executable SHA-256, PE timestamp and image size (Windows);
  - the Linux and macOS binaries' hashes.
- Keep a private copy of every executable. Signature profiles are verified
  against it, and hooks must fail closed on anything unrecorded.
- Expect a day-one patch and frequent patches after it.

## 2. Static recon (Windows executable first)

- **Anti-tamper:** packer or protection sections, section entropy, TLS
  callbacks. Anti-tamper changes the native plan and must be known first.
- **Symbols:**
  - RTTI;
  - MSVC `__FUNCSIG__` and `__FILE__` assert strings, run through the naming
    pipeline from `tpf2-multiplayer/tools/re` and `tools/ghidra`, made
    build-independent first;
  - TPF2-era names: `make_cmd::`, `CommandList::Add`, `GameSim::Step`,
    `CGame::RunGameSimLoop`.
- **Lua:** Lua version strings and the script API table names.
- **Linux and macOS:** repeat the symbol and string survey. Record whether the
  macOS binary is hardened-runtime, library-validated, and allows
  `DYLD_INSERT_LIBRARIES` (`codesign -dv --entitlements - <binary>`).

## 3. Script API recon (a probe mod, all three platforms)

- `_VERSION`; availability of `io`, `os`, `require`, `package`, `debug`,
  `load`/`loadstring` in both the game-script and GUI states.
- Dump `api.*` and `game.interface.*`; list `api.cmd.make.*` factories.
- Number formatting (`%.17g`), `math.random` behaviour, `pairs` order
  stability for string keys across runs.
- The mod layout and `mod.lua` format, and what a Mod Hub script mod may
  contain.
- How to list the active mods in load order, each with a name and a
  version, and the game's build: the hook reports them over the bridge,
  and the agent declares them (`ContentManifest`) in place of the
  `--game-build` and `--mods` options it takes until then.

## 4. Determinism measurement (the D2 calibration)

Run two instances from the same save with no input. Hash these lanes every
100 steps for 60 in-game days (the TPF2 baseline):

- vehicle count;
- vehicle positions at 1 m;
- edge geometry at 0.1 m;
- the construction list;
- town building counts;
- money per player;
- people count.

Then repeat with a scripted input sequence. Record, per lane, the step where
two runs first differ:

| pair | expectation |
|---|---|
| same PC, same binary | identical (TPF2 was) |
| Intel vs AMD, Windows | identical if no CPU-dispatched math paths |
| Windows vs Linux native | probably drifts (different compilers and C runtime) |
| Windows vs Linux under Proton | measure: same binary, but Wine's math library |
| Windows vs macOS arm64 | expected to drift |

The results decide how far same-binary rooms may relax drift control. They
do not change D2.

## 5. Hook feasibility, per platform

The launcher starts the game with the hook in it, and nothing else does
(D11 in [DECISIONS.md](DECISIONS.md); `crates/tpf3mp-launch`). Check, on
each platform, that a game started that way plays as one Steam starts:

- **The executable.** `find_executable` in `crates/tpf3mp-launch` looks for
  `TransportFever3(.exe)` or `Transport Fever 3(.exe)`, or on Windows the
  only other program in the folder. Correct the names, and drop the
  `TODO(TF3 release)`. Where the game is started through a script (a
  `.sh` that sets up libraries and runs the binary), start what the
  script starts, with its environment.
- **Steam.** Started directly with `SteamAppId` and `SteamGameId` 3493540,
  and Steam running, the game must start signed in, with achievements and
  the Workshop, and must not restart itself through Steam (which would
  lose the hook). If it does restart, find out why: a `steam_appid.txt`
  would be a file in the game's folder, against D11.
- **Windows:** the hook loads into the suspended game before its title menu,
  `hook.log` names the build, and the game runs on as usual. If the game's
  protection hides its modules or refuses a thread made before its own,
  let the loader run first as TPF2MP's injector does (`--launch`): resume
  the game until its main thread is inside its executable, suspend it,
  then load the hook.
- **Linux:** the same with `LD_PRELOAD`. Find out whether Steam starts the
  game inside its Linux runtime (pressure-vessel): if the game needs it,
  start it the same way, with the hook preloaded inside. TPF2's Linux build
  starts through a `run.sh` next to it; tearded's TPF2 multiplayer
  launcher puts its preload into that script. If TPF3's starts the same
  way, start the script with `LD_PRELOAD` in its environment. The script
  must `exec` the game: otherwise the game's parent is the script, not
  the launcher, and the hook stays out (`TPF3MP_LAUNCHER_PID`, HOOKS.md).
- **macOS:** how to get a library into the game at all: the binary's
  hardened runtime, library validation and `DYLD_INSERT_LIBRARIES` (§2).
  Test whether code pages can be patched under the process's code-signing
  flags.

## 6. Command pipeline and time

- Locate the command factories and the command queue, then prototype capture
  and cancel for one command (road build) on Windows. Use ground-truth sweeps
  through `api.cmd.make.*` rather than inferring from player clicks.
- Locate the simulation step, the step size in game time, speed and pause
  control, and the injection point just before a step runs.

## 7. Saves

- Force a save and load a named save from native code.
- Load a save made on Windows on Linux and macOS, and the reverse.
- Measure save sizes for small, medium and large maps.
- Find what a save can make the game run (script state, mod code), and
  apply a check to a received save before the game loads it, as TPF2MP's
  `save_metadata.py` did. Its rule for TPF2's `.sav.lua` sidecar, pure data
  under `function data() return { ... } end`, is ported as
  `tpf3mp_agent::save_check::check_lua_data`; if TPF3 saves carry such a
  file, check it where the bridge takes a fetched world (`Done::Fetched` in
  `crates/tpf3mp-agent/src/bridge.rs`) and refuse to load on failure. Until
  then a received save is only as trustworthy as the player who uploaded
  it.
- Run `measure --pair` on two saves of one world taken minutes apart, to
  see how well snapshots deduplicate (SNAPSHOTS.md).

## Deliverable

A dated recon report containing:

- the determinism table;
- a go or no-go per platform for the hook;
- the first build's signature profile;
- the list of script API capabilities.

Start it from `investigation/TPF3_RECON_TEMPLATE.md`.

## Tools

The tools that turn this plan into findings live under `tools/`. They are
read-only (no game is launched or modified) and their output is deterministic.
They were built and validated against Transport Fever 2 build 35924; the baseline
outputs are in `investigation/tpf2-baseline/`. One-time setup:
`python -m venv .venv && .venv/Scripts/pip install -r tools/requirements.txt`.

| tool | what it does | feeds |
|---|---|---|
| `tools/re/binary_survey.py <bin> -o out.md` | identity, sections+entropy, imports (system vs game-folder, with proxy-loader ranking), exports, TLS callbacks, packer/anti-tamper, RTTI, Lua version, `__FUNCSIG__`/`__FILE__` counts, TPF2-era names, and macOS code-signing posture. Handles PE, ELF and Mach-O. | §1, §2, §5 |
| `tools/re/name_functions.py <bin> -o dir [--validate spec]` | recovers a symbol map (RVA -> name, source file) from assert strings, build-independent: `.pdata` bounds on PE, LIEF function starts on ELF/Mach-O, x86-64 and arm64 reference resolution. Emits JSON+CSV and Ghidra/x64dbg/IDA scripts. | §2, §6 |
| `tools/re/diff_builds.py OLD.symbols.json NEW.symbols.json` | which named functions moved, resized, appeared or disappeared between two builds -- run it after a day-two patch to re-verify hook signatures. | §1, §2 |
| `tools/re/make_profile.py <bin> <symbols.json> NAME ... -o profile.toml` | writes the hook profile for named functions: the build identity, and per function a unique signature with every displacement wildcarded and the exact prologue the detour steals (docs/HOOKS.md, release-day procedure). Refuses what it cannot make safe. x86-64 only. | §2.4, the hook profile |
| `tools/re/test_make_profile.py [--update]` | checks `make_profile.py` on a synthetic PE and keeps the fixture hookcore's test resolves current. | -- |
| `tools/re/selftest.py` | validates the ELF / Mach-O / arm64 code paths on synthetic fixtures (no game binary needed). | -- |
| `tools/probe/script_api_dump/` | game-script mod: dumps the Lua sandbox and `api.*`/`game.interface.*` from the engine and GUI states. | §3 |
| `tools/probe/determinism_probe/` | game-script mod: hashes the §4 lanes every N steps to a per-instance log. | §4 |
| `tools/probe/compare_runs.py A.log B.log` | first per-lane divergence between two determinism logs. | §4 |
| `tools/probe/check_lua.py` | syntax-checks the probe Lua with a real Lua 5.2. | §3, §4 |

Release-day order:

1. **Archive + static (per platform):** run `binary_survey.py` on each build
   (§1 hashes/sizes, §2 loader/packer/Lua/RTTI), then `name_functions.py` on each
   to get the symbol maps and hook-target RVAs (§2.4). Validate known targets
   with a `--validate` spec once they are located, then write the build's hook
   profile with `make_profile.py` from the binary, its symbol map and the target
   names.
2. **On a patch:** re-run `name_functions.py` and `diff_builds.py` the old and new
   maps; re-verify any hook whose target is listed resized/appeared/disappeared,
   and run `make_profile.py` on the new build for its profile.
3. **Script API (per platform):** `check_lua.py` first, then install the
   `script_api_dump` mod, launch, and collect the engine/GUI dumps (§3).
4. **Determinism (the D2 calibration):** install `determinism_probe` on two
   instances from the same save (set a distinct `$TPF3MP_PROBE_INSTANCE` each),
   run the pairs in the §4 table, then `compare_runs.py` the logs.
5. Fill `investigation/TPF3_RECON_TEMPLATE.md` as results arrive.
6. **Player log bundle:** find where Transport Fever 3 writes its log
   (`stdout.txt` for TPF2) and crash dumps on each platform. The bundle
   already looks in its Steam folder (app 3493540) where TPF2 kept them:
   correct `game_candidates_in` in `crates/tpf3mp-agent/src/logs.rs` where
   that is wrong, drop the "TPF2 location, confirm on TF3" mark from the
   confirmed ones, and update "Sending your logs" in `docs/PLAYING.md`.
7. **Installer:** confirm on each platform that the game loads mods from
   `<Steam>/userdata/<account>/3493540/local/mods`, where TPF2 kept a
   player's own, and how a game enables the TPF3-MP mod. Where not,
   correct `Find-ModsDir` in `packaging/windows/tools/install.ps1` and
   `find_mods_dir` in `packaging/unix/install.sh`, with their tests in
   `packaging/*/test-install.*`. Their `Find-Game` and `find_game` find
   the game's folder only to see whether the game is running.
8. **Starting the game:** work through §5 and start a real game from the
   launcher in a room, on each platform: **Start Transport Fever 3**
   starts it, the Game part shows it connected, and the same game started
   from Steam shows nothing of TPF3-MP (no `hook.log` lines).

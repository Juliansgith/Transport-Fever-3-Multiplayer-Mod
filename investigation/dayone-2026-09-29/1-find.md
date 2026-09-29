# Transport Fever 3 on this computer

Made by tools/dayone/dayone.py on 2026-09-29 18:34.

- Game folder: `F:\SteamLibrary\steamapps\common\Transport Fever 3`
- Steam build: 25533170
- Depot 3493541: manifest 3423371049532759875, size 70679695094
- Depot 3493542: manifest 7273310841282218484, size 175082017
- Depot 3979620: manifest 1657141639276536190, size 2062545581
- Depot 3979630: manifest 2906427112187058072, size 698613418

## Executables
- `TransportFever3.exe` (pe, 69,711,288 bytes)

## Guess 1: the executable's name (crates/tpf3mp-launch, find_executable)
GO: the launcher finds `TransportFever3.exe` by its expected name. Drop the TODO(TF3 release).
Start scripts next to it: ModelEditor.bat: see whether the game must be started through one (DAY_ONE.md section 5, Linux).

## Guess 2: the game's log and crash dumps (crates/tpf3mp-agent/src/logs.rs)
- TPF2's place: userdata/<account>/3493540/local: `c:\program files (x86)\steam\userdata\63389028\3493540\local`
  - `crash_dump\stdout.txt` (314,861 bytes, 2026-09-29 18:32)
Correct game_candidates_in to the places above, and drop the TPF2 mark from those confirmed.

## Mods folders (packaging/*/install)
- `c:\program files (x86)\steam\userdata\63389028\3493540\local\staging_area`: there
- `c:\program files (x86)\steam\userdata\63389028\3493540\local\mods`: there

Verdict: GO

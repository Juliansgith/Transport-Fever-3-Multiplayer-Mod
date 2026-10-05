# The paths the game tools share (docs/GAME_TESTING.md). Dot-source it:
#   . "$PSScriptRoot\env.ps1"
# Each can be set beforehand in the environment:
#   TPF3MP_GAME_EXE    TransportFever3.exe (else found through Steam)
#   TPF3MP_GAME_LOCAL  the game's per-user folder, Steam's
#                      userdata\<account>\3493540\local (else the active or only account)
#   TPF3MP_GAME_WORK   where runs, screenshots and logs go
#                      (else target\game-runs in the repository)
#   TPF3MP_BIN         the built tpf3mp-rig.exe and tpf3mp_hook.dll
#                      (else $CARGO_TARGET_DIR\release or target\release)

$Repo = (Resolve-Path "$PSScriptRoot\..\..").Path
$AppId = "3493540"

function Get-SteamRoot {
  $key = Get-ItemProperty "HKCU:\Software\Valve\Steam" -ErrorAction SilentlyContinue
  if ($key -and $key.SteamPath) { return ($key.SteamPath -replace "/", "\") }
  return "C:\Program Files (x86)\Steam"
}

function Find-GameExe {
  if ($env:TPF3MP_GAME_EXE) { return $env:TPF3MP_GAME_EXE }
  $steam = Get-SteamRoot
  $libraries = @($steam)
  $vdf = "$steam\steamapps\libraryfolders.vdf"
  if (Test-Path $vdf) {
    foreach ($m in [regex]::Matches((Get-Content $vdf -Raw), '"path"\s+"([^"]+)"')) {
      $libraries += ($m.Groups[1].Value -replace "\\\\", "\")
    }
  }
  foreach ($library in $libraries) {
    $exe = "$library\steamapps\common\Transport Fever 3\TransportFever3.exe"
    if (Test-Path $exe) { return $exe }
  }
  return $null
}

function Find-GameLocal {
  if ($env:TPF3MP_GAME_LOCAL) { return $env:TPF3MP_GAME_LOCAL }
  $active = (Get-ItemProperty 'HKCU:\Software\Valve\Steam\ActiveProcess' -ErrorAction SilentlyContinue).ActiveUser
  if ($active) {
    $local = "$(Get-SteamRoot)\userdata\$active\$AppId\local"
    if (Test-Path -LiteralPath $local -PathType Container) { return $local }
    throw 'The active Steam account has no game folder; set TPF3MP_GAME_LOCAL explicitly'
  }
  $found = @(Get-ChildItem "$(Get-SteamRoot)\userdata\*\$AppId\local" -Directory -ErrorAction SilentlyContinue)
  if ($found.Count -gt 1) { throw 'Several Steam accounts found; set TPF3MP_GAME_LOCAL explicitly' }
  if ($found.Count -eq 1) { return $found[0].FullName }
  return $null
}

$GameExe = Find-GameExe
$GameLocal = Find-GameLocal
# Every game on this PC writes its console output here, one file for all.
$GameStdout = if ($GameLocal) { "$GameLocal\crash_dump\stdout.txt" } else { $null }
$GameSaves = if ($GameLocal) { "$GameLocal\save" } else { $null }
# Where the game loads a mod from without Steam Workshop or mod.io.
$ModStaging = if ($GameLocal) { "$GameLocal\staging_area\tpf3mp_1" } else { $null }
$Work = if ($env:TPF3MP_GAME_WORK) { $env:TPF3MP_GAME_WORK } else { "$Repo\target\game-runs" }
$Bin = if ($env:TPF3MP_BIN) { $env:TPF3MP_BIN }
  elseif ($env:CARGO_TARGET_DIR) { "$env:CARGO_TARGET_DIR\release" }
  else { "$Repo\target\release" }
New-Item -ItemType Directory -Force $Work | Out-Null

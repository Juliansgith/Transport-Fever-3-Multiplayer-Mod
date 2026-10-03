# Starts a room of real games on this PC and gets them playing one world
# (docs/GAME_TESTING.md, "Starting a room"):
#
# 1. installs the repository's mod into the game's staging area;
# 2. starts tpf3mp-rig: a throwaway local server, one agent and one game
#    per player, each game with the hook loaded, all in one room;
# 3. once the room's game starts, loads the fixture save in the host (p1)
#    through its console; the room saves that world, and the guests load it
#    from their main menus; a guest that has not after -GuestWait seconds
#    fails the setup instead of loading a different world;
# 4. closes the host's console and zooms each game in a little.
#
# Prints "run: <folder>" and "games: p1=<pid> p2=<pid> ..."; the rig keeps
# running until the games quit (quit.ps1).
#
#   room.ps1 [-Players 2] [-Run name] [-Fixture tpf3mp_fixture3] [-NoInstall]
#
# Refuses while any Transport Fever 3, tpf3mp-rig or tpf3mp-server runs:
# they may be someone else's test. Existing processes are never stopped.
param(
  [ValidateRange(2,8)][int]$Players = 2,
  [ValidatePattern("^[A-Za-z0-9_-]+$")][string]$Run = ("run-" + (Get-Date -Format "MMdd-HHmmss")),
  [ValidatePattern("^[A-Za-z0-9_-]+$")][string]$Fixture = "tpf3mp_fixture3",
  [string]$GameBuild = "40408",
  [int]$Stagger = 25,
  [int]$MenuSeconds = 55,
  [int]$GuestWait = 150,
  [string]$CloseConsoleAt = "557,47",
  [switch]$NoInstall
)
$ErrorActionPreference = "Stop"
. "$PSScriptRoot\env.ps1"
if (-not $GameExe) { throw "Transport Fever 3 not found; set TPF3MP_GAME_EXE" }
if (-not $GameLocal) { throw "the game's userdata folder not found; set TPF3MP_GAME_LOCAL" }
$rig = "$Bin\tpf3mp-rig.exe"
if (-not (Test-Path $rig)) { throw "no $rig; build it: cargo build --release -p tpf3mp-testkit --bin tpf3mp-rig; then cargo build --release -p tpf3mp-hook --lib" }
if (-not (Test-Path "$Bin\tpf3mp_hook.dll")) { throw "no tpf3mp_hook.dll in $Bin; cargo build --release -p tpf3mp-hook" }
if (-not (Test-Path "$GameSaves\$Fixture.sav")) { throw "no save $Fixture in $GameSaves (docs/GAME_TESTING.md, 'Fixture saves')" }

$games = @(Get-Process TransportFever3 -ErrorAction SilentlyContinue)
$ours = @(Get-Process tpf3mp-rig, tpf3mp-server -ErrorAction SilentlyContinue)
if ($games.Count -gt 0 -or $ours.Count -gt 0) {
  $list = (@($games) + @($ours) | ForEach-Object { "$($_.ProcessName) $($_.Id)" }) -join ", "
  throw "Already running: $list. Quit only your own sessions first; this script never stops existing games or servers."
}

$runDir = "$Work\$Run"
if (Test-Path -LiteralPath $runDir) { throw "Run directory already exists: $runDir" }

if (-not $NoInstall) {
  $parent = [IO.Path]::GetFullPath((Join-Path $GameLocal 'staging_area'))
  $target = [IO.Path]::GetFullPath($ModStaging)
  if ($target -ne (Join-Path $parent 'tpf3mp_1')) { throw 'Unexpected staging target' }
  foreach ($path in @($GameLocal,$parent,$target)) {
    if ((Test-Path -LiteralPath $path) -and ((Get-Item -LiteralPath $path).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Refusing linked staging path: $path" }
  }
  if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target -Recurse -Force -ErrorAction Stop }
  New-Item -ItemType Directory -Force $parent | Out-Null
  Copy-Item -Recurse "$Repo\mod\tpf3mp_1" $ModStaging
  "installed the mod into $ModStaging"
}

New-Item -ItemType Directory -Force $runDir | Out-Null
$rigArgs = @("--players", "$Players", "--stagger", "$Stagger", "--wait-for-games", "--server", "local",
  "--game", "`"$GameExe`"", "--data-root", "`"$runDir`"", "--game-build", $GameBuild)
$proc = Start-Process -FilePath $rig -ArgumentList $rigArgs -PassThru -WindowStyle Hidden `
  -RedirectStandardOutput "$runDir\rig.out" -RedirectStandardError "$runDir\rig.err"
Set-Content "$runDir\rig.pid" $proc.Id
"run: $runDir (rig pid $($proc.Id))"

# The room's game starts once every game's hook has attached.
$out = "$runDir\rig.out"
$deadline = (Get-Date).AddSeconds(600)
while ((Get-Date) -lt $deadline -and -not ((Test-Path $out) -and (Select-String -Path $out -Pattern "game started with|rig: stopped" -Quiet))) {
  if ($proc.HasExited) { break }
  Start-Sleep -Seconds 3
}
if (-not ((Test-Path $out) -and (Select-String -Path $out -Pattern "game started with" -Quiet))) {
  "the room did not start; the rig said:"
  Get-Content $out, "$runDir\rig.err" -ErrorAction SilentlyContinue | Select-Object -Last 15
  exit 1
}
$pids = @(Select-String -Path $out -Pattern "\(game pid (\d+)\)" | ForEach-Object { [int]$_.Matches[0].Groups[1].Value })
if ($pids.Count -ne $Players) { throw 'Rig did not report every game PID' }

$load = 'local ns=app.SaveGameNamespace.getSavegame() for _,i in ipairs(app.findAllSavegames(ns)) do ' +
  'if i.saveName=="' + $Fixture + '" then local id=api.type.SavegameId.new() id.path=i.path ' +
  'id.saveGameName=i.saveName id.saveGameNamespace=ns print("@@loading ' + $Fixture + '") app.loadGame(id,false,nil) end end'
$consoled = @()
function Load-Fixture([int]$gp) {
  # The console takes input once the game is at its main menu.
  $ready = (Get-Process -Id $gp -ErrorAction Stop).StartTime.AddSeconds($MenuSeconds)
  while ((Get-Date) -lt $ready) { Start-Sleep -Seconds 2 }
  & "$PSScriptRoot\console.ps1" -GamePid $gp -Lua $load -Open | Out-Null
  $script:consoled += $gp
}
function Hook-Says([string]$player, [string]$pattern) {
  $log = "$runDir\$player\hook.log"
  (Test-Path $log) -and (Select-String -Path $log -Pattern $pattern -Quiet)
}

Load-Fixture $pids[0]
$deadline = (Get-Date).AddSeconds(150)
while ((Get-Date) -lt $deadline -and -not (Hook-Says "p1" "saved the world|not saved|holding")) { Start-Sleep -Seconds 3 }
for ($i = 1; $i -lt $pids.Count; $i++) {
  $player = "p$($i + 1)"
  $deadline = (Get-Date).AddSeconds($GuestWait)
  while ((Get-Date) -lt $deadline -and -not (Hook-Says $player "playing the room's world from its save")) { Start-Sleep -Seconds 3 }
  if (-not (Hook-Says $player "playing the room's world from its save")) {
    throw "$player did not load the shared snapshot. Test is not ready; games left running for diagnosis in $runDir"
  }
}
if (-not (Hook-Says "p1" "playing the room's world from its save")) {
  throw "Host has not loaded the shared snapshot; test not ready in $runDir"
}
Start-Sleep -Seconds 15

$g = "$PSScriptRoot\gamewin.ps1"
$c = $CloseConsoleAt -split ","
foreach ($gp in $pids) {
  try {
    # The console the fixture's load left open.
    if ($consoled -contains $gp) { & $g click $c[0] $c[1] -GamePid $gp | Out-Null; Start-Sleep -Seconds 1 }
    & $g scroll 650 380 -Clicks 6 -GamePid $gp | Out-Null; Start-Sleep -Seconds 1
  } catch { "could not reach game $gp`: $_" }
}
for ($i = 0; $i -lt $pids.Count; $i++) {
  $player = "p$($i + 1)"
  "== $player"
  Get-Content "$runDir\$player\hook.log" -ErrorAction SilentlyContinue |
    Select-String "playing the room's world|holding|saved the world|from its save" |
    Select-Object -Last 2 | ForEach-Object { $_.Line }
}
"games: " + ((0..($pids.Count - 1) | ForEach-Object { "p$($_ + 1)=$($pids[$_])" }) -join " ")

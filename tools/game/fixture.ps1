# Makes the plain fixture save that room.ps1 starts rooms from
# (docs/GAME_TESTING.md, "Fixture saves"): starts one game without the hook,
# has its console start a new world (16 by 16 tiles, seed "tpf3mp", the
# tutorial off, as docs/DEVELOPMENT.md describes), saves it under -Name,
# and quits the game. Nothing is clicked: the console does it all, so the
# window's size does not matter.
#
#   fixture.ps1 [-Name tpf3mp_fixture] [-Seed tpf3mp] [-Tiles 16]
#
# Needs the game's console (debugMode = true in settings.lua) and Steam
# running. Refuses while any game runs, never stops one, and never
# overwrites a save. A game that does not answer is left running, and its
# PID printed, for diagnosis.
param(
  [ValidatePattern("^[A-Za-z0-9_-]+$")][string]$Name = "tpf3mp_fixture",
  [ValidatePattern("^[A-Za-z0-9_-]+$")][string]$Seed = "tpf3mp",
  [ValidateRange(4, 64)][int]$Tiles = 16,
  [int]$MenuWait = 180,
  [int]$WorldWait = 300,
  [int]$SaveWait = 120,
  [int]$QuitWait = 60
)
$ErrorActionPreference = "Stop"
. "$PSScriptRoot\env.ps1"
if (-not $GameExe) { throw "Transport Fever 3 not found; set TPF3MP_GAME_EXE" }
if (-not $GameLocal) { throw "the game's userdata folder not found; set TPF3MP_GAME_LOCAL" }
$save = "$GameSaves\$Name.sav"
if (Test-Path -LiteralPath $save) { throw "a save named $Name already exists at $save; this helper never overwrites one" }
$settings = "$GameLocal\settings.lua"
if (-not ((Test-Path $settings) -and (Select-String -Path $settings -Pattern '^\s*debugMode\s*=\s*true' -Quiet))) {
  throw "the game's console is off: set debugMode = true in $settings (docs/GAME_TESTING.md, 'Setup')"
}
if (-not (Get-Process steam -ErrorAction SilentlyContinue)) { throw "Steam is not running; the game needs it" }
if (@(Get-Process TransportFever3 -ErrorAction SilentlyContinue).Count -gt 0) {
  throw "A game is already running; quit it first. This helper never stops one"
}

# The game's log as it stands, read shared because the game keeps it open.
# A marker counts only after a "Starting up" line this game wrote: one
# stamped after the process started. Until the game rewrites the log, the
# file still holds the previous session, whose markers must not pass.
function Read-Log {
  if (-not (Test-Path $GameStdout)) { return "" }
  $fs = [System.IO.File]::Open($GameStdout, 'Open', 'Read', 'ReadWrite')
  try { (New-Object System.IO.StreamReader($fs)).ReadToEnd() } finally { $fs.Close() }
}
function Log-Says([string]$pattern) {
  $text = Read-Log
  $starts = [regex]::Matches($text, '\[(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d)Z[^\]]*\]\s*Starting up build version')
  if ($starts.Count -eq 0) { return $false }
  $last = $starts[$starts.Count - 1]
  $styles = [Globalization.DateTimeStyles]::AssumeUniversal -bor [Globalization.DateTimeStyles]::AdjustToUniversal
  $stamped = [DateTime]::ParseExact($last.Groups[1].Value, 'yyyy-MM-dd HH:mm:ss', [Globalization.CultureInfo]::InvariantCulture, $styles)
  if ($stamped -lt $proc.StartTime.ToUniversalTime().AddSeconds(-10)) { return $false }
  return $text.Substring($last.Index) -match $pattern
}
function Wait-For([string]$pattern, [int]$seconds, [string]$what) {
  $deadline = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $deadline) {
    if ($proc.HasExited) { throw "the game quit while waiting for $what" }
    if (Log-Says $pattern) { return }
    Start-Sleep -Seconds 2
  }
  throw "no '$what' within $seconds s; the game is left running for diagnosis (pid $($proc.Id))"
}

# Started as the launcher starts it: from its own folder, told its Steam
# app so it does not hand itself back to Steam.
$env:SteamAppId = "3493540"
$env:SteamGameId = "3493540"
$proc = Start-Process -FilePath $GameExe -WorkingDirectory (Split-Path $GameExe) -PassThru
"started the game (pid $($proc.Id))"
Wait-For "Main menu is ready" $MenuWait "the main menu"
Start-Sleep -Seconds 8

$g = "$PSScriptRoot\gamewin.ps1"
$start = 'local p = api.type.StartGameParams.new() p.numTiles = api.type.Vec2i.new(' + $Tiles + ', ' + $Tiles + ') ' +
  'p.seed = "' + $Seed + '" p.modParams = { [""] = { ["guideSystemConfig.tutorial"] = 1 } } ' +
  'print("@@fixture starting") app.startGame(p)'
& $g console $start -GamePid $proc.Id -Open | Out-Null
Wait-For "@@fixture starting" 20 "the console's echo of the start"
Wait-For "Game is ready" $WorldWait "the new world"
Start-Sleep -Seconds 10

# The console stays open through the load, but the new world takes the
# keyboard from its input line: closing it (its key, scan code 0x29) and
# opening it again gives the input line the keyboard back.
function Reopen-Console {
  & $g vk C0 29 -GamePid $proc.Id | Out-Null
  Start-Sleep -Seconds 1
}

$write = 'app.saveGame("' + $Name + '", function() print("@@fixture saved") end, false, true)'
Reopen-Console
& $g console $write -GamePid $proc.Id -Open | Out-Null
Wait-For "@@fixture saved" $SaveWait "the save"
$deadline = (Get-Date).AddSeconds(30)
while ((Get-Date) -lt $deadline -and -not (Test-Path -LiteralPath $save)) { Start-Sleep -Seconds 2 }
if (-not (Test-Path -LiteralPath $save)) { throw "the game said it saved, but $save is not there (pid $($proc.Id) left running)" }
"saved $save ($([math]::Round((Get-Item -LiteralPath $save).Length / 1MB, 1)) MB)"

Reopen-Console
& $g console 'app.quit(false)' -GamePid $proc.Id -Open | Out-Null
$deadline = (Get-Date).AddSeconds($QuitWait)
while ((Get-Date) -lt $deadline -and -not $proc.HasExited) { Start-Sleep -Seconds 2 }
if (-not $proc.HasExited) {
  Write-Error "the game did not quit within $QuitWait s; left running (pid $($proc.Id))"
  exit 1
}
"quit the game"
"fixture: $Name"

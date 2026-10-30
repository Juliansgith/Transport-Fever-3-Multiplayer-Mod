# Quits games as a player does: the window's close button opens the pause
# menu, then Quit, then Return to Desktop. A game whose room's save is under
# way when its pause menu opens shows "Saving..." with Quit greyed until it
# runs on (its pause stops the step that finishes the save): such a game is
# resumed, given a few seconds, and asked again, three times at most
# (2026-10-30). A game still running a minute later is left running and
# reported as a failure. Only named games are touched; this helper never
# force-kills a game.
#   quit.ps1 -GamePids 1234,5678
# -QuitAt / -DesktopAt / -ResumeAt: the buttons in capture coordinates, when
# the window's size differs from the one they were read on
# (docs/GAME_TESTING.md).
param([string[]]$GamePids, [string]$QuitAt = "314,726", [string]$DesktopAt = "1177,726",
  [string]$ResumeAt = "314,352")
. "$PSScriptRoot\env.ps1"
# "1234,5678" from powershell -File, or an array from a script.
$GamePids = @($GamePids | ForEach-Object { $_ -split "," } | Where-Object { $_ } | ForEach-Object { [int]$_ })
$g = "$PSScriptRoot\gamewin.ps1"
$q = $QuitAt -split ","; $d = $DesktopAt -split ","; $r = $ResumeAt -split ","
function Running([int]$gp) {
  $p = Get-Process -Id $gp -ErrorAction SilentlyContinue
  return [bool]($p -and $p.ProcessName -eq "TransportFever3")
}
foreach ($gp in $GamePids) {
  for ($try = 1; $try -le 3 -and (Running $gp); $try++) {
    if ($try -gt 1) {
      # Quit was greyed: the game resumed so its save finishes, then asked
      # again.
      try { & $g click $r[0] $r[1] -GamePid $gp | Out-Null } catch {}
      Start-Sleep -Seconds 8
    }
    $p = Get-Process -Id $gp -ErrorAction SilentlyContinue
    if (-not $p) { break }
    [void]$p.CloseMainWindow()
    Start-Sleep -Seconds 2
    try { & $g click $q[0] $q[1] -GamePid $gp | Out-Null } catch {}
    Start-Sleep -Seconds 2
    try { & $g click $d[0] $d[1] -GamePid $gp | Out-Null } catch {}
    # Quitting saves nothing more: a game still here after this asked again.
    $wait = (Get-Date).AddSeconds(10)
    while ((Get-Date) -lt $wait -and (Running $gp)) { Start-Sleep -Seconds 1 }
  }
}
$deadline = (Get-Date).AddSeconds(60)
while ((Get-Date) -lt $deadline -and @($GamePids | Where-Object { Running $_ }).Count -gt 0) { Start-Sleep -Seconds 2 }
foreach ($gp in $GamePids) {
  if (Running $gp) {
    Write-Error "Game $gp did not quit; left running for diagnosis"
    $quitFailed = $true
  } else {
    "quit $gp"
  }
}

if ($quitFailed) { exit 1 }
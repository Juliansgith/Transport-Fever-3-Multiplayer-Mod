# Quits games as a player does: the window's close button opens the pause
# menu, then Quit, then Return to Desktop. A game still running a minute
# later is stopped by its PID (it then reports an abnormal termination at
# its next start). Only the games named are touched.
#   quit.ps1 -GamePids 1234,5678
# -QuitAt / -DesktopAt: the two buttons in capture coordinates, when the
# window's size differs from the one they were read on (docs/GAME_TESTING.md).
param([string[]]$GamePids, [string]$QuitAt = "214,508", [string]$DesktopAt = "793,508")
. "$PSScriptRoot\env.ps1"
# "1234,5678" from powershell -File, or an array from a script.
$GamePids = @($GamePids | ForEach-Object { $_ -split "," } | Where-Object { $_ } | ForEach-Object { [int]$_ })
$g = "$PSScriptRoot\gamewin.ps1"
$q = $QuitAt -split ","; $d = $DesktopAt -split ","
foreach ($gp in $GamePids) {
  $p = Get-Process -Id $gp -ErrorAction SilentlyContinue
  if (-not $p -or $p.ProcessName -ne "TransportFever3") { continue }
  [void]$p.CloseMainWindow()
  Start-Sleep -Seconds 2
  try { & $g click $q[0] $q[1] -GamePid $gp | Out-Null } catch {}
  Start-Sleep -Seconds 2
  try { & $g click $d[0] $d[1] -GamePid $gp | Out-Null } catch {}
  Start-Sleep -Seconds 2
}
$deadline = (Get-Date).AddSeconds(60)
while ((Get-Date) -lt $deadline -and @($GamePids | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue }).Count -gt 0) { Start-Sleep -Seconds 2 }
foreach ($gp in $GamePids) {
  $p = Get-Process -Id $gp -ErrorAction SilentlyContinue
  if ($p -and $p.ProcessName -eq "TransportFever3") {
    Stop-Process -Id $gp -Force -ErrorAction SilentlyContinue
    "stopped $gp (it did not quit)"
  } else {
    "quit $gp"
  }
}

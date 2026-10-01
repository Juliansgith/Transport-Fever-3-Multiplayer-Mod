# Uses one game tool and prints what every player's hook said since
# (docs/GAME_TESTING.md, "Trying a tool"): clicks the tool's menu item,
# hovers the point, then clicks it or drags from it, waits, and captures.
#   tool.ps1 -Run <run> -GamePid <pid> [-Item 300,652] -At 805,200 [-To 830,170] [-Shot name]
# Coordinates are a capture's (gamewin.ps1). Leave -Item out when the tool
# is already picked.
param([string]$Run, [int]$GamePid, [string]$Item = "", [string]$At, [string]$To = "", [string]$Shot = "tool", [int]$Settle = 4)
. "$PSScriptRoot\env.ps1"
$g = "$PSScriptRoot\gamewin.ps1"
$runDir = "$Work\$Run"
$logs = @(Get-ChildItem "$runDir\p*\hook.log" -ErrorAction SilentlyContinue | Sort-Object FullName)
if ($logs.Count -eq 0) { throw "no hook logs in $runDir" }
$seen = @{}
foreach ($log in $logs) { $seen[$log.FullName] = @(Get-Content $log.FullName).Count }
if ($Item) {
  $i = $Item -split ","
  & $g click $i[0] $i[1] -GamePid $GamePid | Out-Null
  Start-Sleep -Seconds 2
}
$a = $At -split ","
& $g move $a[0] $a[1] -GamePid $GamePid | Out-Null
Start-Sleep -Seconds 1
if ($To) {
  & $g drag $At $To -GamePid $GamePid | Out-Null
} else {
  & $g click $a[0] $a[1] -GamePid $GamePid | Out-Null
}
Start-Sleep -Seconds $Settle
& $g shot $Shot -GamePid $GamePid
foreach ($log in $logs) {
  "== $($log.Directory.Name)"
  Get-Content $log.FullName | Select-Object -Skip $seen[$log.FullName] |
    Select-String -NotMatch "perf:|probe|ticks|order fix" |
    ForEach-Object { $_.Line.Substring(0, [Math]::Min(420, $_.Line.Length)) }
}

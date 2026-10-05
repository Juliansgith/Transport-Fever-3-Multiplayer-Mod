# Compares the determinism probe's samples of every player of a run
# (docs/GAME_TESTING.md, "Do the games agree?"). The mod's probe writes
# "[tpf3mp-probe det] step=<n> v=.. p=.. e=.. c=.. t=.. m=.. n=.." to each
# hook.log every 100 simulation steps; games playing one world in step must
# write the same line for every step both sampled.
#   probes.ps1 -Run <run> [-Last 0]
# Prints the steps compared, and every step whose lanes differ (lane by
# lane); exits 1 when any does. -Last n compares only the last n steps all
# sampled.
param([string]$Run, [int]$Last = 0)
. "$PSScriptRoot\env.ps1"
$logs = @(Get-ChildItem "$Work\$Run\p*\hook.log" -ErrorAction SilentlyContinue | Sort-Object FullName)
if ($logs.Count -lt 2) { throw "fewer than two hook logs in $Work\$Run" }
$samples = @{}
foreach ($log in $logs) {
  $byStep = @{}
  foreach ($m in (Select-String -Path $log.FullName -Pattern "\[tpf3mp-probe det\] step=(\d+) time=\S+ (.*)$")) {
    $step = [long]$m.Matches[0].Groups[1].Value
    $row = $m.Matches[0].Groups[2].Value.Trim()
    if ($row -match '=(err|error|nil)(\s|$)') { throw "Unreadable probe lane in $($log.Directory.Name) at step $step" }
    if ($byStep.ContainsKey($step) -and $byStep[$step] -ne $row) { throw "Different duplicate probe samples in $($log.Directory.Name) at step $step" }
    $byStep[$step] = $row
  }
  $samples[$log.Directory.Name] = $byStep
}
$players = @($samples.Keys | Sort-Object)
# Within the overlap every sampled step must exist in every game.
$starts = @(); $ends = @()
foreach ($player in $players) {
  $range = @($samples[$player].Keys | Sort-Object)
  if (-not $range.Count) { throw "No samples for $player" }
  "${player}: $($range[0])..$($range[-1])"
  $starts += $range[0]; $ends += $range[-1]
}
$lo = ($starts | Measure-Object -Maximum).Maximum
$hi = ($ends | Measure-Object -Minimum).Minimum
$union = @($samples.Values | ForEach-Object { $_.Keys } | Where-Object { $_ -ge $lo -and $_ -le $hi } | Sort-Object -Unique)
foreach ($step in $union) {
  foreach ($player in $players) {
    if (-not $samples[$player].ContainsKey($step)) { throw "Missing sample for $player at step $step" }
  }
}
$common = @($samples[$players[0]].Keys | Where-Object { $s = $_; @($players | Where-Object { $samples[$_].ContainsKey($s) }).Count -eq $players.Count } | Sort-Object)
if ($Last -gt 0) { $common = @($common | Select-Object -Last $Last) }
if ($common.Count -eq 0) { "no step sampled by every player"; exit 1 }
$differ = 0
foreach ($step in $common) {
  $lines = @($players | ForEach-Object { $samples[$_][$step] })
  if (@($lines | Select-Object -Unique).Count -le 1) { continue }
  $differ++
  "step $step differs:"
  $lanes = @{}
  foreach ($p in $players) {
    foreach ($pair in ($samples[$p][$step] -split " ")) {
      $k, $v = $pair -split "=", 2
      if (-not $lanes.ContainsKey($k)) { $lanes[$k] = @{} }
      $lanes[$k][$p] = $v
    }
  }
  foreach ($k in $lanes.Keys) {
    if (@($lanes[$k].Values | Select-Object -Unique).Count -gt 1) {
      "  ${k}: " + (($players | ForEach-Object { "$_=$($lanes[$k][$_])" }) -join "  ")
    }
  }
}
"$($common.Count) steps compared ($($common[0])..$($common[-1])), $differ differ"
if ($differ -gt 0) { exit 1 }

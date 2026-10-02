# Read-only helper checks: no game or Steam process is started or controlled.
$ErrorActionPreference = 'Stop'
foreach ($file in Get-ChildItem $PSScriptRoot -Filter '*.ps1') {
  $tokens = $null; $errors = $null
  [void][System.Management.Automation.Language.Parser]::ParseFile($file.FullName, [ref]$tokens, [ref]$errors)
  if ($errors.Count) { throw "$($file.Name): $errors" }
}
$root = Join-Path ([IO.Path]::GetTempPath()) ('tpf3mp-probe-tests-' + [guid]::NewGuid())
$saved = @{}
foreach ($name in @('TPF3MP_GAME_WORK','TPF3MP_GAME_LOCAL','TPF3MP_GAME_EXE')) {
  $saved[$name] = [Environment]::GetEnvironmentVariable($name)
}
try {
  $env:TPF3MP_GAME_WORK = $root
  $env:TPF3MP_GAME_LOCAL = $root
  $env:TPF3MP_GAME_EXE = Join-Path $root 'unused.exe'
  foreach ($player in @('p1','p2')) { New-Item -ItemType Directory -Force "$root\fixture\$player" | Out-Null }
  function Sample([int]$step, [string]$lane = 'ok') {
    "[tpf3mp-probe det] step=$step time=0 v=$lane p=ok e=ok c=ok t=ok m=ok n=ok"
  }
  function Check([string]$name, [string[]]$one, [string[]]$two, [bool]$passes) {
    Set-Content "$root\fixture\p1\hook.log" $one
    Set-Content "$root\fixture\p2\hook.log" $two
    # Child process isolates probes.ps1's exit code and terminating errors.
    $process = Start-Process powershell.exe -WindowStyle Hidden -Wait -PassThru -ArgumentList @(
      '-NoProfile','-ExecutionPolicy','Bypass','-File',"`"$PSScriptRoot\probes.ps1`"",'-Run','fixture'
    ) -RedirectStandardOutput "$root\out.txt" -RedirectStandardError "$root\err.txt"
    if (($process.ExitCode -eq 0) -ne $passes) {
      throw "$name gave exit $($process.ExitCode): $(Get-Content $root\out.txt,$root\err.txt -Raw)"
    }
    "PASS $name"
  }
  $rows = @((Sample 100),(Sample 200),(Sample 300))
  Check 'matching samples' $rows $rows $true
  Check 'different lane' $rows @((Sample 100),(Sample 200 'different'),(Sample 300)) $false
  Check 'missing sample inside overlap' $rows @((Sample 100),(Sample 300)) $false
  Check 'unreadable in both games' @((Sample 100 'err')) @((Sample 100 'err')) $false
  Check 'conflicting duplicate' @((Sample 100),(Sample 100 'changed')) @((Sample 100)) $false
  Check 'identical duplicate' @((Sample 100),(Sample 100)) @((Sample 100)) $true
  Check 'no samples' @('unrelated log line') $rows $false
  Check 'no overlapping steps' @((Sample 100)) @((Sample 300)) $false
  Check 'reported partial overlap' @((Sample 100),(Sample 200)) @((Sample 200),(Sample 300)) $true
} finally {
  foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
  $resolved = [IO.Path]::GetFullPath($root)
  $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
  if (-not $resolved.StartsWith($temp, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unexpected test folder' }
  if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}

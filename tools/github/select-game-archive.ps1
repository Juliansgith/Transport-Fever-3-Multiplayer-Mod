# Pick the private archive for the native bundle selected by this checkout.
# The verifier independently compares SHA-256, size, timestamp and every hook
# target; this mapping never approves an executable by itself.
param(
  [Parameter(Mandatory = $true)][string]$SelectedBuild,
  [Parameter(Mandatory = $true)][string]$ReleaseArchive,
  [string]$UpdatedArchive
)
$ErrorActionPreference = 'Stop'

switch ($SelectedBuild.Trim()) {
  'tf3_build40408_steam_windows' { $archive = $ReleaseArchive; break }
  'tf3_build40420_steam_windows' { $archive = $UpdatedArchive; break }
  default { throw "No private archive mapping for selected native bundle '$SelectedBuild'" }
}
if ([string]::IsNullOrWhiteSpace($archive)) {
  throw "No private archive configured for selected native bundle '$SelectedBuild'"
}
if (-not (Test-Path -LiteralPath $archive -PathType Container)) {
  throw "Private archive for selected native bundle '$SelectedBuild' is missing"
}
(Resolve-Path -LiteralPath $archive).Path

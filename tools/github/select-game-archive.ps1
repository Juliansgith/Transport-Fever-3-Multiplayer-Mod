# Pick the private archive for the native bundle selected by this checkout.
# The verifier independently compares SHA-256, size, timestamp and every hook
# target; this mapping never approves an executable by itself.
param(
  [Parameter(Mandatory = $true)][string]$SelectedBuild,
  [Parameter(Mandatory = $true)][string]$ReleaseArchive,
  [string]$UpdatedArchive,
  [string]$Archive40423,
  [string]$Archive40425,
  [string]$Archive40444
)
$ErrorActionPreference = 'Stop'

switch ($SelectedBuild.Trim()) {
  'tf3_build40408_steam_windows' { $archive = $ReleaseArchive; break }
  'tf3_build40420_steam_windows' { $archive = $UpdatedArchive; break }
  'tf3_build40423_steam_windows' { $archive = $Archive40423; break }
  'tf3_build40425_steam_windows' { $archive = $Archive40425; break }
  'tf3_build40444_steam_windows' { $archive = $Archive40444; break }
  default { throw "No private archive mapping for selected native bundle '$SelectedBuild'" }
}
if ([string]::IsNullOrWhiteSpace($archive)) {
  throw "No private archive configured for selected native bundle '$SelectedBuild'"
}
if (-not (Test-Path -LiteralPath $archive -PathType Container)) {
  throw "Private archive for selected native bundle '$SelectedBuild' is missing"
}
(Resolve-Path -LiteralPath $archive).Path

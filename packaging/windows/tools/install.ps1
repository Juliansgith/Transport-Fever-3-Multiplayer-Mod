<#
Installs TPF3-MP into Transport Fever 3, or takes it out again. This file is
the whole installer: INSTALL_TPF3MP.cmd and UNINSTALL_TPF3MP.cmd only start
it. It needs no administrator rights.

    INSTALL_TPF3MP.cmd                   finds the game through Steam
    INSTALL_TPF3MP.cmd "<game folder>"   or drop the game's folder onto it
    UNINSTALL_TPF3MP.cmd                 takes everything out again

What it changes, and nothing else:

- The TPF3-MP mod, `mod\tpf3mp_1` in this package, goes into Steam's folder
  for your Transport Fever 3 mods, <Steam>\userdata\<account>\3493540\local\mods,
  or the folder -ModsDir names. A tpf3mp_1 already there is moved to
  %LOCALAPPDATA%\TPF3-MP\backups first.
- When this package has a proxy DLL, `proxy\<name>.dll`, it goes into the
  game's folder in place of the game's own <name>.dll, which is kept
  beside it as <name>_real.dll: the proxy passes every call on to it, and
  loads the hook. The hook, tpf3mp_hook.dll, goes next to it.
- tpf3mp-install.json in the game's folder records what was installed, so
  a reinstall replaces only TPF3-MP's own files and the uninstall puts the
  game's own DLL back.

It changes nothing when anything looks wrong: the game is running, the
folder is not the game's, another mod already replaced that DLL, the
game's own DLL has gone missing, or the record names anything but
TPF3-MP's own files. When a step fails, the steps before it are undone.
Nothing is deleted: what it replaces or takes out goes to
%LOCALAPPDATA%\TPF3-MP\backups.
#>
[CmdletBinding()]
param(
    # The folder that holds the game's executable. Without it, the game is
    # found through Steam.
    [Parameter(Position = 0)]
    [string]$GameDir,
    # Where the mod goes. Without it, Steam's folder for your Transport
    # Fever 3 mods.
    [string]$ModsDir,
    # Where Steam is, when it is not where the registry says.
    [string]$SteamRoot,
    # Take out what an earlier install put in.
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$SteamApp = '3493540'
$ModName = 'tpf3mp_1'
$HookName = 'tpf3mp_hook.dll'
$RecordName = 'tpf3mp-install.json'
# A DLL the proxy may stand in for: a plain file name, never a path.
$DllPattern = '^([A-Za-z0-9_][A-Za-z0-9_.-]*)\.[dD][lL][lL]$'
$Package = Split-Path -Parent $PSScriptRoot
$Steps = New-Object System.Collections.Generic.List[hashtable]

function Say([string]$Text) { Write-Host $Text }

function Get-Field($Object, [string]$Name) {
    if ($null -eq $Object) { return $null }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) { return $null }
    return $property.Value
}

function Get-Sha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

# The name the game's own DLL is kept under beside the proxy.
function Get-RealName([string]$Name) {
    if ($Name.Length -le 64 -and $Name -match $DllPattern) { return $Matches[1] + '_real.dll' }
    return $null
}

# Moves a file or folder to $Target, across drives too. It removes the
# original only once the copy is complete.
function Move-Path([string]$Path, [string]$Target) {
    if (Test-Path -LiteralPath $Target) { throw "$Target is already there" }
    try {
        Move-Item -LiteralPath $Path -Destination $Target
    }
    catch {
        if (-not (Test-Path -LiteralPath $Path)) { throw }
        Copy-Item -LiteralPath $Path -Destination $Target -Recurse
        Remove-Item -LiteralPath $Path -Recurse -Force
    }
}

# Moves a file or folder into the backups, and says where it went.
function Move-Into([string]$Path, [string]$Folder) {
    New-Item -ItemType Directory -Force -Path $Folder | Out-Null
    $target = Join-Path $Folder (Split-Path -Leaf $Path)
    Move-Path $Path $target
    return $target
}

# Remembers how to take back a step, should a later one fail: move $Path
# back to $Back, remove $Path (a copy this run made), or rewrite $Path with
# the text $Back had (removing it when it had none).
function On-Failure([string]$Undo, [string]$Path, $Back) {
    $Steps.Insert(0, @{ Undo = $Undo; Path = $Path; Back = $Back })
}

function Get-SteamRoots {
    $candidates = New-Object System.Collections.Generic.List[string]
    if ($SteamRoot) {
        $candidates.Add($SteamRoot)
    }
    else {
        try {
            $path = (Get-ItemProperty -LiteralPath 'HKCU:\Software\Valve\Steam' -Name SteamPath).SteamPath
            if ($path) { $candidates.Add($path.Replace('/', '\')) }
        }
        catch { }
        foreach ($base in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
            if ($base) { $candidates.Add((Join-Path $base 'Steam')) }
        }
    }
    $seen = @{}
    foreach ($candidate in $candidates) {
        if (-not (Test-Path -LiteralPath $candidate -PathType Container)) { continue }
        $full = [IO.Path]::GetFullPath($candidate).TrimEnd('\')
        if ($seen.ContainsKey($full.ToLowerInvariant())) { continue }
        $seen[$full.ToLowerInvariant()] = $true
        $full
    }
}

# The game's folder, from Steam's list of libraries and the game's manifest
# in one of them.
function Find-Game {
    foreach ($root in @(Get-SteamRoots)) {
        $libraries = @($root)
        $folders = Join-Path $root 'steamapps\libraryfolders.vdf'
        if (Test-Path -LiteralPath $folders -PathType Leaf) {
            foreach ($line in Get-Content -LiteralPath $folders) {
                if ($line -match '^\s*"path"\s+"(.+)"\s*$') { $libraries += $Matches[1].Replace('\\', '\') }
            }
        }
        foreach ($library in $libraries) {
            $manifest = Join-Path $library "steamapps\appmanifest_$SteamApp.acf"
            if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) { continue }
            foreach ($line in Get-Content -LiteralPath $manifest) {
                # One plain folder name under steamapps\common, nothing more.
                if ($line -match '^\s*"installdir"\s+"([^"\\/:*?<>|]+)"\s*$' -and $Matches[1] -notmatch '^\.+$') {
                    $dir = Join-Path $library ('steamapps\common\' + $Matches[1])
                    if (Test-Path -LiteralPath $dir -PathType Container) { return $dir }
                }
            }
        }
    }
    return $null
}

# Steam's folder for this player's Transport Fever 3 mods: in the account
# that played it last. The game makes <account>\3493540\local when it
# first runs.
function Find-ModsDir {
    $locals = @()
    foreach ($root in @(Get-SteamRoots)) {
        $userdata = Join-Path $root 'userdata'
        if (-not (Test-Path -LiteralPath $userdata -PathType Container)) { continue }
        foreach ($account in @(Get-ChildItem -LiteralPath $userdata -Directory)) {
            if ($account.Name -notmatch '^\d+$') { continue }
            $local = Join-Path $account.FullName "$SteamApp\local"
            if (Test-Path -LiteralPath $local -PathType Container) { $locals += Get-Item -LiteralPath $local }
        }
    }
    if ($locals.Count -eq 0) { return $null }
    $latest = @($locals | Sort-Object LastWriteTimeUtc -Descending)
    if ($latest.Count -gt 1) {
        Say "More than one Steam account has played Transport Fever 3 here; using the one that played last. -ModsDir names another folder."
    }
    return (Join-Path $latest[0].FullName 'mods')
}

function Assert-GameClosed([string]$Game) {
    $prefix = $Game.TrimEnd('\') + '\'
    foreach ($process in @(Get-Process)) {
        $path = $null
        try { $path = $process.Path } catch { }
        if ($path -and $path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Close Transport Fever 3 first: $($process.ProcessName) is running from its folder."
        }
    }
}

# What an earlier install recorded, refused unless it names only TPF3-MP's
# own files: the record says what to rename and move.
function Read-Record([string]$Game) {
    $path = Join-Path $Game $RecordName
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return $null }
    try { $record = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json }
    catch { throw "$path is damaged." }
    $hook = Get-Field $record 'hook'
    if ($null -ne $hook -and $hook -cne $HookName) { throw "$path names files TPF3-MP does not install." }
    $proxy = Get-Field $record 'proxy'
    if ($null -ne $proxy) {
        $name = [string](Get-Field $proxy 'name')
        $real = [string](Get-Field $proxy 'real')
        $sha = [string](Get-Field $proxy 'sha256')
        if (-not (Get-RealName $name) -or $real -cne (Get-RealName $name) -or $sha -notmatch '^[0-9a-f]{64}$') {
            throw "$path names files TPF3-MP does not install."
        }
    }
    $mod = Get-Field $record 'mod'
    if ($null -ne $mod -and (-not [IO.Path]::IsPathRooted([string]$mod) -or (Split-Path -Leaf ([string]$mod)) -cne $ModName)) {
        throw "$path names files TPF3-MP does not install."
    }
    return $record
}

function Write-Record([string]$Game, [string]$Version, $Proxy, $Hook, $Mod) {
    $record = [ordered]@{ version = $Version; proxy = $Proxy; hook = $Hook; mod = $Mod }
    $path = Join-Path $Game $RecordName
    $old = $null
    if (Test-Path -LiteralPath $path) { $old = Get-Content -LiteralPath $path -Raw }
    [IO.File]::WriteAllText($path, ($record | ConvertTo-Json -Depth 4))
    On-Failure 'Rewrite' $path $old
}

# The package's proxy DLL, if it has one: one file in proxy\, named as the
# DLL it stands in for.
function Get-PackageProxy {
    $dir = Join-Path $Package 'proxy'
    if (-not (Test-Path -LiteralPath $dir -PathType Container)) { return $null }
    $files = @(Get-ChildItem -LiteralPath $dir -File)
    if ($files.Count -ne 1) { throw "$dir must hold exactly one DLL, named as the game's DLL it stands in for." }
    $real = Get-RealName $files[0].Name
    if (-not $real) { throw "$($files[0].FullName) is not a DLL." }
    return @{ Name = $files[0].Name; Real = $real; Path = $files[0].FullName }
}

# Refuses a game folder the proxy cannot go into safely, before anything
# changes.
function Assert-ProxyFits([string]$Game, $Proxy, $Recorded) {
    $current = Join-Path $Game $Proxy.Name
    $real = Join-Path $Game $Proxy.Real
    if (-not (Test-Path -LiteralPath $current -PathType Leaf)) {
        throw "There is no $($Proxy.Name) in $Game. Give the folder that holds the game's executable."
    }
    $ours = $null -ne $Recorded -and (Get-Field $Recorded 'name') -ceq $Proxy.Name
    if ((Test-Path -LiteralPath $real) -and -not $ours) {
        throw "$Game already has a $($Proxy.Real) that TPF3-MP did not put there, perhaps another mod's. Remove that mod, or have Steam verify the game's files, and install again."
    }
    # The proxy passes every call on to the game's own DLL: without it the
    # game cannot start, and a new proxy would not change that.
    if ($ours -and -not (Test-Path -LiteralPath $real) -and (Get-Sha256 $current) -ceq (Get-Field $Recorded 'sha256')) {
        throw "The game's own $($Proxy.Real) is missing from $Game. Have Steam verify the game's files, and install again."
    }
}

# Puts the proxy in place of the game's own DLL, which becomes <name>_real.dll.
function Install-Proxy([string]$Game, $Proxy, $Recorded, [string]$Backups) {
    $current = Join-Path $Game $Proxy.Name
    $real = Join-Path $Game $Proxy.Real
    $ours = $null -ne $Recorded -and (Get-Field $Recorded 'name') -ceq $Proxy.Name -and
        (Get-Sha256 $current) -ceq (Get-Field $Recorded 'sha256')
    if ($ours) {
        # An older proxy of ours: replace it; the game's own stays as it is.
        $kept = Move-Into $current $Backups
        On-Failure 'MoveBack' $kept $current
        Copy-Item -LiteralPath $Proxy.Path -Destination $current
        On-Failure 'Remove' $current
        Say "Updated the proxy $($Proxy.Name)."
    }
    else {
        # The game's own DLL: a first install, or a game update put it back
        # over the proxy. It becomes the one the proxy passes calls on to.
        if (Test-Path -LiteralPath $real) {
            $stale = Move-Into $real $Backups
            On-Failure 'MoveBack' $stale $real
        }
        Move-Item -LiteralPath $current -Destination $real
        On-Failure 'MoveBack' $real $current
        Copy-Item -LiteralPath $Proxy.Path -Destination $current
        On-Failure 'Remove' $current
        Say "Installed the proxy $($Proxy.Name); the game's own is now $($Proxy.Real)."
    }
    return [ordered]@{ name = $Proxy.Name; real = $Proxy.Real; sha256 = (Get-Sha256 $current) }
}

# Puts the game's own DLL back in place of an installed proxy.
function Restore-Proxy([string]$Game, $Recorded, [string]$Backups) {
    $name = Get-Field $Recorded 'name'
    $current = Join-Path $Game $name
    $real = Join-Path $Game (Get-Field $Recorded 'real')
    $stillOurs = (Test-Path -LiteralPath $current -PathType Leaf) -and
        (Get-Sha256 $current) -ceq (Get-Field $Recorded 'sha256')
    if (-not (Test-Path -LiteralPath $real)) {
        if ($stillOurs) {
            # Without the original the proxy only keeps the game from starting.
            $kept = Move-Into $current $Backups
            On-Failure 'MoveBack' $kept $current
            Say "Took the proxy $name out, but the game's own was missing: have Steam verify the game's files."
        }
        return
    }
    if ($stillOurs -or -not (Test-Path -LiteralPath $current)) {
        if (Test-Path -LiteralPath $current) {
            $kept = Move-Into $current $Backups
            On-Failure 'MoveBack' $kept $current
        }
        Move-Item -LiteralPath $real -Destination $current
        On-Failure 'MoveBack' $current $real
        Say "Put the game's own $name back."
    }
    else {
        # A game update already put its own DLL back; the kept one is older.
        $stale = Move-Into $real $Backups
        On-Failure 'MoveBack' $stale $real
        Say "The game's own $name was already back, from a game update; moved the older copy out."
    }
}

function Install-Hook([string]$Game, [string]$Backups) {
    $target = Join-Path $Game $HookName
    if (Test-Path -LiteralPath $target) {
        $kept = Move-Into $target $Backups
        On-Failure 'MoveBack' $kept $target
    }
    Copy-Item -LiteralPath (Join-Path $Package $HookName) -Destination $target
    On-Failure 'Remove' $target
    Say "Installed $HookName."
}

# Copies the mod beside its place first, so the swap is a rename.
function Install-Mod([string]$Mods, [string]$Backups) {
    New-Item -ItemType Directory -Force -Path $Mods | Out-Null
    $target = Join-Path $Mods $ModName
    $staging = Join-Path $Mods ('.' + $ModName + '-install-' + [guid]::NewGuid().ToString('N'))
    try {
        Copy-Item -LiteralPath (Join-Path $Package "mod\$ModName") -Destination $staging -Recurse
        if (Test-Path -LiteralPath $target) {
            $kept = Move-Into $target $Backups
            On-Failure 'MoveBack' $kept $target
        }
        Move-Item -LiteralPath $staging -Destination $target
        On-Failure 'Remove' $target
    }
    finally {
        if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
    }
    Say "Installed the mod in $target."
    return $target
}

function Install([string]$Game, $Record, [string]$Backups) {
    $version = 'unknown'
    $marker = Join-Path $Package 'tpf3mp-package.json'
    if (Test-Path -LiteralPath $marker) { $version = [string](Get-Field (Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json) 'version') }
    $proxy = Get-PackageProxy
    $hasHook = Test-Path -LiteralPath (Join-Path $Package $HookName) -PathType Leaf
    $hasMod = Test-Path -LiteralPath (Join-Path $Package "mod\$ModName") -PathType Container
    $recordedProxy = Get-Field $Record 'proxy'

    # Every check before the first change.
    if ($proxy) { Assert-ProxyFits $Game $proxy $recordedProxy }
    $mods = $null
    if ($hasMod) {
        $mods = $ModsDir
        if (-not $mods) { $mods = Find-ModsDir }
        if (-not $mods) {
            throw "Steam has no folder for your Transport Fever 3 mods yet: start the game once, then install again. -ModsDir names the mods folder instead."
        }
        Say "Mods folder: $mods"
    }

    $installedProxy = $recordedProxy
    if ($proxy) {
        if ($recordedProxy -and (Get-Field $recordedProxy 'name') -cne $proxy.Name) {
            Restore-Proxy $Game $recordedProxy $Backups
            $recordedProxy = $null
        }
        $installedProxy = Install-Proxy $Game $proxy $recordedProxy $Backups
    }
    $hook = Get-Field $Record 'hook'
    if ($hasHook -and $proxy) {
        Install-Hook $Game $Backups
        $hook = $HookName
    }
    elseif ($hasHook) {
        # Without the proxy that loads it, the hook would never run.
        Say "This package has no proxy DLL to load the hook yet, so the hook was not installed: the release names the DLL once the game is out."
    }
    $mod = Get-Field $Record 'mod'
    if ($hasMod) { $mod = Install-Mod $mods $Backups } else { Say "This package has no TPF3-MP mod yet." }
    Write-Record $Game $version $installedProxy $hook $mod
    Say ''
    Say "TPF3-MP $version is installed into Transport Fever 3. Start TPF3-MP.exe to play."
}

function Remove-Install([string]$Game, $Record, [string]$Backups) {
    if ($null -eq $Record) { throw "TPF3-MP is not installed in $Game." }
    $proxy = Get-Field $Record 'proxy'
    if ($proxy) { Restore-Proxy $Game $proxy $Backups }
    $hook = Get-Field $Record 'hook'
    if ($hook -and (Test-Path -LiteralPath (Join-Path $Game $hook))) {
        $path = Join-Path $Game $hook
        $kept = Move-Into $path $Backups
        On-Failure 'MoveBack' $kept $path
        Say "Took $hook out."
    }
    $mod = [string](Get-Field $Record 'mod')
    if ($mod -and (Test-Path -LiteralPath $mod -PathType Container)) {
        $kept = Move-Into $mod $Backups
        On-Failure 'MoveBack' $kept $mod
        Say "Took the mod out of $(Split-Path -Parent $mod)."
    }
    $path = Join-Path $Game $RecordName
    $kept = Move-Into $path $Backups
    On-Failure 'MoveBack' $kept $path
    Say ''
    Say "TPF3-MP is taken out of Transport Fever 3."
}

try {
    if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA is not set, so there is nowhere for backups.' }
    $backups = Join-Path $env:LOCALAPPDATA ('TPF3-MP\backups\' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
    if ($GameDir) {
        if (-not (Test-Path -LiteralPath $GameDir -PathType Container)) { throw "$GameDir is not a folder." }
        $game = (Resolve-Path -LiteralPath $GameDir).ProviderPath.TrimEnd('\')
    }
    else {
        $game = Find-Game
        if (-not $game) {
            throw "Transport Fever 3 was not found in Steam. Drop the game's folder onto INSTALL_TPF3MP.cmd: in Steam, right-click the game, Manage, Browse local files."
        }
    }
    Say "Transport Fever 3: $game"
    Assert-GameClosed $game
    $record = Read-Record $game
    if ($Uninstall) { Remove-Install $game $record $backups } else { Install $game $record $backups }
    if (Test-Path -LiteralPath $backups) { Say "What was replaced or taken out is in $backups." }
    exit 0
}
catch {
    $failure = $_.Exception.Message
    $undone = $true
    foreach ($step in $Steps) {
        try {
            switch ($step.Undo) {
                'MoveBack' { Move-Path $step.Path $step.Back }
                'Remove' { Remove-Item -LiteralPath $step.Path -Recurse -Force }
                'Rewrite' {
                    if ($null -ne $step.Back) { [IO.File]::WriteAllText($step.Path, $step.Back) }
                    else { Remove-Item -LiteralPath $step.Path -Force }
                }
            }
        }
        catch {
            $undone = $false
            Write-Host "Could not put back $($step.Path): $($_.Exception.Message)" -ForegroundColor Yellow
        }
    }
    Write-Host ''
    if ($undone) {
        Write-Host "Nothing was changed: $failure" -ForegroundColor Red
    }
    else {
        Write-Host "It failed, and not everything could be put back (see above): $failure" -ForegroundColor Red
        Write-Host "Have Steam verify the game's files; backups are in $backups." -ForegroundColor Red
    }
    exit 1
}

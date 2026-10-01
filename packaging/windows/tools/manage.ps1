<#
Readable per-user launcher registration and removal. No administrator rights.
Only %LOCALAPPDATA%\Programs\TPF3-MP is managed. Saves, identities, settings
and other mods are never removed. Uninstall moves the launcher to backups
after its window closes; the caller removes the mod with install.ps1 first.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Root,
    [switch]$DesktopShortcut,
    [switch]$Uninstall,
    [int]$WaitForPid = 0,
    # Tests use private locations instead of this user's shortcuts/registry.
    [string]$ProgramsDir = [Environment]::GetFolderPath('Programs'),
    [string]$DesktopDir = [Environment]::GetFolderPath('Desktop'),
    [string]$RegistryKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\TPF3-MP'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0
try {
    if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA is missing.' }
    $expected = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Programs\TPF3-MP'))
    $Root = [IO.Path]::GetFullPath($Root).TrimEnd('\')
    if (-not $Root.Equals($expected, [StringComparison]::OrdinalIgnoreCase)) { throw 'This is not the managed TPF3-MP installation.' }
    # Refuse junctions/symlinks before moving anything recursively.
    $current = Get-Item -LiteralPath $Root
    while ($null -ne $current) {
        if ($current.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'The managed installation uses a link; refusing to change it.' }
        $current = $current.Parent
    }
    $exe = Join-Path $Root 'TPF3-MP.exe'
    $marker = Join-Path $Root 'tpf3mp-managed.json'
    $package = Get-Content -LiteralPath (Join-Path $Root 'tpf3mp-package.json') -Raw | ConvertFrom-Json
    if ($package.platform -ne 'windows-x64' -or -not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'This is not a Windows launcher package.' }
    $links = @((Join-Path $ProgramsDir 'TPF3-MP.lnk'), (Join-Path $DesktopDir 'TPF3-MP.lnk'))
    $shell = New-Object -ComObject WScript.Shell
    if ($Uninstall) {
        if (-not (Test-Path -LiteralPath $marker -PathType Leaf)) { throw 'The launcher was not installed by setup.' }
        if ($WaitForPid -gt 0) {
            Wait-Process -Id $WaitForPid -ErrorAction SilentlyContinue
            if (Get-Process -Id $WaitForPid -ErrorAction SilentlyContinue) { throw 'Close TPF3-MP to finish uninstalling.' }
        }
        $backups = Join-Path $env:LOCALAPPDATA ('TPF3-MP\backups\launcher-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $backups) | Out-Null
        # Both absolute endpoints are confined to the user's TPF3-MP locations.
        Move-Item -LiteralPath $Root -Destination $backups
        foreach ($link in $links) {
            if ((Test-Path -LiteralPath $link) -and $shell.CreateShortcut($link).TargetPath -eq $exe) { Remove-Item -LiteralPath $link }
        }
        if (Test-Path -LiteralPath $RegistryKey) {
            $entry = Get-ItemProperty -LiteralPath $RegistryKey
            if ($entry.InstallLocation -eq $Root) { Remove-Item -LiteralPath $RegistryKey }
        }
        Write-Output 'Launcher removed. Saves and settings kept.'
    }
    else {
        $wanted = @($links[0])
        if ($DesktopShortcut) { $wanted += $links[1] }
        foreach ($link in $wanted) {
            New-Item -ItemType Directory -Force -Path (Split-Path -Parent $link) | Out-Null
            if ((Test-Path -LiteralPath $link) -and $shell.CreateShortcut($link).TargetPath -ne $exe) { throw "A different shortcut already exists at $link." }
            $shortcut = $shell.CreateShortcut($link)
            $shortcut.TargetPath = $exe
            $shortcut.WorkingDirectory = $Root
            $shortcut.Description = 'Transport Fever 3 Multiplayer'
            $shortcut.Save()
        }
        New-Item -Path $RegistryKey -Force | Out-Null
        $values = @{ DisplayName = 'TPF3-MP'; DisplayVersion = [string]$package.version; Publisher = 'TPF3-MP contributors'; InstallLocation = $Root; DisplayIcon = $exe; UninstallString = ('"' + $exe + '" --uninstall') }
        foreach ($name in $values.Keys) { New-ItemProperty -LiteralPath $RegistryKey -Name $name -Value $values[$name] -PropertyType String -Force | Out-Null }
        [IO.File]::WriteAllText($marker, '{"managed":true}')
        Write-Output 'Launcher shortcuts and Apps settings entry installed.'
    }
    exit 0
}
catch {
    if ($env:LOCALAPPDATA) {
        $log = Join-Path $env:LOCALAPPDATA 'TPF3-MP\uninstall-error.txt'
        [IO.File]::WriteAllText($log, $_.Exception.Message)
    }
    Write-Error $_
    exit 1
}

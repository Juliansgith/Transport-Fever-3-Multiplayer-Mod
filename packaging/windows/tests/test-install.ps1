# Tests tools\install.ps1 against made-up packages and game folders, the
# way players run it: in its own Windows PowerShell, through its exit code
# and the files it leaves. CI runs it on Windows; so can anyone:
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File packaging\windows\tests\test-install.ps1

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$Installer = Join-Path (Split-Path -Parent $PSScriptRoot) 'tools\install.ps1'
$Root = Join-Path ([IO.Path]::GetTempPath()) ('tpf3mp-install-test-' + [guid]::NewGuid().ToString('N'))
$Failures = 0

function Write-File([string]$Path, [string]$Text) {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Path) | Out-Null
    [IO.File]::WriteAllText($Path, $Text)
}

function Read-File([string]$Path) { [IO.File]::ReadAllText($Path) }

function Check([bool]$Condition, [string]$What) {
    if (-not $Condition) { throw "expected: $What" }
}

# A package with a proxy for alut.dll, the hook and the mod, and a game
# folder with its own alut.dll. Backups go to a folder of the test's own.
function New-Setup([string]$Name) {
    $dir = Join-Path $Root $Name
    $package = Join-Path $dir 'package'
    New-Item -ItemType Directory -Force -Path (Join-Path $package 'tools') | Out-Null
    Copy-Item -LiteralPath $Installer -Destination (Join-Path $package 'tools\install.ps1')
    Write-File (Join-Path $package 'tpf3mp-package.json') '{"version":"9.8.7","platform":"windows-x64"}'
    Write-File (Join-Path $package 'tpf3mp_hook.dll') 'hook'
    Write-File (Join-Path $package 'proxy\alut.dll') 'proxy 1'
    Write-File (Join-Path $package 'mod\tpf3mp_1\mod.lua') '-- mod'
    Write-File (Join-Path $package 'mod\tpf3mp_1\res\x.lua') '-- x'
    $game = Join-Path $dir 'game'
    Write-File (Join-Path $game 'game.exe') 'exe'
    Write-File (Join-Path $game 'alut.dll') 'original'
    return @{
        Package = $package
        Game = $game
        Mods = (Join-Path $dir 'mods')
        Data = (Join-Path $dir 'localappdata')
    }
}

# Runs the package's installer as a player would; returns its exit code
# and what it printed.
function Invoke-Installer($Setup, [string[]]$Arguments) {
    $saved = $env:LOCALAPPDATA
    $env:LOCALAPPDATA = $Setup.Data
    try {
        $script = Join-Path $Setup.Package 'tools\install.ps1'
        $output = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $script @Arguments 2>&1 | Out-String
        return @{ Code = $LASTEXITCODE; Output = $output }
    }
    finally {
        $env:LOCALAPPDATA = $saved
    }
}

function Install($Setup, [string[]]$More = @()) {
    Invoke-Installer $Setup (@('-GameDir', $Setup.Game, '-ModsDir', $Setup.Mods) + $More)
}

function Assert-Code($Result, [int]$Code) {
    Check ($Result.Code -eq $Code) "exit code $Code, got $($Result.Code): $($Result.Output)"
}

function Test([string]$Name, [scriptblock]$Body) {
    try {
        & $Body
        Write-Host "ok   $Name"
    }
    catch {
        $script:Failures += 1
        Write-Host "FAIL $Name`n     $($_.Exception.Message)" -ForegroundColor Red
    }
}

Test 'installs the proxy, the hook and the mod, and takes them out again' {
    $s = New-Setup 'install'
    Assert-Code (Install $s) 0
    Check ((Read-File "$($s.Game)\alut.dll") -eq 'proxy 1') 'the proxy in place'
    Check ((Read-File "$($s.Game)\alut_real.dll") -eq 'original') "the game's own DLL kept"
    Check ((Read-File "$($s.Game)\tpf3mp_hook.dll") -eq 'hook') 'the hook installed'
    Check ((Read-File "$($s.Mods)\tpf3mp_1\res\x.lua") -eq '-- x') 'the mod installed'
    $record = Read-File "$($s.Game)\tpf3mp-install.json" | ConvertFrom-Json
    Check ($record.version -eq '9.8.7') 'the version recorded'
    Check ($record.proxy.real -eq 'alut_real.dll') 'the proxy recorded'

    # Again, with a newer proxy and mod: the game's own DLL stays as it was.
    Write-File "$($s.Package)\proxy\alut.dll" 'proxy 2'
    Write-File "$($s.Package)\mod\tpf3mp_1\res\x.lua" '-- x 2'
    Assert-Code (Install $s) 0
    Check ((Read-File "$($s.Game)\alut.dll") -eq 'proxy 2') 'the newer proxy'
    Check ((Read-File "$($s.Game)\alut_real.dll") -eq 'original') "the game's own DLL untouched"
    Check ((Read-File "$($s.Mods)\tpf3mp_1\res\x.lua") -eq '-- x 2') 'the newer mod'
    Check (@(Get-ChildItem $s.Mods -Force).Count -eq 1) 'no staging folder left in the mods folder'

    $result = Invoke-Installer $s @('-GameDir', $s.Game, '-Uninstall')
    Assert-Code $result 0
    Check ((Read-File "$($s.Game)\alut.dll") -eq 'original') "the game's own DLL back"
    foreach ($gone in @('alut_real.dll', 'tpf3mp_hook.dll', 'tpf3mp-install.json')) {
        Check (-not (Test-Path "$($s.Game)\$gone")) "$gone gone"
    }
    Check (-not (Test-Path "$($s.Mods)\tpf3mp_1")) 'the mod gone'
    # Nothing is deleted: what was taken out is in the backups.
    Check (@(Get-ChildItem -Recurse -Filter 'x.lua' "$($s.Data)\TPF3-MP\backups").Count -ge 2) 'the mods kept in the backups'
    Assert-Code (Invoke-Installer $s @('-GameDir', $s.Game, '-Uninstall')) 1
}

Test 'notices a game update that put the original back' {
    $s = New-Setup 'update'
    Assert-Code (Install $s) 0
    Write-File "$($s.Game)\alut.dll" 'original 2'
    Assert-Code (Install $s) 0
    Check ((Read-File "$($s.Game)\alut.dll") -eq 'proxy 1') 'the proxy back in place'
    Check ((Read-File "$($s.Game)\alut_real.dll") -eq 'original 2') 'the newer original kept'

    # Updated again, then uninstalled: the newest original stays.
    Write-File "$($s.Game)\alut.dll" 'original 3'
    Assert-Code (Invoke-Installer $s @('-GameDir', $s.Game, '-Uninstall')) 0
    Check ((Read-File "$($s.Game)\alut.dll") -eq 'original 3') 'the newest original'
    Check (-not (Test-Path "$($s.Game)\alut_real.dll")) 'the older original moved out'
}

Test "leaves alone a folder it cannot install into" {
    $s = New-Setup 'refuse'
    # Another mod's proxy.
    Write-File "$($s.Game)\alut_real.dll" "someone's"
    Assert-Code (Install $s) 1
    Check ((Read-File "$($s.Game)\alut.dll") -eq 'original') 'alut.dll untouched'
    Check ((Read-File "$($s.Game)\alut_real.dll") -eq "someone's") "the other mod's DLL untouched"
    Check (-not (Test-Path "$($s.Game)\tpf3mp_hook.dll")) 'no hook'
    Check (-not (Test-Path $s.Mods)) 'no mod'
    # Not the game's folder.
    Remove-Item "$($s.Game)\alut_real.dll", "$($s.Game)\alut.dll"
    Assert-Code (Install $s) 1
    Check (-not (Test-Path "$($s.Game)\tpf3mp-install.json")) 'no record'
}

Test 'refuses a record that names other files' {
    $s = New-Setup 'record'
    Assert-Code (Install $s) 0
    $record = Read-File "$($s.Game)\tpf3mp-install.json"
    foreach ($swap in @(@('"tpf3mp_hook.dll"', '"..\\elsewhere.dll"'), @('"alut_real.dll"', '"game.exe"'), @('"alut.dll"', '"..\\alut.dll"'))) {
        Check ($record.Contains($swap[0])) "the record holds $($swap[0])"
        Write-File "$($s.Game)\tpf3mp-install.json" $record.Replace($swap[0], $swap[1])
        Assert-Code (Install $s) 1
        Assert-Code (Invoke-Installer $s @('-GameDir', $s.Game, '-Uninstall')) 1
        Check ((Read-File "$($s.Game)\alut.dll") -eq 'proxy 1') 'the proxy untouched'
        Check ((Read-File "$($s.Game)\alut_real.dll") -eq 'original') "the game's own DLL untouched"
        Check ((Read-File "$($s.Game)\game.exe") -eq 'exe') 'the game untouched'
        Check ((Read-File "$($s.Game)\tpf3mp_hook.dll") -eq 'hook') 'the hook untouched'
    }
}

Test "takes out, but does not update, a proxy whose original is gone" {
    $s = New-Setup 'gone'
    Assert-Code (Install $s) 0
    Remove-Item "$($s.Game)\alut_real.dll"
    Write-File "$($s.Package)\proxy\alut.dll" 'proxy 2'
    $result = Install $s
    Assert-Code $result 1
    Check ($result.Output -match 'verify') "asks for Steam to verify: $($result.Output)"
    Check ((Read-File "$($s.Game)\alut.dll") -eq 'proxy 1') 'the proxy left as it was'

    $result = Invoke-Installer $s @('-GameDir', $s.Game, '-Uninstall')
    Assert-Code $result 0
    Check ($result.Output -match 'verify') "asks for Steam to verify: $($result.Output)"
    Check (-not (Test-Path "$($s.Game)\alut.dll")) 'the proxy taken out'
    Check (-not (Test-Path "$($s.Game)\tpf3mp-install.json")) 'the record gone'
}

Test 'installs no hook without a proxy to load it' {
    $s = New-Setup 'noproxy'
    Remove-Item -Recurse "$($s.Package)\proxy"
    $result = Install $s
    Assert-Code $result 0
    Check ($result.Output -match 'no proxy DLL') "says why: $($result.Output)"
    Check (-not (Test-Path "$($s.Game)\tpf3mp_hook.dll")) 'no hook'
    Check ((Read-File "$($s.Game)\alut.dll") -eq 'original') 'alut.dll untouched'
    Check (Test-Path "$($s.Mods)\tpf3mp_1\mod.lua") 'the mod installed'
}

Test 'puts everything back when a step fails' {
    $s = New-Setup 'rollback'
    # The mods folder cannot be made: a file stands in its way.
    Write-File $s.Mods 'not a folder'
    Assert-Code (Install $s) 1
    Check ((Read-File "$($s.Game)\alut.dll") -eq 'original') 'alut.dll back'
    Check (-not (Test-Path "$($s.Game)\alut_real.dll")) 'no alut_real.dll'
    Check (-not (Test-Path "$($s.Game)\tpf3mp_hook.dll")) 'no hook'
    Check (-not (Test-Path "$($s.Game)\tpf3mp-install.json")) 'no record'
}

Test 'finds the game and the mods folder through Steam' {
    $s = New-Setup 'steam'
    $steam = Join-Path (Split-Path -Parent $s.Game) 'Steam'
    $library = Join-Path (Split-Path -Parent $s.Game) 'Library'
    $game = Join-Path $library 'steamapps\common\Transport Fever 3'
    Copy-Item -Recurse $s.Game $game
    $escaped = $library.Replace('\', '\\')
    Write-File "$steam\steamapps\libraryfolders.vdf" "`"libraryfolders`"`n{`n`t`"0`"`n`t{`n`t`t`"path`"`t`t`"$escaped`"`n`t}`n}`n"
    Write-File "$library\steamapps\appmanifest_3493540.acf" "`"AppState`"`n{`n`t`"appid`"`t`t`"3493540`"`n`t`"installdir`"`t`t`"Transport Fever 3`"`n}`n"
    New-Item -ItemType Directory -Force -Path "$steam\userdata\12345\3493540\local" | Out-Null
    Assert-Code (Invoke-Installer $s @('-SteamRoot', $steam)) 0
    Check ((Read-File "$game\alut.dll") -eq 'proxy 1') 'installed into the game Steam names'
    Check (Test-Path "$steam\userdata\12345\3493540\local\mods\tpf3mp_1\mod.lua") "the mod in Steam's mods folder"
}

Test 'refuses while the game runs' {
    $s = New-Setup 'running'
    $exe = Join-Path $s.Game 'TransportFever3.exe'
    Copy-Item "$env:SystemRoot\System32\PING.EXE" $exe
    $game = Start-Process -FilePath $exe -ArgumentList '-n', '30', '127.0.0.1' -WindowStyle Hidden -PassThru
    try {
        Start-Sleep -Milliseconds 500
        $result = Install $s
        Assert-Code $result 1
        Check ($result.Output -match 'Close Transport Fever 3') "says to close the game: $($result.Output)"
        Check ((Read-File "$($s.Game)\alut.dll") -eq 'original') 'alut.dll untouched'
    }
    finally {
        Stop-Process -Id $game.Id -Force -ErrorAction SilentlyContinue
    }
}

Remove-Item -Recurse -Force $Root -ErrorAction SilentlyContinue
if ($Failures -gt 0) {
    Write-Host "$Failures installer test(s) failed" -ForegroundColor Red
    exit 1
}
Write-Host 'all installer tests passed'

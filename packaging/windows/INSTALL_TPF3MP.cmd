@echo off
rem Installs the TPF3-MP mod for Transport Fever 3. It only starts
rem tools\install.ps1: open that file to read exactly what it changes.
rem Double-click it to find your mods folder through Steam, or drop the mods
rem folder onto it. It puts nothing in the game's folder.
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0tools\install.ps1" %*
set "TPF3MP_EXIT=%ERRORLEVEL%"
echo.
if not defined TPF3MP_NO_PAUSE pause
exit /b %TPF3MP_EXIT%

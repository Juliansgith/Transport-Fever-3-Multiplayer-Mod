@echo off
rem Takes the TPF3-MP mod out of Transport Fever 3's mods folder again. It
rem only starts tools\install.ps1 -Uninstall: open that file to read exactly
rem what it changes.
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0tools\install.ps1" -Uninstall %*
set "TPF3MP_EXIT=%ERRORLEVEL%"
echo.
if not defined TPF3MP_NO_PAUSE pause
exit /b %TPF3MP_EXIT%

@echo off
rem Installs TPF3-MP into Transport Fever 3. It only starts tools\install.ps1:
rem open that file to read exactly what it changes. Double-click it to find
rem the game through Steam, or drop the game's folder onto it.
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0tools\install.ps1" %*
set "TPF3MP_EXIT=%ERRORLEVEL%"
echo.
if not defined TPF3MP_NO_PAUSE pause
exit /b %TPF3MP_EXIT%

@echo off
rem Takes TPF3-MP out of Transport Fever 3 and puts the game's own files back.
rem It only starts tools\install.ps1 -Uninstall: open that file to read
rem exactly what it changes.
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0tools\install.ps1" -Uninstall %*
set "TPF3MP_EXIT=%ERRORLEVEL%"
echo.
if not defined TPF3MP_NO_PAUSE pause
exit /b %TPF3MP_EXIT%

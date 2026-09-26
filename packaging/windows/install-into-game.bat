@echo off
rem Installs TPF3-MP into Transport Fever 3: drop the game's folder (the one
rem with its executable) onto this file, or run it with that folder. It only
rem runs tpf3mp-agent install-hook; see PLAYING.md, "Installing".
if "%~1"=="" (
  echo Drop the folder that holds Transport Fever 3's executable onto this file.
  pause
  exit /b 1
)
"%~dp0tpf3mp-agent.exe" install-hook --game-dir "%~1" %2 %3 %4
pause

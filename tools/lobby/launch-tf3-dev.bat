@echo off
rem Starts Transport Fever 3 with the TPF3-MP hook in it, for trying the
rem in-game lobby from the main menu (docs/LOBBY.md). Needs a release build
rem of the hook and tpf3mp-launch (cargo build --release -p tpf3mp-hook -p tpf3mp-launch)
rem and the mod installed and active in the game's staging area.
rem A game started through Steam has no hook and shows the plain menu.
setlocal
set "WT=%~dp0..\.."
set "GAME=C:\Program Files (x86)\Steam\steamapps\common\Transport Fever 3\TransportFever3.exe"
if not "%~1"=="" set "GAME=%~1"
"%WT%\target\release\tpf3mp-launch.exe" --exe "%GAME%" --hook "%WT%\target\release\tpf3mp_hook.dll" --env TPF3MP_GAME_LINK=dev-menu
endlocal

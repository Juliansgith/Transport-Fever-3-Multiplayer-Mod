#!/usr/bin/env bash
# Takes the TPF3-MP mod out of Transport Fever 3's mods folder again, and
# moves it to the backups. It only starts install.sh --uninstall, next to
# it: open that file to read exactly what it changes.
exec "$(dirname "$0")/install.sh" --uninstall "$@"

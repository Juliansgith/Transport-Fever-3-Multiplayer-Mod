#!/usr/bin/env bash
# Takes TPF3-MP out of Transport Fever 3 and moves its files to the backups.
# It only starts install.sh --uninstall, next to it: open that file to read
# exactly what it changes.
exec "$(dirname "$0")/install.sh" --uninstall "$@"

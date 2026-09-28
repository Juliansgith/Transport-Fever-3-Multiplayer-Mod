#!/usr/bin/env bash
# Installs the TPF3-MP mod for Transport Fever 3, or takes it out again.
# This file is the whole installer; uninstall.sh only starts it with
# --uninstall. It needs no administrator rights.
#
#   ./install.sh                     finds your mods folder through Steam
#   ./install.sh "<mods folder>"     or puts the mod in the folder given
#   ./install.sh --steam-root <dir>  where Steam is, if not in its usual place
#   ./uninstall.sh                   takes the mod out again
#
# What it changes, and nothing else:
#
# - The TPF3-MP mod, mod/tpf3mp_1 in this package, goes into Steam's folder
#   for your Transport Fever 3 mods, <Steam>/userdata/<account>/3493540/local/staging_area,
#   or the folder given. A tpf3mp_1 already there is moved to TPF3-MP's
#   backups folder first.
# - installed.txt in TPF3-MP's data folder records the version and where
#   the mod went, which the launcher shows and the uninstall takes out.
#   The data folder is ~/.local/share/TPF3-MP on Linux (or
#   $XDG_DATA_HOME/TPF3-MP) and ~/Library/Application Support/TPF3-MP on
#   macOS.
#
# It puts nothing in the game's folder and sets no launch option. The game
# runs TPF3-MP only when the TPF3-MP launcher starts it, for a multiplayer
# session; started from Steam, it is the plain game, and the mod does
# nothing unless a game enables it.
#
# It changes nothing while the game is running, or when its record names
# anything but the mod. When a step fails, the steps before it are undone.
# Nothing is deleted: what it replaces or takes out goes to the backups.
set -euo pipefail

STEAM_APP=3493540
MOD_NAME=tpf3mp_1
PACKAGE=$(cd "$(dirname "$0")" && pwd -P)
case "$(uname -s)" in
  Darwin) DATA_DIR="$HOME/Library/Application Support/TPF3-MP" ;;
  *)
    DATA_DIR="$HOME/.local/share/TPF3-MP"
    # As the launcher reads it: XDG_DATA_HOME only when it is a full path.
    case "${XDG_DATA_HOME:-}" in /*) DATA_DIR="$XDG_DATA_HOME/TPF3-MP" ;; esac
    ;;
esac
RECORD="$DATA_DIR/installed.txt"
BACKUPS="$DATA_DIR/backups/$(date +%Y%m%d-%H%M%S)-$$"

mods_dir=""
steam_root=""
uninstall=0
while [ $# -gt 0 ]; do
  case "$1" in
    --uninstall) uninstall=1 ;;
    --mods-dir)
      mods_dir=${2:?--mods-dir needs a folder}
      shift
      ;;
    --steam-root)
      steam_root=${2:?--steam-root needs a folder}
      shift
      ;;
    -h | --help)
      sed -n '2,/^set /p' "$0" | sed '$d; s/^# \{0,1\}//'
      exit 0
      ;;
    -*)
      echo "unknown option: $1 (--help lists them)" >&2
      exit 2
      ;;
    *) mods_dir=$1 ;;
  esac
  shift
done

say() { printf '%s\n' "$*"; }

# How to take back each step, should a later one fail: move a path back,
# remove a copy this run made, or rewrite the record.
undo_kind=()
undo_path=()
undo_back=()
on_failure() { # KIND PATH [BACK]
  undo_kind+=("$1")
  undo_path+=("$2")
  undo_back+=("${3:-}")
}

reason=""
finished=0
finish() {
  set +e
  [ "$finished" = 1 ] && return
  local i=${#undo_kind[@]} whole=1
  while [ "$i" -gt 0 ]; do
    i=$((i - 1))
    case "${undo_kind[$i]}" in
      moveback) mv -- "${undo_path[$i]}" "${undo_back[$i]}" ;;
      remove) rm -rf -- "${undo_path[$i]}" ;;
      rewrite)
        if [ -n "${undo_back[$i]}" ]; then
          printf '%s\n' "${undo_back[$i]}" >"${undo_path[$i]}"
        else
          rm -f -- "${undo_path[$i]}"
        fi
        ;;
    esac || {
      whole=0
      say "Could not put back ${undo_path[$i]}"
    }
  done
  say ""
  if [ "$whole" = 1 ]; then
    say "Nothing was changed: ${reason:-it stopped part way}"
  else
    say "It failed, and not everything could be put back (see above): ${reason:-it stopped part way}"
    say "The backups are in $BACKUPS."
  fi
}
trap finish EXIT
fail() {
  reason=$1
  exit 1
}

# Moves a file or folder into the backups; prints where it went. It runs in
# $(...), where a failure must be returned, not left to `set -e`.
move_into() { # PATH
  local target
  target="$BACKUPS/$(basename "$1")"
  mkdir -p "$BACKUPS" || return 1
  if [ -e "$target" ]; then
    say "$target is already there" >&2
    return 1
  fi
  mv -- "$1" "$target" || return 1
  printf '%s' "$target"
}

steam_roots() {
  local candidates=() candidate seen=""
  if [ -n "$steam_root" ]; then
    candidates=("$steam_root")
  elif [ "$(uname -s)" = Darwin ]; then
    candidates=("$HOME/Library/Application Support/Steam")
  else
    candidates=("$HOME/.steam/steam" "$HOME/.local/share/Steam"
      "$HOME/.var/app/com.valvesoftware.Steam/.local/share/Steam"
      "$HOME/snap/steam/common/.local/share/Steam")
  fi
  for candidate in "${candidates[@]}"; do
    [ -d "$candidate" ] || continue
    candidate=$(cd "$candidate" && pwd -P)
    case "$seen" in *"|$candidate|"*) continue ;; esac
    seen="$seen|$candidate|"
    printf '%s\n' "$candidate"
  done
}

# The game's folder, from Steam's list of libraries and the game's manifest
# in one of them: only to see whether the game is running.
find_game() {
  local root library installdir
  while IFS= read -r root; do
    {
      printf '%s\n' "$root"
      if [ -f "$root/steamapps/libraryfolders.vdf" ]; then
        sed -n 's/^[[:space:]]*"path"[[:space:]]*"\(.*\)"[[:space:]]*$/\1/p' \
          "$root/steamapps/libraryfolders.vdf"
      fi
    } | while IFS= read -r library; do
      [ -f "$library/steamapps/appmanifest_$STEAM_APP.acf" ] || continue
      # One plain folder name under steamapps/common, nothing more.
      installdir=$(sed -n 's/^[[:space:]]*"installdir"[[:space:]]*"\([^"/]*\)"[[:space:]]*$/\1/p' \
        "$library/steamapps/appmanifest_$STEAM_APP.acf" | head -1)
      case "$installdir" in "" | . | ..) continue ;; esac
      if [ -d "$library/steamapps/common/$installdir" ]; then
        printf '%s\n' "$library/steamapps/common/$installdir"
      fi
    done
  done < <(steam_roots) | head -1
}

# Steam's folder for this player's Transport Fever 3 mods: in the account
# that played it last. The game makes <account>/3493540/local when it
# first runs. Mods made for build 40391 install into its staging_area and
# are then activated in Mod Hub (investigation/TF3_MODS_2026-09-27.md).
find_mods_dir() {
  local root local_dir found="" count=0
  while IFS= read -r root; do
    for local_dir in "$root"/userdata/[0-9]*/"$STEAM_APP"/local; do
      [ -d "$local_dir" ] || continue
      count=$((count + 1))
      if [ -z "$found" ] || [ "$local_dir" -nt "$found" ]; then found=$local_dir; fi
    done
  done < <(steam_roots)
  [ -n "$found" ] || return 0
  if [ "$count" -gt 1 ]; then
    say "More than one Steam account has played Transport Fever 3 here; using the one that played last. Give the mods folder to choose." >&2
  fi
  printf '%s\n' "$found/staging_area"
}

# Looks for the game's folder at the start of every running command, both
# as Steam names it and as it is on disk (~/.steam/steam is often a link).
assert_game_closed() {
  local game physical processes line
  game=$(find_game || true)
  [ -n "$game" ] || return 0
  physical=$(cd "$game" && pwd -P)
  processes=$(ps -A -o args= 2>/dev/null || true)
  while IFS= read -r line; do
    case "$line" in
      "$game"/* | "$physical"/*) fail "Close Transport Fever 3 first: ${line%% *} is running from its folder." ;;
    esac
  done <<<"$processes"
}

# What an earlier install recorded, refused unless it names the mod's own
# folder: the uninstall moves out what it names.
rec_version=""
rec_mod=""
have_record=0
read_record() {
  local line
  [ -f "$RECORD" ] || return 0
  have_record=1
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      version=*) rec_version=${line#version=} ;;
      mod=*) rec_mod=${line#mod=} ;;
      "") ;;
      *) fail "$RECORD names things TPF3-MP does not install." ;;
    esac
  done <"$RECORD"
  case "$rec_mod" in /*/"$MOD_NAME") ;; *) fail "$RECORD names something other than the TPF3-MP mod." ;; esac
  case "$rec_mod" in */../* | */./*) fail "$RECORD names something other than the TPF3-MP mod." ;; esac
  return 0
}

write_record() { # VERSION MOD
  local old=""
  mkdir -p -- "$DATA_DIR"
  [ ! -f "$RECORD" ] || old=$(cat "$RECORD")
  printf 'version=%s\nmod=%s\n' "$1" "$2" >"$RECORD"
  on_failure rewrite "$RECORD" "$old"
}

# Copies the mod beside its place first, so the swap is a rename.
install_mod() { # MODS
  local target="$1/$MOD_NAME" staging="$1/.$MOD_NAME-install-$$" kept
  mkdir -p -- "$1"
  cp -R -- "$PACKAGE/mod/$MOD_NAME" "$staging"
  on_failure remove "$staging"
  if [ -e "$target" ]; then
    kept=$(move_into "$target")
    on_failure moveback "$kept" "$target"
  fi
  mv -- "$staging" "$target"
  on_failure remove "$target"
}

do_install() {
  local version=unknown mods="$mods_dir" mod
  if [ -f "$PACKAGE/tpf3mp-package.json" ]; then
    version=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
      "$PACKAGE/tpf3mp-package.json" | head -1)
  fi
  if [ ! -d "$PACKAGE/mod/$MOD_NAME" ]; then
    say "This package has no TPF3-MP mod yet: there is nothing to install."
    return 0
  fi
  [ -n "$mods" ] || mods=$(find_mods_dir)
  [ -n "$mods" ] || fail "Steam has no folder for your Transport Fever 3 mods yet: start the game once, then install again. Or give the mods folder: ./install.sh \"<mods folder>\"."
  mkdir -p -- "$mods"
  mods=$(cd "$mods" && pwd -P)
  say "Mods folder: $mods"
  install_mod "$mods"
  mod="$mods/$MOD_NAME"
  write_record "$version" "$mod"
  say "Installed the TPF3-MP mod in $mod."
  say "In the game, open Mod Hub, find TPF3-MP under your mods and click Activate."
  say ""
  say "TPF3-MP $version is installed. Play by starting the TPF3-MP launcher: the game runs TPF3-MP only when the launcher starts it."
}

do_uninstall() {
  local kept
  [ "$have_record" = 1 ] || fail "The TPF3-MP mod is not installed."
  if [ -d "$rec_mod" ]; then
    kept=$(move_into "$rec_mod")
    on_failure moveback "$kept" "$rec_mod"
    say "Took the mod out of $(dirname "$rec_mod")."
  fi
  kept=$(move_into "$RECORD")
  on_failure moveback "$kept" "$RECORD"
  say ""
  say "The TPF3-MP mod is taken out."
}

if [ -n "$mods_dir" ] && [ -e "$mods_dir" ] && [ ! -d "$mods_dir" ]; then
  fail "$mods_dir is not a folder."
fi
assert_game_closed
read_record
if [ "$uninstall" = 1 ]; then do_uninstall; else do_install; fi
[ ! -d "$BACKUPS" ] || say "What was replaced or taken out is in $BACKUPS."
finished=1

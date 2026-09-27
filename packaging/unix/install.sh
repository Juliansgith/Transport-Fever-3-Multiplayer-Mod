#!/usr/bin/env bash
# Installs TPF3-MP into Transport Fever 3, or takes it out again. This file
# is the whole installer; uninstall.sh only starts it with --uninstall. It
# needs no administrator rights.
#
#   ./install.sh                     finds the game through Steam
#   ./install.sh "<game folder>"     the folder that holds the game
#   ./install.sh --mods-dir <dir>    where the mod goes
#   ./install.sh --steam-root <dir>  where Steam is, if not in its usual place
#   ./uninstall.sh                   takes everything out again
#
# What it changes, and nothing else:
#
# - The TPF3-MP mod, mod/tpf3mp_1 in this package, goes into Steam's folder
#   for your Transport Fever 3 mods, <Steam>/userdata/<account>/3493540/local/mods,
#   or the folder --mods-dir names. A tpf3mp_1 already there is moved to
#   TPF3-MP's backups folder first (Linux: ~/.local/share/TPF3-MP/backups,
#   macOS: ~/Library/Application Support/TPF3-MP/backups).
# - The hook library goes into the game's folder. On Linux the script then
#   prints the Steam launch option that loads it.
# - tpf3mp-install.txt in the game's folder records what was installed, so
#   a reinstall replaces only TPF3-MP's own files.
#
# It changes nothing when anything looks wrong: the game is running, or the
# record names anything but TPF3-MP's own files. When a step fails, the
# steps before it are undone. Nothing is deleted: what it replaces or takes
# out goes to the backups folder.
set -euo pipefail

STEAM_APP=3493540
MOD_NAME=tpf3mp_1
RECORD_NAME=tpf3mp-install.txt
PACKAGE=$(cd "$(dirname "$0")" && pwd -P)
case "$(uname -s)" in
  Darwin)
    HOOK_NAME=libtpf3mp_hook.dylib
    DATA_DIR="$HOME/Library/Application Support/TPF3-MP"
    ;;
  *)
    HOOK_NAME=libtpf3mp_hook.so
    DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/TPF3-MP"
    ;;
esac
BACKUPS="$DATA_DIR/backups/$(date +%Y%m%d-%H%M%S)-$$"

game_dir=""
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
    *) game_dir=$1 ;;
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
          printf '%s' "${undo_back[$i]}" >"${undo_path[$i]}"
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
    say "Have Steam verify the game's files; backups are in $BACKUPS."
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
# in one of them.
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
# first runs.
find_mods_dir() {
  local root local_dir found=""
  while IFS= read -r root; do
    for local_dir in "$root"/userdata/[0-9]*/"$STEAM_APP"/local; do
      [ -d "$local_dir" ] || continue
      if [ -z "$found" ] || [ "$local_dir" -nt "$found" ]; then found=$local_dir; fi
    done
  done < <(steam_roots)
  [ -n "$found" ] && printf '%s\n' "$found/mods"
}

assert_game_closed() { # GAME
  local processes line
  processes=$(ps -A -o args= 2>/dev/null || true)
  while IFS= read -r line; do
    case "$line" in
      "$1"/*) fail "Close Transport Fever 3 first: ${line%% *} is running from its folder." ;;
    esac
  done <<<"$processes"
}

# What an earlier install recorded, refused unless it names only TPF3-MP's
# own files: the record says what to move out.
rec_version=""
rec_hook=""
rec_mod=""
have_record=0
read_record() { # GAME
  local file="$1/$RECORD_NAME" line
  [ -f "$file" ] || return 0
  have_record=1
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      version=*) rec_version=${line#version=} ;;
      hook=*) rec_hook=${line#hook=} ;;
      mod=*) rec_mod=${line#mod=} ;;
      "") ;;
      *) fail "$file names things TPF3-MP does not install." ;;
    esac
  done <"$file"
  case "$rec_hook" in "" | libtpf3mp_hook.so | libtpf3mp_hook.dylib) ;; *)
    fail "$file names files TPF3-MP does not install."
    ;;
  esac
  case "$rec_mod" in "" | /*/"$MOD_NAME") ;; *)
    fail "$file names files TPF3-MP does not install."
    ;;
  esac
  case "$rec_mod" in */../* | */./*) fail "$file names files TPF3-MP does not install." ;; esac
  return 0
}

write_record() { # GAME VERSION HOOK MOD
  local file="$1/$RECORD_NAME" old=""
  [ ! -f "$file" ] || old=$(cat "$file")
  printf 'version=%s\nhook=%s\nmod=%s\n' "$2" "$3" "$4" >"$file"
  on_failure rewrite "$file" "$old"
}

install_hook() { # GAME
  local target="$1/$HOOK_NAME" kept
  if [ -e "$target" ]; then
    kept=$(move_into "$target")
    on_failure moveback "$kept" "$target"
  fi
  cp -- "$PACKAGE/$HOOK_NAME" "$target"
  on_failure remove "$target"
  say "Installed $HOOK_NAME."
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
  say "Installed the mod in $target."
}

do_install() { # GAME
  local game=$1 version=unknown hook="$rec_hook" mod="$rec_mod" mods="$mods_dir"
  if [ -f "$PACKAGE/tpf3mp-package.json" ]; then
    version=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
      "$PACKAGE/tpf3mp-package.json" | head -1)
  fi
  # Every check before the first change.
  if [ -d "$PACKAGE/mod/$MOD_NAME" ]; then
    [ -n "$mods" ] || mods=$(find_mods_dir || true)
    [ -n "$mods" ] || fail "Steam has no folder for your Transport Fever 3 mods yet: start the game once, then install again. --mods-dir names the mods folder instead."
    say "Mods folder: $mods"
  fi

  if [ -f "$PACKAGE/$HOOK_NAME" ]; then
    install_hook "$game"
    hook=$HOOK_NAME
  else
    say "This package has no hook library."
  fi
  if [ -d "$PACKAGE/mod/$MOD_NAME" ]; then
    install_mod "$mods"
    mod="$mods/$MOD_NAME"
  else
    say "This package has no TPF3-MP mod yet."
  fi
  write_record "$game" "$version" "$hook" "$mod"
  say ""
  say "TPF3-MP $version is installed into Transport Fever 3."
  if [ "$hook" = libtpf3mp_hook.so ]; then
    say "In Steam, set the game's launch options (right-click the game, Properties) to:"
    say "  LD_PRELOAD=\"$game/$HOOK_NAME\" %command%"
  elif [ -n "$hook" ]; then
    say "How the game loads the hook on macOS is known only once the game is out."
  fi
}

do_uninstall() { # GAME
  local game=$1 kept
  [ "$have_record" = 1 ] || fail "TPF3-MP is not installed in $game."
  if [ -n "$rec_hook" ] && [ -e "$game/$rec_hook" ]; then
    kept=$(move_into "$game/$rec_hook")
    on_failure moveback "$kept" "$game/$rec_hook"
    say "Took $rec_hook out."
  fi
  if [ -n "$rec_mod" ] && [ -d "$rec_mod" ]; then
    kept=$(move_into "$rec_mod")
    on_failure moveback "$kept" "$rec_mod"
    say "Took the mod out of $(dirname "$rec_mod")."
  fi
  kept=$(move_into "$game/$RECORD_NAME")
  on_failure moveback "$kept" "$game/$RECORD_NAME"
  say ""
  say "TPF3-MP is taken out of Transport Fever 3."
  if [ "$rec_hook" = libtpf3mp_hook.so ]; then
    say "Clear the LD_PRELOAD launch option in Steam too."
  fi
}

if [ -n "$game_dir" ]; then
  [ -d "$game_dir" ] || fail "$game_dir is not a folder."
  game=$(cd "$game_dir" && pwd -P)
else
  game=$(find_game || true)
  [ -n "$game" ] || fail "Transport Fever 3 was not found in Steam. Give the game's folder: in Steam, right-click the game, Manage, Browse local files."
fi
say "Transport Fever 3: $game"
assert_game_closed "$game"
read_record "$game"
if [ "$uninstall" = 1 ]; then do_uninstall "$game"; else do_install "$game"; fi
[ ! -d "$BACKUPS" ] || say "What was replaced or taken out is in $BACKUPS."
finished=1

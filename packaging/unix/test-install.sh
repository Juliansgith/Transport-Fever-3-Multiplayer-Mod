#!/usr/bin/env bash
# Tests install.sh against made-up packages and game folders, the way
# players run it: through its exit code and the files it leaves. CI runs it
# on Linux and, with macOS's own bash 3.2, on macOS; so can anyone:
#
#   bash packaging/unix/test-install.sh
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd -P)
# Physical paths: macOS keeps temporary files under /var, a link to
# /private/var, and the installer compares physical paths.
ROOT=$(cd "$(mktemp -d "${TMPDIR:-/tmp}/tpf3mp-install-test.XXXXXX")" && pwd -P)
BASH_TO_TEST=${BASH_TO_TEST:-bash}
FAILURES=0
OUTPUT=""
case "$(uname -s)" in
  Darwin) HOOK=libtpf3mp_hook.dylib ;;
  *) HOOK=libtpf3mp_hook.so ;;
esac

write() { # PATH TEXT
  mkdir -p "$(dirname "$1")"
  printf '%s' "$2" >"$1"
}

# A package with the hook and the mod, and a game folder, in a home of
# their own so the backups stay in the test.
setup() { # NAME
  S="$ROOT/$1"
  PACKAGE="$S/package"
  GAME="$S/game"
  MODS="$S/mods"
  mkdir -p "$PACKAGE" "$GAME" "$S/home"
  cp "$HERE/install.sh" "$HERE/uninstall.sh" "$PACKAGE/"
  write "$PACKAGE/tpf3mp-package.json" '{"version":"9.8.7","platform":"test"}'
  write "$PACKAGE/$HOOK" 'hook'
  write "$PACKAGE/mod/tpf3mp_1/mod.lua" '-- mod'
  write "$PACKAGE/mod/tpf3mp_1/res/x.lua" '-- x'
  write "$GAME/game" 'game'
}

# Runs the package's installer as a player would; sets CODE and OUTPUT.
run() { # SCRIPT ARGS...
  local script=$1
  shift
  OUTPUT=$(cd "$S" && HOME="$S/home" XDG_DATA_HOME="" "$BASH_TO_TEST" "$PACKAGE/$script" "$@" 2>&1)
  CODE=$?
}
install() { run install.sh "$GAME" --mods-dir "$MODS" "$@"; }

# Ends the test, which runs in a subshell, at the first thing that is not
# as expected. (`set -e` would not: bash ignores it in a tested command.)
check() { # DESCRIPTION CONDITION...
  local what=$1
  shift
  if ! "$@"; then
    printf 'expected: %s\n%s\n' "$what" "$OUTPUT" >&2
    exit 1
  fi
}
same() { [ "$(cat "$1" 2>/dev/null)" = "$2" ]; }
code() { [ "$CODE" = "$1" ]; }

t() { # NAME FUNCTION
  if ("$2"); then
    echo "ok   $1"
  else
    echo "FAIL $1"
    FAILURES=$((FAILURES + 1))
  fi
}

installs_and_uninstalls() {
  setup install
  install
  check 'installed' code 0
  check 'the hook' same "$GAME/$HOOK" hook
  check 'the mod' same "$MODS/tpf3mp_1/res/x.lua" '-- x'
  check 'the record' grep -q '^version=9.8.7$' "$GAME/tpf3mp-install.txt"
  if [ "$HOOK" = libtpf3mp_hook.so ]; then
    check 'the launch option' grep -q 'LD_PRELOAD=' <<<"$OUTPUT"
  fi

  # Again, with a newer mod: the old one goes to the backups.
  write "$PACKAGE/mod/tpf3mp_1/res/x.lua" '-- x 2'
  install
  check 'reinstalled' code 0
  check 'the newer mod' same "$MODS/tpf3mp_1/res/x.lua" '-- x 2'
  check 'no staging folder left' [ "$(ls -A "$MODS")" = tpf3mp_1 ]

  run uninstall.sh "$GAME"
  check 'uninstalled' code 0
  check 'the hook gone' [ ! -e "$GAME/$HOOK" ]
  check 'the mod gone' [ ! -e "$MODS/tpf3mp_1" ]
  check 'the record gone' [ ! -e "$GAME/tpf3mp-install.txt" ]
  check 'the game untouched' same "$GAME/game" game
  check 'the mods kept in the backups' [ "$(find "$S/home" -name x.lua | wc -l)" -ge 2 ]
  run uninstall.sh "$GAME"
  check 'nothing left to uninstall' code 1
}

refuses_a_record_naming_other_files() {
  setup record
  install
  check 'installed' code 0
  local record bad
  record=$(cat "$GAME/tpf3mp-install.txt")
  for bad in 'hook=../elsewhere.so' 'mod=/etc' "mod=$MODS/../tpf3mp_1" 'rm=-rf /'; do
    printf '%s\n%s\n' "$record" "$bad" >"$GAME/tpf3mp-install.txt"
    install
    check "refused $bad" code 1
    run uninstall.sh "$GAME"
    check "uninstall refused $bad" code 1
    check 'the hook untouched' same "$GAME/$HOOK" hook
    check 'the mod untouched' same "$MODS/tpf3mp_1/mod.lua" '-- mod'
  done
}

finds_the_game_through_steam() {
  setup steam
  local steam="$S/Steam" library="$S/Library"
  local game="$library/steamapps/common/Transport Fever 3"
  mkdir -p "$game" "$steam/userdata/12345/3493540/local"
  write "$steam/steamapps/libraryfolders.vdf" "$(printf '"libraryfolders"\n{\n\t"0"\n\t{\n\t\t"path"\t\t"%s"\n\t}\n}\n' "$library")"
  write "$library/steamapps/appmanifest_3493540.acf" "$(printf '"AppState"\n{\n\t"appid"\t\t"3493540"\n\t"installdir"\t\t"Transport Fever 3"\n}\n')"
  run install.sh --steam-root "$steam"
  check 'installed' code 0
  check 'into the game Steam names' same "$game/$HOOK" hook
  check "the mod in Steam's mods folder" same "$steam/userdata/12345/3493540/local/mods/tpf3mp_1/mod.lua" '-- mod'
}

refuses_while_the_game_runs() {
  setup running
  cp "$(command -v sleep)" "$GAME/TransportFever3"
  "$GAME/TransportFever3" 30 &
  local game=$!
  sleep 0.5
  install
  kill "$game" 2>/dev/null || true
  wait "$game" 2>/dev/null || true
  check 'refused' code 1
  check 'says to close the game' grep -q 'Close Transport Fever 3' <<<"$OUTPUT"
  check 'no hook' [ ! -e "$GAME/$HOOK" ]
}

puts_everything_back_when_a_step_fails() {
  setup rollback
  # The mods folder cannot be made: a file stands in its way.
  write "$MODS" 'not a folder'
  install
  check 'failed' code 1
  check 'no hook' [ ! -e "$GAME/$HOOK" ]
  check 'no record' [ ! -e "$GAME/tpf3mp-install.txt" ]
  check 'says nothing changed' grep -q 'Nothing was changed' <<<"$OUTPUT"
}

t 'installs the hook and the mod, and takes them out again' installs_and_uninstalls
t 'refuses a record that names other files' refuses_a_record_naming_other_files
t 'finds the game and the mods folder through Steam' finds_the_game_through_steam
case "$(uname -s)" in
  # Git Bash's ps is not the one players' systems have.
  MINGW* | MSYS* | CYGWIN*) echo "skip refuses while the game runs (a Windows shell)" ;;
  *) t 'refuses while the game runs' refuses_while_the_game_runs ;;
esac
t 'puts everything back when a step fails' puts_everything_back_when_a_step_fails

rm -rf "$ROOT"
if [ "$FAILURES" -gt 0 ]; then
  echo "$FAILURES installer test(s) failed"
  exit 1
fi
echo 'all installer tests passed'

#!/usr/bin/env bash
# The manual acceptance check "a server upgrade that keeps running games"
# (AGENTS.md), played through with the fake game.
#
#   tools/acceptance/upgrade-check.sh <old bin dir> <new bin dir> [work dir]
#
# Each bin dir holds tpf3mp-server, tpf3mp-agent and tpf3mp-fakegame of one
# version: the old from the release players run (build `main`, or unpack its
# package), the new from the commit being promoted. Two players (an agent
# and a fake game each) play a room on the old server; the server is
# stopped and the new one started on the same data directory, as
# `docker compose pull && docker compose up -d` does. Then:
#
# - the new server must restore the running room;
# - an old launcher must connect, or be told to update TPF3-MP when the
#   protocol changed;
# - the players, on the new version, come back with their invite and must
#   play on from the room's saved world, their worlds agreeing.
#
# The server is stopped with SIGTERM, which it handles as `docker stop`
# does; Git Bash on Windows kills it outright instead, like a crash. Needs
# openssl and curl. Ports: $TPF3MP_CHECK_PORT (UDP, default 39470),
# $TPF3MP_CHECK_ADMIN_PORT (default 39479), and $TPF3MP_CHECK_UI_PORT plus
# the next port (HTTP, default 47471), on loopback.
# Set TPF3MP_CHECK_OLD_CLIENT_EXPECTED to `connect` for an unchanged protocol
# or `update` for a protocol change. Requiring the expected result keeps the
# compatibility check meaningful for either kind of upgrade.
set -uo pipefail

if [ $# -lt 2 ]; then
  sed -n '2,/^set /p' "$0" | sed '$d; s/^# \{0,1\}//'
  exit 2
fi
OLD_CLIENT_EXPECTED=${TPF3MP_CHECK_OLD_CLIENT_EXPECTED:-}
case "$OLD_CLIENT_EXPECTED" in
  connect|update) ;;
  *) echo "set TPF3MP_CHECK_OLD_CLIENT_EXPECTED to connect or update" >&2; exit 2 ;;
esac
OLD=$(cd "$1" && pwd) || exit 2
NEW=$(cd "$2" && pwd) || exit 2
WORK=${3:-$(mktemp -d)}
SERVER=127.0.0.1:${TPF3MP_CHECK_PORT:-39470}
ADMIN=127.0.0.1:${TPF3MP_CHECK_ADMIN_PORT:-39479}
UI_PORT=${TPF3MP_CHECK_UI_PORT:-47471}
UI_PORT_BOB=$((UI_PORT + 1))

exe() { # DIR NAME: the program, with .exe on Windows
  if [ -e "$1/$2.exe" ]; then echo "$1/$2.exe"; else echo "$1/$2"; fi
}
for dir in "$OLD" "$NEW"; do
  for name in tpf3mp-server tpf3mp-agent tpf3mp-fakegame; do
    [ -x "$(exe "$dir" "$name")" ] || {
      echo "no $name in $dir"
      exit 2
    }
  done
done

mkdir -p "$WORK"
cd "$WORK" || exit 2
rm -rf data worlds-* ./*.log ./*.key

PIDS=()
cleanup() {
  for pid in "${PIDS[@]}"; do
    kill "$pid" 2>/dev/null
  done
}
trap cleanup EXIT

say() { printf '[%s] %s\n' "$(date +%H:%M:%S)" "$*"; }
fail() {
  say "FAIL: $*; the logs are in $WORK"
  exit 1
}
wait_for() { # SECONDS DESCRIPTION COMMAND...
  local seconds=$1 what=$2
  shift 2
  for _ in $(seq "$seconds"); do
    "$@" >/dev/null 2>&1 && return 0
    sleep 1
  done
  fail "timed out waiting for $what"
}
metric() {
  curl -sf "http://$ADMIN/metrics" | sed -n "s/^tpf3mp_$1 //p"
}
at_least() { # METRIC VALUE
  local value
  value=$(metric "$1")
  [ -n "$value" ] && [ "$value" -ge "$2" ]
}
start() { # LOG COMMAND...; sets STARTED
  local log=$1
  shift
  "$@" >"$log" 2>&1 &
  STARTED=$!
  PIDS+=("$STARTED")
}
start_server() { # BIN LOG
  RUST_LOG=info start "$2" "$(exe "$1" tpf3mp-server)" --listen "$SERVER" --cert cert.pem --key key.pem \
    --secret-file invite.key --data-dir data --admin-listen "$ADMIN" \
    --save-every-secs 10 --save-gap-secs 5
  SERVER_PID=$STARTED
  wait_for 30 "the server in $2" curl -sf "http://$ADMIN/healthz"
}
launcher_player() { # BIN NAME LOG LINK WORLDS HTTP_PORT
  local bin=$1 name=$2 log=$3 link=$4 worlds=$5 port=$6
  start "$log" "$(exe "$bin" tpf3mp-agent)" launcher --listen "127.0.0.1:$port" \
    --server "$SERVER" --pin-cert cert.der --name "$name" --identity "$name.key" \
    --game-build fake --game-link "$link" --worlds "$worlds" --no-open
}
launcher_token() { # LOG: token from the local page URL printed by the launcher
  sed -n 's|^TPF3-MP launcher: http://127\.0\.0\.1:[0-9]*/#||p' "$1" | head -1
}
api_state() { # HTTP_PORT TOKEN
  curl -sf --max-time 5 -H "X-Launcher-Token: $2" "http://127.0.0.1:$1/api/state"
}
api_action() { # HTTP_PORT TOKEN JSON DESCRIPTION
  if ! curl -sf --max-time 15 -H "X-Launcher-Token: $2" -H 'Content-Type: application/json' \
    --data "$3" "http://127.0.0.1:$1/api/action" >/dev/null; then
    fail "the launcher could not $4"
  fi
}
launcher_connected() { # HTTP_PORT TOKEN
  api_state "$1" "$2" | grep -Fq '"server_version":"'
}
launcher_has_invite() { # HTTP_PORT TOKEN INVITE
  api_state "$1" "$2" | grep -Fq "\"invite\":\"$3\""
}
launcher_invite() { # HTTP_PORT TOKEN
  api_state "$1" "$2" | grep -o '"invite":"[A-Z0-9]*"' | head -1 | sed 's/"invite":"\([^"]*\)"/\1/'
}
launcher_has_room_invite() { # HTTP_PORT TOKEN
  [ -n "$(launcher_invite "$1" "$2")" ]
}
launcher_game_attached() { # HTTP_PORT TOKEN
  api_state "$1" "$2" | grep -Fq '"attached":"fake hook"'
}
launcher_both_ready() { # HTTP_PORT TOKEN
  local ready
  ready=$(api_state "$1" "$2" | grep -o '"ready":true' | wc -l)
  [ "$ready" -ge 2 ]
}
launcher_running() { # HTTP_PORT TOKEN
  api_state "$1" "$2" | grep -Fq '"phase":"running"'
}

# A certificate that stays the same across the upgrade, as a real one does.
cat >req.cnf <<'EOF'
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = localhost
[ext]
basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature
extendedKeyUsage = serverAuth
subjectAltName = DNS:localhost,IP:127.0.0.1
EOF
openssl req -x509 -config req.cnf -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
  -nodes -keyout key.pem -out cert.pem -days 2 2>/dev/null || fail "making a certificate"
openssl x509 -in cert.pem -outform DER -out cert.der || fail "making a certificate"

say "old: $("$(exe "$OLD" tpf3mp-server)" --version); new: $("$(exe "$NEW" tpf3mp-server)" --version)"
start_server "$OLD" server-old.log

say "ann hosts and bob joins, on the old version"
OLD_PIDS=()
launcher_player "$OLD" ann ann-old.log tpf3mp.check.ann worlds-ann-old "$UI_PORT"
OLD_PIDS+=("$STARTED")
wait_for 30 "ann's launcher API" grep -q '^TPF3-MP launcher: http://127\.0\.0\.1:' ann-old.log
ANN_TOKEN=$(launcher_token ann-old.log)
[ -n "$ANN_TOKEN" ] || fail "ann's launcher printed no local API token"
launcher_player "$OLD" bob bob-old.log tpf3mp.check.bob worlds-bob-old "$UI_PORT_BOB"
OLD_PIDS+=("$STARTED")
wait_for 30 "bob's launcher API" grep -q '^TPF3-MP launcher: http://127\.0\.0\.1:' bob-old.log
BOB_TOKEN=$(launcher_token bob-old.log)
[ -n "$BOB_TOKEN" ] || fail "bob's launcher printed no local API token"
api_action "$UI_PORT" "$ANN_TOKEN" "{\"action\":\"connect\",\"server\":\"$SERVER\",\"name\":\"ann\"}" "connect ann to the old server"
api_action "$UI_PORT_BOB" "$BOB_TOKEN" "{\"action\":\"connect\",\"server\":\"$SERVER\",\"name\":\"bob\"}" "connect bob to the old server"
wait_for 30 "both old launchers to connect" launcher_connected "$UI_PORT" "$ANN_TOKEN"
wait_for 30 "bob's old launcher to connect" launcher_connected "$UI_PORT_BOB" "$BOB_TOKEN"
api_action "$UI_PORT" "$ANN_TOKEN" \
  '{"action":"create","room":"upgrade-check","max_players":8,"password":null,"rules":null,"start_save":null,"listing":null,"competitive":false}' \
  "create the old room"
wait_for 30 "ann's invite" launcher_has_room_invite "$UI_PORT" "$ANN_TOKEN"
INVITE=$(launcher_invite "$UI_PORT" "$ANN_TOKEN")
api_action "$UI_PORT_BOB" "$BOB_TOKEN" "{\"action\":\"join\",\"invite\":\"$INVITE\",\"password\":null}" "join ann's room"
wait_for 30 "bob to join ann's room" launcher_has_invite "$UI_PORT_BOB" "$BOB_TOKEN" "$INVITE"
start game-ann-old.log "$(exe "$OLD" tpf3mp-fakegame)" tpf3mp.check.ann --seed 1 --world-seed 7
OLD_PIDS+=("$STARTED")
start game-bob-old.log "$(exe "$OLD" tpf3mp-fakegame)" tpf3mp.check.bob --seed 2 --world-seed 7
OLD_PIDS+=("$STARTED")
wait_for 30 "both old games to attach" launcher_game_attached "$UI_PORT" "$ANN_TOKEN"
wait_for 30 "bob's old game to attach" launcher_game_attached "$UI_PORT_BOB" "$BOB_TOKEN"
api_action "$UI_PORT" "$ANN_TOKEN" '{"action":"ready","ready":true}' "ready ann"
api_action "$UI_PORT_BOB" "$BOB_TOKEN" '{"action":"ready","ready":true}' "ready bob"
wait_for 30 "both old players to be ready" launcher_both_ready "$UI_PORT" "$ANN_TOKEN"
api_action "$UI_PORT" "$ANN_TOKEN" '{"action":"start"}' "start the old game"

wait_for 60 "the game to start" at_least games_started_total 1
wait_for 30 "the old room to enter its running phase" launcher_running "$UI_PORT" "$ANN_TOKEN"
wait_for 90 "a save" at_least saves_total 1
wait_for 30 "at least two turns sealed before the upgrade" at_least turns_sealed_total 2
sleep 5
OLD_TURNS=$(metric turns_sealed_total)
OLD_SAVES=$(metric saves_total)
say "before: $OLD_TURNS turns sealed, $OLD_SAVES saves"

say "upgrading the server"
kill "$SERVER_PID"
wait_for 30 "the old server to stop" bash -c "! curl -sf http://$ADMIN/healthz"
start_server "$NEW" server-new.log
grep -q 'restored running rooms' server-new.log || fail "the new server restored no room"
say "the new server restored $(metric rooms) room(s)"

"$(exe "$OLD" tpf3mp-agent)" connect "$SERVER" --pin-cert cert.der --name carol \
  --identity carol.key >connect-old.log 2>&1
if grep -q '^connected as' connect-old.log; then
  [ "$OLD_CLIENT_EXPECTED" = connect ] || fail "the old launcher connected, but an update was expected"
  say "an old launcher still connects: the protocol is unchanged"
elif grep -q 'update TPF3-MP' connect-old.log; then
  [ "$OLD_CLIENT_EXPECTED" = update ] || fail "the old launcher required an update, but a connection was expected"
  say "an old launcher is told: $(tr -d '\r' <connect-old.log | tail -1)"
else
  fail "an old launcher got neither a connection nor the advice to update"
fi

say "the players come back on the new version, with their invite"
kill "${OLD_PIDS[@]}" 2>/dev/null
sleep 2
launcher_player "$NEW" ann ann-new.log tpf3mp.check.ann2 worlds-ann-new "$UI_PORT"
wait_for 30 "ann's new launcher API" grep -q '^TPF3-MP launcher: http://127\.0\.0\.1:' ann-new.log
ANN_NEW_TOKEN=$(launcher_token ann-new.log)
[ -n "$ANN_NEW_TOKEN" ] || fail "ann's new launcher printed no local API token"
launcher_player "$NEW" bob bob-new.log tpf3mp.check.bob2 worlds-bob-new "$UI_PORT_BOB"
wait_for 30 "bob's new launcher API" grep -q '^TPF3-MP launcher: http://127\.0\.0\.1:' bob-new.log
BOB_NEW_TOKEN=$(launcher_token bob-new.log)
[ -n "$BOB_NEW_TOKEN" ] || fail "bob's new launcher printed no local API token"
api_action "$UI_PORT" "$ANN_NEW_TOKEN" "{\"action\":\"connect\",\"server\":\"$SERVER\",\"name\":\"ann\"}" "connect ann to the new server"
api_action "$UI_PORT_BOB" "$BOB_NEW_TOKEN" "{\"action\":\"connect\",\"server\":\"$SERVER\",\"name\":\"bob\"}" "connect bob to the new server"
wait_for 30 "ann's new launcher to connect" launcher_connected "$UI_PORT" "$ANN_NEW_TOKEN"
wait_for 30 "bob's new launcher to connect" launcher_connected "$UI_PORT_BOB" "$BOB_NEW_TOKEN"
api_action "$UI_PORT" "$ANN_NEW_TOKEN" "{\"action\":\"join\",\"invite\":\"$INVITE\",\"password\":null}" "rejoin ann to the restored room"
api_action "$UI_PORT_BOB" "$BOB_NEW_TOKEN" "{\"action\":\"join\",\"invite\":\"$INVITE\",\"password\":null}" "rejoin bob to the restored room"
wait_for 30 "ann to rejoin the running room" launcher_running "$UI_PORT" "$ANN_NEW_TOKEN"
wait_for 30 "bob to rejoin the running room" launcher_running "$UI_PORT_BOB" "$BOB_NEW_TOKEN"
start game-ann-new.log "$(exe "$NEW" tpf3mp-fakegame)" tpf3mp.check.ann2 --seed 1 \
  --world-seed 7 --steps 400
start game-bob-new.log "$(exe "$NEW" tpf3mp-fakegame)" tpf3mp.check.bob2 --seed 2 \
  --world-seed 7 --steps 400
wait_for 30 "ann's new game to attach" launcher_game_attached "$UI_PORT" "$ANN_NEW_TOKEN"
wait_for 30 "bob's new game to attach" launcher_game_attached "$UI_PORT_BOB" "$BOB_NEW_TOKEN"

wait_for 240 "both games to reach step 400" \
  bash -c 'grep -q "^ran " game-ann-new.log && grep -q "^ran " game-bob-new.log'
for name in ann bob; do
  say "$name: $(grep '^ran ' "game-$name-new.log")"
  grep -q ' 0 divergences' "game-$name-new.log" || fail "$name's game diverged"
  grep -qE 'loaded [1-9][0-9]* worlds from the room' "game-$name-new.log" ||
    fail "$name's game loaded no world from the room"
done
grep '^  lane ' game-ann-new.log >lanes-ann
grep '^  lane ' game-bob-new.log >lanes-bob
cmp -s lanes-ann lanes-bob || fail "the two worlds differ"
[ "$(metric divergences_total)" = 0 ] || fail "the server saw divergences"
say "PASS: the room survived the upgrade and the players played on in agreement"

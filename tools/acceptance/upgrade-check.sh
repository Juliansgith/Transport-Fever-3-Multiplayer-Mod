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
# openssl and curl. Ports: $TPF3MP_CHECK_PORT (UDP, default 39470) and
# $TPF3MP_CHECK_ADMIN_PORT (default 39479), on loopback.
set -uo pipefail

if [ $# -lt 2 ]; then
  sed -n '2,/^set /p' "$0" | sed '$d; s/^# \{0,1\}//'
  exit 2
fi
OLD=$(cd "$1" && pwd) || exit 2
NEW=$(cd "$2" && pwd) || exit 2
WORK=${3:-$(mktemp -d)}
SERVER=127.0.0.1:${TPF3MP_CHECK_PORT:-39470}
ADMIN=127.0.0.1:${TPF3MP_CHECK_ADMIN_PORT:-39479}

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
  start "$2" "$(exe "$1" tpf3mp-server)" --listen "$SERVER" --cert cert.pem --key key.pem \
    --secret-file invite.key --data-dir data --admin-listen "$ADMIN" \
    --save-every-secs 10 --save-gap-secs 5
  SERVER_PID=$STARTED
  wait_for 30 "the server in $2" curl -sf "http://$ADMIN/healthz"
}
player() { # BIN NAME LOG LINK WORLDS COMMAND ARGS...: an agent for NAME
  local bin=$1 name=$2 log=$3 link=$4 worlds=$5 command=$6
  shift 6
  start "$log" "$(exe "$bin" tpf3mp-agent)" "$command" "$SERVER" --pin-cert cert.der \
    --name "$name" --identity "$name.key" --game-build fake --game-link "$link" \
    --worlds "$worlds" "$@"
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
player "$OLD" ann ann-old.log tpf3mp.check.ann worlds-ann-old host --start-with 2 \
  --room-name upgrade-check
OLD_PIDS+=("$STARTED")
wait_for 30 "ann's invite" grep -q '^invite: ' ann-old.log
INVITE=$(sed -n 's/^invite: //p' ann-old.log | head -1)
start game-ann-old.log "$(exe "$OLD" tpf3mp-fakegame)" tpf3mp.check.ann --seed 1 --world-seed 7
OLD_PIDS+=("$STARTED")
player "$OLD" bob bob-old.log tpf3mp.check.bob worlds-bob-old join "$INVITE"
OLD_PIDS+=("$STARTED")
start game-bob-old.log "$(exe "$OLD" tpf3mp-fakegame)" tpf3mp.check.bob --seed 2 --world-seed 7
OLD_PIDS+=("$STARTED")

wait_for 60 "the game to start" at_least games_started_total 1
wait_for 90 "a save" at_least saves_total 1
sleep 5
say "before: $(metric turns_sealed_total) turns sealed, $(metric saves_total) saves"

say "upgrading the server"
kill "$SERVER_PID"
wait_for 30 "the old server to stop" bash -c "! curl -sf http://$ADMIN/healthz"
start_server "$NEW" server-new.log
grep -q 'restored running rooms' server-new.log || fail "the new server restored no room"
say "the new server restored $(metric rooms) room(s)"

"$(exe "$OLD" tpf3mp-agent)" connect "$SERVER" --pin-cert cert.der --name carol \
  --identity carol.key >connect-old.log 2>&1
if grep -q '^connected as' connect-old.log; then
  say "an old launcher still connects: the protocol is unchanged"
elif grep -q 'update TPF3-MP' connect-old.log; then
  say "an old launcher is told: $(tr -d '\r' <connect-old.log | tail -1)"
else
  fail "an old launcher got neither a connection nor the advice to update"
fi

say "the players come back on the new version, with their invite"
kill "${OLD_PIDS[@]}" 2>/dev/null
sleep 2
player "$NEW" ann ann-new.log tpf3mp.check.ann2 worlds-ann-new join "$INVITE"
start game-ann-new.log "$(exe "$NEW" tpf3mp-fakegame)" tpf3mp.check.ann2 --seed 1 \
  --world-seed 7 --steps 400
player "$NEW" bob bob-new.log tpf3mp.check.bob2 worlds-bob-new join "$INVITE"
start game-bob-new.log "$(exe "$NEW" tpf3mp-fakegame)" tpf3mp.check.bob2 --seed 2 \
  --world-seed 7 --steps 400

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

#!/usr/bin/env bash
# Gathers what a bug report about the server needs into one archive: the
# server's log, its state, its metrics and the host's resources. Run it on
# the server host, from this directory:
#
#   ./collect-logs.sh                 # the last 24 hours
#   ./collect-logs.sh --since 2h      # a shorter window
#   ./collect-logs.sh --for s-3f2a... # only lines about one player's session
#
# --for takes anything that appears in the log lines to keep: a support ID
# (session) from a player's launcher, a player ID or a room ID.
#
# The log never contains IP addresses or invite tokens, and nothing here
# reads the data volume, so the archive holds no secrets. It is written to
# the current directory as tpf3mp-logs-<time>.tar.gz.
set -euo pipefail

since="24h"
only=""
while [ $# -gt 0 ]; do
  case "$1" in
    --since) since="${2:?--since needs a duration, such as 2h}"; shift 2 ;;
    --for) only="${2:?--for needs a session, player or room ID}"; shift 2 ;;
    -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

cd "$(dirname "$0")"
service="tpf3mp-server"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
out="$dir/tpf3mp-logs-$stamp"
mkdir -p "$out"

# The log, as the container wrote it, with Docker's timestamps.
if [ -n "$only" ]; then
  docker compose logs --no-color --timestamps --since "$since" "$service" 2>&1 \
    | grep -F -- "$only" > "$out/server.log" || true
else
  docker compose logs --no-color --timestamps --since "$since" "$service" \
    > "$out/server.log" 2>&1 || true
fi

# The container: running or not, restarts, whether it ran out of memory.
{
  echo "== collected $stamp, since $since${only:+, lines with $only}"
  echo "== docker compose ps"
  docker compose ps 2>&1 || true
  container="$(docker compose ps -q "$service" 2>/dev/null || true)"
  if [ -n "$container" ]; then
    echo "== state"
    docker inspect --format \
      'status={{.State.Status}} started={{.State.StartedAt}} restarts={{.RestartCount}} oom_killed={{.State.OOMKilled}} exit={{.State.ExitCode}} image={{.Image}}' \
      "$container" 2>&1 || true
  fi
  echo "== the server's version, from its log"
  docker compose logs --no-color "$service" 2>/dev/null \
    | grep -m1 -oE 'version="[^"]*"|"version":"[^"]*"' || echo "not found"
} > "$out/state.txt"

# Health and metrics, from the admin endpoint on the host's loopback.
curl -fsS --max-time 5 http://127.0.0.1:9470/healthz > "$out/healthz.txt" 2>&1 || echo "unreachable" > "$out/healthz.txt"
curl -fsS --max-time 5 http://127.0.0.1:9470/metrics > "$out/metrics.txt" 2>&1 || echo "unreachable" > "$out/metrics.txt"

# The host: disk, memory and Docker's own resource use.
{
  echo "== uname"; uname -a
  echo "== uptime"; uptime
  echo "== disk"; df -h
  echo "== memory"; free -h 2>/dev/null || true
  echo "== docker"; docker version --format '{{.Server.Version}}' 2>&1 || true
  docker system df 2>&1 || true
} > "$out/host.txt"

tar -czf "tpf3mp-logs-$stamp.tar.gz" -C "$dir" "tpf3mp-logs-$stamp"
echo "wrote tpf3mp-logs-$stamp.tar.gz ($(wc -l < "$out/server.log") log lines)"

#!/usr/bin/env bash
#
# Run Kakiri on Linux with the *release* engine and the UI served by Vite.
#
# Why this exists: `npm run tauri dev` builds the debug profile (slow DSP, and it
# watches/rebuilds the Rust side constantly). A plain `cargo build --release`
# binary is built without the `custom-protocol` feature, so it loads the
# frontend from the dev URL — the audio path is optimised while UI edits still
# reload instantly.
#
#   ./scripts/run.sh                # start the dev server if needed, run release
#   ./scripts/run.sh --rebuild      # force a Rust rebuild first
#   ./scripts/run.sh --server-only  # just the Vite server (browser preview)
#   ./scripts/run.sh --no-server    # run the binary alone (standalone build)
#
# Rust changes: run with --rebuild (or let the script notice newer sources).
#
set -euo pipefail

# resolve symlinks, so "kakiri" can be a link in ~/.local/bin
SELF="$(readlink -f "${BASH_SOURCE[0]}")"
ROOT="$(cd "$(dirname "$SELF")/.." && pwd)"
cd "$ROOT"

BIN="target/release/soundboard"
PORT=1420
LOG="${TMPDIR:-/tmp}/kakiri-vite.log"
MODE="run" # run | server-only | no-server
REBUILD=0

die() { printf '\033[31m%s\033[0m\n' "$*" >&2; exit 1; }
info() { printf '\033[36m▸\033[0m %s\n' "$*"; }
ok() { printf '\033[32m✓\033[0m %s\n' "$*"; }

APP_ARGS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --rebuild) REBUILD=1 ;;
    --server-only) MODE="server-only" ;;
    --no-server) MODE="no-server" ;;
    -h|--help) sed -n '2,/^set -euo/p' "$SELF" | sed 's/^# \{0,1\}//' | sed '$d'; exit 0 ;;
    --) shift; APP_ARGS+=("$@"); break ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
  shift
done

command -v npm >/dev/null || die "npm is not installed"
command -v cargo >/dev/null || die "cargo is not installed"

# --- dev server ---------------------------------------------------------------

server_up() {
  (exec 3<>"/dev/tcp/127.0.0.1/$PORT") 2>/dev/null && exec 3>&- 3<&-
}

start_server() {
  info "starting the dev server (npm run dev)"
  : >"$LOG"
  setsid npm run dev >"$LOG" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 60); do
    if server_up; then
      ok "dev server on http://localhost:$PORT"
      return 0
    fi
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
      tail -20 "$LOG" >&2
      die "the dev server exited (log: $LOG)"
    fi
    sleep 1
  done
  tail -20 "$LOG" >&2
  die "the dev server never opened port $PORT (log: $LOG)"
}

if [ "$MODE" = "server-only" ]; then
  exec npm run dev
fi

STARTED_SERVER=0

# Find whoever is listening on the dev port and stop its whole process group.
# `setsid` detaches the server into its own session, so `$!` is not enough.
stop_server() {
  local pid pgid
  pid=$(ss -ltnpH "sport = :$PORT" 2>/dev/null | sed -n 's/.*pid=\([0-9]\+\).*/\1/p' | head -1)
  if [ -z "$pid" ]; then
    pid=$(pgrep -f "node .*vite dev" | head -1 || true)
  fi
  [ -n "$pid" ] || return 0
  pgid=$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ' || true)
  if [ -n "$pgid" ]; then
    kill -TERM -- "-$pgid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
  else
    kill -TERM "$pid" 2>/dev/null || true
  fi
  for _ in $(seq 1 20); do
    server_up || return 0
    sleep 0.25
  done
  kill -KILL -- "-${pgid:-$pid}" 2>/dev/null || kill -KILL "$pid" 2>/dev/null || true
}

cleanup() {
  if [ "$STARTED_SERVER" = "1" ]; then
    info "stopping the dev server"
    stop_server
  fi
}
trap cleanup EXIT INT TERM

if [ "$MODE" = "run" ]; then
  if server_up; then
    ok "reusing the dev server already on :$PORT"
  else
    start_server
    STARTED_SERVER=1
  fi
fi

# --- release binary -----------------------------------------------------------

sources_newer_than_bin() {
  [ -x "$BIN" ] || return 0
  local newer
  newer=$(find Cargo.toml src-tauri/Cargo.toml src-tauri/build.rs src-tauri/tauri.conf.json \
                src-tauri/src src-tauri/capabilities crates/*/src crates/*/Cargo.toml \
                -newer "$BIN" -print -quit 2>/dev/null || true)
  [ -n "$newer" ]
}

if [ "$REBUILD" = "1" ] || sources_newer_than_bin; then
  info "building the release engine (first run takes a few minutes)"
  cargo build --release -p soundboard
else
  ok "release engine is up to date"
fi

[ -x "$BIN" ] || die "no binary at $BIN (the build did not run?)"

info "starting $BIN"
"./$BIN" ${APP_ARGS[@]+"${APP_ARGS[@]}"}

#!/usr/bin/env bash
# Sequence runner — boot ONE engine session and feed multiple gapped commands.
#
# run.sh sends a single command per boot; m/t/q probes need the SAME live
# engine (a match doesn't survive a reboot). This feeds a command sequence to
# frayremote with real-time gaps so each command's per-frame effect settles
# before the next is dispatched.
#
# Usage:  ./runseq.sh <boot_wait_s> <gap_s> "cmd1" "cmd2" ...
#   boot_wait_s : seconds to wait before the first command (engine boot→READY,
#                 typically ~30). frayremote only reads stdin after READY, so
#                 this must exceed boot time or commands bunch up.
#   gap_s       : seconds between successive commands.
# Env: FRAY_DIR overrides install path; FRAY_TAIL extra hold after last cmd (default 6).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
FRAY_DIR="${FRAY_DIR:-/Users/jimmy/Library/Application Support/Steam/steamapps/common/Fraymakers}"
BOOT_WAIT="${1:?boot_wait_s}"; shift
GAP="${1:?gap_s}"; shift
TAIL="${FRAY_TAIL:-6}"
PORT="${FRAY_PORT:-$(( (RANDOM % 2000) + 18000 ))}"
TOK="fray-$RANDOM$RANDOM"

BOOT="$FRAY_DIR/hlboot-sdl.dat"
CONN="$FRAY_DIR/_conn.dat"
APPID="$FRAY_DIR/steam_appid.txt"
[ -f "$BOOT" ] || { echo "no hlboot-sdl.dat in $FRAY_DIR" >&2; exit 1; }

cleanup() {
  rm -f "$CONN" "$APPID" 2>/dev/null || true
  kill -9 "${ENG:-0}" 2>/dev/null || true
  kill -9 "${BR:-0}"  2>/dev/null || true
}
trap cleanup EXIT INT TERM

[ -x "$HERE/target/release/fray_patch" ] || cargo build --release --manifest-path "$HERE/Cargo.toml" >/dev/null 2>&1
printf '1420350' > "$APPID"
"$HERE/target/release/fray_patch" "$BOOT" "$CONN" connect "$PORT" "$TOK" >/dev/null 2>&1

# Total engine lifetime must cover the whole feeder.
NCMD=$#
TOTAL=$(( BOOT_WAIT + (NCMD * GAP) + TAIL + 2 ))

# Feeder: wait past boot→READY, then echo each command spaced by GAP.
feeder() {
  sleep "$BOOT_WAIT"
  for c in "$@"; do
    printf '%s\n' "$c"
    sleep "$GAP"
  done
  sleep "$TAIL"
}
feeder "$@" | "$HERE/target/release/frayremote" serve --port "$PORT" --token "$TOK" &
BR=$!
sleep 0.7

rm -f "$FRAY_DIR/error.log"
# FRAY_ENGINE_LOG: capture engine stdout/stderr (Engine.log output) to a file.
ENGINE_OUT="${FRAY_ENGINE_LOG:-/dev/null}"
( cd "$FRAY_DIR" && DYLD_LIBRARY_PATH=. ./hl _conn.dat ) >"$ENGINE_OUT" 2>&1 &
ENG=$!
sleep "$TOTAL"

echo "[runseq.sh] done (port=$PORT, cmds=$NCMD, total=${TOTAL}s)"

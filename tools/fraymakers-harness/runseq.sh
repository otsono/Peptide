#!/usr/bin/env bash
# Sequence runner — boot ONE engine session and feed multiple commands.
#
# run.sh sends a single command per boot; m/t/q probes need the SAME live
# engine (a match doesn't survive a reboot). This feeds a command sequence to
# frayremote, which dispatches the FIRST command the instant the engine signals
# READY (event-driven, no artificial pre-delay) and paces SUBSEQUENT commands by
# <gap_s> so each command's per-frame effect settles before the next fires.
#
# Usage:  ./runseq.sh <gap_s> "cmd1" "cmd2" ...
#   gap_s : seconds between successive commands (frayremote-side, after READY).
# Env: FRAY_DIR install path; FRAY_TAIL hold after last cmd (default 6);
#      FRAY_READY_BUDGET seconds allotted for boot→READY before the first
#      command can fire (engine-lifetime budget only; the command still fires at
#      the REAL READY, not after this delay). Default 45.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
FRAY_DIR="${FRAY_DIR:-/Users/jimmy/Library/Application Support/Steam/steamapps/common/Fraymakers}"
GAP="${1:?gap_s}"; shift
TAIL="${FRAY_TAIL:-6}"
READY_BUDGET="${FRAY_READY_BUDGET:-45}"
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

# Engine lifetime must cover boot→READY + all gaps (frayremote-paced) + tail.
NCMD=$#
TOTAL=$(( READY_BUDGET + (NCMD * GAP) + TAIL + 2 ))

# Feeder: dump ALL commands immediately. frayremote holds them until READY,
# fires cmd1 at READY, then paces the rest by FRAY_CMD_GAP. No pre-READY sleep,
# so `s` fires the moment loading completes — like run.sh's single-command path.
feeder() {
  for c in "$@"; do printf '%s\n' "$c"; done
  sleep "$TOTAL"   # keep the pipe open so frayremote's holder doesn't see EOF early
}
feeder "$@" | FRAY_CMD_GAP="$GAP" "$HERE/target/release/frayremote" serve --port "$PORT" --token "$TOK" &
BR=$!
sleep 0.7

rm -f "$FRAY_DIR/error.log"
# FRAY_ENGINE_LOG: capture engine stdout/stderr (Engine.log output) to a file.
ENGINE_OUT="${FRAY_ENGINE_LOG:-/dev/null}"
( cd "$FRAY_DIR" && DYLD_LIBRARY_PATH=. ./hl _conn.dat ) >"$ENGINE_OUT" 2>&1 &
ENG=$!
sleep "$TOTAL"

echo "[runseq.sh] done (port=$PORT, cmds=$NCMD, total=${TOTAL}s)"

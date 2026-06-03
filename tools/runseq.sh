#!/usr/bin/env bash
# Sequence runner — boot ONE engine session and feed multiple commands.
#
# run.sh sends a single command per boot; m/t/q probes need the SAME live
# engine (a match doesn't survive a reboot). This feeds a command sequence to
# peptide, which dispatches the FIRST command the instant the engine signals
# READY (event-driven, no artificial pre-delay) and paces SUBSEQUENT commands by
# <gap_s> so each command's per-frame effect settles before the next fires.
#
# Usage:  ./runseq.sh <gap_s> "cmd1" "cmd2" ...
#   gap_s : seconds between successive commands (peptide-side, after READY).
# Env: FRAY_DIR install path; FRAY_TAIL hold after last cmd (default 3);
#      FRAY_READY_BUDGET seconds allotted for boot→READY before the first
#      command can fire (engine-lifetime budget only; the command still fires at
#      the REAL READY, not after this delay). Default 16 — the skip-title +
#      filtered-required-load boot reaches READY in ~4.5s, so this is slack.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
FRAY_DIR="${FRAY_DIR:-$HOME/Library/Application Support/Steam/steamapps/common/Fraymakers}"
GAP="${1:?gap_s}"; shift
# Fast-boot defaults: skip-title + filtered required-load reaches READY in ~4.5s, so the
# old 45s budget + 6s tail were mostly idle waiting. Override via env if a run needs more.
TAIL="${FRAY_TAIL:-3}"
READY_BUDGET="${FRAY_READY_BUDGET:-16}"
PORT="${FRAY_PORT:-$(( (RANDOM % 2000) + 18000 ))}"
TOK="fray-$RANDOM$RANDOM"

BOOT="$FRAY_DIR/hlboot-sdl.dat"
CONN="$FRAY_DIR/_conn.dat"
APPID="$FRAY_DIR/steam_appid.txt"
[ -f "$BOOT" ] || { echo "no hlboot-sdl.dat in $FRAY_DIR" >&2; exit 1; }

# Reliable, graceful shutdown so we don't leave wedged ./hl orphans between runs.
# SIGTERM first (lets HL exit at a safe point instead of getting stuck U-state in a
# mid-render Metal syscall, which is what `kill -9` during rendering causes), then
# SIGKILL only if it didn't exit.
cleanup() {
  rm -f "$CONN" "$APPID" 2>/dev/null || true
  kill -TERM "${ENG:-0}" 2>/dev/null || true
  kill -TERM "${BR:-0}"  2>/dev/null || true
  for _ in 1 2 3 4 5 6; do kill -0 "${ENG:-0}" 2>/dev/null || break; sleep 0.25; done
  kill -9 "${ENG:-0}" 2>/dev/null || true
  kill -9 "${BR:-0}"  2>/dev/null || true
}
trap cleanup EXIT INT TERM

# Reap any killable stale engine/bridge from a prior run BEFORE launching a fresh one
# (sequential runs only; wedged U-state orphans can't be reaped but consume nothing).
pkill -TERM -f 'hl _conn.dat'   2>/dev/null || true
pkill -TERM -f 'peptide serve' 2>/dev/null || true
sleep 0.3

[ -x "$ROOT/build/release/peptide" ] || cargo build --release --manifest-path "$ROOT/Cargo.toml" >/dev/null 2>&1
printf '1420350' > "$APPID"
# Headless fast-boot (skip Title/menus + filter required load) is triggered by passing a
# CHARACTER to the injector. Defaults to sandbag (the harness's standard target) on thespire
# with commandervideoassist; override via FRAY_CHAR / FRAY_STAGE / FRAY_ASSIST. Set
# FRAY_CHAR="" for a non-headless, TCP-only boot (normal title, no fast-boot, no auto-launch).
CHAR="${FRAY_CHAR-sandbag}"
STAGE="${FRAY_STAGE:-thespire}"
ASSIST="${FRAY_ASSIST:-commandervideoassist}"
if [ -n "$CHAR" ]; then
  "$ROOT/build/release/peptide" "$BOOT" "$CONN" connect "$PORT" "$TOK" "$CHAR" "$STAGE" "$ASSIST" >/dev/null 2>&1
else
  "$ROOT/build/release/peptide" "$BOOT" "$CONN" connect "$PORT" "$TOK" >/dev/null 2>&1
fi

# Engine-lifetime CAP (we poll-wait for the clean 'x' exit and break early, so this is
# only an upper bound). +1 command for the auto-appended 'x' exit.
NCMD=$#
# GAP may be fractional (e.g. 0.05 for frame-level sampling). Bash arithmetic is
# integer-only, so round GAP UP to an integer just for the lifetime-cap math (the real,
# fractional gap is still passed verbatim to FRAY_CMD_GAP below). This is only an upper
# bound — the run breaks early the instant the engine exits.
GAP_CEIL=$(awk -v g="$GAP" 'BEGIN{c=int(g); if(c<g)c++; if(c<1)c=1; print c}')
TOTAL=$(( READY_BUDGET + ((NCMD + 1) * GAP_CEIL) + TAIL + 2 ))

# Feeder: dump ALL user commands, then an 'x' to cleanly exit the engine (hxd.System.exit)
# so it shuts itself down — no kill -9, no wedged orphan. peptide holds them until READY,
# fires cmd1 at READY, then paces the rest (incl. 'x' last) by FRAY_CMD_GAP.
feeder() {
  for c in "$@"; do printf '%s\n' "$c"; done
  printf 'x\n'     # clean engine exit after the user commands
  sleep "$TOTAL"   # keep the pipe open so peptide's holder doesn't see EOF early
}
feeder "$@" | FRAY_CMD_GAP="$GAP" "$ROOT/build/release/peptide" serve --port "$PORT" --token "$TOK" &
BR=$!
sleep 0.7

rm -f "$FRAY_DIR/error.log"
# FRAY_ENGINE_LOG: capture engine stdout/stderr (Engine.log output) to a file.
ENGINE_OUT="${FRAY_ENGINE_LOG:-/dev/null}"
( cd "$FRAY_DIR" && DYLD_LIBRARY_PATH=. ./hl _conn.dat ) >"$ENGINE_OUT" 2>&1 &
ENG=$!
# Wait for the engine to cleanly exit itself (the 'x' command -> hxd.System.exit), capped
# at TOTAL. Breaking early as soon as it dies makes iterations fast AND avoids killing it
# mid-render (the cleanup trap's SIGKILL is just a fallback if 'x' never landed).
waited=0; cap=$(( TOTAL * 4 ))
while kill -0 "$ENG" 2>/dev/null; do
  sleep 0.25; waited=$((waited + 1))
  [ "$waited" -ge "$cap" ] && break
done

echo "[runseq.sh] done (port=$PORT, cmds=$NCMD, exit=$(( waited / 4 ))s, cap=${TOTAL}s)"

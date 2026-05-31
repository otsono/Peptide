#!/usr/bin/env bash
# Self-contained Fraymakers match-launch runner.
#
# IMPORTANT design constraint: the Steam sandbox wipes files we add to the
# install dir, so NOTHING we put there is assumed to persist. Every run
# RECREATES the files it needs from scratch (the patched `_conn.dat` and
# `steam_appid.txt`), launches the patched copy directly, and removes them
# afterward. The pristine `hlboot-sdl.dat` is READ as the patch source but is
# NEVER modified. All harness development happens in OUR patcher (this repo) —
# we never iterate on files inside the Fraymakers dir.
#
# Usage: ./run.sh "<command>" [seconds]
#   FRAY_DIR env overrides the Fraymakers install path.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
FRAY_DIR="${FRAY_DIR:-$HOME/Library/Application Support/Steam/steamapps/common/Fraymakers}"
CMD="${1:-s commandervideo thespire commandervideoassist}"
SECS="${2:-20}"
PORT="${FRAY_PORT:-$(( (RANDOM % 2000) + 18000 ))}"
TOK="fray-$RANDOM$RANDOM"

BOOT="$FRAY_DIR/hlboot-sdl.dat"   # pristine engine bytecode — patch SOURCE, never written
CONN="$FRAY_DIR/_conn.dat"        # patched copy we add, launch, and delete
APPID="$FRAY_DIR/steam_appid.txt" # added so a direct ./hl launch doesn't bounce via Steam
[ -f "$BOOT" ] || { echo "no hlboot-sdl.dat in $FRAY_DIR" >&2; exit 1; }

# Remove the files we add, no matter how we exit. hlboot-sdl.dat is left untouched.
cleanup() {
  rm -f "$CONN" "$APPID" 2>/dev/null || true
  kill -9 "${ENG:-0}" 2>/dev/null || true
  kill -9 "${BR:-0}"  2>/dev/null || true
}
trap cleanup EXIT INT TERM

# 1. Build our tools if needed (dev iteration lives here, in the repo).
[ -x "$HERE/target/release/peptide" ] || cargo build --release --manifest-path "$HERE/Cargo.toml" >/dev/null 2>&1

# 2. (Re)create the added files every run — never assume they survived a restart.
printf '1420350' > "$APPID"
"$HERE/target/release/peptide" "$BOOT" "$CONN" connect "$PORT" "$TOK" >/dev/null 2>&1

# 3. Start the loopback bridge and queue the command.
( printf '%s\n' "$CMD"; sleep "$SECS" ) | "$HERE/target/release/peptide-bridge" serve --port "$PORT" --token "$TOK" &
BR=$!
sleep 0.7

# 4. Launch the patched copy directly (hlboot-sdl.dat stays pristine).
rm -f "$FRAY_DIR/error.log"
( cd "$FRAY_DIR" && DYLD_LIBRARY_PATH=. ./hl _conn.dat ) >/dev/null 2>&1 &
ENG=$!
sleep "$SECS"

# cleanup() runs on EXIT: removes _conn.dat + steam_appid.txt, kills procs.
echo "[run.sh] done (port=$PORT)"

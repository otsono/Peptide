#!/usr/bin/env bash
# Self-contained Fraymakers match-launch runner. Recreates everything needed in
# the Fraymakers install dir (steam_appid.txt, patched _conn.dat), launches the
# engine, and bridges the `s <char> <stage> <assist>` command over loopback TCP.
#
# Usage: ./run.sh "<command>" [seconds]
#   FRAY_DIR env overrides the Fraymakers install path.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
FRAY_DIR="${FRAY_DIR:-/Users/jimmy/Library/Application Support/Steam/steamapps/common/Fraymakers}"
CMD="${1:-s commandervideo thespire commandervideoassist}"
SECS="${2:-20}"
PORT="${FRAY_PORT:-$(( (RANDOM % 2000) + 18000 ))}"
TOK="fray-$RANDOM$RANDOM"

[ -f "$FRAY_DIR/hlboot-sdl.dat" ] || { echo "no hlboot-sdl.dat in $FRAY_DIR" >&2; exit 1; }
# 1. steam_appid.txt (avoids Steam relaunch bounce)
printf '1420350' > "$FRAY_DIR/steam_appid.txt"
# 2. build tools if needed
[ -x "$HERE/target/release/fray_patch" ] || cargo build --release --manifest-path "$HERE/Cargo.toml" >/dev/null 2>&1
# 3. patch bytecode -> _conn.dat in the engine dir
"$HERE/target/release/fray_patch" "$FRAY_DIR/hlboot-sdl.dat" "$FRAY_DIR/_conn.dat" connect "$PORT" "$TOK" >/dev/null 2>&1
# 4. start the loopback bridge, send the command
( printf '%s\n' "$CMD"; sleep "$SECS" ) | "$HERE/target/release/frayremote" serve --port "$PORT" --token "$TOK" &
BR=$!
sleep 0.7
# 5. run the engine
rm -f "$FRAY_DIR/error.log"
( cd "$FRAY_DIR" && DYLD_LIBRARY_PATH=. ./hl _conn.dat ) >/dev/null 2>&1 &
ENG=$!
sleep "$SECS"
kill -9 "$ENG" 2>/dev/null || true
kill -9 "$BR" 2>/dev/null || true
rm -f "$FRAY_DIR/_conn.dat"
echo "[run.sh] done (port=$PORT)"

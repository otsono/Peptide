#!/usr/bin/env bash
# run-ui.sh — launch Fraymakers and open the Peptide console UI.
#
# Boots a patched throwaway copy of the engine (hlboot-sdl.dat stays pristine) and
# runs the full-screen `peptide-bridge ui`. Type commands at the bottom; replies
# stream into the scrollback. Quit with Esc or Ctrl+C.
#
# Usage: ./run-ui.sh
#   FRAY_DIR  overrides the Fraymakers install path
#   FRAY_CHAR default character baked in so a bare `spawn` works (default: sandbag)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
FRAY_DIR="${FRAY_DIR:-$HOME/Library/Application Support/Steam/steamapps/common/Fraymakers}"
CHAR="${FRAY_CHAR:-sandbag}"
STAGE="${FRAY_STAGE:-thespire}"
ASSIST="${FRAY_ASSIST:-commandervideoassist}"
PORT="${FRAY_PORT:-$(( (RANDOM % 2000) + 18000 ))}"
TOK="fray-$RANDOM$RANDOM"

BOOT="$FRAY_DIR/hlboot-sdl.dat"   # pristine engine bytecode — patch SOURCE, never written
CONN="$FRAY_DIR/_conn.dat"        # patched copy we add, launch, and delete
APPID="$FRAY_DIR/steam_appid.txt" # added so a direct ./hl launch doesn't bounce via Steam
[ -f "$BOOT" ] || { echo "no hlboot-sdl.dat in $FRAY_DIR" >&2; exit 1; }

cleanup() {
  rm -f "$CONN" "$APPID" 2>/dev/null || true
  kill -9 "${ENG:-0}" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# 1. Build our tools if needed.
[ -x "$HERE/target/release/peptide-bridge" ] || cargo build --release --manifest-path "$HERE/Cargo.toml" >/dev/null 2>&1

# 2. (Re)create the added files + patch the engine (bakes the default char so a bare
#    `spawn` works; you can still `spawn <other>` interactively).
printf '1420350' > "$APPID"
"$HERE/target/release/peptide" "$BOOT" "$CONN" connect "$PORT" "$TOK" "$CHAR" "$STAGE" "$ASSIST" >/dev/null 2>&1

# 3. Launch the patched engine shortly AFTER the UI starts (so the UI binds the port
#    first, then the engine dials in).
rm -f "$FRAY_DIR/error.log"
( sleep 1.5; cd "$FRAY_DIR" && DYLD_LIBRARY_PATH=. ./hl _conn.dat ) >/dev/null 2>&1 &
ENG=$!

# 4. Run the console UI (foreground; owns the terminal until you quit).
FRAY_CHAR="$CHAR" "$HERE/target/release/peptide-bridge" ui --port "$PORT" --token "$TOK"

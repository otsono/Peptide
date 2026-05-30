#!/usr/bin/env bash
# Self-contained Fraymakers match-launch runner.
#
# Patches the engine's bytecode IN PLACE (overwrites hlboot-sdl.dat) rather than
# adding a new file beside it — the Steam sandbox deletes files we *add* on open,
# but overwriting an existing file survives. A pristine copy is kept as
# hlboot-sdl.dat.orig and is always used as the patch source; the patched file is
# restored from it on exit, so the install is left clean.
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

BOOT="$FRAY_DIR/hlboot-sdl.dat"
ORIG="$FRAY_DIR/hlboot-sdl.dat.orig"
[ -f "$BOOT" ] || { echo "no hlboot-sdl.dat in $FRAY_DIR" >&2; exit 1; }

# Restore the pristine engine file no matter how we exit.
restore() {
  [ -f "$ORIG" ] && cp -f "$ORIG" "$BOOT" || true
  kill -9 "${ENG:-0}" 2>/dev/null || true
  kill -9 "${BR:-0}"  2>/dev/null || true
}
trap restore EXIT INT TERM

# 1. Pristine backup (created once; the source of truth for every patch).
#    If a .orig already exists we assume hlboot-sdl.dat may be a leftover patched
#    copy, so we DON'T overwrite the backup with it.
[ -f "$ORIG" ] || cp "$BOOT" "$ORIG"

# 2. steam_appid.txt so a direct ./hl launch doesn't bounce through Steam
#    (RestartAppIfNecessary). This is an added file, but it's only consulted at
#    process start and we launch hl directly, so the sandbox never touches it.
printf '1420350' > "$FRAY_DIR/steam_appid.txt"

# 3. Build tools if needed.
[ -x "$HERE/target/release/fray_patch" ] || cargo build --release --manifest-path "$HERE/Cargo.toml" >/dev/null 2>&1

# 4. Patch the PRISTINE bytecode and overwrite the real engine file in place.
"$HERE/target/release/fray_patch" "$ORIG" "$BOOT" connect "$PORT" "$TOK" >/dev/null 2>&1

# 5. Start the loopback bridge and queue the command.
( printf '%s\n' "$CMD"; sleep "$SECS" ) | "$HERE/target/release/frayremote" serve --port "$PORT" --token "$TOK" &
BR=$!
sleep 0.7

# 6. Launch the engine directly on the patched hlboot-sdl.dat.
rm -f "$FRAY_DIR/error.log"
( cd "$FRAY_DIR" && DYLD_LIBRARY_PATH=. ./hl hlboot-sdl.dat ) >/dev/null 2>&1 &
ENG=$!
sleep "$SECS"

# restore() runs on EXIT (puts the pristine hlboot-sdl.dat back, kills procs).
echo "[run.sh] done (port=$PORT)"

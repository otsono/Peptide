#!/usr/bin/env bash
# Freeze test v2 — holds the socket open the whole run (so the engine's per-frame
# ack-write never hits EOF), samples CPU+thread-state, and grabs two screenshots
# a few seconds apart. Frozen (non-terminating per-frame loop) => one core pinned
# ~100%, state R, identical screenshots. Playing => low/fluctuating CPU, state S,
# and a rendered match. hlboot-sdl.dat is read-only; a separate _conn.dat is
# launched and deleted on exit.
#
# Usage: ./freeze_test2.sh <label>   (the .fra under test must already be installed)
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
FRAY_DIR="${FRAY_DIR:-/Users/jimmy/Library/Application Support/Steam/steamapps/common/Fraymakers}"
LABEL="${1:-run}"; OUT="/tmp/claude-501/f2_${LABEL}"; mkdir -p "$OUT"; rm -f "$OUT"/*
PORT="$(( (RANDOM % 2000) + 18000 ))"; TOK="fray-$RANDOM$RANDOM"
BOOT="$FRAY_DIR/hlboot-sdl.dat"; CONN="$FRAY_DIR/_conn.dat"; APPID="$FRAY_DIR/steam_appid.txt"
cleanup(){ rm -f "$CONN" "$APPID" 2>/dev/null; kill -9 "${ENG:-0}" "${BR:-0}" 2>/dev/null; }
trap cleanup EXIT INT TERM

printf '1420350' > "$APPID"
"$HERE/target/release/peptide" "$BOOT" "$CONN" connect "$PORT" "$TOK" > "$OUT/patch.log" 2>&1

# Hold stdin open ~34s so serve stays alive and the socket never closes mid-run.
( echo "s sandbag battlefield none"; sleep 34 ) \
  | "$HERE/target/release/peptide-bridge" serve --port "$PORT" --token "$TOK" > "$OUT/serve.log" 2>&1 &
BR=$!
sleep 0.8
rm -f "$FRAY_DIR/error.log"
( cd "$FRAY_DIR" && DYLD_LIBRARY_PATH=. ./hl _conn.dat ) > "$OUT/engine.log" 2>&1 &
ENG=$!

# Let the match start, then sample.
sleep 10
for n in $(seq 1 8); do
  ps -o %cpu= -o state= -o time= -p "$ENG" 2>/dev/null | tr -s ' ' | sed "s/^/cpu_$n:/" >> "$OUT/cpu.log"
  kill -0 "$ENG" 2>/dev/null || { echo "DIED_at_$n" >> "$OUT/cpu.log"; break; }
  if [ "$n" = "1" ]; then screencapture -x "$OUT/shot_A.png" 2>/dev/null; fi
  if [ "$n" = "6" ]; then screencapture -x "$OUT/shot_B.png" 2>/dev/null; fi
  sleep 2
done

ALIVE=$(kill -0 "$ENG" 2>/dev/null && echo YES || echo NO)
ERR=$(test -s "$FRAY_DIR/error.log" && echo NONEMPTY || echo empty)
cp "$FRAY_DIR/error.log" "$OUT/error.log" 2>/dev/null || true
echo "LABEL=$LABEL ALIVE=$ALIVE errorlog=$ERR PORT=$PORT" | tee "$OUT/VERDICT.txt"

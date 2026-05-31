#!/usr/bin/env bash
# Freeze A/B test for a converted character.
#
# Detection principle: the socket-command reader is injected into Main.update,
# which runs ONCE PER FRAME. After starting a match, we send `q` (dump the
# content registry to the engine console). If `q`'s output appears in the engine
# log, Main.update is still ticking -> the match is NOT frozen. If a freeze hangs
# update (e.g. a non-terminating per-frame listener), `q` produces nothing.
#
# hlboot-sdl.dat is READ ONLY (patch source); we launch a separate patched
# _conn.dat copy and delete it on exit. Never swaps/overwrites the engine.
#
# Usage: ./freeze_test.sh <label>
#   Assumes the .fra under test is already installed at custom/sandbag/.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
FRAY_DIR="${FRAY_DIR:-/Users/jimmy/Library/Application Support/Steam/steamapps/common/Fraymakers}"
LABEL="${1:-run}"
OUT="/tmp/claude-501/ft_${LABEL}"
mkdir -p "$OUT"
PORT="$(( (RANDOM % 2000) + 18000 ))"; TOK="fray-$RANDOM$RANDOM"
BOOT="$FRAY_DIR/hlboot-sdl.dat"; CONN="$FRAY_DIR/_conn.dat"; APPID="$FRAY_DIR/steam_appid.txt"

cleanup(){ rm -f "$CONN" "$APPID" 2>/dev/null; kill -9 "${ENG:-0}" "${BR:-0}" 2>/dev/null; }
trap cleanup EXIT INT TERM

printf '1420350' > "$APPID"
"$HERE/target/release/peptide" "$BOOT" "$CONN" connect "$PORT" "$TOK" > "$OUT/patch.log" 2>&1
echo "patch_exit=$?" >> "$OUT/patch.log"

# stdin to serve: start match, let it run, then q (liveness probe), then q again.
( echo "s sandbag battlefield none"; sleep 10; echo "q"; sleep 4; echo "q"; sleep 3 ) \
  | "$HERE/target/release/peptide-bridge" serve --port "$PORT" --token "$TOK" > "$OUT/serve.log" 2>&1 &
BR=$!
sleep 0.8

rm -f "$FRAY_DIR/error.log"
( cd "$FRAY_DIR" && DYLD_LIBRARY_PATH=. ./hl _conn.dat ) > "$OUT/engine.log" 2>&1 &
ENG=$!

# Sample engine CPU over the run (frozen infinite-loop pins ~100% on one core).
for n in 1 2 3 4 5 6; do
  sleep 3
  ps -o %cpu= -o state= -p "$ENG" 2>/dev/null | tr -s ' ' | sed "s/^/cpu_sample_$n: /" >> "$OUT/cpu.log"
  kill -0 "$ENG" 2>/dev/null || { echo "engine_died_at_sample_$n" >> "$OUT/cpu.log"; break; }
done

ALIVE=$(kill -0 "$ENG" 2>/dev/null && echo YES || echo NO)
# Did q produce a registry dump AFTER match start? (search engine + serve logs)
QHITS=$(grep -icE 'registry|characters|stages|namespace|::|content' "$OUT/engine.log" "$OUT/serve.log" 2>/dev/null | awk -F: '{s+=$2} END{print s+0}')
ERR=$(test -s "$FRAY_DIR/error.log" && echo "ERRORLOG_NONEMPTY" || echo "errorlog_empty")
cp "$FRAY_DIR/error.log" "$OUT/error.log" 2>/dev/null || true
echo "LABEL=$LABEL ALIVE=$ALIVE QHITS=$QHITS $ERR PORT=$PORT" | tee "$OUT/VERDICT.txt"

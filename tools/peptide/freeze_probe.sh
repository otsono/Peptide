#!/usr/bin/env bash
# Freeze oracle via update()-liveness.
#
# The harness injects a per-frame command reader into Main.update. So if we keep
# sending `q` after the match starts and the engine keeps answering, update() is
# still ticking => the match is NOT frozen. If the engine PROCESS is alive but
# `q` answers stop, update() is hung in an infinite loop => FROZEN. If the process
# dies, that's a crash (distinct from a freeze).
#
# hlboot-sdl.dat is READ-ONLY (patch source); a separate _conn.dat is launched
# and removed on exit. Uses the hardened `serve` that holds the socket open so a
# stdin/EOF hiccup can't Eof-crash the engine.
#
# Usage: ./freeze_probe.sh <label>   (.fra under test must already be installed)
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
FRAY_DIR="${FRAY_DIR:-$HOME/Library/Application Support/Steam/steamapps/common/Fraymakers}"
LABEL="${1:-run}"; OUT="/tmp/claude-501/fp_${LABEL}"; mkdir -p "$OUT"; rm -f "$OUT"/* 2>/dev/null
PORT="$(( (RANDOM % 2000) + 18000 ))"; TOK="fray-$RANDOM$RANDOM"
BOOT="$FRAY_DIR/hlboot-sdl.dat"; CONN="$FRAY_DIR/_conn.dat"; APPID="$FRAY_DIR/steam_appid.txt"
cleanup(){ rm -f "$CONN" "$APPID" 2>/dev/null; kill -9 "${ENG:-0}" "${BR:-0}" 2>/dev/null; }
trap cleanup EXIT INT TERM

printf '1420350' > "$APPID"
"$HERE/target/release/peptide" "$BOOT" "$CONN" connect "$PORT" "$TOK" > "$OUT/patch.log" 2>&1

# Command stream: start the match, then a `q` heartbeat every 3s for ~33s.
# Each line goes through stdin; the sleeps keep stdin open so serve stays live.
{
  # CRITICAL: wait for async UGC load before `s`. UGC loads async
  # (_onFileLoaded@17838 fires per-.fra); sending `s` too early yields a
  # pooled-but-unfinalized resource whose characterPxfContentMap is null ->
  # spawnPlayer crash (md5 36adae25). ~12s post-READY lets it finish. VERIFIED:
  # with the delay, `s sandbag thespire none` -> LAUNCHED + Q:MATCH_LIVE, no
  # crash (reproduced). Use a VALID stage: thespire works; battlefield/
  # st_battlefield resolve to a stub with null stagePxfContentMap.
  sleep "${FRAY_LOAD_WAIT:-12}"
  echo "s sandbag ${FRAY_STAGE:-thespire} none"
  for n in $(seq 1 11); do sleep 3; echo "q $n"; done
  sleep 2
} | FRAY_HOLD_SECS=60 "$HERE/target/release/peptide-bridge" serve --port "$PORT" --token "$TOK" > "$OUT/serve.log" 2>&1 &
BR=$!
sleep 0.8
rm -f "$FRAY_DIR/error.log"
( cd "$FRAY_DIR" && DYLD_LIBRARY_PATH=. ./hl _conn.dat ) > "$OUT/engine.log" 2>&1 &
ENG=$!

# Sample engine liveness + CPU every 3s for ~36s.
for n in $(seq 1 12); do
  sleep 3
  if kill -0 "$ENG" 2>/dev/null; then
    ps -o %cpu= -o state= -p "$ENG" 2>/dev/null | tr -s ' ' | sed "s/^/sample_$n:/" >> "$OUT/cpu.log"
  else
    echo "sample_$n: ENGINE_DEAD" >> "$OUT/cpu.log"; break
  fi
done

ALIVE=$(kill -0 "$ENG" 2>/dev/null && echo YES || echo NO)
# Count engine replies that arrived AFTER the LAUNCHED ack (update-liveness proof).
LAUNCH_LINE=$(grep -n "LAUNCHED" "$OUT/serve.log" | head -1 | cut -d: -f1)
if [ -n "${LAUNCH_LINE:-}" ]; then
  REPLIES_AFTER=$(tail -n +"$LAUNCH_LINE" "$OUT/serve.log" | grep -c '^<< ')
else
  REPLIES_AFTER=0
fi
TOTAL_REPLIES=$(grep -c '^<< ' "$OUT/serve.log")
ERR=$(test -s "$FRAY_DIR/error.log" && echo NONEMPTY || echo empty)
cp "$FRAY_DIR/error.log" "$OUT/error.log" 2>/dev/null || true
{
  echo "LABEL=$LABEL"
  echo "ALIVE_AT_END=$ALIVE"
  echo "LAUNCHED=$([ -n "${LAUNCH_LINE:-}" ] && echo YES || echo NO)"
  echo "TOTAL_REPLIES=$TOTAL_REPLIES"
  echo "REPLIES_AFTER_LAUNCH=$REPLIES_AFTER"
  echo "errorlog=$ERR"
  echo "PORT=$PORT"
} > "$OUT/VERDICT.txt"
cat "$OUT/VERDICT.txt"

#!/usr/bin/env bash
# buried-Vfx fix test: `l` (load+cache sandbag sprite under REAL spriteContent key) then
# `s` (launch+spawn). Success: serve.log has SC:<key>, SPR:1, LAUNCHED, Q:MATCH_LIVE, and
# error.log has NO Character.hx:769 null. File-based oracle only.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
FRAY_DIR="${FRAY_DIR:-/Users/jimmy/Library/Application Support/Steam/steamapps/common/Fraymakers}"
LABEL="${1:-sc}"; OUT="/tmp/sc_test_${LABEL}"; mkdir -p "$OUT"; rm -f "$OUT"/*
PORT="$(( (RANDOM % 2000) + 19000 ))"; TOK="fray-$RANDOM$RANDOM"
BOOT="$FRAY_DIR/hlboot-sdl.dat"; CONN="$FRAY_DIR/_conn.dat"; APPID="$FRAY_DIR/steam_appid.txt"
cleanup(){ rm -f "$CONN" "$APPID" 2>/dev/null; kill -9 "${ENG:-0}" "${BR:-0}" 2>/dev/null; }
trap cleanup EXIT INT TERM
printf '1420350' > "$APPID"
"$HERE/target/release/peptide" "$BOOT" "$CONN" connect "$PORT" "$TOK" > "$OUT/patch.log" 2>&1
echo "PATCH_EXIT=$?" >> "$OUT/FACTS.txt"
( sleep 12; echo "l"; sleep 4; echo "s private::sandbag.sandbag thespire commandervideoassist"; sleep 8; \
  for i in $(seq 1 12); do echo "q"; echo "t"; sleep 1; done; sleep 4 ) \
  | "$HERE/target/release/peptide-bridge" serve --port "$PORT" --token "$TOK" > "$OUT/serve.log" 2>&1 &
BR=$!
sleep 0.8
rm -f "$FRAY_DIR/error.log" "$FRAY_DIR/crash.log"
( cd "$FRAY_DIR" && DYLD_LIBRARY_PATH=. ./hl _conn.dat ) > "$OUT/engine.log" 2>&1 &
ENG=$!
sleep 44
ALIVE=$(kill -0 "$ENG" 2>/dev/null && echo YES || echo NO)
{
  echo "LABEL=$LABEL PORT=$PORT"
  echo "ALIVE=$ALIVE"
  echo "ERR_MD5=$(md5 -q "$FRAY_DIR/error.log" 2>/dev/null || echo NONE)"
  echo "SC_LINE=$(grep -m1 'SC:' "$OUT/serve.log" 2>/dev/null || echo NONE)"
  echo "SPR=$(grep -m1 'SPR:' "$OUT/serve.log" 2>/dev/null || echo NONE)"
  echo "LAUNCHED=$(grep -c 'LAUNCHED' "$OUT/serve.log" 2>/dev/null || echo 0)"
  echo "Q_MATCH_LIVE=$(grep -c 'Q:MATCH_LIVE' "$OUT/serve.log" 2>/dev/null || echo 0)"
  echo "CRASH_769=$(grep -c '769' "$FRAY_DIR/error.log" 2>/dev/null || echo 0)"
} | tee -a "$OUT/FACTS.txt"
echo "--- serve.log (<<) ---"; grep '<<' "$OUT/serve.log" 2>/dev/null | head -40
echo "--- error.log head ---"; head -20 "$FRAY_DIR/error.log" 2>/dev/null

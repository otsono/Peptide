#!/usr/bin/env bash
# Pool-array-index-loop test.
# Success oracle (reliable): error.log md5 is NEITHER 36adae25 NOR 3537a487,
# AND serve.log has LAUNCHED. Verify by file reads (not shell narrative).
# Run: ./pool_test.sh [label]
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
FRAY_DIR="${FRAY_DIR:-/Users/jimmy/Library/Application Support/Steam/steamapps/common/Fraymakers}"
LABEL="${1:-test}"; OUT="/tmp/pool_test_${LABEL}"; mkdir -p "$OUT"; rm -f "$OUT"/*
PORT="$(( (RANDOM % 2000) + 19000 ))"; TOK="fray-$RANDOM$RANDOM"
BOOT="$FRAY_DIR/hlboot-sdl.dat"; CONN="$FRAY_DIR/_conn.dat"; APPID="$FRAY_DIR/steam_appid.txt"
cleanup(){ rm -f "$CONN" "$APPID" 2>/dev/null; kill -9 "${ENG:-0}" "${BR:-0}" 2>/dev/null; }
trap cleanup EXIT INT TERM
printf '1420350' > "$APPID"

# Patch with the new resolver
"$HERE/target/release/fray_patch" "$BOOT" "$CONN" connect "$PORT" "$TOK" > "$OUT/patch.log" 2>&1
echo "PATCH_EXIT=$?" >> "$OUT/FACTS.txt"

# Serve: hold socket open with 14s pre-s delay (let async UGC load finish), then q x6
( sleep 14; echo "s sandbag thespire none"; sleep 2; \
  for i in 1 2 3 4 5 6; do echo "q"; sleep 1; done; \
  sleep 5 ) \
  | "$HERE/target/release/frayremote" serve --port "$PORT" --token "$TOK" > "$OUT/serve.log" 2>&1 &
BR=$!
sleep 0.8
rm -f "$FRAY_DIR/error.log" "$FRAY_DIR/crash.log"
( cd "$FRAY_DIR" && DYLD_LIBRARY_PATH=. ./hl _conn.dat ) > "$OUT/engine.log" 2>&1 &
ENG=$!
sleep 30

# FACTS: reliable file-based oracles only
ALIVE=$(kill -0 "$ENG" 2>/dev/null && echo YES || echo NO)
ERR_MD5=$(md5 -q "$FRAY_DIR/error.log" 2>/dev/null || echo NONE)
LAUNCHED=$(grep -c "LAUNCHED" "$OUT/serve.log" 2>/dev/null || echo 0)
Q_REPLIES=$(grep -c "^<< Q:" "$OUT/serve.log" 2>/dev/null || echo 0)
Q_MATCH=$(grep -c "Q:MATCH_LIVE\|Q:MATCHES_NONEMPTY" "$OUT/serve.log" 2>/dev/null || echo 0)
{
  echo "LABEL=$LABEL"
  echo "ALIVE=$ALIVE"
  echo "ERROR_MD5=$ERR_MD5"
  echo "LAUNCHED=$LAUNCHED"
  echo "Q_REPLIES=$Q_REPLIES"
  echo "Q_MATCH=$Q_MATCH"
  echo "VERDICT=$([ "$ERR_MD5" = "NONE" ] && echo "NO_CRASH" || echo "CRASH_$ERR_MD5")"
} > "$OUT/FACTS.txt"
cat "$OUT/FACTS.txt"
cp "$FRAY_DIR/error.log" "$OUT/error.log" 2>/dev/null || true
grep "LAUNCHED\|<< Q:" "$OUT/serve.log" | head -20

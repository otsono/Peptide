#!/usr/bin/env bash
# Batch export + in-engine spawn-test a set of characters. Records PASS/FAIL.
# PASS = reached ANIM:STAND after spawn, move jab returned M:OK, no rosetta crash.
set -u
# repo root = two dirs up from this script (tools/peptide/ -> repo root)
cd "$(cd "$(dirname "$0")/../.." && pwd)"
RESULTS="${BATCH_RESULTS:-/tmp/batch_results.txt}"
: > "$RESULTS"

# One spawn+drive attempt for char $1 on port $2. Echoes the PASS/FAIL line.
run_one() {
  local c="$1" port="$2"
  ( cd tools/peptide && FRAY_CHAR="$c" FRAY_PORT="$port" FRAY_ENGINE_LOG=/tmp/${c}_eng.log ./runseq.sh 3 \
      "spawn $c thespire commandervideoassist" "state" "move jab" "state" "move special_neutral" "physics" \
      >/tmp/${c}_test.log 2>&1 )
  local stood mok crash launched bind
  stood=$(grep -c "ANIM:STAND" /tmp/${c}_test.log)
  mok=$(grep -c "M:OK" /tmp/${c}_test.log)
  crash=$(grep -ic "rosetta error\|exception" /tmp/${c}_eng.log 2>/dev/null)
  launched=$(grep -c "LAUNCHED" /tmp/${c}_test.log)
  bind=$(grep -c "Address already in use" /tmp/${c}_test.log)
  if [ "$launched" -ge 1 ] && [ "$stood" -ge 1 ] && [ "$mok" -ge 1 ] && [ "$crash" -eq 0 ]; then
    echo "PASS (stand=$stood mok=$mok)"
  elif [ "$bind" -ge 1 ] && [ "$launched" -eq 0 ]; then
    echo "RETRY"  # port collision, not a real failure — caller retries
  else
    echo "FAIL (launched=$launched stand=$stood mok=$mok crash=$crash)"
  fi
}

i=0
for c in "$@"; do
  i=$((i + 1))
  # 1. ensure fresh source (regen is cheap + deterministic)
  ./target/release/ssf2_converter ../ssf2-ssfs/$c.ssf >/tmp/${c}_conv.log 2>&1 || { echo "$c CONVERT_FAIL" >>"$RESULTS"; continue; }
  # 2. export via FrayTools
  node tools/fraytools-harness/export-in-fraytools.js --project "$PWD/characters/$c/$c.fraytools" >/tmp/${c}_export.log 2>&1 || { echo "$c EXPORT_FAIL" >>"$RESULTS"; continue; }
  # 3. spawn + drive — deterministic per-char port (avoids the random-port collisions
  #    that produced false launched=0 fails); retry once on a port-bind collision.
  pkill -f 'peptide-bridge serve' 2>/dev/null || true; sleep 0.3
  res=$(run_one "$c" $((20100 + i)))
  if [ "$res" = "RETRY" ]; then
    pkill -f 'peptide-bridge serve' 2>/dev/null || true; sleep 1
    res=$(run_one "$c" $((20600 + i)))
    [ "$res" = "RETRY" ] && res="FAIL (port-collision x2)"
  fi
  echo "$c $res" >>"$RESULTS"
done
echo "BATCH_DONE" >>"$RESULTS"

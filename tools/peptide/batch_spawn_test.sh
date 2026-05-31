#!/usr/bin/env bash
# Batch export + in-engine spawn-test a set of characters. Records PASS/FAIL.
# PASS = reached ANIM:STAND after spawn, move jab returned M:OK, no rosetta crash.
set -u
cd /Users/jimmy/.openclaw/workspace-main/ssf2-fraymakers-converter
RESULTS=/tmp/batch_results.txt
: > "$RESULTS"
for c in "$@"; do
  # 1. ensure fresh source (already converted in the earlier batch, but regen is cheap+deterministic)
  ./target/release/ssf2_converter ../ssf2-ssfs/$c.ssf >/tmp/${c}_conv.log 2>&1 || { echo "$c CONVERT_FAIL" >>"$RESULTS"; continue; }
  # 2. export via FrayTools
  node tools/fraytools-harness/export-in-fraytools.js --project "$PWD/characters/$c/$c.fraytools" >/tmp/${c}_export.log 2>&1 || { echo "$c EXPORT_FAIL" >>"$RESULTS"; continue; }
  # 3. spawn + drive in engine
  ( cd tools/peptide && FRAY_CHAR=$c FRAY_ENGINE_LOG=/tmp/${c}_eng.log ./runseq.sh 3 \
      "spawn $c thespire commandervideoassist" "state" "move jab" "state" "move special_neutral" "physics" \
      >/tmp/${c}_test.log 2>&1 )
  stood=$(grep -c "ANIM:STAND" /tmp/${c}_test.log)
  mok=$(grep -c "M:OK" /tmp/${c}_test.log)
  crash=$(grep -ic "rosetta error\|exception" /tmp/${c}_eng.log 2>/dev/null)
  launched=$(grep -c "LAUNCHED" /tmp/${c}_test.log)
  if [ "$launched" -ge 1 ] && [ "$stood" -ge 1 ] && [ "$mok" -ge 1 ] && [ "$crash" -eq 0 ]; then
    echo "$c PASS (stand=$stood mok=$mok)" >>"$RESULTS"
  else
    echo "$c FAIL (launched=$launched stand=$stood mok=$mok crash=$crash)" >>"$RESULTS"
  fi
done
echo "BATCH_DONE" >>"$RESULTS"

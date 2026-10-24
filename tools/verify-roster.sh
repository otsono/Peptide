#!/usr/bin/env bash
# every fighter's six gates on the runtime's path, one at a time, from a frozen copy of the build
# (a long run holds its own binary and tables while the tree moves; see tools/freeze.sh).
#
#   tools/verify-roster.sh [fighter ...]      the whole roster when none are named
#
# one line a fighter goes to build/verify/roster.txt (`<id> <passed>/6 <first failure>`), each
# fighter's full report to build/verify/roster/<id>.log. hours for the whole roster: both engines
# are held throughout, so run nothing else live alongside it.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
FROZEN="$ROOT/build/frozen-roster"
tools/freeze.sh "$FROZEN" >/dev/null
OUT="$ROOT/build/verify/roster.txt"
LOGS="$ROOT/build/verify/roster"
mkdir -p "$LOGS"
: > "$OUT"
if [ $# -gt 0 ]; then
    roster="$*"
else
    roster=$(python3 -c "import json;print(' '.join(sorted(json.load(open('build/verify/ssf2-data.json'))['character'])))")
fi
for c in $roster; do
    log="$LOGS/$c.log"
    PEPTIDE_BIN="$FROZEN/peptide" PEPTIDE_RUNTIME=1 python3 tools/verify.py char "$c" --ssf2 > "$log" 2>&1
    passed=$(grep -cE '^\s+C[1-6] \S+\s+PASS' "$log")
    first=$(grep -E '^\s+C[1-6] \S+\s+FAIL' "$log" | head -1 | cut -c1-200)
    echo "$c $passed/6 $first" | tee -a "$OUT"
done
echo "$(grep -c ' 6/6' "$OUT") of $(wc -l < "$OUT" | tr -d ' ') fighters pass all six gates" | tee -a "$OUT"
